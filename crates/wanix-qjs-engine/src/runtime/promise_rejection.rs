use super::QuickJsRuntime;
use crate::QuickJsPromiseRejection;
use anyhow::{Result, anyhow};
use wasmtime::error::Context as _;

impl QuickJsRuntime {
    /// Installs or replaces the Rust-side QuickJS promise rejection handler.
    ///
    /// QuickJS calls the handler when a promise is rejected without a handler
    /// and again with [`QuickJsPromiseRejection::is_handled`] set when a handler
    /// is later attached. The event contains a copied reason string. Raw promise
    /// and reason handles stay private to the host import and are freed before
    /// control returns to QuickJS.
    ///
    /// The Rust closure is host state and is not serialized into snapshots;
    /// restored runtimes must install a handler again when they need rejection
    /// diagnostics. If the handler panics, Rust's panic hook still runs, but the
    /// host import catches the panic so unwinding does not cross the Wasm
    /// boundary.
    ///
    /// # Errors
    ///
    /// Returns an error if the wasm module does not export the promise rejection
    /// helper or if enabling the QuickJS tracker fails.
    pub fn set_promise_rejection_handler<F>(&mut self, handler: F) -> Result<()>
    where
        F: FnMut(QuickJsPromiseRejection) + Send + 'static,
    {
        let set_handler = self
            .qjs_set_promise_rejection_handler
            .clone()
            .ok_or_else(|| {
                promise_rejection_unsupported_error("qjs_set_promise_rejection_handler")
            })?;
        set_handler
            .call(&mut self.store, 1)
            .context("failed to enable QuickJS promise rejection handler")?;
        self.store
            .data_mut()
            .set_promise_rejection_handler(Box::new(handler));
        Ok(())
    }

    /// Clears the Rust-side promise rejection handler and disables C dispatch.
    ///
    /// # Errors
    ///
    /// Returns an error if the wasm module does not export the promise rejection
    /// helper or if disabling the QuickJS tracker fails.
    pub fn clear_promise_rejection_handler(&mut self) -> Result<()> {
        let set_handler = self
            .qjs_set_promise_rejection_handler
            .clone()
            .ok_or_else(|| {
                promise_rejection_unsupported_error("qjs_set_promise_rejection_handler")
            })?;
        set_handler
            .call(&mut self.store, 0)
            .context("failed to disable QuickJS promise rejection handler")?;
        self.store.data_mut().clear_promise_rejection_handler();
        Ok(())
    }
}

fn promise_rejection_unsupported_error(export_name: &str) -> anyhow::Error {
    anyhow!(
        "QuickJS WASM module does not export {export_name}; promise rejection tracking is not supported"
    )
}
