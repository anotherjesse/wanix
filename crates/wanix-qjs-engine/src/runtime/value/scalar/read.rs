use super::{QuickJsRuntime, scalar_value_unsupported_error};
use crate::QuickJsValue;
use crate::runtime::raw_value::RawJsValue;
use anyhow::{Result, bail};
use wasmtime::error::Context as _;

impl QuickJsRuntime {
    pub(in crate::runtime) fn maybe_raw_value_to_scalar_ref(
        &mut self,
        value: &RawJsValue,
    ) -> Result<Option<QuickJsValue>> {
        self.ensure_scalar_value_capability()?;

        if let Some(value) = self.read_undefined_scalar(value)? {
            return Ok(Some(value));
        }
        if let Some(value) = self.read_null_scalar(value)? {
            return Ok(Some(value));
        }
        if let Some(value) = self.read_bool_scalar(value)? {
            return Ok(Some(value));
        }
        if let Some(value) = self.read_number_scalar(value)? {
            return Ok(Some(value));
        }
        if let Some(value) = self.read_string_scalar(value)? {
            return Ok(Some(value));
        }
        self.read_big_int_scalar(value)
    }

    fn read_undefined_scalar(&mut self, value: &RawJsValue) -> Result<Option<QuickJsValue>> {
        let qjs_is_undefined = self
            .qjs_is_undefined
            .clone()
            .ok_or_else(|| scalar_value_unsupported_error("qjs_is_undefined"))?;
        if qjs_is_undefined
            .call(&mut self.store, value.ptr())
            .context("failed to inspect undefined result")?
            != 0
        {
            Ok(Some(QuickJsValue::Undefined))
        } else {
            Ok(None)
        }
    }

    fn read_null_scalar(&mut self, value: &RawJsValue) -> Result<Option<QuickJsValue>> {
        let qjs_is_null = self
            .qjs_is_null
            .clone()
            .ok_or_else(|| scalar_value_unsupported_error("qjs_is_null"))?;
        if qjs_is_null
            .call(&mut self.store, value.ptr())
            .context("failed to inspect null result")?
            != 0
        {
            Ok(Some(QuickJsValue::Null))
        } else {
            Ok(None)
        }
    }

    fn read_bool_scalar(&mut self, value: &RawJsValue) -> Result<Option<QuickJsValue>> {
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
            Ok(Some(QuickJsValue::Bool(
                qjs_get_bool
                    .call(&mut self.store, value.ptr())
                    .context("failed to read bool result")?
                    != 0,
            )))
        } else {
            Ok(None)
        }
    }

    fn read_number_scalar(&mut self, value: &RawJsValue) -> Result<Option<QuickJsValue>> {
        let is_number = self
            .qjs_is_number
            .call(&mut self.store, value.ptr())
            .context("failed to inspect number result")?;
        if is_number != 0 {
            Ok(Some(QuickJsValue::Number(
                self.qjs_get_float64
                    .call(&mut self.store, value.ptr())
                    .context("failed to read number result")?,
            )))
        } else {
            Ok(None)
        }
    }

    fn read_string_scalar(&mut self, value: &RawJsValue) -> Result<Option<QuickJsValue>> {
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
            self.read_and_free_quickjs_c_string(c_string, "failed to free QuickJS C string")
                .map(QuickJsValue::String)
                .map(Some)
        } else {
            Ok(None)
        }
    }

    fn read_big_int_scalar(&mut self, value: &RawJsValue) -> Result<Option<QuickJsValue>> {
        if let Some(qjs_is_big_int) = self.qjs_is_big_int.clone()
            && qjs_is_big_int
                .call(&mut self.store, value.ptr())
                .context("failed to inspect BigInt result")?
                != 0
        {
            self.read_big_int64_value(value)
                .map(QuickJsValue::BigIntI64)
                .map(Some)
        } else {
            Ok(None)
        }
    }
}
