use super::QuickJsRuntime;
use super::cleanup::finish_with_cleanup;
use super::raw_value::{RawJsValue, validate_global_property_name};
use crate::{QuickJsBinaryValue, QuickJsCopiedValue, QuickJsTypedArrayKind, QuickJsValue};
use anyhow::{Error, Result, anyhow, bail};
use wasmtime::error::Context as _;

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

    pub(super) fn raw_value_to_binary(&mut self, value: RawJsValue) -> Result<QuickJsBinaryValue> {
        self.with_owned_raw_value(value, |runtime, value| {
            runtime.raw_value_to_binary_ref(value)
        })
    }

    fn raw_value_to_binary_ref(&mut self, value: &RawJsValue) -> Result<QuickJsBinaryValue> {
        if let Some(value) = self.try_raw_value_to_binary_ref(value)? {
            return Ok(value);
        }
        bail!("QuickJS value is not a supported copied binary value");
    }

    fn try_raw_value_to_binary_ref(
        &mut self,
        value: &RawJsValue,
    ) -> Result<Option<QuickJsBinaryValue>> {
        self.ensure_binary_value_capability()?;

        let qjs_is_array_buffer = self
            .qjs_is_array_buffer
            .clone()
            .ok_or_else(|| binary_value_unsupported_error("qjs_is_array_buffer"))?;
        if qjs_is_array_buffer
            .call(&mut self.store, value.ptr())
            .context("failed to inspect ArrayBuffer result")?
            != 0
        {
            return self
                .read_array_buffer_bytes(value)
                .map(QuickJsBinaryValue::ArrayBuffer)
                .map(Some);
        }

        let qjs_is_uint8_array = self
            .qjs_is_uint8_array
            .clone()
            .ok_or_else(|| binary_value_unsupported_error("qjs_is_uint8_array"))?;
        if qjs_is_uint8_array
            .call(&mut self.store, value.ptr())
            .context("failed to inspect Uint8Array result")?
            != 0
        {
            return self
                .read_uint8_array_bytes(value)
                .map(QuickJsBinaryValue::Uint8Array)
                .map(Some);
        }

        if let Some(qjs_get_typed_array_type) = self.qjs_get_typed_array_type.clone() {
            let kind_abi = qjs_get_typed_array_type
                .call(&mut self.store, value.ptr())
                .context("failed to inspect typed array result")?;
            if let Some(kind) = QuickJsTypedArrayKind::from_abi(kind_abi) {
                let bytes = self.read_typed_array_bytes(value, kind)?;
                return QuickJsBinaryValue::typed_array_from_bytes(kind, bytes).map(Some);
            }
        }

        let Some(qjs_is_data_view) = self.qjs_is_data_view.clone() else {
            return Ok(None);
        };
        if qjs_is_data_view
            .call(&mut self.store, value.ptr())
            .context("failed to inspect DataView result")?
            != 0
        {
            return self
                .read_data_view_bytes(value)
                .map(QuickJsBinaryValue::DataView)
                .map(Some);
        }

        Ok(None)
    }

    pub(super) fn binary_to_raw_value(&mut self, value: &QuickJsBinaryValue) -> Result<RawJsValue> {
        match value {
            QuickJsBinaryValue::ArrayBuffer(bytes) => self.new_array_buffer_raw(bytes),
            QuickJsBinaryValue::Uint8Array(bytes) => self.new_uint8_array_raw(bytes),
            QuickJsBinaryValue::TypedArray { kind, bytes } => {
                self.new_typed_array_raw(*kind, bytes)
            }
            QuickJsBinaryValue::DataView(bytes) => self.new_data_view_raw(bytes),
        }
    }

    fn raw_value_to_copied(&mut self, value: RawJsValue) -> Result<QuickJsCopiedValue> {
        self.with_owned_raw_value(value, |runtime, value| {
            runtime.raw_value_to_copied_ref(value)
        })
    }

    fn raw_value_to_copied_ref(&mut self, value: &RawJsValue) -> Result<QuickJsCopiedValue> {
        self.ensure_copied_value_capability()?;
        if let Some(value) = self.try_raw_value_to_binary_ref(value)? {
            return Ok(QuickJsCopiedValue::Binary(value));
        }
        if let Some(value) = self.maybe_raw_value_to_scalar_ref(value)? {
            return Ok(QuickJsCopiedValue::Scalar(value));
        }
        bail!("QuickJS value is not a supported copied scalar or binary value");
    }

    fn copied_args_to_raw_values(
        &mut self,
        args: &[QuickJsCopiedValue],
    ) -> Result<Vec<RawJsValue>> {
        let mut raw_args = Vec::new();
        raw_args
            .try_reserve_exact(args.len())
            .map_err(|err| anyhow!("QuickJS copied argument allocation failed: {err}"))?;
        for arg in args {
            let raw_arg = match arg {
                QuickJsCopiedValue::Scalar(value) => self.scalar_to_raw_value(value.clone()),
                QuickJsCopiedValue::Binary(value) => self.binary_to_raw_value(value),
            };
            match raw_arg {
                Ok(value) => raw_args.push(value),
                Err(err) => {
                    let cleanup = self.free_values(raw_args);
                    return finish_with_cleanup(Err(err), cleanup.err());
                }
            }
        }
        Ok(raw_args)
    }

    fn new_array_buffer_raw(&mut self, bytes: &[u8]) -> Result<RawJsValue> {
        let qjs_new_array_buffer = self
            .qjs_new_array_buffer
            .clone()
            .ok_or_else(|| binary_value_unsupported_error("qjs_new_array_buffer"))?;
        self.new_binary_raw(bytes, &qjs_new_array_buffer, "qjs_new_array_buffer")
    }

    fn new_uint8_array_raw(&mut self, bytes: &[u8]) -> Result<RawJsValue> {
        let qjs_new_uint8_array = self
            .qjs_new_uint8_array
            .clone()
            .ok_or_else(|| binary_value_unsupported_error("qjs_new_uint8_array"))?;
        self.new_binary_raw(bytes, &qjs_new_uint8_array, "qjs_new_uint8_array")
    }

    fn new_typed_array_raw(
        &mut self,
        kind: QuickJsTypedArrayKind,
        bytes: &[u8],
    ) -> Result<RawJsValue> {
        kind.validate_byte_len(bytes.len())?;
        if kind == QuickJsTypedArrayKind::Uint8 {
            return self.new_uint8_array_raw(bytes);
        }
        let qjs_new_typed_array = self
            .qjs_new_typed_array
            .clone()
            .ok_or_else(|| binary_value_unsupported_error("qjs_new_typed_array"))?;
        self.new_typed_binary_raw(bytes, kind, &qjs_new_typed_array, "qjs_new_typed_array")
    }

    fn new_data_view_raw(&mut self, bytes: &[u8]) -> Result<RawJsValue> {
        let qjs_new_data_view = self
            .qjs_new_data_view
            .clone()
            .ok_or_else(|| binary_value_unsupported_error("qjs_new_data_view"))?;
        self.new_binary_raw(bytes, &qjs_new_data_view, "qjs_new_data_view")
    }

    pub(super) fn ensure_binary_value_capability(&self) -> Result<()> {
        ensure_binary_optional_export(self.qjs_new_array_buffer.is_some(), "qjs_new_array_buffer")?;
        ensure_binary_optional_export(self.qjs_new_uint8_array.is_some(), "qjs_new_uint8_array")?;
        ensure_binary_optional_export(self.qjs_is_array_buffer.is_some(), "qjs_is_array_buffer")?;
        ensure_binary_optional_export(self.qjs_is_uint8_array.is_some(), "qjs_is_uint8_array")?;
        ensure_binary_optional_export(self.qjs_get_array_buffer.is_some(), "qjs_get_array_buffer")?;
        ensure_binary_optional_export(self.qjs_get_uint8_array.is_some(), "qjs_get_uint8_array")
    }

    fn ensure_copied_value_capability(&self) -> Result<()> {
        self.ensure_binary_value_capability()?;
        self.ensure_scalar_value_capability()
    }
}

fn ensure_binary_optional_export(has_export: bool, name: &str) -> Result<()> {
    if !has_export {
        return Err(binary_value_unsupported_error(name));
    }
    Ok(())
}

fn binary_value_unsupported_error(name: &str) -> Error {
    anyhow!("QuickJS WASM module does not export {name}; binary values are not supported")
}
