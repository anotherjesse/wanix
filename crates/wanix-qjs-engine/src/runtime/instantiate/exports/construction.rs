use crate::host::HostState;
use anyhow::Result;
use wasmtime::{Instance, Store, TypedFunc};

use super::{optional_typed, typed};

pub(in crate::runtime::instantiate) struct ConstructorExports {
    pub(in crate::runtime::instantiate) qjs_new_string: TypedFunc<(i32, i32), i32>,
    pub(in crate::runtime::instantiate) qjs_new_array_buffer: Option<TypedFunc<(i32, i32), i32>>,
    pub(in crate::runtime::instantiate) qjs_new_uint8_array: Option<TypedFunc<(i32, i32), i32>>,
    pub(in crate::runtime::instantiate) qjs_new_typed_array:
        Option<TypedFunc<(i32, i32, i32), i32>>,
    pub(in crate::runtime::instantiate) qjs_new_data_view: Option<TypedFunc<(i32, i32), i32>>,
    pub(in crate::runtime::instantiate) qjs_new_number: Option<TypedFunc<f64, i32>>,
    pub(in crate::runtime::instantiate) qjs_new_big_int64: Option<TypedFunc<(i32, i32), i32>>,
}

impl ConstructorExports {
    pub(in crate::runtime::instantiate) fn bind(
        instance: &Instance,
        store: &mut Store<HostState>,
    ) -> Result<Self> {
        Ok(Self {
            qjs_new_string: typed(instance, store, "qjs_new_string")?,
            qjs_new_array_buffer: optional_typed(instance, store, "qjs_new_array_buffer")?,
            qjs_new_uint8_array: optional_typed(instance, store, "qjs_new_uint8_array")?,
            qjs_new_typed_array: optional_typed(instance, store, "qjs_new_typed_array")?,
            qjs_new_data_view: optional_typed(instance, store, "qjs_new_data_view")?,
            qjs_new_number: optional_typed(instance, store, "qjs_new_number")?,
            qjs_new_big_int64: optional_typed(instance, store, "qjs_new_big_int64")?,
        })
    }
}

pub(in crate::runtime::instantiate) struct HostControlExports {
    pub(in crate::runtime::instantiate) qjs_new_host_function:
        Option<TypedFunc<(i32, i32, i32), i32>>,
    pub(in crate::runtime::instantiate) qjs_set_interrupt_handler: Option<TypedFunc<i32, ()>>,
    pub(in crate::runtime::instantiate) qjs_set_module_loader: Option<TypedFunc<i32, ()>>,
    pub(in crate::runtime::instantiate) qjs_set_memory_limit: Option<TypedFunc<i32, ()>>,
    pub(in crate::runtime::instantiate) qjs_set_max_stack_size: Option<TypedFunc<i32, ()>>,
    pub(in crate::runtime::instantiate) qjs_set_promise_rejection_handler:
        Option<TypedFunc<i32, ()>>,
}

impl HostControlExports {
    pub(in crate::runtime::instantiate) fn bind(
        instance: &Instance,
        store: &mut Store<HostState>,
    ) -> Result<Self> {
        Ok(Self {
            qjs_new_host_function: optional_typed(instance, store, "qjs_new_host_function")?,
            qjs_set_interrupt_handler: optional_typed(
                instance,
                store,
                "qjs_set_interrupt_handler",
            )?,
            qjs_set_module_loader: optional_typed(instance, store, "qjs_set_module_loader")?,
            qjs_set_memory_limit: optional_typed(instance, store, "qjs_set_memory_limit")?,
            qjs_set_max_stack_size: optional_typed(instance, store, "qjs_set_max_stack_size")?,
            qjs_set_promise_rejection_handler: optional_typed(
                instance,
                store,
                "qjs_set_promise_rejection_handler",
            )?,
        })
    }
}

pub(in crate::runtime::instantiate) struct MemoryPolicyExports {
    pub(in crate::runtime::instantiate) qjs_run_gc: Option<TypedFunc<(), ()>>,
    pub(in crate::runtime::instantiate) qjs_set_gc_threshold: Option<TypedFunc<i32, ()>>,
    pub(in crate::runtime::instantiate) qjs_get_gc_threshold: Option<TypedFunc<(), i32>>,
    pub(in crate::runtime::instantiate) qjs_compute_memory_usage: Option<TypedFunc<i32, ()>>,
}

impl MemoryPolicyExports {
    pub(in crate::runtime::instantiate) fn bind(
        instance: &Instance,
        store: &mut Store<HostState>,
    ) -> Result<Self> {
        Ok(Self {
            qjs_run_gc: optional_typed(instance, store, "qjs_run_gc")?,
            qjs_set_gc_threshold: optional_typed(instance, store, "qjs_set_gc_threshold")?,
            qjs_get_gc_threshold: optional_typed(instance, store, "qjs_get_gc_threshold")?,
            qjs_compute_memory_usage: optional_typed(instance, store, "qjs_compute_memory_usage")?,
        })
    }
}

pub(in crate::runtime::instantiate) struct ValueConstantExports {
    pub(in crate::runtime::instantiate) qjs_get_undefined: TypedFunc<(), i32>,
    pub(in crate::runtime::instantiate) qjs_get_null: Option<TypedFunc<(), i32>>,
    pub(in crate::runtime::instantiate) qjs_get_true: Option<TypedFunc<(), i32>>,
    pub(in crate::runtime::instantiate) qjs_get_false: Option<TypedFunc<(), i32>>,
}

impl ValueConstantExports {
    pub(in crate::runtime::instantiate) fn bind(
        instance: &Instance,
        store: &mut Store<HostState>,
    ) -> Result<Self> {
        Ok(Self {
            qjs_get_undefined: typed(instance, store, "qjs_get_undefined")?,
            qjs_get_null: optional_typed(instance, store, "qjs_get_null")?,
            qjs_get_true: optional_typed(instance, store, "qjs_get_true")?,
            qjs_get_false: optional_typed(instance, store, "qjs_get_false")?,
        })
    }
}

pub(in crate::runtime::instantiate) struct PropertyExports {
    pub(in crate::runtime::instantiate) qjs_get_global: TypedFunc<(), i32>,
    pub(in crate::runtime::instantiate) qjs_get_prop_string: TypedFunc<(i32, i32), i32>,
    pub(in crate::runtime::instantiate) qjs_set_prop_string:
        Option<TypedFunc<(i32, i32, i32), i32>>,
    pub(in crate::runtime::instantiate) qjs_call: TypedFunc<(i32, i32, i32, i32), i32>,
}

impl PropertyExports {
    pub(in crate::runtime::instantiate) fn bind(
        instance: &Instance,
        store: &mut Store<HostState>,
    ) -> Result<Self> {
        Ok(Self {
            qjs_get_global: typed(instance, store, "qjs_get_global")?,
            qjs_get_prop_string: typed(instance, store, "qjs_get_prop_string")?,
            qjs_set_prop_string: optional_typed(instance, store, "qjs_set_prop_string")?,
            qjs_call: typed(instance, store, "qjs_call")?,
        })
    }
}
