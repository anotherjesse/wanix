use super::QuickJsRuntime;
use super::cleanup::{CleanupScope, finish_with_cleanup};
use super::raw_value::{RawJsValue, validate_global_property_name};
use crate::guest::host_len_i32;
use crate::host::{HostCallbackEntry, HostCallbackMode, scalar_host_callback};
use crate::{QuickJsCallbackValue, QuickJsHostValue};
use anyhow::{Result, anyhow, bail};
use wasmtime::error::Context as _;

impl QuickJsRuntime {
    /// Defines a JavaScript global function backed by a Rust callback.
    ///
    /// The QuickJS function stores `name` in the guest heap, so a snapshot keeps
    /// the JavaScript function object. The Rust closure is host state and must
    /// be reattached after restore with [`Self::register_host_callback`].
    ///
    /// # Errors
    ///
    /// Returns an error if `name` is empty, contains a NUL byte, is already
    /// registered in this runtime, the wasm module does not export the callback
    /// helper functions, the JavaScript global cannot be updated, or cleanup
    /// fails.
    pub fn define_global_host_function<F>(
        &mut self,
        name: impl Into<String>,
        callback: F,
    ) -> Result<()>
    where
        F: FnMut(&[QuickJsHostValue]) -> Result<QuickJsHostValue> + Send + 'static,
    {
        let name = name.into();
        validate_host_callback_name(&name)?;
        self.ensure_host_callback_define_capability()?;
        if self.store.data().contains_host_callback(&name) {
            bail!("host callback '{name}' is already registered");
        }

        let function = self.create_host_function_raw(&name)?;
        let set_global = self.set_global_prop_raw(&name, &function);
        self.finish_with_raw_value_cleanup(set_global, function)?;

        self.store.data_mut().insert_host_callback(
            name,
            HostCallbackEntry::new(scalar_host_callback(callback), HostCallbackMode::Scalar),
        )
    }

    /// Defines a JavaScript global function backed by a Rust callback that can
    /// accept and return copied scalar and binary values.
    ///
    /// The QuickJS function stores `name` in the guest heap, so a snapshot keeps
    /// the JavaScript function object. The Rust closure is host state and must
    /// be reattached after restore with
    /// [`Self::register_host_callback_with_binary_values`].
    ///
    /// # Errors
    ///
    /// Returns an error if `name` is empty, contains a NUL byte, is already
    /// registered in this runtime, the wasm module does not export the callback
    /// or binary helper functions, the JavaScript global cannot be updated, or
    /// cleanup fails.
    pub fn define_global_host_function_with_binary_values<F>(
        &mut self,
        name: impl Into<String>,
        callback: F,
    ) -> Result<()>
    where
        F: FnMut(&[QuickJsCallbackValue]) -> Result<QuickJsCallbackValue> + Send + 'static,
    {
        let name = name.into();
        validate_host_callback_name(&name)?;
        self.ensure_host_callback_define_capability()?;
        self.ensure_binary_value_capability()?;
        if self.store.data().contains_host_callback(&name) {
            bail!("host callback '{name}' is already registered");
        }

        let function = self.create_host_function_raw(&name)?;
        let set_global = self.set_global_prop_raw(&name, &function);
        self.finish_with_raw_value_cleanup(set_global, function)?;

        self.store.data_mut().insert_host_callback(
            name,
            HostCallbackEntry::new(Box::new(callback), HostCallbackMode::BinaryCapable),
        )
    }

    /// Registers a Rust callback implementation for an existing QuickJS host function.
    ///
    /// Use this after restoring a snapshot that already contains host function
    /// objects created by [`Self::define_global_host_function`]. The name must
    /// match the stable name used to create the function before the snapshot.
    ///
    /// # Errors
    ///
    /// Returns an error if `name` is empty, contains a NUL byte, or is already
    /// registered in this runtime, or if the wasm module does not export the
    /// callback helper functions needed for host-call dispatch.
    pub fn register_host_callback<F>(&mut self, name: impl Into<String>, callback: F) -> Result<()>
    where
        F: FnMut(&[QuickJsHostValue]) -> Result<QuickJsHostValue> + Send + 'static,
    {
        let name = name.into();
        validate_host_callback_name(&name)?;
        self.ensure_host_callback_call_capability()?;
        self.store.data_mut().insert_host_callback(
            name,
            HostCallbackEntry::new(scalar_host_callback(callback), HostCallbackMode::Scalar),
        )
    }

    /// Registers a Rust callback implementation that can accept and return
    /// copied scalar and binary values for an existing QuickJS host function.
    ///
    /// Use this after restoring a snapshot that already contains host function
    /// objects created by [`Self::define_global_host_function_with_binary_values`].
    /// The name must match the stable name used to create the function before
    /// the snapshot.
    ///
    /// # Errors
    ///
    /// Returns an error if `name` is empty, contains a NUL byte, is already
    /// registered in this runtime, or if the wasm module does not export the
    /// callback or binary helper functions needed for host-call dispatch.
    pub fn register_host_callback_with_binary_values<F>(
        &mut self,
        name: impl Into<String>,
        callback: F,
    ) -> Result<()>
    where
        F: FnMut(&[QuickJsCallbackValue]) -> Result<QuickJsCallbackValue> + Send + 'static,
    {
        let name = name.into();
        validate_host_callback_name(&name)?;
        self.ensure_host_callback_call_capability()?;
        self.ensure_binary_value_capability()?;
        self.store.data_mut().insert_host_callback(
            name,
            HostCallbackEntry::new(Box::new(callback), HostCallbackMode::BinaryCapable),
        )
    }

    fn ensure_host_callback_define_capability(&self) -> Result<()> {
        self.ensure_host_callback_call_capability()?;
        ensure_optional_export(
            self.qjs_new_host_function.is_some(),
            "qjs_new_host_function",
        )?;
        ensure_optional_export(self.qjs_set_prop_string.is_some(), "qjs_set_prop_string")
    }

    fn ensure_host_callback_call_capability(&self) -> Result<()> {
        ensure_optional_export(self.qjs_is_undefined.is_some(), "qjs_is_undefined")?;
        ensure_optional_export(self.qjs_new_number.is_some(), "qjs_new_number")?;
        ensure_optional_export(self.qjs_throw.is_some(), "qjs_throw")
    }

    fn create_host_function_raw(&mut self, name: &str) -> Result<RawJsValue> {
        let function = self
            .qjs_new_host_function
            .clone()
            .ok_or_else(|| anyhow!("QuickJS WASM module does not export qjs_new_host_function"))?;
        let name = self.write_c_string(name)?;
        let name_len = match host_len_i32(name.len) {
            Some(len) => len,
            None => {
                let cleanup = self.guest_free(name.ptr);
                return finish_with_cleanup(
                    Err(anyhow!(
                        "host callback name length exceeds supported host call range"
                    )),
                    cleanup.err(),
                );
            }
        };
        let result = function
            .call(&mut self.store, (name.ptr, name_len, 0))
            .context("failed to create QuickJS host function");
        let mut cleanup = CleanupScope::new();
        cleanup.record(self.guest_free(name.ptr));
        self.finish_raw_value(result, "qjs_new_host_function", cleanup)
    }

    pub(super) fn set_global_prop_raw(&mut self, name: &str, value: &RawJsValue) -> Result<()> {
        validate_global_property_name(name)?;
        let set_prop = self
            .qjs_set_prop_string
            .clone()
            .ok_or_else(|| anyhow!("QuickJS WASM module does not export qjs_set_prop_string"))?;
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

        let result = set_prop
            .call(&mut self.store, (global.ptr(), name.ptr, value.ptr()))
            .context("failed to set global QuickJS property");
        let mut cleanup = CleanupScope::new();
        cleanup.record(self.guest_free(name.ptr));
        cleanup.record(self.free_value(global));
        let result = match result {
            Ok(result) => {
                if result < 0 {
                    match self.take_exception_string() {
                        Ok(message) => Err(anyhow!("QuickJS exception: {message}")),
                        Err(err) => Err(err),
                    }
                } else {
                    Ok(())
                }
            }
            Err(err) => Err(anyhow!("{err:#}")),
        };
        cleanup.finish(result)
    }
}

fn validate_host_callback_name(name: &str) -> Result<()> {
    if name.is_empty() {
        bail!("host callback name must not be empty");
    }
    if name.as_bytes().contains(&0) {
        bail!("host callback name must not contain NUL bytes");
    }
    Ok(())
}

fn ensure_optional_export(has_export: bool, name: &str) -> Result<()> {
    if !has_export {
        bail!("QuickJS WASM module does not export {name}; host callbacks are not supported");
    }
    Ok(())
}
