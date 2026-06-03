use crate::guest::guest_i32;
use crate::host::HostState;
use crate::{QuickJsCreateOptions, QuickJsHostConfig, QuickJsIntrinsics, QuickJsModule};
use anyhow::{Result, bail};
use std::path::Path;
use wasmtime::error::Context as _;
use wasmtime::{Engine, Global, Memory, Store, TypedFunc};

type QjsCompileFunc = TypedFunc<(i32, i32, i32, i32, i32, i32), i32>;

mod binary;
mod bytecode;
mod cleanup;
mod exception;
mod guest_memory;
mod host_callback;
mod instantiate;
mod limits;
mod module_loader;
mod promise_rejection;
mod raw_value;
mod snapshot_lifecycle;
mod value;

/// A live QuickJS runtime hosted inside a WASI WebAssembly instance.
///
/// Public methods only expose ordinary Rust values and opaque [`crate::Snapshot`]s.
/// Guest pointers, raw QuickJS handles, and linear-memory capabilities stay
/// private to the runtime implementation.
pub struct QuickJsRuntime {
    store: Store<HostState>,
    memory: Memory,
    stack_pointer: Global,
    wasm_sha256: [u8; 32],
    qjs_destroy: TypedFunc<(), ()>,
    qjs_eval: TypedFunc<(i32, i32, i32, i32), i32>,
    qjs_compile: Option<QjsCompileFunc>,
    qjs_free_bytecode: Option<TypedFunc<i32, ()>>,
    qjs_eval_bytecode: Option<TypedFunc<(i32, i32), i32>>,
    qjs_new_string: TypedFunc<(i32, i32), i32>,
    qjs_new_array_buffer: Option<TypedFunc<(i32, i32), i32>>,
    qjs_new_uint8_array: Option<TypedFunc<(i32, i32), i32>>,
    qjs_new_typed_array: Option<TypedFunc<(i32, i32, i32), i32>>,
    qjs_new_data_view: Option<TypedFunc<(i32, i32), i32>>,
    qjs_new_number: Option<TypedFunc<f64, i32>>,
    qjs_new_big_int64: Option<TypedFunc<(i32, i32), i32>>,
    qjs_new_host_function: Option<TypedFunc<(i32, i32, i32), i32>>,
    qjs_set_interrupt_handler: Option<TypedFunc<i32, ()>>,
    qjs_set_module_loader: Option<TypedFunc<i32, ()>>,
    qjs_set_memory_limit: Option<TypedFunc<i32, ()>>,
    qjs_set_max_stack_size: Option<TypedFunc<i32, ()>>,
    qjs_run_gc: Option<TypedFunc<(), ()>>,
    qjs_set_gc_threshold: Option<TypedFunc<i32, ()>>,
    qjs_get_gc_threshold: Option<TypedFunc<(), i32>>,
    qjs_compute_memory_usage: Option<TypedFunc<i32, ()>>,
    qjs_set_promise_rejection_handler: Option<TypedFunc<i32, ()>>,
    qjs_get_undefined: TypedFunc<(), i32>,
    qjs_get_null: Option<TypedFunc<(), i32>>,
    qjs_get_true: Option<TypedFunc<(), i32>>,
    qjs_get_false: Option<TypedFunc<(), i32>>,
    qjs_get_global: TypedFunc<(), i32>,
    qjs_get_prop_string: TypedFunc<(i32, i32), i32>,
    qjs_set_prop_string: Option<TypedFunc<(i32, i32, i32), i32>>,
    qjs_call: TypedFunc<(i32, i32, i32, i32), i32>,
    qjs_is_exception: TypedFunc<i32, i32>,
    qjs_is_undefined: Option<TypedFunc<i32, i32>>,
    qjs_is_null: Option<TypedFunc<i32, i32>>,
    qjs_is_bool: Option<TypedFunc<i32, i32>>,
    qjs_is_number: TypedFunc<i32, i32>,
    qjs_is_string: TypedFunc<i32, i32>,
    qjs_is_big_int: Option<TypedFunc<i32, i32>>,
    qjs_is_array_buffer: Option<TypedFunc<i32, i32>>,
    qjs_is_uint8_array: Option<TypedFunc<i32, i32>>,
    qjs_get_typed_array_type: Option<TypedFunc<i32, i32>>,
    qjs_is_data_view: Option<TypedFunc<i32, i32>>,
    qjs_get_exception: TypedFunc<(), i32>,
    qjs_throw: Option<TypedFunc<i32, i32>>,
    qjs_get_bool: Option<TypedFunc<i32, i32>>,
    qjs_get_float64: TypedFunc<i32, f64>,
    qjs_get_big_int64: Option<TypedFunc<(i32, i32, i32), i32>>,
    qjs_get_string: TypedFunc<i32, i32>,
    qjs_get_array_buffer: Option<TypedFunc<(i32, i32), i32>>,
    qjs_get_uint8_array: Option<TypedFunc<(i32, i32), i32>>,
    qjs_get_typed_array_buffer: Option<TypedFunc<(i32, i32, i32, i32), i32>>,
    qjs_get_data_view_buffer: Option<TypedFunc<(i32, i32, i32), i32>>,
    qjs_free_cstring: TypedFunc<i32, ()>,
    qjs_free_value: TypedFunc<i32, ()>,
    qjs_is_job_pending: TypedFunc<(), i32>,
    qjs_execute_pending_job: TypedFunc<(), i32>,
    qjs_get_runtime_ptr: TypedFunc<(), i32>,
    qjs_get_context_ptr: TypedFunc<(), i32>,
    qjs_set_runtime_and_context: TypedFunc<(i32, i32), ()>,
    wasm_malloc: TypedFunc<i32, i32>,
    wasm_free: TypedFunc<i32, ()>,
}

impl QuickJsRuntime {
    /// Reads and compiles a QuickJS WebAssembly module from disk.
    ///
    /// This is a convenience wrapper around [`QuickJsModule::from_file`].
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read or Wasmtime cannot compile
    /// the module.
    pub fn module_from_file(engine: &Engine, path: impl AsRef<Path>) -> Result<QuickJsModule> {
        QuickJsModule::from_file(engine, path)
    }

    /// Creates and initializes a runtime with the default host configuration.
    ///
    /// Prefer [`QuickJsModule::create_runtime`](crate::QuickJsModule::create_runtime)
    /// when you already have a module.
    ///
    /// # Errors
    ///
    /// Returns an error if `engine` did not compile `module`, the wasm module
    /// cannot be instantiated, required exports are missing, or QuickJS
    /// initialization fails.
    pub fn create(engine: &Engine, module: &QuickJsModule) -> Result<Self> {
        Self::create_with_host_config(engine, module, QuickJsHostConfig::default())
    }

    /// Creates and initializes a runtime with explicit host import settings.
    ///
    /// Prefer
    /// [`QuickJsModule::create_runtime_with_host_config`](crate::QuickJsModule::create_runtime_with_host_config)
    /// when you already have a module.
    ///
    /// # Errors
    ///
    /// Returns an error if `engine` did not compile `module`, the wasm module
    /// cannot be instantiated, required exports are missing, or QuickJS
    /// initialization fails.
    pub fn create_with_host_config(
        engine: &Engine,
        module: &QuickJsModule,
        config: QuickJsHostConfig,
    ) -> Result<Self> {
        module.ensure_engine(engine)?;
        module.create_runtime_with_host_config(config)
    }

    /// Creates and initializes a runtime with explicit QuickJS create options.
    ///
    /// Prefer
    /// [`QuickJsModule::create_runtime_with_options`](crate::QuickJsModule::create_runtime_with_options)
    /// when you already have a module.
    ///
    /// # Errors
    ///
    /// Returns an error if `engine` did not compile `module`, the wasm module
    /// cannot be instantiated, required exports are missing, explicit
    /// intrinsic selection is requested without `qjs_init2`, or QuickJS
    /// initialization fails.
    pub fn create_with_options(
        engine: &Engine,
        module: &QuickJsModule,
        options: QuickJsCreateOptions,
    ) -> Result<Self> {
        module.ensure_engine(engine)?;
        module.create_runtime_with_options(options)
    }

    /// Creates and initializes a runtime with selective QuickJS intrinsics.
    ///
    /// Prefer
    /// [`QuickJsModule::create_runtime_with_intrinsics`](crate::QuickJsModule::create_runtime_with_intrinsics)
    /// when you already have a module.
    ///
    /// # Errors
    ///
    /// Returns an error if `engine` did not compile `module`, the wasm module
    /// cannot be instantiated, `qjs_init2` is missing or mistyped, or QuickJS
    /// initialization fails.
    pub fn create_with_intrinsics(
        engine: &Engine,
        module: &QuickJsModule,
        intrinsics: QuickJsIntrinsics,
    ) -> Result<Self> {
        Self::create_with_options(
            engine,
            module,
            QuickJsCreateOptions::new().with_intrinsics(intrinsics),
        )
    }

    pub(crate) fn create_for_module(
        module: &QuickJsModule,
        config: QuickJsHostConfig,
    ) -> Result<Self> {
        Self::create_for_module_with_options(
            module,
            QuickJsCreateOptions::new().with_host_config(config),
        )
    }

    pub(crate) fn create_for_module_with_options(
        module: &QuickJsModule,
        options: QuickJsCreateOptions,
    ) -> Result<Self> {
        let (config, intrinsics) = options.into_parts();
        let mut instantiated = Self::instantiate(module.engine(), module, config)?;

        instantiated
            .initialize
            .call(&mut instantiated.vm.store, ())
            .context("failed to initialize WASI reactor")?;

        let init_result = match intrinsics {
            Some(intrinsics) => call_qjs_init2(&mut instantiated, intrinsics)?,
            None => instantiated
                .qjs_init
                .call(&mut instantiated.vm.store, ())
                .context("failed to call qjs_init")?,
        };
        if init_result != 0 {
            bail!("QuickJS runtime initialization failed with code {init_result}");
        }

        Ok(instantiated.vm)
    }

    /// Returns bytes captured from WASI stdout writes.
    ///
    /// This buffer is populated only when
    /// [`QuickJsHostConfig::with_stdout_capture`] is enabled for the runtime.
    /// [`QuickJsHostConfig::with_limited_stdout_capture`] and
    /// [`QuickJsHostConfig::with_limited_stdio_capture`] enable capture with a
    /// retained byte limit.
    /// If stdout capture has a byte limit, writes that would exceed the
    /// retained buffer limit return an error.
    #[must_use]
    pub fn captured_stdout(&self) -> &[u8] {
        self.store.data().captured_stdout()
    }

    /// Takes and clears bytes captured from WASI stdout writes.
    ///
    /// Returns an empty buffer when stdout capture is disabled or no stdout
    /// writes have been observed. Clearing the buffer also resets the retained
    /// byte count used by stdout capture limits.
    #[must_use]
    pub fn take_captured_stdout(&mut self) -> Vec<u8> {
        self.store.data_mut().take_captured_stdout()
    }

    /// Returns bytes captured from WASI stderr writes.
    ///
    /// This buffer is populated only when
    /// [`QuickJsHostConfig::with_stderr_capture`] is enabled for the runtime.
    /// [`QuickJsHostConfig::with_limited_stderr_capture`] and
    /// [`QuickJsHostConfig::with_limited_stdio_capture`] enable capture with a
    /// retained byte limit.
    /// If stderr capture has a byte limit, writes that would exceed the
    /// retained buffer limit return an error.
    #[must_use]
    pub fn captured_stderr(&self) -> &[u8] {
        self.store.data().captured_stderr()
    }

    /// Takes and clears bytes captured from WASI stderr writes.
    ///
    /// Returns an empty buffer when stderr capture is disabled or no stderr
    /// writes have been observed. Clearing the buffer also resets the retained
    /// byte count used by stderr capture limits.
    #[must_use]
    pub fn take_captured_stderr(&mut self) -> Vec<u8> {
        self.store.data_mut().take_captured_stderr()
    }
}

fn call_qjs_init2(
    instantiated: &mut instantiate::InstantiatedRuntime,
    intrinsics: QuickJsIntrinsics,
) -> Result<i32> {
    let qjs_init2 = instantiated.qjs_init2.clone().ok_or_else(|| {
        anyhow::anyhow!(
            "QuickJS WASM module does not export qjs_init2; configurable intrinsics are not supported"
        )
    })?;
    Ok(qjs_init2
        .call(&mut instantiated.vm.store, guest_i32(intrinsics.bits()))
        .context("failed to call qjs_init2")?)
}

impl Drop for QuickJsRuntime {
    fn drop(&mut self) {
        // Drop cannot surface guest cleanup errors, so runtime destruction is best-effort.
        let _ = self.qjs_destroy.call(&mut self.store, ());
    }
}
