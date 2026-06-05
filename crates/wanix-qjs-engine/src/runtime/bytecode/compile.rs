use super::{QuickJsRuntime, bytecode_unsupported_error};
use crate::bytecode::{QuickJsBytecode, QuickJsBytecodeCompileOptions};
use crate::guest::host_len_i32;
use crate::runtime::guest_memory::GuestString;
use anyhow::{Result, anyhow, bail};
use wasmtime::error::Context as _;

use crate::runtime::cleanup::{CleanupScope, finish_with_cleanup};

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
        let exports = self.bytecode_compile_exports()?;
        let inputs = self.write_bytecode_compile_inputs(code, filename)?;
        let out_len_ptr = match self.guest_malloc(OUT_LEN_SIZE) {
            Ok(ptr) => ptr,
            Err(err) => {
                let mut cleanup = CleanupScope::new();
                cleanup.record(self.guest_free(inputs.code.ptr));
                cleanup.record(self.guest_free(inputs.filename.ptr));
                return cleanup.finish(Err(err));
            }
        };

        let result = call_qjs_compile(
            &mut self.store,
            &exports.qjs_compile,
            &inputs,
            options,
            out_len_ptr,
        );

        let mut cleanup = CleanupScope::new();
        cleanup.record(self.guest_free(inputs.code.ptr));
        cleanup.record(self.guest_free(inputs.filename.ptr));

        let buf_ptr = match result {
            Ok(buf_ptr) => buf_ptr,
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

        let bytes = match self.read_compiled_bytecode(
            &exports.qjs_free_bytecode,
            buf_ptr,
            out_len_ptr,
            &mut cleanup,
        ) {
            Ok(bytes) => bytes,
            Err(err) => return cleanup.finish(Err(err)),
        };
        cleanup.finish(QuickJsBytecode::new(self.wasm_sha256, bytes))
    }

    fn bytecode_compile_exports(&self) -> Result<BytecodeCompileExports> {
        Ok(BytecodeCompileExports {
            qjs_compile: self
                .qjs_compile
                .clone()
                .ok_or_else(|| bytecode_unsupported_error("qjs_compile"))?,
            qjs_free_bytecode: self
                .qjs_free_bytecode
                .clone()
                .ok_or_else(|| bytecode_unsupported_error("qjs_free_bytecode"))?,
        })
    }

    fn write_bytecode_compile_inputs(
        &mut self,
        code: &str,
        filename: &str,
    ) -> Result<BytecodeCompileInputs> {
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
        Ok(BytecodeCompileInputs {
            code,
            code_len,
            filename,
        })
    }

    fn read_compiled_bytecode(
        &mut self,
        qjs_free_bytecode: &wasmtime::TypedFunc<i32, ()>,
        buf_ptr: i32,
        out_len_ptr: i32,
        cleanup: &mut CleanupScope,
    ) -> Result<Vec<u8>> {
        let out_len = match self.read_guest_u32(out_len_ptr, "QuickJS bytecode length") {
            Ok(out_len) => {
                cleanup.record(self.guest_free(out_len_ptr));
                out_len
            }
            Err(err) => {
                cleanup.record(self.guest_free(out_len_ptr));
                cleanup.record(free_bytecode_buffer(
                    &mut self.store,
                    qjs_free_bytecode,
                    buf_ptr,
                ));
                return Err(err);
            }
        };
        let bytes = self.read_guest_bytes(buf_ptr, out_len, "QuickJS bytecode");
        cleanup.record(free_bytecode_buffer(
            &mut self.store,
            qjs_free_bytecode,
            buf_ptr,
        ));
        bytes
    }
}

struct BytecodeCompileExports {
    qjs_compile: wasmtime::TypedFunc<(i32, i32, i32, i32, i32, i32), i32>,
    qjs_free_bytecode: wasmtime::TypedFunc<i32, ()>,
}

struct BytecodeCompileInputs {
    code: GuestString,
    code_len: i32,
    filename: GuestString,
}

fn call_qjs_compile(
    store: &mut wasmtime::Store<crate::host::HostState>,
    qjs_compile: &wasmtime::TypedFunc<(i32, i32, i32, i32, i32, i32), i32>,
    inputs: &BytecodeCompileInputs,
    options: QuickJsBytecodeCompileOptions,
    out_len_ptr: i32,
) -> Result<i32> {
    Ok(qjs_compile
        .call(
            store,
            (
                inputs.code.ptr,
                inputs.code_len,
                inputs.filename.ptr,
                options.eval_flags(),
                options.write_flags(),
                out_len_ptr,
            ),
        )
        .context("failed to call qjs_compile")?)
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
