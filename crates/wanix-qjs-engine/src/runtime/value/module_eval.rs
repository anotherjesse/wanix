use super::QuickJsRuntime;
use crate::QuickJsPromiseRejection;
use crate::runtime::raw_value::JS_EVAL_TYPE_MODULE;
use anyhow::{Result, anyhow, bail};
use std::sync::{Arc, Mutex};

impl QuickJsRuntime {
    /// Evaluates JavaScript as an ES module and discards the resulting value.
    ///
    /// Module imports use the Rust-side loader installed with
    /// [`Self::set_module_loader`] or [`Self::set_module_loader_with_normalizer`].
    /// `filename` is the module name QuickJS uses for diagnostics and relative
    /// import resolution.
    ///
    /// Unlike a classic script, an ES module's top-level body evaluates as a
    /// promise, so a synchronous top-level `throw` (or `ReferenceError`) surfaces
    /// as an *unhandled promise rejection* rather than a thrown exception and is
    /// otherwise silently dropped. This installs a temporary rejection capture
    /// for the duration of evaluation and converts a captured unhandled rejection
    /// into an `Err`, so a module that throws is reported instead of exiting `0`.
    ///
    /// # Errors
    ///
    /// Returns an error if `filename` is empty or contains a NUL byte, if module
    /// parsing/evaluation throws (synchronously or as an unhandled top-level
    /// rejection), if an import cannot be normalized or loaded, or if the
    /// resulting QuickJS handle cannot be freed.
    pub fn eval_module_discard(&mut self, code: &str, filename: &str) -> Result<()> {
        validate_module_filename(filename)?;
        let captured = Arc::new(Mutex::new(None::<String>));
        let sink = Arc::clone(&captured);
        self.set_promise_rejection_handler(move |event: QuickJsPromiseRejection| {
            // QuickJS fires this callback twice per unhandled rejection (and
            // again with `is_handled` once a handler attaches). Keep only the
            // first genuinely-unhandled reason so the error reports exactly once.
            if event.is_handled() {
                return;
            }
            let Ok(mut slot) = sink.lock() else {
                return;
            };
            if slot.is_none() {
                *slot = Some(event.reason().to_string());
            }
        })?;
        // Evaluate, then always tear down the temporary handler. Surface a
        // synchronous parse/eval/proc_exit failure before any cleanup error.
        let eval = self.eval_raw_with_filename_and_flags(code, filename, JS_EVAL_TYPE_MODULE);
        let eval = eval.and_then(|value| self.free_value(value));
        let clear = self.clear_promise_rejection_handler();
        eval?;
        clear?;
        let reason = captured
            .lock()
            .map_err(|_| anyhow!("promise rejection capture lock poisoned"))?
            .take();
        if let Some(reason) = reason {
            bail!("QuickJS exception: {reason}");
        }
        Ok(())
    }
}

fn validate_module_filename(filename: &str) -> Result<()> {
    if filename.is_empty() {
        bail!("module filename must not be empty");
    }
    if filename.as_bytes().contains(&0) {
        bail!("module filename must not contain NUL bytes");
    }
    Ok(())
}
