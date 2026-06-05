use super::{QuickJsRuntime, binary_value_unsupported_error};
use crate::QuickJsTypedArrayKind;
use crate::allocation::try_copy_bytes;
use crate::guest::{guest_i32_add, guest_offset};
use crate::runtime::cleanup::{CleanupScope, finish_with_cleanup};
use crate::runtime::raw_value::RawJsValue;
use anyhow::{Error, Result, anyhow, bail};
use wasmtime::error::Context as _;

impl QuickJsRuntime {
    pub(in crate::runtime::binary) fn read_typed_array_bytes(
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

    pub(in crate::runtime::binary) fn read_data_view_bytes(
        &mut self,
        value: &RawJsValue,
    ) -> Result<Vec<u8>> {
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
}
