use super::QuickJsRuntime;
use super::raw_value::{RawJsValue, validate_global_property_name};
use crate::{QuickJsBinaryValue, QuickJsCopiedValue, QuickJsValue};
use anyhow::{Error, Result, anyhow};

mod copied;
mod raw;

impl QuickJsRuntime {
    /// Evaluates JavaScript and returns a copied binary value.
    ///
    /// Supports `ArrayBuffer`, `Uint8Array`, typed-array, and `DataView`
    /// results. Bytes are copied out of QuickJS before the returned JavaScript
    /// value is freed.
    ///
    /// # Errors
    ///
    /// Returns an error if evaluation throws, the result is not a supported
    /// copied binary value, binary helper exports are unavailable, bytes cannot
    /// be copied from guest memory, or QuickJS/wasm cleanup fails.
    pub fn eval_binary_value(&mut self, code: &str) -> Result<QuickJsBinaryValue> {
        self.ensure_binary_value_capability()?;
        let value = self.eval_raw(code)?;
        self.raw_value_to_binary(value)
    }

    /// Reads a global JavaScript property as a copied binary value.
    ///
    /// # Errors
    ///
    /// Returns an error if `name` contains a NUL byte, the property lookup
    /// throws, the result is not a supported copied binary value, binary helper
    /// exports are unavailable, bytes cannot be copied from guest memory, or
    /// QuickJS/wasm cleanup fails.
    pub fn get_global_binary_value(&mut self, name: &str) -> Result<QuickJsBinaryValue> {
        validate_global_property_name(name)?;
        self.ensure_binary_value_capability()?;
        let value = self.global_prop_raw(name)?;
        self.raw_value_to_binary(value)
    }

    /// Sets a global JavaScript property from a copied binary value.
    ///
    /// # Errors
    ///
    /// Returns an error if `name` contains a NUL byte, the binary value cannot
    /// be copied into QuickJS, binary helper exports are unavailable, the global
    /// property update throws, or QuickJS/wasm cleanup fails.
    pub fn set_global_binary_value(&mut self, name: &str, value: QuickJsBinaryValue) -> Result<()> {
        validate_global_property_name(name)?;
        self.ensure_binary_value_capability()?;
        let value = self.binary_to_raw_value(&value)?;
        let set_global = self.set_global_prop_raw(name, &value);
        self.finish_with_raw_value_cleanup(set_global, value)
    }

    /// Calls a global JavaScript function with copied scalar arguments and
    /// returns a copied binary result.
    ///
    /// The function is read from `globalThis[name]` and called with
    /// `this = undefined`, matching [`Self::call_global_function`]. Arguments
    /// remain scalar-only; use a JavaScript wrapper when binary inputs are
    /// needed.
    ///
    /// # Errors
    ///
    /// Returns an error if `name` contains a NUL byte, the function or scalar
    /// arguments cannot be created, the call throws, the result is not a
    /// supported copied binary value, binary helper exports are unavailable, or
    /// QuickJS/wasm cleanup fails.
    pub fn call_global_function_binary(
        &mut self,
        name: &str,
        args: &[QuickJsValue],
    ) -> Result<QuickJsBinaryValue> {
        validate_global_property_name(name)?;
        self.ensure_binary_value_capability()?;
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
                let binary = self.raw_value_to_binary(result);
                self.finish_with_raw_values_cleanup(
                    binary,
                    [function, this_value].into_iter().chain(arg_values),
                )
            }
            Err(err) => self.finish_with_raw_values_cleanup(
                Err(err),
                [function, this_value].into_iter().chain(arg_values),
            ),
        }
    }

    /// Calls a global JavaScript function with copied scalar and binary
    /// arguments and returns a copied scalar or binary result.
    ///
    /// The function is read from `globalThis[name]` and called with
    /// `this = undefined`, matching [`Self::call_global_function`]. This is a
    /// copied value API: arguments are copied into QuickJS handles for the call
    /// and the result is copied back before all raw handles are freed.
    ///
    /// # Errors
    ///
    /// Returns an error if `name` contains a NUL byte, the function or any
    /// argument cannot be created, the call throws, the result is not a
    /// supported copied scalar or binary value, scalar or binary helper exports
    /// are unavailable, or QuickJS/wasm cleanup fails.
    pub fn call_global_function_with_values(
        &mut self,
        name: &str,
        args: &[QuickJsCopiedValue],
    ) -> Result<QuickJsCopiedValue> {
        validate_global_property_name(name)?;
        self.ensure_copied_value_capability()?;
        let function = self.global_prop_raw(name)?;
        let this_value = match self.undefined_raw() {
            Ok(value) => value,
            Err(err) => {
                return self.finish_with_raw_value_cleanup(Err(err), function);
            }
        };
        let arg_values = match self.copied_args_to_raw_values(args) {
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
                let copied = self.raw_value_to_copied(result);
                self.finish_with_raw_values_cleanup(
                    copied,
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

fn binary_value_unsupported_error(name: &str) -> Error {
    anyhow!("QuickJS WASM module does not export {name}; binary values are not supported")
}
