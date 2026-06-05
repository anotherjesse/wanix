use super::{RawJsValue, validate_global_property_name};
use crate::runtime::QuickJsRuntime;
use crate::runtime::cleanup::CleanupScope;
use anyhow::Result;
use wasmtime::error::Context as _;

impl QuickJsRuntime {
    pub(in crate::runtime) fn global_prop_raw(&mut self, name: &str) -> Result<RawJsValue> {
        validate_global_property_name(name)?;
        let global = self.global_object_raw()?;
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

    fn global_object_raw(&mut self) -> Result<RawJsValue> {
        let global_ptr = self
            .qjs_get_global
            .call(&mut self.store, ())
            .context("failed to get global object")?;
        RawJsValue::from_non_null(global_ptr, "qjs_get_global")
    }
}
