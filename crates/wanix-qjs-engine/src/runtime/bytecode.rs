mod compile;

use super::QuickJsRuntime;
use super::cleanup::{CleanupScope, finish_with_cleanup};
use super::raw_value::RawJsValue;
use crate::QuickJsValue;
use crate::bytecode::QuickJsBytecode;
use crate::guest::{guest_offset, host_len_i32};
use anyhow::{Result, anyhow, bail};
use wasmtime::error::Context as _;

impl QuickJsRuntime {
    /// Evaluates trusted QuickJS bytecode and discards the result.
    ///
    /// # Errors
    ///
    /// Returns an error if bytecode is empty or bound to a different QuickJS
    /// WASM module identity, if the bytecode helper export is unavailable, if
    /// bytecode evaluation throws, or if cleanup fails.
    pub fn eval_bytecode_discard(&mut self, bytecode: &QuickJsBytecode) -> Result<()> {
        let value = self.eval_bytecode_raw(bytecode)?;
        self.free_value(value)
    }

    /// Evaluates trusted QuickJS bytecode and returns a copied scalar result.
    ///
    /// # Errors
    ///
    /// Returns an error if bytecode is empty or bound to a different QuickJS
    /// WASM module identity, if the bytecode helper export is unavailable, if
    /// bytecode evaluation throws, if the result is not a supported copied
    /// scalar, or if cleanup fails.
    pub fn eval_bytecode_value(&mut self, bytecode: &QuickJsBytecode) -> Result<QuickJsValue> {
        self.ensure_bytecode_evaluable(bytecode)?;
        self.ensure_scalar_value_capability()?;
        let value = self.eval_bytecode_raw(bytecode)?;
        self.raw_value_to_scalar(value)
    }

    fn eval_bytecode_raw(&mut self, bytecode: &QuickJsBytecode) -> Result<RawJsValue> {
        self.ensure_bytecode_evaluable(bytecode)?;
        let qjs_eval_bytecode = self
            .qjs_eval_bytecode
            .clone()
            .ok_or_else(|| bytecode_unsupported_error("qjs_eval_bytecode"))?;
        let len = u32::try_from(bytecode.bytes().len())
            .map_err(|_| anyhow!("QuickJS bytecode length exceeds supported host call range"))?;
        let len_i32 = host_len_i32(len).ok_or_else(|| anyhow!("QuickJS bytecode is too large"))?;
        if len == 0 {
            bail!("QuickJS bytecode must not be empty");
        }
        let ptr = self.guest_malloc(len)?;
        let write = self
            .memory
            .write(&mut self.store, guest_offset(ptr), bytecode.bytes())
            .context("failed to write QuickJS bytecode into guest memory");
        if let Err(err) = write {
            let cleanup = self.guest_free(ptr);
            return finish_with_cleanup(Err(err), cleanup.err());
        }

        let result = qjs_eval_bytecode
            .call(&mut self.store, (ptr, len_i32))
            .context("failed to call qjs_eval_bytecode");
        let mut cleanup = CleanupScope::new();
        cleanup.record(self.guest_free(ptr));
        self.finish_raw_value(result, "qjs_eval_bytecode", cleanup)
    }

    fn ensure_bytecode_evaluable(&self, bytecode: &QuickJsBytecode) -> Result<()> {
        if bytecode.wasm_sha256() != self.wasm_sha256 {
            bail!(
                "QuickJS bytecode is bound to wasm module SHA-256 {}, but runtime uses {}",
                format_sha256_hex(&bytecode.wasm_sha256()),
                format_sha256_hex(&self.wasm_sha256)
            );
        }
        if bytecode.bytes().is_empty() {
            bail!("QuickJS bytecode must not be empty");
        }
        if self.qjs_eval_bytecode.is_none() {
            return Err(bytecode_unsupported_error("qjs_eval_bytecode"));
        }
        Ok(())
    }
}

pub(super) fn bytecode_unsupported_error(export_name: &str) -> anyhow::Error {
    anyhow!("QuickJS WASM module does not export {export_name}; bytecode is not supported")
}

fn format_sha256_hex(bytes: &[u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut hex = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        hex.push(char::from(HEX[usize::from(byte >> 4)]));
        hex.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    hex
}
