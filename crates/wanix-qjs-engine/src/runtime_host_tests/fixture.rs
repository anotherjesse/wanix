use super::*;
use crate::snapshot::{QUICKJS_WASM_ABI_VERSION, SNAPSHOT_FORMAT_VERSION};
use anyhow::{Result, bail};
use wasmtime::Engine;

pub(super) const STDIO_RUNTIME_WAT: &str = r#"
(module
  (import "wasi_snapshot_preview1" "fd_write" (func $fd_write (param i32 i32 i32 i32) (result i32)))

  (memory (export "memory") 1)
  (global $__stack_pointer (export "__stack_pointer") (mut i32) (i32.const 65536))
  (global $heap (mut i32) (i32.const 2048))

  (data (i32.const 1024) "stdio event\n")

  (func (export "_initialize"))

  (func (export "qjs_init") (result i32)
    i32.const 0)

  (func (export "qjs_destroy"))

  (func (export "qjs_eval")
    (param $code i32)
    (param $code_len i32)
    (param $filename i32)
    (param $flags i32)
    (result i32)
    i32.const 64
    i32.const 1024
    i32.store
    i32.const 68
    i32.const 12
    i32.store
    i32.const 1
    i32.const 64
    i32.const 1
    i32.const 80
    call $fd_write
    drop
    i32.const 4)

  (func (export "qjs_new_string") (param $ptr i32) (param $len i32) (result i32)
    i32.const 4)
  (func (export "qjs_get_undefined") (result i32)
    i32.const 4)
  (func (export "qjs_get_global") (result i32)
    i32.const 4)
  (func (export "qjs_get_prop_string") (param $global i32) (param $name i32) (result i32)
    i32.const 4)
  (func (export "qjs_call")
    (param $function i32)
    (param $this_value i32)
    (param $argc i32)
    (param $argv i32)
    (result i32)
    i32.const 4)
  (func (export "qjs_is_exception") (param $value i32) (result i32)
    i32.const 0)
  (func (export "qjs_is_number") (param $value i32) (result i32)
    i32.const 0)
  (func (export "qjs_is_string") (param $value i32) (result i32)
    i32.const 0)
  (func (export "qjs_get_exception") (result i32)
    i32.const 4)
  (func (export "qjs_get_float64") (param $value i32) (result f64)
    f64.const 0)
  (func (export "qjs_get_string") (param $value i32) (result i32)
    i32.const 0)
  (func (export "qjs_free_cstring") (param $ptr i32))
  (func (export "qjs_free_value") (param $value i32))
  (func (export "qjs_is_job_pending") (result i32)
    i32.const 0)
  (func (export "qjs_execute_pending_job") (result i32)
    i32.const 0)
  (func (export "qjs_get_runtime_ptr") (result i32)
    i32.const 256)
  (func (export "qjs_get_context_ptr") (result i32)
    i32.const 512)
  (func (export "qjs_set_runtime_and_context") (param $runtime i32) (param $context i32))

  (func (export "wasm_malloc") (param $size i32) (result i32)
    (local $ptr i32)
    global.get $heap
    local.set $ptr
    global.get $heap
    local.get $size
    i32.add
    global.set $heap
    local.get $ptr)

  (func (export "wasm_free") (param $ptr i32))
)
"#;

pub(super) fn stdio_runtime_fixture() -> Result<(Engine, QuickJsModule)> {
    runtime_fixture(STDIO_RUNTIME_WAT)
}

pub(super) fn runtime_fixture(wat: &str) -> Result<(Engine, QuickJsModule)> {
    let engine = Engine::default();
    let module = QuickJsModule::from_bytes(&engine, wat.as_bytes())?;
    Ok((engine, module))
}

pub(super) fn runtime_fixture_with_replacement(from: &str, to: &str) -> String {
    let wat = STDIO_RUNTIME_WAT.replacen(from, to, 1);
    assert_ne!(wat, STDIO_RUNTIME_WAT, "fixture replacement should match");
    wat
}

pub(super) fn expect_create_error(wat: &str, expected: &str) -> Result<()> {
    let (engine, module) = runtime_fixture(wat)?;
    let err = match QuickJsRuntime::create(&engine, &module) {
        Ok(_) => bail!("runtime creation should fail"),
        Err(err) => err,
    };
    let message = format!("{err:#}");
    assert!(
        message.contains(expected),
        "expected error to contain {expected:?}, got {message:?}"
    );
    Ok(())
}

pub(super) fn synthetic_snapshot(module: &QuickJsModule, memory_len: usize) -> Snapshot {
    Snapshot {
        format_version: SNAPSHOT_FORMAT_VERSION,
        abi_version: QUICKJS_WASM_ABI_VERSION,
        wasm_sha256: module.wasm_sha256(),
        memory: vec![0; memory_len],
        stack_pointer: u32::try_from(memory_len).expect("test memory length should fit in u32"),
        runtime_ptr: 256,
        context_ptr: 512,
    }
}

pub(super) fn expect_restore_error(
    engine: &Engine,
    module: &QuickJsModule,
    snapshot: &Snapshot,
    expected: &str,
) -> Result<()> {
    let err = match QuickJsRuntime::restore(engine, module, snapshot) {
        Ok(_) => bail!("runtime restore should fail"),
        Err(err) => err,
    };
    let message = format!("{err:#}");
    assert!(
        message.contains(expected),
        "expected error to contain {expected:?}, got {message:?}"
    );
    Ok(())
}
