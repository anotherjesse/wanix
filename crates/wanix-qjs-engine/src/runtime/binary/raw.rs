use super::binary_value_unsupported_error;
use crate::QuickJsTypedArrayKind;
use crate::allocation::try_copy_bytes;
use crate::guest::host_len_i32;
use crate::runtime::QuickJsRuntime;
use crate::runtime::cleanup::{CleanupScope, finish_with_cleanup};
use crate::runtime::raw_value::RawJsValue;
use anyhow::{Result, anyhow};
use wasmtime::TypedFunc;
use wasmtime::error::Context as _;

mod view;

impl QuickJsRuntime {
    pub(super) fn new_binary_raw(
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

    pub(super) fn read_array_buffer_bytes(&mut self, value: &RawJsValue) -> Result<Vec<u8>> {
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

    pub(super) fn read_uint8_array_bytes(&mut self, value: &RawJsValue) -> Result<Vec<u8>> {
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

    pub(super) fn new_typed_binary_raw(
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
}
