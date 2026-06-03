use super::QuickJsRuntime;
use crate::host::{HostState, QuickJsWasiHostHandle, define_env_imports, define_wasi_imports};
use crate::{QuickJsHostConfig, QuickJsModule};
use anyhow::{Result, anyhow};
use wasmtime::error::Context as _;
use wasmtime::{Engine, Instance, Linker, Store, TypedFunc};

pub(super) struct InstantiatedRuntime {
    pub(super) vm: QuickJsRuntime,
    pub(super) initialize: TypedFunc<(), ()>,
    pub(super) qjs_init: TypedFunc<(), i32>,
    pub(super) qjs_init2: Option<TypedFunc<i32, i32>>,
}

impl QuickJsRuntime {
    pub(super) fn instantiate(
        engine: &Engine,
        module: &QuickJsModule,
        config: QuickJsHostConfig,
        wasi_host: Option<QuickJsWasiHostHandle>,
    ) -> Result<InstantiatedRuntime> {
        let mut linker = Linker::<HostState>::new(engine);
        define_env_imports(&mut linker)?;
        define_wasi_imports(&mut linker)?;

        let mut store = Store::new(engine, HostState::new_with_wasi_host(config, wasi_host));
        let instance = linker
            .instantiate(&mut store, module.wasmtime_module())
            .context("failed to instantiate QuickJS WASM module")?;

        let memory = instance
            .get_memory(&mut store, "memory")
            .ok_or_else(|| anyhow!("QuickJS WASM module does not export memory"))?;
        store.data_mut().set_memory(memory);

        let stack_pointer = instance
            .get_global(&mut store, "__stack_pointer")
            .ok_or_else(|| anyhow!("QuickJS WASM module does not export __stack_pointer"))?;

        let initialize = instance
            .get_typed_func::<(), ()>(&mut store, "_initialize")
            .context("missing _initialize export")?;
        let qjs_init = instance
            .get_typed_func::<(), i32>(&mut store, "qjs_init")
            .context("missing qjs_init export")?;
        let qjs_init2 = optional_typed(&instance, &mut store, "qjs_init2")?;

        let vm = Self {
            qjs_destroy: typed(&instance, &mut store, "qjs_destroy")?,
            qjs_eval: typed(&instance, &mut store, "qjs_eval")?,
            qjs_compile: optional_typed(&instance, &mut store, "qjs_compile")?,
            qjs_free_bytecode: optional_typed(&instance, &mut store, "qjs_free_bytecode")?,
            qjs_eval_bytecode: optional_typed(&instance, &mut store, "qjs_eval_bytecode")?,
            qjs_new_string: typed(&instance, &mut store, "qjs_new_string")?,
            qjs_new_array_buffer: optional_typed(&instance, &mut store, "qjs_new_array_buffer")?,
            qjs_new_uint8_array: optional_typed(&instance, &mut store, "qjs_new_uint8_array")?,
            qjs_new_typed_array: optional_typed(&instance, &mut store, "qjs_new_typed_array")?,
            qjs_new_data_view: optional_typed(&instance, &mut store, "qjs_new_data_view")?,
            qjs_new_number: optional_typed(&instance, &mut store, "qjs_new_number")?,
            qjs_new_big_int64: optional_typed(&instance, &mut store, "qjs_new_big_int64")?,
            qjs_new_host_function: optional_typed(&instance, &mut store, "qjs_new_host_function")?,
            qjs_set_interrupt_handler: optional_typed(
                &instance,
                &mut store,
                "qjs_set_interrupt_handler",
            )?,
            qjs_set_module_loader: optional_typed(&instance, &mut store, "qjs_set_module_loader")?,
            qjs_set_memory_limit: optional_typed(&instance, &mut store, "qjs_set_memory_limit")?,
            qjs_set_max_stack_size: optional_typed(
                &instance,
                &mut store,
                "qjs_set_max_stack_size",
            )?,
            qjs_run_gc: optional_typed(&instance, &mut store, "qjs_run_gc")?,
            qjs_set_gc_threshold: optional_typed(&instance, &mut store, "qjs_set_gc_threshold")?,
            qjs_get_gc_threshold: optional_typed(&instance, &mut store, "qjs_get_gc_threshold")?,
            qjs_compute_memory_usage: optional_typed(
                &instance,
                &mut store,
                "qjs_compute_memory_usage",
            )?,
            qjs_set_promise_rejection_handler: optional_typed(
                &instance,
                &mut store,
                "qjs_set_promise_rejection_handler",
            )?,
            qjs_get_undefined: typed(&instance, &mut store, "qjs_get_undefined")?,
            qjs_get_null: optional_typed(&instance, &mut store, "qjs_get_null")?,
            qjs_get_true: optional_typed(&instance, &mut store, "qjs_get_true")?,
            qjs_get_false: optional_typed(&instance, &mut store, "qjs_get_false")?,
            qjs_get_global: typed(&instance, &mut store, "qjs_get_global")?,
            qjs_get_prop_string: typed(&instance, &mut store, "qjs_get_prop_string")?,
            qjs_set_prop_string: optional_typed(&instance, &mut store, "qjs_set_prop_string")?,
            qjs_call: typed(&instance, &mut store, "qjs_call")?,
            qjs_is_exception: typed(&instance, &mut store, "qjs_is_exception")?,
            qjs_is_undefined: optional_typed(&instance, &mut store, "qjs_is_undefined")?,
            qjs_is_null: optional_typed(&instance, &mut store, "qjs_is_null")?,
            qjs_is_bool: optional_typed(&instance, &mut store, "qjs_is_bool")?,
            qjs_is_number: typed(&instance, &mut store, "qjs_is_number")?,
            qjs_is_string: typed(&instance, &mut store, "qjs_is_string")?,
            qjs_is_big_int: optional_typed(&instance, &mut store, "qjs_is_big_int")?,
            qjs_is_array_buffer: optional_typed(&instance, &mut store, "qjs_is_array_buffer")?,
            qjs_is_uint8_array: optional_typed(&instance, &mut store, "qjs_is_uint8_array")?,
            qjs_get_typed_array_type: optional_typed(
                &instance,
                &mut store,
                "qjs_get_typed_array_type",
            )?,
            qjs_is_data_view: optional_typed(&instance, &mut store, "qjs_is_data_view")?,
            qjs_get_exception: typed(&instance, &mut store, "qjs_get_exception")?,
            qjs_throw: optional_typed(&instance, &mut store, "qjs_throw")?,
            qjs_get_bool: optional_typed(&instance, &mut store, "qjs_get_bool")?,
            qjs_get_float64: typed(&instance, &mut store, "qjs_get_float64")?,
            qjs_get_big_int64: optional_typed(&instance, &mut store, "qjs_get_big_int64")?,
            qjs_get_string: typed(&instance, &mut store, "qjs_get_string")?,
            qjs_get_array_buffer: optional_typed(&instance, &mut store, "qjs_get_array_buffer")?,
            qjs_get_uint8_array: optional_typed(&instance, &mut store, "qjs_get_uint8_array")?,
            qjs_get_typed_array_buffer: optional_typed(
                &instance,
                &mut store,
                "qjs_get_typed_array_buffer",
            )?,
            qjs_get_data_view_buffer: optional_typed(
                &instance,
                &mut store,
                "qjs_get_data_view_buffer",
            )?,
            qjs_free_cstring: typed(&instance, &mut store, "qjs_free_cstring")?,
            qjs_free_value: typed(&instance, &mut store, "qjs_free_value")?,
            qjs_is_job_pending: typed(&instance, &mut store, "qjs_is_job_pending")?,
            qjs_execute_pending_job: typed(&instance, &mut store, "qjs_execute_pending_job")?,
            js_std_loop_once: optional_typed(&instance, &mut store, "js_std_loop_once")?,
            js_std_poll_io: optional_typed(&instance, &mut store, "js_std_poll_io")?,
            qjs_get_runtime_ptr: typed(&instance, &mut store, "qjs_get_runtime_ptr")?,
            qjs_get_context_ptr: typed(&instance, &mut store, "qjs_get_context_ptr")?,
            qjs_set_runtime_and_context: typed(
                &instance,
                &mut store,
                "qjs_set_runtime_and_context",
            )?,
            wasm_malloc: typed(&instance, &mut store, "wasm_malloc")?,
            wasm_free: typed(&instance, &mut store, "wasm_free")?,
            store,
            memory,
            stack_pointer,
            wasm_sha256: module.wasm_sha256(),
        };

        Ok(InstantiatedRuntime {
            vm,
            initialize,
            qjs_init,
            qjs_init2,
        })
    }
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
