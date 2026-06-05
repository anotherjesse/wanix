use super::QuickJsRuntime;
use crate::QuickJsValue;
use crate::guest::{guest_i32_add, i64_from_guest_u32_halves, i64_to_guest_i32_halves};
use crate::runtime::cleanup::{CleanupScope, finish_with_cleanup};
use crate::runtime::raw_value::RawJsValue;
use anyhow::{Result, anyhow, bail};
use wasmtime::error::Context as _;

impl QuickJsRuntime {
    pub(in crate::runtime) fn raw_value_to_scalar(
        &mut self,
        value: RawJsValue,
    ) -> Result<QuickJsValue> {
        self.with_owned_raw_value(value, |runtime, value| {
            runtime.raw_value_to_scalar_ref(value)
        })
    }

    pub(in crate::runtime) fn raw_value_to_scalar_ref(
        &mut self,
        value: &RawJsValue,
    ) -> Result<QuickJsValue> {
        if let Some(value) = self.maybe_raw_value_to_scalar_ref(value)? {
            return Ok(value);
        }
        bail!("QuickJS value is not a supported copied scalar");
    }

    pub(in crate::runtime) fn maybe_raw_value_to_scalar_ref(
        &mut self,
        value: &RawJsValue,
    ) -> Result<Option<QuickJsValue>> {
        self.ensure_scalar_value_capability()?;

        let qjs_is_undefined = self
            .qjs_is_undefined
            .clone()
            .ok_or_else(|| scalar_value_unsupported_error("qjs_is_undefined"))?;
        if qjs_is_undefined
            .call(&mut self.store, value.ptr())
            .context("failed to inspect undefined result")?
            != 0
        {
            return Ok(Some(QuickJsValue::Undefined));
        }

        let qjs_is_null = self
            .qjs_is_null
            .clone()
            .ok_or_else(|| scalar_value_unsupported_error("qjs_is_null"))?;
        if qjs_is_null
            .call(&mut self.store, value.ptr())
            .context("failed to inspect null result")?
            != 0
        {
            return Ok(Some(QuickJsValue::Null));
        }

        let qjs_is_bool = self
            .qjs_is_bool
            .clone()
            .ok_or_else(|| scalar_value_unsupported_error("qjs_is_bool"))?;
        if qjs_is_bool
            .call(&mut self.store, value.ptr())
            .context("failed to inspect bool result")?
            != 0
        {
            let qjs_get_bool = self
                .qjs_get_bool
                .clone()
                .ok_or_else(|| scalar_value_unsupported_error("qjs_get_bool"))?;
            return Ok(Some(QuickJsValue::Bool(
                qjs_get_bool
                    .call(&mut self.store, value.ptr())
                    .context("failed to read bool result")?
                    != 0,
            )));
        }

        let is_number = self
            .qjs_is_number
            .call(&mut self.store, value.ptr())
            .context("failed to inspect number result")?;
        if is_number != 0 {
            return Ok(Some(QuickJsValue::Number(
                self.qjs_get_float64
                    .call(&mut self.store, value.ptr())
                    .context("failed to read number result")?,
            )));
        }

        let is_string = self
            .qjs_is_string
            .call(&mut self.store, value.ptr())
            .context("failed to inspect string result")?;
        if is_string != 0 {
            let c_string = self
                .qjs_get_string
                .call(&mut self.store, value.ptr())
                .context("failed to convert result to string")?;
            if c_string == 0 {
                bail!("QuickJS failed to convert string result to a C string");
            }
            return self
                .read_and_free_quickjs_c_string(c_string, "failed to free QuickJS C string")
                .map(QuickJsValue::String)
                .map(Some);
        }

        if let Some(qjs_is_big_int) = self.qjs_is_big_int.clone()
            && qjs_is_big_int
                .call(&mut self.store, value.ptr())
                .context("failed to inspect BigInt result")?
                != 0
        {
            return self
                .read_big_int64_value(value)
                .map(QuickJsValue::BigIntI64)
                .map(Some);
        }

        Ok(None)
    }

    pub(in crate::runtime) fn scalar_to_raw_value(
        &mut self,
        value: QuickJsValue,
    ) -> Result<RawJsValue> {
        match value {
            QuickJsValue::Undefined => self.undefined_raw(),
            QuickJsValue::Null => self.null_raw(),
            QuickJsValue::Bool(value) => self.bool_raw(value),
            QuickJsValue::Number(value) => self.number_raw(value),
            QuickJsValue::String(value) => self.new_string_raw(&value),
            QuickJsValue::BigIntI64(value) => self.big_int64_raw(value),
        }
    }

    pub(in crate::runtime) fn scalar_args_to_raw_values(
        &mut self,
        args: &[QuickJsValue],
    ) -> Result<Vec<RawJsValue>> {
        let mut raw_args = Vec::new();
        raw_args
            .try_reserve_exact(args.len())
            .map_err(|err| anyhow!("QuickJS scalar argument allocation failed: {err}"))?;
        for arg in args {
            match self.scalar_to_raw_value(arg.clone()) {
                Ok(value) => raw_args.push(value),
                Err(err) => {
                    let cleanup = self.free_values(raw_args);
                    return finish_with_cleanup(Err(err), cleanup.err());
                }
            }
        }
        Ok(raw_args)
    }

    pub(in crate::runtime) fn ensure_scalar_value_capability(&self) -> Result<()> {
        ensure_scalar_optional_export(self.qjs_is_undefined.is_some(), "qjs_is_undefined")?;
        ensure_scalar_optional_export(self.qjs_get_null.is_some(), "qjs_get_null")?;
        ensure_scalar_optional_export(self.qjs_get_true.is_some(), "qjs_get_true")?;
        ensure_scalar_optional_export(self.qjs_get_false.is_some(), "qjs_get_false")?;
        ensure_scalar_optional_export(self.qjs_is_null.is_some(), "qjs_is_null")?;
        ensure_scalar_optional_export(self.qjs_is_bool.is_some(), "qjs_is_bool")?;
        ensure_scalar_optional_export(self.qjs_get_bool.is_some(), "qjs_get_bool")?;
        ensure_scalar_optional_export(self.qjs_new_number.is_some(), "qjs_new_number")
    }

    fn read_big_int64_value(&mut self, value: &RawJsValue) -> Result<i64> {
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

    fn big_int64_raw(&mut self, value: i64) -> Result<RawJsValue> {
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

fn ensure_scalar_optional_export(has_export: bool, name: &str) -> Result<()> {
    if !has_export {
        return Err(scalar_value_unsupported_error(name));
    }
    Ok(())
}

fn scalar_value_unsupported_error(name: &str) -> anyhow::Error {
    anyhow!("QuickJS WASM module does not export {name}; scalar values are not supported")
}
