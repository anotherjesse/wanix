use super::QuickJsRuntime;
use super::cleanup::{CleanupScope, finish_with_cleanup};
use super::raw_value::RawJsValue;
use crate::QuickJsValue;
use crate::bytecode::{QuickJsBytecode, QuickJsBytecodeCompileOptions};
use crate::guest::{guest_offset, host_len_i32};
use anyhow::{Result, anyhow, bail};
use wasmtime::error::Context as _;

const OUT_LEN_SIZE: u32 = 4;

impl QuickJsRuntime {
    /// Compiles JavaScript source to trusted QuickJS bytecode.
    ///
    /// Uses the default filename `"<compile>"` and script compilation options.
    ///
    /// # Errors
    ///
    /// Returns an error if the bytecode helper export is unavailable, source
    /// compilation throws, guest memory transfer fails, or cleanup fails.
    pub fn compile_bytecode(&mut self, code: &str) -> Result<QuickJsBytecode> {
        self.compile_bytecode_with_options(
            code,
            "<compile>",
            QuickJsBytecodeCompileOptions::default(),
        )
    }

    /// Compiles JavaScript source to trusted QuickJS bytecode with options.
    ///
    /// Bytecode is bound to this runtime's exact QuickJS WASM module SHA-256.
    /// Treat returned bytes as trusted data; QuickJS bytecode is not a portable
    /// or sandbox-verifiable interchange format.
    ///
    /// # Errors
    ///
    /// Returns an error if `filename` is empty or contains a NUL byte, the
    /// bytecode helper export is unavailable, source compilation throws, guest
    /// memory transfer fails, or cleanup fails.
    pub fn compile_bytecode_with_options(
        &mut self,
        code: &str,
        filename: &str,
        options: QuickJsBytecodeCompileOptions,
    ) -> Result<QuickJsBytecode> {
        validate_bytecode_filename(filename)?;
        let qjs_compile = self
            .qjs_compile
            .clone()
            .ok_or_else(|| bytecode_unsupported_error("qjs_compile"))?;
        let qjs_free_bytecode = self
            .qjs_free_bytecode
            .clone()
            .ok_or_else(|| bytecode_unsupported_error("qjs_free_bytecode"))?;

        let code = self.write_c_string(code)?;
        let code_len = match host_len_i32(code.len) {
            Some(len) => len,
            None => {
                let cleanup = self.guest_free(code.ptr);
                return finish_with_cleanup(
                    Err(anyhow!(
                        "QuickJS bytecode source length exceeds supported host call range"
                    )),
                    cleanup.err(),
                );
            }
        };
        let filename = match self.write_c_string(filename) {
            Ok(filename) => filename,
            Err(err) => {
                let cleanup = self.guest_free(code.ptr);
                return finish_with_cleanup(Err(err), cleanup.err());
            }
        };
        let out_len_ptr = match self.guest_malloc(OUT_LEN_SIZE) {
            Ok(ptr) => ptr,
            Err(err) => {
                let mut cleanup = CleanupScope::new();
                cleanup.record(self.guest_free(code.ptr));
                cleanup.record(self.guest_free(filename.ptr));
                return cleanup.finish(Err(err));
            }
        };

        let result = qjs_compile
            .call(
                &mut self.store,
                (
                    code.ptr,
                    code_len,
                    filename.ptr,
                    options.eval_flags(),
                    options.write_flags(),
                    out_len_ptr,
                ),
            )
            .context("failed to call qjs_compile");

        let mut cleanup = CleanupScope::new();
        cleanup.record(self.guest_free(code.ptr));
        cleanup.record(self.guest_free(filename.ptr));

        let buf_ptr = match result {
            Ok(ptr) => ptr,
            Err(err) => {
                cleanup.record(self.guest_free(out_len_ptr));
                return cleanup.finish(Err(err));
            }
        };
        if buf_ptr == 0 {
            cleanup.record(self.guest_free(out_len_ptr));
            let message = self.take_exception_string();
            let err = match message {
                Ok(message) => anyhow!("QuickJS bytecode compilation failed: {message}"),
                Err(err) => err,
            };
            return cleanup.finish(Err(err));
        }

        let out_len = match self.read_guest_u32(out_len_ptr, "QuickJS bytecode length") {
            Ok(out_len) => {
                cleanup.record(self.guest_free(out_len_ptr));
                out_len
            }
            Err(err) => {
                cleanup.record(self.guest_free(out_len_ptr));
                cleanup.record(free_bytecode_buffer(
                    &mut self.store,
                    &qjs_free_bytecode,
                    buf_ptr,
                ));
                return cleanup.finish(Err(err));
            }
        };
        let bytes = self.read_guest_bytes(buf_ptr, out_len, "QuickJS bytecode");
        cleanup.record(free_bytecode_buffer(
            &mut self.store,
            &qjs_free_bytecode,
            buf_ptr,
        ));
        let bytes = match bytes {
            Ok(bytes) => bytes,
            Err(err) => return cleanup.finish(Err(err)),
        };
        cleanup.finish(QuickJsBytecode::new(self.wasm_sha256, bytes))
    }

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

fn validate_bytecode_filename(filename: &str) -> Result<()> {
    if filename.is_empty() {
        bail!("bytecode filename must not be empty");
    }
    if filename.as_bytes().contains(&0) {
        bail!("bytecode filename must not contain NUL bytes");
    }
    Ok(())
}

fn bytecode_unsupported_error(export_name: &str) -> anyhow::Error {
    anyhow!("QuickJS WASM module does not export {export_name}; bytecode is not supported")
}

fn free_bytecode_buffer(
    store: &mut wasmtime::Store<crate::host::HostState>,
    free_bytecode: &wasmtime::TypedFunc<i32, ()>,
    ptr: i32,
) -> Result<()> {
    free_bytecode
        .call(store, ptr)
        .context("failed to call qjs_free_bytecode")?;
    Ok(())
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
