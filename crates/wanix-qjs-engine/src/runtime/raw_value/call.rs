use super::RawJsValue;
use crate::allocation::try_reserve_bytes;
use crate::guest::{guest_offset, guest_u32, host_count_i32, host_len_i32};
use crate::runtime::QuickJsRuntime;
use crate::runtime::cleanup::{CleanupScope, finish_with_cleanup};
use anyhow::{Result, anyhow};
use wasmtime::error::Context as _;

const RAW_JS_VALUE_PTR_BYTES: usize = 4;

struct RawCallArgv {
    ptr: i32,
}

impl RawCallArgv {
    fn empty() -> Self {
        Self { ptr: 0 }
    }

    fn ptr(&self) -> i32 {
        self.ptr
    }

    fn record_cleanup(self, runtime: &mut QuickJsRuntime, cleanup: &mut CleanupScope) {
        if self.ptr != 0 {
            cleanup.record(runtime.guest_free(self.ptr));
        }
    }
}

impl QuickJsRuntime {
    pub(in crate::runtime) fn call_function_raw(
        &mut self,
        function: &RawJsValue,
        this_value: &RawJsValue,
        args: &[&RawJsValue],
    ) -> Result<RawJsValue> {
        let argc = raw_call_arg_count(args.len())?;
        let argv = self.write_raw_call_argv(args)?;
        let result = self
            .qjs_call
            .call(
                &mut self.store,
                (function.ptr(), this_value.ptr(), argc, argv.ptr()),
            )
            .context("failed to call QuickJS function");

        let mut cleanup = CleanupScope::new();
        argv.record_cleanup(self, &mut cleanup);
        self.finish_raw_value(result, "qjs_call", cleanup)
    }

    fn write_raw_call_argv(&mut self, args: &[&RawJsValue]) -> Result<RawCallArgv> {
        if args.is_empty() {
            return Ok(RawCallArgv::empty());
        }
        let argv_len = raw_call_argv_len(args.len())?;
        let bytes = raw_call_argv_bytes(args)?;
        let ptr = self.guest_malloc(argv_len)?;
        let write = self
            .memory
            .write(&mut self.store, guest_offset(ptr), &bytes)
            .context("failed to write argv into guest memory");
        if let Err(err) = write {
            let cleanup = self.guest_free(ptr);
            return finish_with_cleanup(Err(err), cleanup.err());
        }
        Ok(RawCallArgv { ptr })
    }
}

fn raw_call_arg_count(len: usize) -> Result<i32> {
    host_count_i32(len).ok_or_else(|| anyhow!("too many QuickJS call arguments"))
}

fn raw_call_argv_len(len: usize) -> Result<u32> {
    let byte_len = raw_call_argv_byte_len(len)?;
    let argv_len =
        u32::try_from(byte_len).map_err(|_| anyhow!("too many QuickJS call arguments"))?;
    host_len_i32(argv_len).ok_or_else(|| anyhow!("too many QuickJS call arguments"))?;
    Ok(argv_len)
}

fn raw_call_argv_byte_len(len: usize) -> Result<usize> {
    len.checked_mul(RAW_JS_VALUE_PTR_BYTES)
        .ok_or_else(|| anyhow!("too many QuickJS call arguments"))
}

fn raw_call_argv_bytes(args: &[&RawJsValue]) -> Result<Vec<u8>> {
    let mut bytes = try_reserve_bytes(raw_call_argv_byte_len(args.len())?, "QuickJS argv buffer")?;
    for arg in args {
        bytes.extend_from_slice(&guest_u32(arg.ptr()).to_le_bytes());
    }
    Ok(bytes)
}
