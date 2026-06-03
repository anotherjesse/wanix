use super::QuickJsRuntime;
use super::cleanup::{CleanupScope, finish_with_cleanup};
use super::raw_value::{JS_EVAL_TYPE_MODULE, RawJsValue, validate_global_property_name};
use crate::QuickJsValue;
use crate::guest::{guest_i32_add, i64_from_guest_u32_halves, i64_to_guest_i32_halves};
use anyhow::{Error, Result, anyhow, bail};
use wasmtime::error::Context as _;

impl QuickJsRuntime {
    /// Evaluates JavaScript and returns a numeric result.
    ///
    /// # Errors
    ///
    /// Returns an error if evaluation throws, the result is not a number, or
    /// QuickJS/wasm cleanup fails.
    pub fn eval_number(&mut self, code: &str) -> Result<f64> {
        let value = self.eval_raw(code)?;
        self.with_owned_raw_value(value, |runtime, value| {
            let is_number = runtime
                .qjs_is_number
                .call(&mut runtime.store, value.ptr())
                .context("failed to inspect number result")?;
            if is_number == 0 {
                bail!("QuickJS evaluation result is not a number");
            }
            Ok(runtime
                .qjs_get_float64
                .call(&mut runtime.store, value.ptr())
                .context("failed to read number result")?)
        })
    }

    /// Evaluates JavaScript and returns a string result.
    ///
    /// # Errors
    ///
    /// Returns an error if evaluation throws, the result is not a string, the
    /// string cannot be copied from guest memory, or QuickJS/wasm cleanup fails.
    pub fn eval_string(&mut self, code: &str) -> Result<String> {
        let value = self.eval_raw(code)?;
        self.with_owned_raw_value(value, |runtime, value| {
            let is_string = runtime
                .qjs_is_string
                .call(&mut runtime.store, value.ptr())
                .context("failed to inspect string result")?;
            if is_string == 0 {
                bail!("QuickJS evaluation result is not a string");
            }
            let c_string = runtime
                .qjs_get_string
                .call(&mut runtime.store, value.ptr())
                .context("failed to convert result to string")?;
            if c_string == 0 {
                bail!("QuickJS failed to convert string result to a C string");
            }
            runtime.read_and_free_quickjs_c_string(c_string, "failed to free QuickJS C string")
        })
    }

    /// Evaluates JavaScript and returns a copied scalar value.
    ///
    /// This intentionally supports only `undefined`, `null`, booleans, numbers,
    /// strings, and exact signed 64-bit BigInts. Objects, arrays, functions,
    /// promises, symbols, and larger BigInts remain private QuickJS values
    /// until a future handle API is designed.
    ///
    /// # Errors
    ///
    /// Returns an error if evaluation throws, the result is not a supported
    /// copied scalar, scalar helper exports are unavailable, or QuickJS/wasm
    /// cleanup fails.
    pub fn eval_value(&mut self, code: &str) -> Result<QuickJsValue> {
        self.ensure_scalar_value_capability()?;
        let value = self.eval_raw(code)?;
        self.raw_value_to_scalar(value)
    }

    /// Evaluates JavaScript and discards the resulting value.
    ///
    /// # Errors
    ///
    /// Returns an error if evaluation throws or the resulting QuickJS handle
    /// cannot be freed.
    pub fn eval_discard(&mut self, code: &str) -> Result<()> {
        let value = self.eval_raw(code)?;
        self.free_value(value)
    }

    /// Evaluates JavaScript as an ES module and discards the resulting value.
    ///
    /// Module imports use the Rust-side loader installed with
    /// [`Self::set_module_loader`] or [`Self::set_module_loader_with_normalizer`].
    /// `filename` is the module name QuickJS uses for diagnostics and relative
    /// import resolution.
    ///
    /// # Errors
    ///
    /// Returns an error if `filename` is empty or contains a NUL byte, if
    /// module parsing/evaluation throws, if an import cannot be normalized or
    /// loaded, or if the resulting QuickJS handle cannot be freed.
    pub fn eval_module_discard(&mut self, code: &str, filename: &str) -> Result<()> {
        validate_module_filename(filename)?;
        let value = self.eval_raw_with_filename_and_flags(code, filename, JS_EVAL_TYPE_MODULE)?;
        self.free_value(value)
    }

    /// Reads a global JavaScript property as a copied scalar value.
    ///
    /// # Errors
    ///
    /// Returns an error if `name` contains a NUL byte, the property lookup
    /// throws, the value is not a supported copied scalar, scalar helper
    /// exports are unavailable, or QuickJS/wasm cleanup fails.
    pub fn get_global_value(&mut self, name: &str) -> Result<QuickJsValue> {
        validate_global_property_name(name)?;
        self.ensure_scalar_value_capability()?;
        let value = self.global_prop_raw(name)?;
        self.raw_value_to_scalar(value)
    }

    /// Sets a global JavaScript property from a copied scalar value.
    ///
    /// # Errors
    ///
    /// Returns an error if `name` contains a NUL byte or cannot be copied to
    /// the guest, the scalar value cannot be created, scalar helper exports are
    /// unavailable, the global property update throws, or QuickJS/wasm cleanup
    /// fails.
    pub fn set_global_value(&mut self, name: &str, value: QuickJsValue) -> Result<()> {
        validate_global_property_name(name)?;
        self.ensure_scalar_value_capability()?;
        let value = self.scalar_to_raw_value(value)?;
        let set_global = self.set_global_prop_raw(name, &value);
        self.finish_with_raw_value_cleanup(set_global, value)
    }

    /// Calls a global JavaScript function with one string argument.
    ///
    /// This helper is intentionally narrow while the public value API is still
    /// being shaped.
    ///
    /// # Errors
    ///
    /// Returns an error if the function or argument cannot be created, the call
    /// throws, or QuickJS/wasm cleanup fails.
    pub fn call_global_function_with_string(&mut self, name: &str, arg: &str) -> Result<()> {
        let function = self.global_prop_raw(name)?;
        let this_value = match self.undefined_raw() {
            Ok(value) => value,
            Err(err) => {
                return self.finish_with_raw_value_cleanup(Err(err), function);
            }
        };
        let arg_value = match self.new_string_raw(arg) {
            Ok(value) => value,
            Err(err) => {
                return self.finish_with_raw_values_cleanup(Err(err), [function, this_value]);
            }
        };

        match self.call_function_raw(&function, &this_value, &[&arg_value]) {
            Ok(result) => self.finish_with_raw_values_cleanup(
                Ok::<(), Error>(()),
                [function, this_value, arg_value, result],
            ),
            Err(err) => {
                self.finish_with_raw_values_cleanup(Err(err), [function, this_value, arg_value])
            }
        }
    }

    /// Calls a global JavaScript function with copied scalar arguments.
    ///
    /// The function is read from `globalThis[name]` and called with
    /// `this = undefined`, matching an extracted function call. Returns the
    /// function result as a copied scalar. This is intentionally a scalar-only
    /// API while public raw QuickJS handles remain private.
    ///
    /// # Errors
    ///
    /// Returns an error if `name` contains a NUL byte, the function or any
    /// argument cannot be created, the call throws, the result is not a
    /// supported copied scalar, scalar helper exports are unavailable, or
    /// QuickJS/wasm cleanup fails.
    pub fn call_global_function(
        &mut self,
        name: &str,
        args: &[QuickJsValue],
    ) -> Result<QuickJsValue> {
        validate_global_property_name(name)?;
        self.ensure_scalar_value_capability()?;
        let function = self.global_prop_raw(name)?;
        let this_value = match self.undefined_raw() {
            Ok(value) => value,
            Err(err) => {
                return self.finish_with_raw_value_cleanup(Err(err), function);
            }
        };
        let arg_values = match self.scalar_args_to_raw_values(args) {
            Ok(values) => values,
            Err(err) => {
                return self.finish_with_raw_values_cleanup(Err(err), [function, this_value]);
            }
        };

        let call_result = {
            let arg_refs: Vec<&RawJsValue> = arg_values.iter().collect();
            self.call_function_raw(&function, &this_value, &arg_refs)
        };

        match call_result {
            Ok(result) => {
                let scalar = self.raw_value_to_scalar(result);
                self.finish_with_raw_values_cleanup(
                    scalar,
                    [function, this_value].into_iter().chain(arg_values),
                )
            }
            Err(err) => self.finish_with_raw_values_cleanup(
                Err(err),
                [function, this_value].into_iter().chain(arg_values),
            ),
        }
    }

    /// Executes pending QuickJS jobs until the job queue is empty.
    ///
    /// This method is unbounded. Use [`Self::execute_pending_jobs_with_limit`]
    /// when JavaScript may schedule more jobs while the queue is being drained.
    ///
    /// # Errors
    ///
    /// Returns an error if QuickJS reports a failing job or the job queue cannot
    /// be inspected/executed.
    pub fn execute_pending_jobs(&mut self) -> Result<usize> {
        self.execute_pending_jobs_inner(None)
    }

    /// Executes up to `max_jobs` pending QuickJS jobs.
    ///
    /// Returns an error if the queue is still non-empty after `max_jobs` jobs
    /// have been executed. A limit of zero checks the queue but does not execute
    /// any jobs.
    ///
    /// # Errors
    ///
    /// Returns an error if QuickJS reports a failing job, the job queue cannot
    /// be inspected/executed, or the configured job limit is reached before the
    /// queue becomes empty.
    pub fn execute_pending_jobs_with_limit(&mut self, max_jobs: usize) -> Result<usize> {
        self.execute_pending_jobs_inner(Some(max_jobs))
    }

    fn execute_pending_jobs_inner(&mut self, max_jobs: Option<usize>) -> Result<usize> {
        let mut executed = 0;
        while self
            .qjs_is_job_pending
            .call(&mut self.store, ())
            .context("failed to check QuickJS job queue")?
            != 0
        {
            if let Some(max_jobs) = max_jobs
                && executed >= max_jobs
            {
                bail!("QuickJS pending job limit reached after {executed} jobs");
            }
            let result = self
                .qjs_execute_pending_job
                .call(&mut self.store, ())
                .context("failed to execute QuickJS pending job")?;
            if result < 0 {
                bail!(
                    "QuickJS pending job failed: {}",
                    self.take_exception_string()?
                );
            }
            executed = executed
                .checked_add(1)
                .ok_or_else(|| anyhow::anyhow!("QuickJS pending job count overflowed"))?;
        }
        Ok(executed)
    }
}

impl QuickJsRuntime {
    pub(super) fn raw_value_to_scalar(&mut self, value: RawJsValue) -> Result<QuickJsValue> {
        self.with_owned_raw_value(value, |runtime, value| {
            runtime.raw_value_to_scalar_ref(value)
        })
    }

    pub(super) fn raw_value_to_scalar_ref(&mut self, value: &RawJsValue) -> Result<QuickJsValue> {
        if let Some(value) = self.maybe_raw_value_to_scalar_ref(value)? {
            return Ok(value);
        }
        bail!("QuickJS value is not a supported copied scalar");
    }

    pub(super) fn maybe_raw_value_to_scalar_ref(
        &mut self,
        value: &RawJsValue,
    ) -> Result<Option<QuickJsValue>> {
        self.ensure_scalar_value_capability()?;

        let qjs_is_undefined = self
            .qjs_is_undefined
            .clone()
            .ok_or_else(|| scalar_value_unsupported_error("qjs_is_undefined"))?;
        if qjs_is_undefined
            .call(&mut self.store, value.ptr())
            .context("failed to inspect undefined result")?
            != 0
        {
            return Ok(Some(QuickJsValue::Undefined));
        }

        let qjs_is_null = self
            .qjs_is_null
            .clone()
            .ok_or_else(|| scalar_value_unsupported_error("qjs_is_null"))?;
        if qjs_is_null
            .call(&mut self.store, value.ptr())
            .context("failed to inspect null result")?
            != 0
        {
            return Ok(Some(QuickJsValue::Null));
        }

        let qjs_is_bool = self
            .qjs_is_bool
            .clone()
            .ok_or_else(|| scalar_value_unsupported_error("qjs_is_bool"))?;
        if qjs_is_bool
            .call(&mut self.store, value.ptr())
            .context("failed to inspect bool result")?
            != 0
        {
            let qjs_get_bool = self
                .qjs_get_bool
                .clone()
                .ok_or_else(|| scalar_value_unsupported_error("qjs_get_bool"))?;
            return Ok(Some(QuickJsValue::Bool(
                qjs_get_bool
                    .call(&mut self.store, value.ptr())
                    .context("failed to read bool result")?
                    != 0,
            )));
        }

        let is_number = self
            .qjs_is_number
            .call(&mut self.store, value.ptr())
            .context("failed to inspect number result")?;
        if is_number != 0 {
            return Ok(Some(QuickJsValue::Number(
                self.qjs_get_float64
                    .call(&mut self.store, value.ptr())
                    .context("failed to read number result")?,
            )));
        }

        let is_string = self
            .qjs_is_string
            .call(&mut self.store, value.ptr())
            .context("failed to inspect string result")?;
        if is_string != 0 {
            let c_string = self
                .qjs_get_string
                .call(&mut self.store, value.ptr())
                .context("failed to convert result to string")?;
            if c_string == 0 {
                bail!("QuickJS failed to convert string result to a C string");
            }
            return self
                .read_and_free_quickjs_c_string(c_string, "failed to free QuickJS C string")
                .map(QuickJsValue::String)
                .map(Some);
        }

        if let Some(qjs_is_big_int) = self.qjs_is_big_int.clone()
            && qjs_is_big_int
                .call(&mut self.store, value.ptr())
                .context("failed to inspect BigInt result")?
                != 0
        {
            return self
                .read_big_int64_value(value)
                .map(QuickJsValue::BigIntI64)
                .map(Some);
        }

        Ok(None)
    }

    pub(super) fn scalar_to_raw_value(&mut self, value: QuickJsValue) -> Result<RawJsValue> {
        match value {
            QuickJsValue::Undefined => self.undefined_raw(),
            QuickJsValue::Null => self.null_raw(),
            QuickJsValue::Bool(value) => self.bool_raw(value),
            QuickJsValue::Number(value) => self.number_raw(value),
            QuickJsValue::String(value) => self.new_string_raw(&value),
            QuickJsValue::BigIntI64(value) => self.big_int64_raw(value),
        }
    }

    pub(super) fn scalar_args_to_raw_values(
        &mut self,
        args: &[QuickJsValue],
    ) -> Result<Vec<RawJsValue>> {
        let mut raw_args = Vec::new();
        raw_args
            .try_reserve_exact(args.len())
            .map_err(|err| anyhow!("QuickJS scalar argument allocation failed: {err}"))?;
        for arg in args {
            match self.scalar_to_raw_value(arg.clone()) {
                Ok(value) => raw_args.push(value),
                Err(err) => {
                    let cleanup = self.free_values(raw_args);
                    return finish_with_cleanup(Err(err), cleanup.err());
                }
            }
        }
        Ok(raw_args)
    }

    pub(super) fn ensure_scalar_value_capability(&self) -> Result<()> {
        ensure_scalar_optional_export(self.qjs_is_undefined.is_some(), "qjs_is_undefined")?;
        ensure_scalar_optional_export(self.qjs_get_null.is_some(), "qjs_get_null")?;
        ensure_scalar_optional_export(self.qjs_get_true.is_some(), "qjs_get_true")?;
        ensure_scalar_optional_export(self.qjs_get_false.is_some(), "qjs_get_false")?;
        ensure_scalar_optional_export(self.qjs_is_null.is_some(), "qjs_is_null")?;
        ensure_scalar_optional_export(self.qjs_is_bool.is_some(), "qjs_is_bool")?;
        ensure_scalar_optional_export(self.qjs_get_bool.is_some(), "qjs_get_bool")?;
        ensure_scalar_optional_export(self.qjs_new_number.is_some(), "qjs_new_number")
    }

    fn read_big_int64_value(&mut self, value: &RawJsValue) -> Result<i64> {
        let qjs_get_big_int64 = self
            .qjs_get_big_int64
            .clone()
            .ok_or_else(|| scalar_value_unsupported_error("qjs_get_big_int64"))?;
        let out = self.guest_malloc(8)?;

        let result = (|| {
            let hi_out = guest_i32_add(out, 4)
                .ok_or_else(|| anyhow!("QuickJS BigInt metadata pointer overflowed"))?;
            let ret = qjs_get_big_int64
                .call(&mut self.store, (value.ptr(), out, hi_out))
                .context("failed to read BigInt result")?;
            if ret != 0 {
                match self.take_exception_string() {
                    Ok(message) => bail!("qjs_get_big_int64 failed: {message}"),
                    Err(err) => return Err(err),
                }
            }

            let lo = self.read_guest_u32(out, "QuickJS BigInt low word")?;
            let hi = self.read_guest_u32(hi_out, "QuickJS BigInt high word")?;
            Ok(i64_from_guest_u32_halves(lo, hi))
        })();

        let cleanup = self.guest_free(out);
        finish_with_cleanup(result, cleanup.err())
    }

    fn big_int64_raw(&mut self, value: i64) -> Result<RawJsValue> {
        let qjs_new_big_int64 = self
            .qjs_new_big_int64
            .clone()
            .ok_or_else(|| scalar_value_unsupported_error("qjs_new_big_int64"))?;
        let (lo, hi) = i64_to_guest_i32_halves(value);
        let result = qjs_new_big_int64
            .call(&mut self.store, (lo, hi))
            .context("failed to create BigInt handle");
        self.finish_raw_value(result, "qjs_new_big_int64", CleanupScope::new())
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

fn ensure_scalar_optional_export(has_export: bool, name: &str) -> Result<()> {
    if !has_export {
        return Err(scalar_value_unsupported_error(name));
    }
    Ok(())
}

fn scalar_value_unsupported_error(name: &str) -> anyhow::Error {
    anyhow!("QuickJS WASM module does not export {name}; scalar values are not supported")
}
