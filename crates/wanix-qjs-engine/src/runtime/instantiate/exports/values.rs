use crate::host::HostState;
use anyhow::Result;
use wasmtime::{Instance, Store, TypedFunc};

use super::{optional_typed, typed};

pub(in crate::runtime::instantiate) struct TypeCheckExports {
    pub(in crate::runtime::instantiate) qjs_is_exception: TypedFunc<i32, i32>,
    pub(in crate::runtime::instantiate) qjs_is_undefined: Option<TypedFunc<i32, i32>>,
    pub(in crate::runtime::instantiate) qjs_is_null: Option<TypedFunc<i32, i32>>,
    pub(in crate::runtime::instantiate) qjs_is_bool: Option<TypedFunc<i32, i32>>,
    pub(in crate::runtime::instantiate) qjs_is_number: TypedFunc<i32, i32>,
    pub(in crate::runtime::instantiate) qjs_is_string: TypedFunc<i32, i32>,
    pub(in crate::runtime::instantiate) qjs_is_big_int: Option<TypedFunc<i32, i32>>,
    pub(in crate::runtime::instantiate) qjs_is_array_buffer: Option<TypedFunc<i32, i32>>,
    pub(in crate::runtime::instantiate) qjs_is_uint8_array: Option<TypedFunc<i32, i32>>,
    pub(in crate::runtime::instantiate) qjs_get_typed_array_type: Option<TypedFunc<i32, i32>>,
    pub(in crate::runtime::instantiate) qjs_is_data_view: Option<TypedFunc<i32, i32>>,
}

impl TypeCheckExports {
    pub(in crate::runtime::instantiate) fn bind(
        instance: &Instance,
        store: &mut Store<HostState>,
    ) -> Result<Self> {
        Ok(Self {
            qjs_is_exception: typed(instance, store, "qjs_is_exception")?,
            qjs_is_undefined: optional_typed(instance, store, "qjs_is_undefined")?,
            qjs_is_null: optional_typed(instance, store, "qjs_is_null")?,
            qjs_is_bool: optional_typed(instance, store, "qjs_is_bool")?,
            qjs_is_number: typed(instance, store, "qjs_is_number")?,
            qjs_is_string: typed(instance, store, "qjs_is_string")?,
            qjs_is_big_int: optional_typed(instance, store, "qjs_is_big_int")?,
            qjs_is_array_buffer: optional_typed(instance, store, "qjs_is_array_buffer")?,
            qjs_is_uint8_array: optional_typed(instance, store, "qjs_is_uint8_array")?,
            qjs_get_typed_array_type: optional_typed(instance, store, "qjs_get_typed_array_type")?,
            qjs_is_data_view: optional_typed(instance, store, "qjs_is_data_view")?,
        })
    }
}

pub(in crate::runtime::instantiate) struct AccessorExports {
    pub(in crate::runtime::instantiate) qjs_get_exception: TypedFunc<(), i32>,
    pub(in crate::runtime::instantiate) qjs_throw: Option<TypedFunc<i32, i32>>,
    pub(in crate::runtime::instantiate) qjs_get_bool: Option<TypedFunc<i32, i32>>,
    pub(in crate::runtime::instantiate) qjs_get_float64: TypedFunc<i32, f64>,
    pub(in crate::runtime::instantiate) qjs_get_big_int64: Option<TypedFunc<(i32, i32, i32), i32>>,
    pub(in crate::runtime::instantiate) qjs_get_string: TypedFunc<i32, i32>,
    pub(in crate::runtime::instantiate) qjs_get_array_buffer: Option<TypedFunc<(i32, i32), i32>>,
    pub(in crate::runtime::instantiate) qjs_get_uint8_array: Option<TypedFunc<(i32, i32), i32>>,
    pub(in crate::runtime::instantiate) qjs_get_typed_array_buffer:
        Option<TypedFunc<(i32, i32, i32, i32), i32>>,
    pub(in crate::runtime::instantiate) qjs_get_data_view_buffer:
        Option<TypedFunc<(i32, i32, i32), i32>>,
}

impl AccessorExports {
    pub(in crate::runtime::instantiate) fn bind(
        instance: &Instance,
        store: &mut Store<HostState>,
    ) -> Result<Self> {
        Ok(Self {
            qjs_get_exception: typed(instance, store, "qjs_get_exception")?,
            qjs_throw: optional_typed(instance, store, "qjs_throw")?,
            qjs_get_bool: optional_typed(instance, store, "qjs_get_bool")?,
            qjs_get_float64: typed(instance, store, "qjs_get_float64")?,
            qjs_get_big_int64: optional_typed(instance, store, "qjs_get_big_int64")?,
            qjs_get_string: typed(instance, store, "qjs_get_string")?,
            qjs_get_array_buffer: optional_typed(instance, store, "qjs_get_array_buffer")?,
            qjs_get_uint8_array: optional_typed(instance, store, "qjs_get_uint8_array")?,
            qjs_get_typed_array_buffer: optional_typed(
                instance,
                store,
                "qjs_get_typed_array_buffer",
            )?,
            qjs_get_data_view_buffer: optional_typed(instance, store, "qjs_get_data_view_buffer")?,
        })
    }
}

pub(in crate::runtime::instantiate) struct JobExports {
    pub(in crate::runtime::instantiate) qjs_is_job_pending: TypedFunc<(), i32>,
    pub(in crate::runtime::instantiate) qjs_execute_pending_job: TypedFunc<(), i32>,
    pub(in crate::runtime::instantiate) js_std_loop_once: Option<TypedFunc<i32, i32>>,
    pub(in crate::runtime::instantiate) js_std_poll_io: Option<TypedFunc<(i32, i32), i32>>,
}

impl JobExports {
    pub(in crate::runtime::instantiate) fn bind(
        instance: &Instance,
        store: &mut Store<HostState>,
    ) -> Result<Self> {
        Ok(Self {
            qjs_is_job_pending: typed(instance, store, "qjs_is_job_pending")?,
            qjs_execute_pending_job: typed(instance, store, "qjs_execute_pending_job")?,
            js_std_loop_once: optional_typed(instance, store, "js_std_loop_once")?,
            js_std_poll_io: optional_typed(instance, store, "js_std_poll_io")?,
        })
    }
}

pub(in crate::runtime::instantiate) struct SnapshotExports {
    pub(in crate::runtime::instantiate) qjs_get_runtime_ptr: TypedFunc<(), i32>,
    pub(in crate::runtime::instantiate) qjs_get_context_ptr: TypedFunc<(), i32>,
    pub(in crate::runtime::instantiate) qjs_set_runtime_and_context: TypedFunc<(i32, i32), ()>,
}

impl SnapshotExports {
    pub(in crate::runtime::instantiate) fn bind(
        instance: &Instance,
        store: &mut Store<HostState>,
    ) -> Result<Self> {
        Ok(Self {
            qjs_get_runtime_ptr: typed(instance, store, "qjs_get_runtime_ptr")?,
            qjs_get_context_ptr: typed(instance, store, "qjs_get_context_ptr")?,
            qjs_set_runtime_and_context: typed(instance, store, "qjs_set_runtime_and_context")?,
        })
    }
}
