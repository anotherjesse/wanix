use super::{QuickJsRuntime, binary_value_unsupported_error};
use crate::runtime::cleanup::finish_with_cleanup;
use crate::runtime::raw_value::RawJsValue;
use crate::{QuickJsBinaryValue, QuickJsCopiedValue, QuickJsTypedArrayKind};
use anyhow::{Result, anyhow, bail};
use wasmtime::error::Context as _;

impl QuickJsRuntime {
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

    pub(super) fn raw_value_to_copied(&mut self, value: RawJsValue) -> Result<QuickJsCopiedValue> {
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

    pub(super) fn copied_args_to_raw_values(
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

    pub(in crate::runtime) fn ensure_binary_value_capability(&self) -> Result<()> {
        ensure_binary_optional_export(self.qjs_new_array_buffer.is_some(), "qjs_new_array_buffer")?;
        ensure_binary_optional_export(self.qjs_new_uint8_array.is_some(), "qjs_new_uint8_array")?;
        ensure_binary_optional_export(self.qjs_is_array_buffer.is_some(), "qjs_is_array_buffer")?;
        ensure_binary_optional_export(self.qjs_is_uint8_array.is_some(), "qjs_is_uint8_array")?;
        ensure_binary_optional_export(self.qjs_get_array_buffer.is_some(), "qjs_get_array_buffer")?;
        ensure_binary_optional_export(self.qjs_get_uint8_array.is_some(), "qjs_get_uint8_array")
    }

    pub(in crate::runtime) fn ensure_copied_value_capability(&self) -> Result<()> {
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
