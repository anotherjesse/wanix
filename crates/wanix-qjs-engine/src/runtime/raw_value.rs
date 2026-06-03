use super::QuickJsRuntime;
use super::cleanup::{CleanupScope, finish_with_cleanup};
use crate::allocation::try_reserve_bytes;
use crate::guest::{guest_offset, guest_u32, host_count_i32, host_len_i32};
use anyhow::{Error, Result, anyhow, bail};
use wasmtime::error::Context as _;

#[derive(Debug, PartialEq, Eq)]
#[must_use]
pub(super) struct RawJsValue(i32);

pub(super) const JS_EVAL_TYPE_MODULE: i32 = 1;

impl RawJsValue {
    pub(super) fn from_non_null(ptr: i32, source: &str) -> Result<Self> {
        if ptr == 0 {
            bail!("{source} returned a null JSValue pointer");
        }
        Ok(Self(ptr))
    }

    pub(super) fn ptr(&self) -> i32 {
        self.0
    }
}

pub(super) fn validate_global_property_name(name: &str) -> Result<()> {
    if name.as_bytes().contains(&0) {
        bail!("global property name must not contain NUL bytes");
    }
    Ok(())
}

impl QuickJsRuntime {
    pub(super) fn new_string_raw(&mut self, value: &str) -> Result<RawJsValue> {
        let guest = self.write_c_string(value)?;
        let guest_len = match host_len_i32(guest.len) {
            Some(len) => len,
            None => {
                let cleanup = self.guest_free(guest.ptr);
                return finish_with_cleanup(
                    Err(anyhow!(
                        "QuickJS string length exceeds supported host call range"
                    )),
                    cleanup.err(),
                );
            }
        };
        let result = self
            .qjs_new_string
            .call(&mut self.store, (guest.ptr, guest_len))
            .context("failed to create QuickJS string");
        let mut cleanup = CleanupScope::new();
        cleanup.record(self.guest_free(guest.ptr));
        self.finish_raw_value(result, "qjs_new_string", cleanup)
    }

    pub(super) fn global_prop_raw(&mut self, name: &str) -> Result<RawJsValue> {
        validate_global_property_name(name)?;
        let global_ptr = self
            .qjs_get_global
            .call(&mut self.store, ())
            .context("failed to get global object")?;
        let global = RawJsValue::from_non_null(global_ptr, "qjs_get_global")?;
        let name = match self.write_c_string(name) {
            Ok(name) => name,
            Err(err) => {
                return self.finish_with_raw_value_cleanup(Err(err), global);
            }
        };
        let value = self
            .qjs_get_prop_string
            .call(&mut self.store, (global.ptr(), name.ptr))
            .context("failed to get global property");
        let mut cleanup = CleanupScope::new();
        cleanup.record(self.guest_free(name.ptr));
        cleanup.record(self.free_value(global));
        self.finish_raw_value(value, "qjs_get_prop_string", cleanup)
    }

    pub(super) fn undefined_raw(&mut self) -> Result<RawJsValue> {
        let ptr = self
            .qjs_get_undefined
            .call(&mut self.store, ())
            .context("failed to create undefined handle")?;
        RawJsValue::from_non_null(ptr, "qjs_get_undefined")
    }

    pub(super) fn null_raw(&mut self) -> Result<RawJsValue> {
        let qjs_get_null = self
            .qjs_get_null
            .clone()
            .ok_or_else(|| anyhow!("QuickJS WASM module does not export qjs_get_null"))?;
        let ptr = qjs_get_null
            .call(&mut self.store, ())
            .context("failed to create null handle")?;
        RawJsValue::from_non_null(ptr, "qjs_get_null")
    }

    pub(super) fn bool_raw(&mut self, value: bool) -> Result<RawJsValue> {
        let name = if value {
            "qjs_get_true"
        } else {
            "qjs_get_false"
        };
        let qjs_get_bool = if value {
            self.qjs_get_true.clone()
        } else {
            self.qjs_get_false.clone()
        }
        .ok_or_else(|| anyhow!("QuickJS WASM module does not export {name}"))?;
        let ptr = qjs_get_bool
            .call(&mut self.store, ())
            .with_context(|| format!("failed to create bool handle with {name}"))?;
        RawJsValue::from_non_null(ptr, name)
    }

    pub(super) fn number_raw(&mut self, value: f64) -> Result<RawJsValue> {
        let qjs_new_number = self
            .qjs_new_number
            .clone()
            .ok_or_else(|| anyhow!("QuickJS WASM module does not export qjs_new_number"))?;
        let ptr = qjs_new_number
            .call(&mut self.store, value)
            .context("failed to create number handle")?;
        RawJsValue::from_non_null(ptr, "qjs_new_number")
    }

    pub(super) fn call_function_raw(
        &mut self,
        function: &RawJsValue,
        this_value: &RawJsValue,
        args: &[&RawJsValue],
    ) -> Result<RawJsValue> {
        let argc =
            host_count_i32(args.len()).ok_or_else(|| anyhow!("too many QuickJS call arguments"))?;
        let argv_ptr = if args.is_empty() {
            0
        } else {
            let argv_len_usize = args
                .len()
                .checked_mul(4)
                .ok_or_else(|| anyhow!("too many QuickJS call arguments"))?;
            let argv_len = u32::try_from(argv_len_usize)
                .map_err(|_| anyhow!("too many QuickJS call arguments"))?;
            host_len_i32(argv_len).ok_or_else(|| anyhow!("too many QuickJS call arguments"))?;
            let mut bytes = try_reserve_bytes(argv_len_usize, "QuickJS argv buffer")?;
            let ptr = self.guest_malloc(argv_len)?;
            for arg in args {
                bytes.extend_from_slice(&guest_u32(arg.ptr()).to_le_bytes());
            }
            let write = self
                .memory
                .write(&mut self.store, guest_offset(ptr), &bytes)
                .context("failed to write argv into guest memory");
            if let Err(err) = write {
                let cleanup = self.guest_free(ptr);
                return finish_with_cleanup(Err(err), cleanup.err());
            }
            ptr
        };

        let result = self
            .qjs_call
            .call(
                &mut self.store,
                (function.ptr(), this_value.ptr(), argc, argv_ptr),
            )
            .context("failed to call QuickJS function");

        let mut cleanup = CleanupScope::new();
        if argv_ptr != 0 {
            cleanup.record(self.guest_free(argv_ptr));
        }

        self.finish_raw_value(result, "qjs_call", cleanup)
    }

    pub(super) fn free_value(&mut self, value: RawJsValue) -> Result<()> {
        Ok(self
            .qjs_free_value
            .call(&mut self.store, value.ptr())
            .context("failed to free QuickJS value")?)
    }

    pub(super) fn free_values<I>(&mut self, values: I) -> Result<()>
    where
        I: IntoIterator<Item = RawJsValue>,
    {
        let mut cleanup = CleanupScope::new();
        for value in values {
            cleanup.record(self.free_value(value));
        }
        cleanup.finish(Ok::<(), Error>(()))
    }

    pub(super) fn finish_with_raw_value_cleanup<T, E>(
        &mut self,
        result: std::result::Result<T, E>,
        value: RawJsValue,
    ) -> Result<T>
    where
        E: Into<Error>,
    {
        let cleanup = self.free_value(value);
        finish_with_cleanup(result, cleanup.err())
    }

    pub(super) fn finish_with_raw_values_cleanup<T, E, I>(
        &mut self,
        result: std::result::Result<T, E>,
        values: I,
    ) -> Result<T>
    where
        E: Into<Error>,
        I: IntoIterator<Item = RawJsValue>,
    {
        let cleanup = self.free_values(values);
        finish_with_cleanup(result, cleanup.err())
    }

    pub(super) fn with_owned_raw_value<T>(
        &mut self,
        value: RawJsValue,
        f: impl FnOnce(&mut Self, &RawJsValue) -> Result<T>,
    ) -> Result<T> {
        // The raw value is always freed after the closure returns, even when
        // the closure succeeds.
        let result = f(self, &value);
        self.finish_with_raw_value_cleanup(result, value)
    }

    pub(super) fn eval_raw(&mut self, code: &str) -> Result<RawJsValue> {
        self.eval_raw_with_filename_and_flags(code, "<rust>", 0)
    }

    pub(super) fn eval_raw_with_filename_and_flags(
        &mut self,
        code: &str,
        filename: &str,
        flags: i32,
    ) -> Result<RawJsValue> {
        let code = self.write_c_string(code)?;
        let code_len = match host_len_i32(code.len) {
            Some(len) => len,
            None => {
                let cleanup = self.guest_free(code.ptr);
                return finish_with_cleanup(
                    Err(anyhow!(
                        "QuickJS eval source length exceeds supported host call range"
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
        let result = self
            .qjs_eval
            .call(&mut self.store, (code.ptr, code_len, filename.ptr, flags))
            .context("failed to call qjs_eval");
        let mut cleanup = CleanupScope::new();
        cleanup.record(self.guest_free(code.ptr));
        cleanup.record(self.guest_free(filename.ptr));
        self.finish_raw_value(result, "qjs_eval", cleanup)
    }
}
