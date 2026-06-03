use super::QuickJsRuntime;
use super::cleanup::{CleanupScope, finish_with_cleanup};
use super::raw_value::{RawJsValue, validate_global_property_name};
use crate::allocation::try_copy_bytes;
use crate::guest::{guest_i32_add, guest_offset, host_len_i32};
use crate::{QuickJsBinaryValue, QuickJsCopiedValue, QuickJsTypedArrayKind, QuickJsValue};
use anyhow::{Error, Result, anyhow, bail};
use wasmtime::TypedFunc;
use wasmtime::error::Context as _;

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

    fn new_binary_raw(
        &mut self,
        bytes: &[u8],
        create: &TypedFunc<(i32, i32), i32>,
        source: &'static str,
    ) -> Result<RawJsValue> {
        let guest = self.write_guest_bytes(bytes, source)?;
        let guest_len = match host_len_i32(guest.len) {
            Some(len) => len,
            None => {
                let cleanup = self.guest_free(guest.ptr);
                return finish_with_cleanup(
                    Err(anyhow!(
                        "QuickJS binary value length exceeds supported host call range"
                    )),
                    cleanup.err(),
                );
            }
        };
        let result = create
            .call(&mut self.store, (guest.ptr, guest_len))
            .with_context(|| format!("failed to create QuickJS binary value with {source}"));
        let mut cleanup = CleanupScope::new();
        cleanup.record(self.guest_free(guest.ptr));
        self.finish_raw_value(result, source, cleanup)
    }

    fn read_array_buffer_bytes(&mut self, value: &RawJsValue) -> Result<Vec<u8>> {
        let qjs_get_array_buffer = self
            .qjs_get_array_buffer
            .clone()
            .ok_or_else(|| binary_value_unsupported_error("qjs_get_array_buffer"))?;
        self.read_binary_bytes_with_len_out(
            value,
            &qjs_get_array_buffer,
            "qjs_get_array_buffer",
            "QuickJS ArrayBuffer",
        )
    }

    fn read_uint8_array_bytes(&mut self, value: &RawJsValue) -> Result<Vec<u8>> {
        let qjs_get_uint8_array = self
            .qjs_get_uint8_array
            .clone()
            .ok_or_else(|| binary_value_unsupported_error("qjs_get_uint8_array"))?;
        self.read_binary_bytes_with_len_out(
            value,
            &qjs_get_uint8_array,
            "qjs_get_uint8_array",
            "QuickJS Uint8Array",
        )
    }

    fn read_binary_bytes_with_len_out(
        &mut self,
        value: &RawJsValue,
        get_bytes: &TypedFunc<(i32, i32), i32>,
        source: &'static str,
        label: &'static str,
    ) -> Result<Vec<u8>> {
        let len_out = self.guest_malloc(4)?;
        let data_ptr = get_bytes
            .call(&mut self.store, (value.ptr(), len_out))
            .with_context(|| format!("failed to get {label} data pointer with {source}"));

        let bytes = match data_ptr {
            Ok(data_ptr) => {
                if data_ptr == 0 {
                    match self.take_exception_string() {
                        Ok(message) => Err(anyhow!("{source} failed: {message}")),
                        Err(err) => Err(err),
                    }
                } else {
                    let len = self.read_guest_u32(len_out, &format!("{label} length"));
                    match len {
                        Ok(0) => try_copy_bytes(&[], label),
                        Ok(len) => self.read_guest_bytes(data_ptr, len, label),
                        Err(err) => Err(err),
                    }
                }
            }
            Err(err) => Err(err.into()),
        };

        let cleanup = self.guest_free(len_out);
        finish_with_cleanup(bytes, cleanup.err())
    }

    fn new_typed_binary_raw(
        &mut self,
        bytes: &[u8],
        kind: QuickJsTypedArrayKind,
        create: &TypedFunc<(i32, i32, i32), i32>,
        source: &'static str,
    ) -> Result<RawJsValue> {
        let guest = self.write_guest_bytes(bytes, source)?;
        let guest_len = match host_len_i32(guest.len) {
            Some(len) => len,
            None => {
                let cleanup = self.guest_free(guest.ptr);
                return finish_with_cleanup(
                    Err(anyhow!(
                        "QuickJS typed array byte length exceeds supported host call range"
                    )),
                    cleanup.err(),
                );
            }
        };
        let result = create
            .call(&mut self.store, (kind.abi(), guest.ptr, guest_len))
            .with_context(|| format!("failed to create QuickJS typed array with {source}"));
        let mut cleanup = CleanupScope::new();
        cleanup.record(self.guest_free(guest.ptr));
        self.finish_raw_value(result, source, cleanup)
    }

    fn read_typed_array_bytes(
        &mut self,
        value: &RawJsValue,
        kind: QuickJsTypedArrayKind,
    ) -> Result<Vec<u8>> {
        let qjs_get_typed_array_buffer = self
            .qjs_get_typed_array_buffer
            .clone()
            .ok_or_else(|| binary_value_unsupported_error("qjs_get_typed_array_buffer"))?;
        let meta_out = self.guest_malloc(12)?;

        let result = (|| {
            let byte_offset_out = meta_out;
            let byte_length_out = guest_i32_add(meta_out, 4)
                .ok_or_else(|| anyhow!("QuickJS typed array metadata pointer overflowed"))?;
            let bytes_per_element_out = guest_i32_add(meta_out, 8)
                .ok_or_else(|| anyhow!("QuickJS typed array metadata pointer overflowed"))?;
            let array_buffer_ptr = qjs_get_typed_array_buffer
                .call(
                    &mut self.store,
                    (
                        value.ptr(),
                        byte_offset_out,
                        byte_length_out,
                        bytes_per_element_out,
                    ),
                )
                .context("failed to get typed array backing ArrayBuffer")?;
            let array_buffer = self.finish_raw_value(
                Ok::<i32, Error>(array_buffer_ptr),
                "qjs_get_typed_array_buffer",
                CleanupScope::new(),
            )?;
            self.with_owned_raw_value(array_buffer, |runtime, array_buffer| {
                let byte_offset =
                    runtime.read_guest_u32(byte_offset_out, "QuickJS typed array byte offset")?;
                let byte_length =
                    runtime.read_guest_u32(byte_length_out, "QuickJS typed array byte length")?;
                let bytes_per_element = runtime.read_guest_u32(
                    bytes_per_element_out,
                    "QuickJS typed array bytes per element",
                )?;
                if usize::try_from(bytes_per_element).ok() != Some(kind.bytes_per_element()) {
                    bail!(
                        "QuickJS {} reported inconsistent element width {bytes_per_element}",
                        kind.js_name()
                    );
                }
                runtime.read_array_buffer_slice_bytes(
                    array_buffer,
                    byte_offset,
                    byte_length,
                    kind.js_name(),
                )
            })
        })();

        let cleanup = self.guest_free(meta_out);
        finish_with_cleanup(result, cleanup.err())
    }

    fn read_data_view_bytes(&mut self, value: &RawJsValue) -> Result<Vec<u8>> {
        let qjs_get_data_view_buffer = self
            .qjs_get_data_view_buffer
            .clone()
            .ok_or_else(|| binary_value_unsupported_error("qjs_get_data_view_buffer"))?;
        let meta_out = self.guest_malloc(8)?;

        let result = (|| {
            let byte_offset_out = meta_out;
            let byte_length_out = guest_i32_add(meta_out, 4)
                .ok_or_else(|| anyhow!("QuickJS DataView metadata pointer overflowed"))?;
            let array_buffer_ptr = qjs_get_data_view_buffer
                .call(
                    &mut self.store,
                    (value.ptr(), byte_offset_out, byte_length_out),
                )
                .context("failed to get DataView backing ArrayBuffer")?;
            let array_buffer = self.finish_raw_value(
                Ok::<i32, Error>(array_buffer_ptr),
                "qjs_get_data_view_buffer",
                CleanupScope::new(),
            )?;
            self.with_owned_raw_value(array_buffer, |runtime, array_buffer| {
                let byte_offset =
                    runtime.read_guest_u32(byte_offset_out, "QuickJS DataView byte offset")?;
                let byte_length =
                    runtime.read_guest_u32(byte_length_out, "QuickJS DataView byte length")?;
                runtime.read_array_buffer_slice_bytes(
                    array_buffer,
                    byte_offset,
                    byte_length,
                    "DataView",
                )
            })
        })();

        let cleanup = self.guest_free(meta_out);
        finish_with_cleanup(result, cleanup.err())
    }

    fn read_array_buffer_slice_bytes(
        &mut self,
        value: &RawJsValue,
        byte_offset: u32,
        byte_length: u32,
        label: &'static str,
    ) -> Result<Vec<u8>> {
        let qjs_get_array_buffer = self
            .qjs_get_array_buffer
            .clone()
            .ok_or_else(|| binary_value_unsupported_error("qjs_get_array_buffer"))?;
        let len_out = self.guest_malloc(4)?;

        let result = (|| match qjs_get_array_buffer
            .call(&mut self.store, (value.ptr(), len_out))
            .context("failed to get typed array backing ArrayBuffer data pointer")
        {
            Ok(data_ptr) => {
                if data_ptr == 0 {
                    match self.take_exception_string() {
                        Ok(message) => Err(anyhow!("qjs_get_array_buffer failed: {message}")),
                        Err(err) => Err(err),
                    }
                } else {
                    let buffer_len = self.read_guest_u32(len_out, "QuickJS ArrayBuffer length")?;
                    let end = byte_offset
                        .checked_add(byte_length)
                        .ok_or_else(|| anyhow!("{label} byte range overflowed"))?;
                    if end > buffer_len {
                        bail!(
                            "{label} byte range [{byte_offset}, {end}) exceeds backing ArrayBuffer length {buffer_len}"
                        );
                    }
                    let start = guest_offset(data_ptr)
                        .checked_add(
                            usize::try_from(byte_offset)
                                .context("typed array byte offset does not fit host usize")?,
                        )
                        .ok_or_else(|| anyhow!("{label} pointer offset overflowed"))?;
                    let len = usize::try_from(byte_length)
                        .context("typed array byte length does not fit host usize")?;
                    let end = start
                        .checked_add(len)
                        .ok_or_else(|| anyhow!("{label} pointer offset overflowed"))?;
                    let memory = self.memory.data(&self.store);
                    if end > memory.len() {
                        bail!(
                            "{label} range [{start}, {end}) is outside memory length {}",
                            memory.len()
                        );
                    }
                    try_copy_bytes(&memory[start..end], label)
                }
            }
            Err(err) => Err(err.into()),
        })();

        let cleanup = self.guest_free(len_out);
        finish_with_cleanup(result, cleanup.err())
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
