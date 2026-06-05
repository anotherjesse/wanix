use super::{QuickJsRuntime, scalar_value_unsupported_error};
use crate::guest::{guest_i32_add, i64_from_guest_u32_halves, i64_to_guest_i32_halves};
use crate::runtime::cleanup::{CleanupScope, finish_with_cleanup};
use crate::runtime::raw_value::RawJsValue;
use anyhow::{Result, anyhow, bail};
use wasmtime::error::Context as _;

impl QuickJsRuntime {
    pub(super) fn read_big_int64_value(&mut self, value: &RawJsValue) -> Result<i64> {
        let qjs_get_big_int64 = self
            .qjs_get_big_int64
            .clone()
            .ok_or_else(|| scalar_value_unsupported_error("qjs_get_big_int64"))?;
        let out = self.guest_malloc(8)?;

        let result = (|| {
            let hi_out = guest_i32_add(out, 4)
                .ok_or_else(|| anyhow!("QuickJS BigInt metadata pointer overflowed"))?;
            let ret = qjs_get_big_int64
                .call(&mut self.store, (value.ptr(), out, hi_out))
                .context("failed to read BigInt result")?;
            if ret != 0 {
                match self.take_exception_string() {
                    Ok(message) => bail!("qjs_get_big_int64 failed: {message}"),
                    Err(err) => return Err(err),
                }
            }

            let lo = self.read_guest_u32(out, "QuickJS BigInt low word")?;
            let hi = self.read_guest_u32(hi_out, "QuickJS BigInt high word")?;
            Ok(i64_from_guest_u32_halves(lo, hi))
        })();

        let cleanup = self.guest_free(out);
        finish_with_cleanup(result, cleanup.err())
    }

    pub(super) fn big_int64_raw(&mut self, value: i64) -> Result<RawJsValue> {
        let qjs_new_big_int64 = self
            .qjs_new_big_int64
            .clone()
            .ok_or_else(|| scalar_value_unsupported_error("qjs_new_big_int64"))?;
        let (lo, hi) = i64_to_guest_i32_halves(value);
        let result = qjs_new_big_int64
            .call(&mut self.store, (lo, hi))
            .context("failed to create BigInt handle");
        self.finish_raw_value(result, "qjs_new_big_int64", CleanupScope::new())
    }
}
