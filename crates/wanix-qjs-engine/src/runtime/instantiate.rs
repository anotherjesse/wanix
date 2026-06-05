use super::QuickJsRuntime;
use crate::host::{HostState, QuickJsWasiHostHandle, define_env_imports, define_wasi_imports};
use crate::{QuickJsHostConfig, QuickJsModule};
use anyhow::Result;
use wasmtime::error::Context as _;
use wasmtime::{Engine, Linker, Store, TypedFunc};

mod exports;

use exports::{RuntimeExports, RuntimeInitExports, bind_memory, bind_stack_pointer};

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

        let exports = RuntimeExports::bind(&instance, &mut store)?;
        let memory = bind_memory(&instance, &mut store)?;
        let stack_pointer = bind_stack_pointer(&instance, &mut store)?;
        let init = RuntimeInitExports::bind(&instance, &mut store)?;

        let vm = Self {
            qjs_destroy: exports.lifetime.qjs_destroy,
            qjs_eval: exports.lifetime.qjs_eval,
            qjs_compile: exports.bytecode.qjs_compile,
            qjs_free_bytecode: exports.bytecode.qjs_free_bytecode,
            qjs_eval_bytecode: exports.bytecode.qjs_eval_bytecode,
            qjs_new_string: exports.constructors.qjs_new_string,
            qjs_new_array_buffer: exports.constructors.qjs_new_array_buffer,
            qjs_new_uint8_array: exports.constructors.qjs_new_uint8_array,
            qjs_new_typed_array: exports.constructors.qjs_new_typed_array,
            qjs_new_data_view: exports.constructors.qjs_new_data_view,
            qjs_new_number: exports.constructors.qjs_new_number,
            qjs_new_big_int64: exports.constructors.qjs_new_big_int64,
            qjs_new_host_function: exports.host.qjs_new_host_function,
            qjs_set_interrupt_handler: exports.host.qjs_set_interrupt_handler,
            qjs_set_module_loader: exports.host.qjs_set_module_loader,
            qjs_set_memory_limit: exports.host.qjs_set_memory_limit,
            qjs_set_max_stack_size: exports.host.qjs_set_max_stack_size,
            qjs_run_gc: exports.memory_policy.qjs_run_gc,
            qjs_set_gc_threshold: exports.memory_policy.qjs_set_gc_threshold,
            qjs_get_gc_threshold: exports.memory_policy.qjs_get_gc_threshold,
            qjs_compute_memory_usage: exports.memory_policy.qjs_compute_memory_usage,
            qjs_set_promise_rejection_handler: exports.host.qjs_set_promise_rejection_handler,
            qjs_get_undefined: exports.values.qjs_get_undefined,
            qjs_get_null: exports.values.qjs_get_null,
            qjs_get_true: exports.values.qjs_get_true,
            qjs_get_false: exports.values.qjs_get_false,
            qjs_get_global: exports.properties.qjs_get_global,
            qjs_get_prop_string: exports.properties.qjs_get_prop_string,
            qjs_set_prop_string: exports.properties.qjs_set_prop_string,
            qjs_call: exports.properties.qjs_call,
            qjs_is_exception: exports.type_checks.qjs_is_exception,
            qjs_is_undefined: exports.type_checks.qjs_is_undefined,
            qjs_is_null: exports.type_checks.qjs_is_null,
            qjs_is_bool: exports.type_checks.qjs_is_bool,
            qjs_is_number: exports.type_checks.qjs_is_number,
            qjs_is_string: exports.type_checks.qjs_is_string,
            qjs_is_big_int: exports.type_checks.qjs_is_big_int,
            qjs_is_array_buffer: exports.type_checks.qjs_is_array_buffer,
            qjs_is_uint8_array: exports.type_checks.qjs_is_uint8_array,
            qjs_get_typed_array_type: exports.type_checks.qjs_get_typed_array_type,
            qjs_is_data_view: exports.type_checks.qjs_is_data_view,
            qjs_get_exception: exports.accessors.qjs_get_exception,
            qjs_throw: exports.accessors.qjs_throw,
            qjs_get_bool: exports.accessors.qjs_get_bool,
            qjs_get_float64: exports.accessors.qjs_get_float64,
            qjs_get_big_int64: exports.accessors.qjs_get_big_int64,
            qjs_get_string: exports.accessors.qjs_get_string,
            qjs_get_array_buffer: exports.accessors.qjs_get_array_buffer,
            qjs_get_uint8_array: exports.accessors.qjs_get_uint8_array,
            qjs_get_typed_array_buffer: exports.accessors.qjs_get_typed_array_buffer,
            qjs_get_data_view_buffer: exports.accessors.qjs_get_data_view_buffer,
            qjs_free_cstring: exports.lifetime.qjs_free_cstring,
            qjs_free_value: exports.lifetime.qjs_free_value,
            qjs_is_job_pending: exports.jobs.qjs_is_job_pending,
            qjs_execute_pending_job: exports.jobs.qjs_execute_pending_job,
            js_std_loop_once: exports.jobs.js_std_loop_once,
            js_std_poll_io: exports.jobs.js_std_poll_io,
            qjs_get_runtime_ptr: exports.snapshot.qjs_get_runtime_ptr,
            qjs_get_context_ptr: exports.snapshot.qjs_get_context_ptr,
            qjs_set_runtime_and_context: exports.snapshot.qjs_set_runtime_and_context,
            wasm_malloc: exports.wasm_memory.wasm_malloc,
            wasm_free: exports.wasm_memory.wasm_free,
            store,
            memory,
            stack_pointer,
            wasm_sha256: module.wasm_sha256(),
        };

        Ok(InstantiatedRuntime {
            vm,
            initialize: init.initialize,
            qjs_init: init.qjs_init,
            qjs_init2: init.qjs_init2,
        })
    }
}
