use super::QuickJsRuntime;
use crate::allocation::try_copy_bytes;
use crate::guest::{guest_i32, guest_u32};
use crate::snapshot::{
    QUICKJS_WASM_ABI_VERSION, SNAPSHOT_FORMAT_VERSION, snapshot_memory_page_count,
};
use crate::{QuickJsHostConfig, QuickJsModule, Snapshot};
use anyhow::{Result, bail};
use wasmtime::Engine;
use wasmtime::Val;
use wasmtime::error::Context as _;

impl QuickJsRuntime {
    /// Restores a runtime from a snapshot with the default host configuration.
    ///
    /// Prefer [`QuickJsModule::restore_runtime`](crate::QuickJsModule::restore_runtime)
    /// when you already have a module.
    ///
    /// # Errors
    ///
    /// Returns an error if `engine` did not compile `module`, the snapshot is
    /// not compatible with `module`, the wasm module cannot be instantiated,
    /// memory cannot be grown, or the saved QuickJS pointers cannot be
    /// reattached and verified.
    pub fn restore(engine: &Engine, module: &QuickJsModule, snapshot: &Snapshot) -> Result<Self> {
        Self::restore_with_host_config(engine, module, snapshot, QuickJsHostConfig::default())
    }

    /// Restores a runtime from a snapshot with explicit host import settings.
    ///
    /// Prefer
    /// [`QuickJsModule::restore_runtime_with_host_config`](crate::QuickJsModule::restore_runtime_with_host_config)
    /// when you already have a module.
    ///
    /// The host config is not serialized inside the snapshot. Passing it here
    /// intentionally reattaches host behavior to the resumed runtime.
    ///
    /// # Errors
    ///
    /// Returns an error if `engine` did not compile `module`, the snapshot is
    /// not compatible with `module`, the wasm module cannot be instantiated,
    /// memory cannot be grown, or the saved QuickJS pointers cannot be
    /// reattached and verified.
    pub fn restore_with_host_config(
        engine: &Engine,
        module: &QuickJsModule,
        snapshot: &Snapshot,
        config: QuickJsHostConfig,
    ) -> Result<Self> {
        module.ensure_engine(engine)?;
        module.restore_runtime_with_host_config(snapshot, config)
    }

    pub(crate) fn restore_for_module(
        module: &QuickJsModule,
        snapshot: &Snapshot,
        config: QuickJsHostConfig,
    ) -> Result<Self> {
        snapshot.validate_for(module)?;

        let mut vm = Self::instantiate(module.engine(), module, config, None)?.vm;

        let needed_pages = snapshot_memory_page_count(snapshot.memory.len())?;
        let current_pages = vm.memory.size(&vm.store);
        if needed_pages > current_pages {
            vm.memory
                .grow(&mut vm.store, needed_pages - current_pages)
                .context("failed to grow memory for snapshot restore")?;
        }

        vm.memory
            .write(&mut vm.store, 0, &snapshot.memory)
            .context("failed to copy snapshot memory into restored instance")?;

        vm.qjs_set_runtime_and_context
            .call(
                &mut vm.store,
                (
                    guest_i32(snapshot.runtime_ptr),
                    guest_i32(snapshot.context_ptr),
                ),
            )
            .context("failed to restore QuickJS runtime/context pointers")?;

        vm.verify_restored_runtime_context_matches_snapshot(snapshot)?;

        vm.stack_pointer
            .set(&mut vm.store, Val::I32(guest_i32(snapshot.stack_pointer)))
            .context("failed to restore __stack_pointer")?;

        vm.verify_restored_stack_pointer_matches_snapshot(snapshot)?;

        Ok(vm)
    }

    /// Captures the runtime as a linear-memory snapshot.
    ///
    /// # Errors
    ///
    /// Returns an error if the runtime metadata exported by QuickJS cannot be
    /// read, the linear memory copy cannot be allocated, or the metadata is
    /// structurally invalid.
    pub fn snapshot(&mut self) -> Result<Snapshot> {
        if self.store.data().host_callback_depth() != 0 {
            bail!("cannot snapshot while a host callback is active");
        }
        if self.store.data().module_loader_depth() != 0 {
            bail!("cannot snapshot while a module loader callback is active");
        }
        if self.store.data().interrupt_handler_depth() != 0 {
            bail!("cannot snapshot while an interrupt handler is active");
        }
        if self.store.data().promise_rejection_handler_depth() != 0 {
            bail!("cannot snapshot while a promise rejection handler is active");
        }
        if self.store.data().open_virtual_file_count() != 0 {
            bail!("cannot snapshot while a virtual file descriptor is open");
        }

        let stack_pointer = self.read_stack_pointer()?;
        let runtime_ptr = self.read_runtime_ptr()?;
        let context_ptr = self.read_context_ptr()?;

        let memory = try_copy_bytes(self.memory.data(&self.store), "runtime snapshot memory")?;
        let snapshot = Snapshot {
            format_version: SNAPSHOT_FORMAT_VERSION,
            abi_version: QUICKJS_WASM_ABI_VERSION,
            wasm_sha256: self.wasm_sha256,
            memory,
            stack_pointer,
            runtime_ptr,
            context_ptr,
        };
        snapshot
            .validate_structure()
            .map_err(|err| err.context("captured snapshot metadata is invalid"))?;
        Ok(snapshot)
    }

    fn verify_restored_runtime_context_matches_snapshot(
        &mut self,
        snapshot: &Snapshot,
    ) -> Result<()> {
        let runtime_ptr = self
            .read_runtime_ptr()
            .map_err(|err| err.context("failed to verify restored JSRuntime pointer"))?;
        if runtime_ptr != snapshot.runtime_ptr {
            bail!(
                "restored JSRuntime pointer {runtime_ptr} does not match snapshot runtime_ptr {}",
                snapshot.runtime_ptr
            );
        }

        let context_ptr = self
            .read_context_ptr()
            .map_err(|err| err.context("failed to verify restored JSContext pointer"))?;
        if context_ptr != snapshot.context_ptr {
            bail!(
                "restored JSContext pointer {context_ptr} does not match snapshot context_ptr {}",
                snapshot.context_ptr
            );
        }

        Ok(())
    }

    fn verify_restored_stack_pointer_matches_snapshot(
        &mut self,
        snapshot: &Snapshot,
    ) -> Result<()> {
        let stack_pointer = self.read_stack_pointer()?;
        if stack_pointer != snapshot.stack_pointer {
            bail!(
                "restored __stack_pointer {stack_pointer} does not match snapshot stack_pointer {}",
                snapshot.stack_pointer
            );
        }

        Ok(())
    }

    fn read_stack_pointer(&mut self) -> Result<u32> {
        match self.stack_pointer.get(&mut self.store) {
            Val::I32(value) => Ok(guest_u32(value)),
            other => bail!("unexpected __stack_pointer type: {other:?}"),
        }
    }

    fn read_runtime_ptr(&mut self) -> Result<u32> {
        Ok(guest_u32(
            self.qjs_get_runtime_ptr
                .call(&mut self.store, ())
                .context("failed to read JSRuntime pointer")?,
        ))
    }

    fn read_context_ptr(&mut self) -> Result<u32> {
        Ok(guest_u32(
            self.qjs_get_context_ptr
                .call(&mut self.store, ())
                .context("failed to read JSContext pointer")?,
        ))
    }
}
