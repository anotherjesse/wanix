use super::super::callback::{HostCallbackEntry, HostCallbackMode, QuickJsCopiedValue};
use super::super::module_loader::ModuleLoader;
use super::super::promise_rejection::{PromiseRejectionHandler, QuickJsPromiseRejection};
use super::HostState;
use crate::allocation::try_copy_str;
use anyhow::{Result, bail};
use std::panic::{AssertUnwindSafe, catch_unwind};

impl HostState {
    pub(crate) fn contains_host_callback(&self, name: &str) -> bool {
        self.host_callbacks.contains_key(name)
    }

    pub(crate) fn insert_host_callback(
        &mut self,
        name: String,
        callback: HostCallbackEntry,
    ) -> Result<()> {
        if self.host_callbacks.contains_key(&name) {
            bail!("host callback '{name}' is already registered");
        }
        self.host_callbacks.insert(name, callback);
        Ok(())
    }

    pub(crate) fn host_callback_mode(&self, name: &str) -> Result<HostCallbackMode> {
        self.host_callbacks
            .get(name)
            .map(HostCallbackEntry::mode)
            .ok_or_else(|| anyhow::anyhow!("host callback '{name}' is not registered"))
    }

    pub(crate) fn host_callback_depth(&self) -> usize {
        self.host_callback_depth
    }

    pub(crate) fn set_module_loader(&mut self, module_loader: ModuleLoader) {
        self.module_loader = Some(module_loader);
    }

    pub(crate) fn module_loader_depth(&self) -> usize {
        self.module_loader_depth
    }

    pub(crate) fn set_interrupt_handler(
        &mut self,
        interrupt_handler: Box<dyn FnMut() -> bool + Send + 'static>,
    ) {
        self.interrupt_handler = Some(interrupt_handler);
    }

    pub(crate) fn clear_interrupt_handler(&mut self) {
        self.interrupt_handler = None;
    }

    pub(crate) fn interrupt_handler_depth(&self) -> usize {
        self.interrupt_handler_depth
    }

    pub(crate) fn set_promise_rejection_handler(&mut self, handler: PromiseRejectionHandler) {
        self.promise_rejection_handler = Some(handler);
    }

    pub(crate) fn clear_promise_rejection_handler(&mut self) {
        self.promise_rejection_handler = None;
    }

    pub(crate) fn has_promise_rejection_handler(&self) -> bool {
        self.promise_rejection_handler.is_some()
    }

    pub(crate) fn promise_rejection_handler_depth(&self) -> usize {
        self.promise_rejection_handler_depth
    }

    pub(in crate::host) fn interrupt_requested(&mut self) -> bool {
        if self.interrupt_handler.is_none() {
            return false;
        }
        let Some(next_depth) = self.interrupt_handler_depth.checked_add(1) else {
            return true;
        };
        self.interrupt_handler_depth = next_depth;
        let result = {
            let _guard = InterruptHandlerDepthGuard {
                depth: &mut self.interrupt_handler_depth,
            };
            if let Some(interrupt_handler) = self.interrupt_handler.as_mut() {
                catch_unwind(AssertUnwindSafe(interrupt_handler))
            } else {
                return false;
            }
        };
        result.unwrap_or(true)
    }

    pub(in crate::host) fn handle_promise_rejection(&mut self, event: QuickJsPromiseRejection) {
        if self.promise_rejection_handler.is_none() {
            return;
        }
        let Some(next_depth) = self.promise_rejection_handler_depth.checked_add(1) else {
            return;
        };
        self.promise_rejection_handler_depth = next_depth;
        let _guard = PromiseRejectionHandlerDepthGuard {
            depth: &mut self.promise_rejection_handler_depth,
        };
        if let Some(handler) = self.promise_rejection_handler.as_mut() {
            let _ = catch_unwind(AssertUnwindSafe(|| handler(event)));
        }
    }

    pub(in crate::host) fn normalize_module(
        &mut self,
        base_name: &str,
        name: &str,
    ) -> Result<String> {
        self.with_module_loader("module normalizer", |module_loader| {
            match module_loader.normalize.as_mut() {
                Some(normalize) => normalize(base_name, name),
                None => try_copy_str(name, "module specifier"),
            }
        })
    }

    pub(in crate::host) fn load_module(&mut self, name: &str) -> Result<String> {
        self.with_module_loader("module loader", |module_loader| (module_loader.load)(name))
    }

    pub(in crate::host) fn call_host_callback(
        &mut self,
        name: &str,
        args: &[QuickJsCopiedValue],
    ) -> Result<QuickJsCopiedValue> {
        if !self.host_callbacks.contains_key(name) {
            bail!("host callback '{name}' is not registered");
        }
        self.host_callback_depth = self
            .host_callback_depth
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("host callback depth overflowed"))?;
        let result = {
            let _guard = HostCallbackDepthGuard {
                depth: &mut self.host_callback_depth,
            };
            if let Some(entry) = self.host_callbacks.get_mut(name) {
                catch_unwind(AssertUnwindSafe(|| (entry.callback_mut())(args)))
            } else {
                return Err(anyhow::anyhow!("host callback '{name}' is not registered"));
            }
        };
        match result {
            Ok(result) => result,
            Err(_payload) => Err(anyhow::anyhow!("host callback panicked")),
        }
    }

    fn with_module_loader<T>(
        &mut self,
        callback_label: &'static str,
        callback: impl FnOnce(&mut ModuleLoader) -> Result<T>,
    ) -> Result<T> {
        if self.module_loader.is_none() {
            bail!("module loader is not registered");
        }
        self.module_loader_depth = self
            .module_loader_depth
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("module loader depth overflowed"))?;
        let result = {
            let _guard = ModuleLoaderDepthGuard {
                depth: &mut self.module_loader_depth,
            };
            if let Some(module_loader) = self.module_loader.as_mut() {
                catch_unwind(AssertUnwindSafe(|| callback(module_loader)))
            } else {
                return Err(anyhow::anyhow!("module loader is not registered"));
            }
        };
        match result {
            Ok(result) => result,
            Err(_payload) => Err(anyhow::anyhow!("{callback_label} panicked")),
        }
    }
}

struct HostCallbackDepthGuard<'a> {
    depth: &'a mut usize,
}

impl Drop for HostCallbackDepthGuard<'_> {
    fn drop(&mut self) {
        *self.depth -= 1;
    }
}

struct ModuleLoaderDepthGuard<'a> {
    depth: &'a mut usize,
}

struct InterruptHandlerDepthGuard<'a> {
    depth: &'a mut usize,
}

struct PromiseRejectionHandlerDepthGuard<'a> {
    depth: &'a mut usize,
}

impl Drop for InterruptHandlerDepthGuard<'_> {
    fn drop(&mut self) {
        *self.depth -= 1;
    }
}

impl Drop for PromiseRejectionHandlerDepthGuard<'_> {
    fn drop(&mut self) {
        *self.depth -= 1;
    }
}

impl Drop for ModuleLoaderDepthGuard<'_> {
    fn drop(&mut self) {
        *self.depth -= 1;
    }
}
