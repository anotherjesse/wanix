use crate::host::HostState;
use wasmtime::{Global, Memory, Store, TypedFunc};

type QjsCompileFunc = TypedFunc<(i32, i32, i32, i32, i32, i32), i32>;

mod binary;
mod bytecode;
mod cleanup;
mod create;
mod event_loop;
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

pub use event_loop::QuickJsEventLoopStatus;

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
    js_std_loop_once: Option<TypedFunc<i32, i32>>,
    js_std_poll_io: Option<TypedFunc<(i32, i32), i32>>,
    qjs_get_runtime_ptr: TypedFunc<(), i32>,
    qjs_get_context_ptr: TypedFunc<(), i32>,
    qjs_set_runtime_and_context: TypedFunc<(i32, i32), ()>,
    wasm_malloc: TypedFunc<i32, i32>,
    wasm_free: TypedFunc<i32, ()>,
}

impl QuickJsRuntime {
    /// Returns bytes captured from WASI stdout writes.
    ///
    /// This buffer is populated only when
    /// [`crate::QuickJsHostConfig::with_stdout_capture`] is enabled for the runtime.
    /// [`crate::QuickJsHostConfig::with_limited_stdout_capture`] and
    /// [`crate::QuickJsHostConfig::with_limited_stdio_capture`] enable capture with a
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
    /// [`crate::QuickJsHostConfig::with_stderr_capture`] is enabled for the runtime.
    /// [`crate::QuickJsHostConfig::with_limited_stderr_capture`] and
    /// [`crate::QuickJsHostConfig::with_limited_stdio_capture`] enable capture with a
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

impl Drop for QuickJsRuntime {
    fn drop(&mut self) {
        // Drop cannot surface guest cleanup errors, so runtime destruction is best-effort.
        if self.store.data().process_exited() {
            return;
        }
        let _ = self.qjs_destroy.call(&mut self.store, ());
    }
}
