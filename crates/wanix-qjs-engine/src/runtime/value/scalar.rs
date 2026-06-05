use super::QuickJsRuntime;
use crate::QuickJsValue;
use crate::runtime::cleanup::finish_with_cleanup;
use crate::runtime::raw_value::RawJsValue;
use anyhow::{Result, anyhow, bail};

mod big_int;
mod read;

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
