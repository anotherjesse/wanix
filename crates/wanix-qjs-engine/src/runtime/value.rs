use super::QuickJsRuntime;
use super::raw_value::{RawJsValue, validate_global_property_name};
use crate::QuickJsValue;
use anyhow::{Error, Result, bail};
use wasmtime::error::Context as _;

mod module_eval;
mod scalar;

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
}
