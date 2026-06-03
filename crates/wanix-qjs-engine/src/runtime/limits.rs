use super::QuickJsRuntime;
use super::cleanup::CleanupScope;
use crate::QuickJsMemoryUsage;
use crate::guest::{guest_i32, guest_u32};
use crate::memory::QUICKJS_MEMORY_USAGE_BYTE_LEN;
use anyhow::{Result, anyhow};
use wasmtime::TypedFunc;
use wasmtime::error::Context as _;

impl QuickJsRuntime {
    /// Sets the QuickJS heap allocation limit in bytes.
    ///
    /// A limit of `0` disables the QuickJS malloc limit. QuickJS stores this
    /// numeric limit in its runtime state, so a snapshot may carry the current
    /// value. Hosts that require a specific policy should set it explicitly
    /// after creating or restoring a runtime.
    ///
    /// # Errors
    ///
    /// Returns an error if the wasm module does not export the memory-limit
    /// helper or if calling it fails.
    pub fn set_memory_limit(&mut self, bytes: u32) -> Result<()> {
        call_limit_export(
            &mut self.store,
            self.qjs_set_memory_limit.clone(),
            "qjs_set_memory_limit",
            bytes,
        )
    }

    /// Disables the QuickJS heap allocation limit.
    ///
    /// # Errors
    ///
    /// Returns an error if the wasm module does not export the memory-limit
    /// helper or if calling it fails.
    pub fn clear_memory_limit(&mut self) -> Result<()> {
        self.set_memory_limit(0)
    }

    /// Explicitly runs QuickJS garbage collection.
    ///
    /// This can reclaim unreachable QuickJS heap objects before a known
    /// lifecycle point such as snapshotting. It does not shrink WebAssembly
    /// linear memory.
    ///
    /// # Errors
    ///
    /// Returns an error if the wasm module does not export the GC helper or if
    /// calling it fails.
    pub fn run_gc(&mut self) -> Result<()> {
        let run_gc = self
            .qjs_run_gc
            .clone()
            .ok_or_else(|| runtime_limit_unsupported_error("qjs_run_gc"))?;
        run_gc
            .call(&mut self.store, ())
            .context("failed to call qjs_run_gc")?;
        Ok(())
    }

    /// Sets the QuickJS automatic GC threshold in bytes.
    ///
    /// QuickJS stores this numeric threshold in runtime state, so snapshots may
    /// carry the current value. Hosts that require a specific policy should set
    /// it explicitly after creating or restoring a runtime. Use
    /// [`Self::disable_automatic_gc`] to disable automatic GC.
    ///
    /// # Errors
    ///
    /// Returns an error if the wasm module does not export the GC-threshold
    /// helper or if calling it fails.
    pub fn set_gc_threshold(&mut self, bytes: u32) -> Result<()> {
        call_limit_export(
            &mut self.store,
            self.qjs_set_gc_threshold.clone(),
            "qjs_set_gc_threshold",
            bytes,
        )
    }

    /// Disables QuickJS automatic GC.
    ///
    /// QuickJS uses `size_t::MAX` as its disabled threshold sentinel. On the
    /// wasm32 ABI, this is represented as `u32::MAX`.
    ///
    /// # Errors
    ///
    /// Returns an error if the wasm module does not export the GC-threshold
    /// helper or if calling it fails.
    pub fn disable_automatic_gc(&mut self) -> Result<()> {
        self.set_gc_threshold(u32::MAX)
    }

    /// Returns the current QuickJS automatic GC threshold in bytes.
    ///
    /// # Errors
    ///
    /// Returns an error if the wasm module does not export the GC-threshold
    /// helper or if calling it fails.
    pub fn gc_threshold(&mut self) -> Result<u32> {
        let get_gc_threshold = self
            .qjs_get_gc_threshold
            .clone()
            .ok_or_else(|| runtime_limit_unsupported_error("qjs_get_gc_threshold"))?;
        let threshold = get_gc_threshold
            .call(&mut self.store, ())
            .context("failed to call qjs_get_gc_threshold")?;
        Ok(guest_u32(threshold))
    }

    /// Returns copied QuickJS runtime memory usage counters.
    ///
    /// The counters describe QuickJS heap accounting. They are useful for
    /// diagnostics and policy decisions, but they do not imply that WebAssembly
    /// linear memory will shrink after garbage collection.
    ///
    /// # Errors
    ///
    /// Returns an error if the wasm module does not export the memory-usage
    /// helper, if the scratch buffer cannot be allocated or read, if calling
    /// the helper fails, or if cleanup fails.
    pub fn memory_usage(&mut self) -> Result<QuickJsMemoryUsage> {
        let compute_memory_usage = self
            .qjs_compute_memory_usage
            .clone()
            .ok_or_else(|| runtime_limit_unsupported_error("qjs_compute_memory_usage"))?;
        let ptr = self.guest_malloc(QUICKJS_MEMORY_USAGE_BYTE_LEN)?;

        let result = compute_memory_usage
            .call(&mut self.store, ptr)
            .context("failed to call qjs_compute_memory_usage");
        let mut cleanup = CleanupScope::new();
        if let Err(err) = result {
            cleanup.record(self.guest_free(ptr));
            return cleanup.finish(Err(err));
        }

        let bytes =
            self.read_guest_bytes(ptr, QUICKJS_MEMORY_USAGE_BYTE_LEN, "QuickJS memory usage");
        cleanup.record(self.guest_free(ptr));
        let bytes = match bytes {
            Ok(bytes) => bytes,
            Err(err) => return cleanup.finish(Err(err)),
        };
        cleanup.finish(QuickJsMemoryUsage::from_le_bytes(&bytes))
    }

    /// Sets the QuickJS native stack limit in bytes.
    ///
    /// A limit of `0` disables the QuickJS stack limit. QuickJS stores this
    /// numeric limit in its runtime state, so a snapshot may carry the current
    /// value. Hosts that require a specific policy should set it explicitly
    /// after creating or restoring a runtime.
    ///
    /// # Errors
    ///
    /// Returns an error if the wasm module does not export the stack-limit
    /// helper or if calling it fails.
    pub fn set_max_stack_size(&mut self, bytes: u32) -> Result<()> {
        call_limit_export(
            &mut self.store,
            self.qjs_set_max_stack_size.clone(),
            "qjs_set_max_stack_size",
            bytes,
        )
    }

    /// Disables the QuickJS native stack limit.
    ///
    /// # Errors
    ///
    /// Returns an error if the wasm module does not export the stack-limit
    /// helper or if calling it fails.
    pub fn clear_max_stack_size(&mut self) -> Result<()> {
        self.set_max_stack_size(0)
    }

    /// Installs or replaces the Rust-side QuickJS interrupt handler.
    ///
    /// QuickJS calls the handler periodically while running JavaScript. Return
    /// `true` to interrupt the current execution, which QuickJS reports as an
    /// exception. The Rust closure is host state and is not serialized into
    /// snapshots; restored runtimes must install a handler again when they need
    /// cancellation behavior.
    ///
    /// If the handler panics, Rust's panic hook still runs, but the host import
    /// treats the panic as an interrupt request so unwinding does not cross the
    /// Wasm boundary.
    ///
    /// # Errors
    ///
    /// Returns an error if the wasm module does not export the interrupt helper
    /// or if enabling the QuickJS interrupt handler fails.
    pub fn set_interrupt_handler<F>(&mut self, handler: F) -> Result<()>
    where
        F: FnMut() -> bool + Send + 'static,
    {
        let set_interrupt_handler = self
            .qjs_set_interrupt_handler
            .clone()
            .ok_or_else(|| runtime_limit_unsupported_error("qjs_set_interrupt_handler"))?;
        set_interrupt_handler
            .call(&mut self.store, 1)
            .context("failed to enable QuickJS interrupt handler")?;
        self.store
            .data_mut()
            .set_interrupt_handler(Box::new(handler));
        Ok(())
    }

    /// Clears the Rust-side QuickJS interrupt handler and disables C dispatch.
    ///
    /// # Errors
    ///
    /// Returns an error if the wasm module does not export the interrupt helper
    /// or if disabling the QuickJS interrupt handler fails.
    pub fn clear_interrupt_handler(&mut self) -> Result<()> {
        let set_interrupt_handler = self
            .qjs_set_interrupt_handler
            .clone()
            .ok_or_else(|| runtime_limit_unsupported_error("qjs_set_interrupt_handler"))?;
        set_interrupt_handler
            .call(&mut self.store, 0)
            .context("failed to disable QuickJS interrupt handler")?;
        self.store.data_mut().clear_interrupt_handler();
        Ok(())
    }
}

fn call_limit_export(
    store: &mut wasmtime::Store<crate::host::HostState>,
    export: Option<TypedFunc<i32, ()>>,
    export_name: &'static str,
    bytes: u32,
) -> Result<()> {
    let export = export.ok_or_else(|| runtime_limit_unsupported_error(export_name))?;
    export
        .call(store, guest_i32(bytes))
        .with_context(|| format!("failed to call {export_name}"))?;
    Ok(())
}

fn runtime_limit_unsupported_error(export_name: &str) -> anyhow::Error {
    anyhow!("QuickJS WASM module does not export {export_name}; runtime limits are not supported")
}
