use crate::host::HostState;
use anyhow::{Result, anyhow};
use wasmtime::error::Context as _;
use wasmtime::{Global, Instance, Memory, Store, TypedFunc};

type QjsCompileFunc = TypedFunc<(i32, i32, i32, i32, i32, i32), i32>;

pub(super) struct RuntimeInitExports {
    pub(super) initialize: TypedFunc<(), ()>,
    pub(super) qjs_init: TypedFunc<(), i32>,
    pub(super) qjs_init2: Option<TypedFunc<i32, i32>>,
}

impl RuntimeInitExports {
    pub(super) fn bind(instance: &Instance, store: &mut Store<HostState>) -> Result<Self> {
        Ok(Self {
            initialize: instance
                .get_typed_func::<(), ()>(&mut *store, "_initialize")
                .context("missing _initialize export")?,
            qjs_init: instance
                .get_typed_func::<(), i32>(&mut *store, "qjs_init")
                .context("missing qjs_init export")?,
            qjs_init2: optional_typed(instance, store, "qjs_init2")?,
        })
    }
}
pub(super) struct RuntimeExports {
    pub(super) lifetime: LifetimeExports,
    pub(super) bytecode: BytecodeExports,
    pub(super) constructors: ConstructorExports,
    pub(super) host: HostControlExports,
    pub(super) memory_policy: MemoryPolicyExports,
    pub(super) values: ValueConstantExports,
    pub(super) properties: PropertyExports,
    pub(super) type_checks: TypeCheckExports,
    pub(super) accessors: AccessorExports,
    pub(super) jobs: JobExports,
    pub(super) snapshot: SnapshotExports,
    pub(super) wasm_memory: WasmMemoryExports,
}

impl RuntimeExports {
    pub(super) fn bind(instance: &Instance, store: &mut Store<HostState>) -> Result<Self> {
        Ok(Self {
            lifetime: LifetimeExports::bind(instance, store)?,
            bytecode: BytecodeExports::bind(instance, store)?,
            constructors: ConstructorExports::bind(instance, store)?,
            host: HostControlExports::bind(instance, store)?,
            memory_policy: MemoryPolicyExports::bind(instance, store)?,
            values: ValueConstantExports::bind(instance, store)?,
            properties: PropertyExports::bind(instance, store)?,
            type_checks: TypeCheckExports::bind(instance, store)?,
            accessors: AccessorExports::bind(instance, store)?,
            jobs: JobExports::bind(instance, store)?,
            snapshot: SnapshotExports::bind(instance, store)?,
            wasm_memory: WasmMemoryExports::bind(instance, store)?,
        })
    }
}
pub(super) struct LifetimeExports {
    pub(super) qjs_destroy: TypedFunc<(), ()>,
    pub(super) qjs_eval: TypedFunc<(i32, i32, i32, i32), i32>,
    pub(super) qjs_free_cstring: TypedFunc<i32, ()>,
    pub(super) qjs_free_value: TypedFunc<i32, ()>,
}

impl LifetimeExports {
    fn bind(instance: &Instance, store: &mut Store<HostState>) -> Result<Self> {
        Ok(Self {
            qjs_destroy: typed(instance, store, "qjs_destroy")?,
            qjs_eval: typed(instance, store, "qjs_eval")?,
            qjs_free_cstring: typed(instance, store, "qjs_free_cstring")?,
            qjs_free_value: typed(instance, store, "qjs_free_value")?,
        })
    }
}
pub(super) struct BytecodeExports {
    pub(super) qjs_compile: Option<QjsCompileFunc>,
    pub(super) qjs_free_bytecode: Option<TypedFunc<i32, ()>>,
    pub(super) qjs_eval_bytecode: Option<TypedFunc<(i32, i32), i32>>,
}

impl BytecodeExports {
    fn bind(instance: &Instance, store: &mut Store<HostState>) -> Result<Self> {
        Ok(Self {
            qjs_compile: optional_typed(instance, store, "qjs_compile")?,
            qjs_free_bytecode: optional_typed(instance, store, "qjs_free_bytecode")?,
            qjs_eval_bytecode: optional_typed(instance, store, "qjs_eval_bytecode")?,
        })
    }
}
pub(super) struct ConstructorExports {
    pub(super) qjs_new_string: TypedFunc<(i32, i32), i32>,
    pub(super) qjs_new_array_buffer: Option<TypedFunc<(i32, i32), i32>>,
    pub(super) qjs_new_uint8_array: Option<TypedFunc<(i32, i32), i32>>,
    pub(super) qjs_new_typed_array: Option<TypedFunc<(i32, i32, i32), i32>>,
    pub(super) qjs_new_data_view: Option<TypedFunc<(i32, i32), i32>>,
    pub(super) qjs_new_number: Option<TypedFunc<f64, i32>>,
    pub(super) qjs_new_big_int64: Option<TypedFunc<(i32, i32), i32>>,
}

impl ConstructorExports {
    fn bind(instance: &Instance, store: &mut Store<HostState>) -> Result<Self> {
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
pub(super) struct HostControlExports {
    pub(super) qjs_new_host_function: Option<TypedFunc<(i32, i32, i32), i32>>,
    pub(super) qjs_set_interrupt_handler: Option<TypedFunc<i32, ()>>,
    pub(super) qjs_set_module_loader: Option<TypedFunc<i32, ()>>,
    pub(super) qjs_set_memory_limit: Option<TypedFunc<i32, ()>>,
    pub(super) qjs_set_max_stack_size: Option<TypedFunc<i32, ()>>,
    pub(super) qjs_set_promise_rejection_handler: Option<TypedFunc<i32, ()>>,
}

impl HostControlExports {
    fn bind(instance: &Instance, store: &mut Store<HostState>) -> Result<Self> {
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
pub(super) struct MemoryPolicyExports {
    pub(super) qjs_run_gc: Option<TypedFunc<(), ()>>,
    pub(super) qjs_set_gc_threshold: Option<TypedFunc<i32, ()>>,
    pub(super) qjs_get_gc_threshold: Option<TypedFunc<(), i32>>,
    pub(super) qjs_compute_memory_usage: Option<TypedFunc<i32, ()>>,
}

impl MemoryPolicyExports {
    fn bind(instance: &Instance, store: &mut Store<HostState>) -> Result<Self> {
        Ok(Self {
            qjs_run_gc: optional_typed(instance, store, "qjs_run_gc")?,
            qjs_set_gc_threshold: optional_typed(instance, store, "qjs_set_gc_threshold")?,
            qjs_get_gc_threshold: optional_typed(instance, store, "qjs_get_gc_threshold")?,
            qjs_compute_memory_usage: optional_typed(instance, store, "qjs_compute_memory_usage")?,
        })
    }
}
pub(super) struct ValueConstantExports {
    pub(super) qjs_get_undefined: TypedFunc<(), i32>,
    pub(super) qjs_get_null: Option<TypedFunc<(), i32>>,
    pub(super) qjs_get_true: Option<TypedFunc<(), i32>>,
    pub(super) qjs_get_false: Option<TypedFunc<(), i32>>,
}

impl ValueConstantExports {
    fn bind(instance: &Instance, store: &mut Store<HostState>) -> Result<Self> {
        Ok(Self {
            qjs_get_undefined: typed(instance, store, "qjs_get_undefined")?,
            qjs_get_null: optional_typed(instance, store, "qjs_get_null")?,
            qjs_get_true: optional_typed(instance, store, "qjs_get_true")?,
            qjs_get_false: optional_typed(instance, store, "qjs_get_false")?,
        })
    }
}
pub(super) struct PropertyExports {
    pub(super) qjs_get_global: TypedFunc<(), i32>,
    pub(super) qjs_get_prop_string: TypedFunc<(i32, i32), i32>,
    pub(super) qjs_set_prop_string: Option<TypedFunc<(i32, i32, i32), i32>>,
    pub(super) qjs_call: TypedFunc<(i32, i32, i32, i32), i32>,
}

impl PropertyExports {
    fn bind(instance: &Instance, store: &mut Store<HostState>) -> Result<Self> {
        Ok(Self {
            qjs_get_global: typed(instance, store, "qjs_get_global")?,
            qjs_get_prop_string: typed(instance, store, "qjs_get_prop_string")?,
            qjs_set_prop_string: optional_typed(instance, store, "qjs_set_prop_string")?,
            qjs_call: typed(instance, store, "qjs_call")?,
        })
    }
}
pub(super) struct TypeCheckExports {
    pub(super) qjs_is_exception: TypedFunc<i32, i32>,
    pub(super) qjs_is_undefined: Option<TypedFunc<i32, i32>>,
    pub(super) qjs_is_null: Option<TypedFunc<i32, i32>>,
    pub(super) qjs_is_bool: Option<TypedFunc<i32, i32>>,
    pub(super) qjs_is_number: TypedFunc<i32, i32>,
    pub(super) qjs_is_string: TypedFunc<i32, i32>,
    pub(super) qjs_is_big_int: Option<TypedFunc<i32, i32>>,
    pub(super) qjs_is_array_buffer: Option<TypedFunc<i32, i32>>,
    pub(super) qjs_is_uint8_array: Option<TypedFunc<i32, i32>>,
    pub(super) qjs_get_typed_array_type: Option<TypedFunc<i32, i32>>,
    pub(super) qjs_is_data_view: Option<TypedFunc<i32, i32>>,
}

impl TypeCheckExports {
    fn bind(instance: &Instance, store: &mut Store<HostState>) -> Result<Self> {
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
pub(super) struct AccessorExports {
    pub(super) qjs_get_exception: TypedFunc<(), i32>,
    pub(super) qjs_throw: Option<TypedFunc<i32, i32>>,
    pub(super) qjs_get_bool: Option<TypedFunc<i32, i32>>,
    pub(super) qjs_get_float64: TypedFunc<i32, f64>,
    pub(super) qjs_get_big_int64: Option<TypedFunc<(i32, i32, i32), i32>>,
    pub(super) qjs_get_string: TypedFunc<i32, i32>,
    pub(super) qjs_get_array_buffer: Option<TypedFunc<(i32, i32), i32>>,
    pub(super) qjs_get_uint8_array: Option<TypedFunc<(i32, i32), i32>>,
    pub(super) qjs_get_typed_array_buffer: Option<TypedFunc<(i32, i32, i32, i32), i32>>,
    pub(super) qjs_get_data_view_buffer: Option<TypedFunc<(i32, i32, i32), i32>>,
}

impl AccessorExports {
    fn bind(instance: &Instance, store: &mut Store<HostState>) -> Result<Self> {
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
pub(super) struct JobExports {
    pub(super) qjs_is_job_pending: TypedFunc<(), i32>,
    pub(super) qjs_execute_pending_job: TypedFunc<(), i32>,
    pub(super) js_std_loop_once: Option<TypedFunc<i32, i32>>,
    pub(super) js_std_poll_io: Option<TypedFunc<(i32, i32), i32>>,
}

impl JobExports {
    fn bind(instance: &Instance, store: &mut Store<HostState>) -> Result<Self> {
        Ok(Self {
            qjs_is_job_pending: typed(instance, store, "qjs_is_job_pending")?,
            qjs_execute_pending_job: typed(instance, store, "qjs_execute_pending_job")?,
            js_std_loop_once: optional_typed(instance, store, "js_std_loop_once")?,
            js_std_poll_io: optional_typed(instance, store, "js_std_poll_io")?,
        })
    }
}
pub(super) struct SnapshotExports {
    pub(super) qjs_get_runtime_ptr: TypedFunc<(), i32>,
    pub(super) qjs_get_context_ptr: TypedFunc<(), i32>,
    pub(super) qjs_set_runtime_and_context: TypedFunc<(i32, i32), ()>,
}

impl SnapshotExports {
    fn bind(instance: &Instance, store: &mut Store<HostState>) -> Result<Self> {
        Ok(Self {
            qjs_get_runtime_ptr: typed(instance, store, "qjs_get_runtime_ptr")?,
            qjs_get_context_ptr: typed(instance, store, "qjs_get_context_ptr")?,
            qjs_set_runtime_and_context: typed(instance, store, "qjs_set_runtime_and_context")?,
        })
    }
}
pub(super) struct WasmMemoryExports {
    pub(super) wasm_malloc: TypedFunc<i32, i32>,
    pub(super) wasm_free: TypedFunc<i32, ()>,
}

impl WasmMemoryExports {
    fn bind(instance: &Instance, store: &mut Store<HostState>) -> Result<Self> {
        Ok(Self {
            wasm_malloc: typed(instance, store, "wasm_malloc")?,
            wasm_free: typed(instance, store, "wasm_free")?,
        })
    }
}
pub(super) fn bind_memory(instance: &Instance, store: &mut Store<HostState>) -> Result<Memory> {
    let memory = instance
        .get_memory(&mut *store, "memory")
        .ok_or_else(|| anyhow!("QuickJS WASM module does not export memory"))?;
    store.data_mut().set_memory(memory);
    Ok(memory)
}

pub(super) fn bind_stack_pointer(
    instance: &Instance,
    store: &mut Store<HostState>,
) -> Result<Global> {
    instance
        .get_global(store, "__stack_pointer")
        .ok_or_else(|| anyhow!("QuickJS WASM module does not export __stack_pointer"))
}

fn typed<P, R>(
    instance: &Instance,
    store: &mut Store<HostState>,
    name: &str,
) -> Result<TypedFunc<P, R>>
where
    P: wasmtime::WasmParams,
    R: wasmtime::WasmResults,
{
    Ok(instance
        .get_typed_func::<P, R>(&mut *store, name)
        .with_context(|| format!("missing or mistyped {name} export"))?)
}

fn optional_typed<P, R>(
    instance: &Instance,
    store: &mut Store<HostState>,
    name: &str,
) -> Result<Option<TypedFunc<P, R>>>
where
    P: wasmtime::WasmParams,
    R: wasmtime::WasmResults,
{
    if instance.get_func(&mut *store, name).is_none() {
        return Ok(None);
    }
    typed(instance, store, name).map(Some)
}
