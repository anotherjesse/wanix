use super::QuickJsRuntime;
use super::cleanup::CleanupScope;
use super::raw_value::RawJsValue;
use anyhow::{Error, Result, bail};
use wasmtime::error::Context as _;

impl QuickJsRuntime {
    pub(super) fn take_exception_string(&mut self) -> Result<String> {
        let exception_ptr = self
            .qjs_get_exception
            .call(&mut self.store, ())
            .context("failed to take QuickJS exception")?;
        let exception = RawJsValue::from_non_null(exception_ptr, "qjs_get_exception")?;
        self.with_owned_raw_value(exception, |runtime, exception| {
            let c_string = runtime
                .qjs_get_string
                .call(&mut runtime.store, exception.ptr())
                .context("failed to stringify QuickJS exception")?;
            if c_string == 0 {
                return Ok("<failed to stringify QuickJS exception>".to_string());
            }
            runtime.read_and_free_quickjs_c_string(c_string, "failed to free exception C string")
        })
    }

    pub(super) fn finish_raw_value<E>(
        &mut self,
        result: std::result::Result<i32, E>,
        source: &str,
        cleanup: CleanupScope,
    ) -> Result<RawJsValue>
    where
        E: Into<Error>,
    {
        let ptr = match result {
            Ok(ptr) => ptr,
            Err(err) => {
                return cleanup.finish(Err::<RawJsValue, Error>(err.into()));
            }
        };
        let value = match RawJsValue::from_non_null(ptr, source) {
            Ok(value) => value,
            Err(err) => return cleanup.finish(Err::<RawJsValue, Error>(err)),
        };
        let value = match self.throw_if_exception(value) {
            Ok(value) => value,
            Err(err) => return cleanup.finish(Err::<RawJsValue, Error>(err)),
        };
        if let Some(err) = cleanup.into_error() {
            return self.finish_with_raw_value_cleanup(Err(err), value);
        }
        Ok(value)
    }

    pub(super) fn throw_if_exception(&mut self, value: RawJsValue) -> Result<RawJsValue> {
        let is_exception = match self
            .qjs_is_exception
            .call(&mut self.store, value.ptr())
            .context("failed to inspect QuickJS value")
        {
            Ok(is_exception) => is_exception,
            Err(err) => {
                return self.finish_with_raw_value_cleanup(Err(err), value);
            }
        };
        if is_exception == 0 {
            return Ok(value);
        }

        let message = self.take_exception_string();
        let message = self.finish_with_raw_value_cleanup(message, value)?;
        bail!("QuickJS exception: {message}")
    }
}
