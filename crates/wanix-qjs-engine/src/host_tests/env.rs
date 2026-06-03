use super::*;
use crate::host::{HostCallbackEntry, HostCallbackMode, define_env_imports};
use crate::{QuickJsBinaryValue, QuickJsCallbackValue, QuickJsTypedArrayKind, QuickJsValue};
use std::sync::{Arc, Mutex};
use wasmtime::{Engine, Linker, Module, Store, TypedFunc};

const ENV_IMPORT_WAT: &str = r#"
(module
  (import "env" "host_get_timezone_offset" (func $host_get_timezone_offset (param i32 i32) (result i32)))
  (import "env" "host_interrupt" (func $host_interrupt (result i32)))
  (import "env" "host_promise_rejection" (func $host_promise_rejection (param i32 i32 i32)))
  (import "env" "host_module_normalize" (func $host_module_normalize (param i32 i32) (result i32)))
  (import "env" "host_module_load" (func $host_module_load (param i32 i32) (result i32)))
  (import "env" "host_call" (func $host_call (param i32 i32 i32 i32 i32) (result i32)))

  (memory (export "memory") 1)
  (global $free_count (mut i32) (i32.const 0))
  (global $wasm_free_count (mut i32) (i32.const 0))
  (global $binary_get_count (mut i32) (i32.const 0))
  (global $cstring_free_count (mut i32) (i32.const 0))
  (data (i32.const 1024) "reason text\00")
  (data (i32.const 1040) "hostBytes")
  (data (i32.const 1080) "\d2\04\00\00")
  (data (i32.const 1088) "\29\09\00\00")
  (data (i32.const 1200) "\0a\14\1e")

  (func (export "host_get_timezone_offset") (param i32 i32) (result i32)
    local.get 0
    local.get 1
    call $host_get_timezone_offset)
  (func (export "host_interrupt") (result i32)
    call $host_interrupt)
  (func (export "host_promise_rejection") (param i32 i32 i32)
    local.get 0
    local.get 1
    local.get 2
    call $host_promise_rejection)
  (func (export "host_module_normalize") (param i32 i32) (result i32)
    local.get 0
    local.get 1
    call $host_module_normalize)
  (func (export "host_module_load") (param i32 i32) (result i32)
    local.get 0
    local.get 1
    call $host_module_load)
  (func (export "host_call") (param i32 i32 i32 i32 i32) (result i32)
    local.get 0
    local.get 1
    local.get 2
    local.get 3
    local.get 4
    call $host_call)
  (func (export "qjs_get_string") (param $value i32) (result i32)
    i32.const 1024)
  (func (export "qjs_get_undefined") (result i32)
    i32.const 555)
  (func (export "qjs_is_undefined") (param $value i32) (result i32)
    i32.const 0)
  (func (export "qjs_is_number") (param $value i32) (result i32)
    i32.const 0)
  (func (export "qjs_is_string") (param $value i32) (result i32)
    i32.const 0)
  (func (export "qjs_is_big_int") (param $value i32) (result i32)
    local.get $value
    i32.const 6789
    i32.eq)
  (func (export "qjs_get_big_int64")
    (param $value i32)
    (param $lo_out i32)
    (param $hi_out i32)
    (result i32)
    local.get $lo_out
    i32.const 42
    i32.store
    local.get $hi_out
    i32.const 1
    i32.store
    i32.const 0)
  (func (export "qjs_new_big_int64") (param $lo i32) (param $hi i32) (result i32)
    i32.const 7777)
  (func (export "qjs_is_array_buffer") (param $value i32) (result i32)
    local.get $value
    i32.const 1234
    i32.eq
    local.get $value
    i32.const 2345
    i32.eq
    i32.or)
  (func (export "qjs_is_uint8_array") (param $value i32) (result i32)
    i32.const 0)
  (func (export "qjs_get_typed_array_type") (param $value i32) (result i32)
    local.get $value
    i32.const 3456
    i32.eq
    if (result i32)
      i32.const 3
    else
      i32.const -1
    end)
  (func (export "qjs_is_data_view") (param $value i32) (result i32)
    local.get $value
    i32.const 5678
    i32.eq)
  (func (export "qjs_get_array_buffer") (param $value i32) (param $len_out i32) (result i32)
    global.get $binary_get_count
    i32.const 1
    i32.add
    global.set $binary_get_count
    local.get $len_out
    i32.const 3
    i32.store
    local.get $value
    i32.const 2345
    i32.eq
    if (result i32)
      i32.const 0
    else
      i32.const 1200
    end)
  (func (export "qjs_is_exception") (param $value i32) (result i32)
    i32.const 0)
  (func (export "qjs_get_typed_array_buffer")
    (param $value i32)
    (param $byte_offset_out i32)
    (param $byte_length_out i32)
    (param $bytes_per_element_out i32)
    (result i32)
    local.get $byte_offset_out
    i32.const 1
    i32.store
    local.get $byte_length_out
    i32.const 2
    i32.store
    local.get $bytes_per_element_out
    i32.const 2
    i32.store
    i32.const 4567)
  (func (export "qjs_get_data_view_buffer")
    (param $value i32)
    (param $byte_offset_out i32)
    (param $byte_length_out i32)
    (result i32)
    local.get $byte_offset_out
    i32.const 1
    i32.store
    local.get $byte_length_out
    i32.const 2
    i32.store
    i32.const 4567)
  (func (export "qjs_get_exception") (result i32)
    i32.const 6543)
  (func (export "qjs_new_string") (param $ptr i32) (param $len i32) (result i32)
    i32.const 6000)
  (func (export "qjs_throw") (param $value i32) (result i32)
    i32.const 7000)
  (func (export "qjs_free_cstring") (param $ptr i32)
    global.get $cstring_free_count
    i32.const 1
    i32.add
    global.set $cstring_free_count)
  (func (export "qjs_free_value") (param $value i32)
    global.get $free_count
    i32.const 1
    i32.add
    global.set $free_count)
  (func (export "wasm_malloc") (param $size i32) (result i32)
    i32.const 4096)
  (func (export "wasm_free") (param $ptr i32)
    global.get $wasm_free_count
    i32.const 1
    i32.add
    global.set $wasm_free_count)
  (func (export "free_count") (result i32)
    global.get $free_count)
  (func (export "wasm_free_count") (result i32)
    global.get $wasm_free_count)
  (func (export "binary_get_count") (result i32)
    global.get $binary_get_count)
  (func (export "cstring_free_count") (result i32)
    global.get $cstring_free_count)
)
"#;

struct EnvImportHarness {
    store: Store<HostState>,
    memory: wasmtime::Memory,
    host_get_timezone_offset: TypedFunc<(i32, i32), i32>,
    host_interrupt: TypedFunc<(), i32>,
    host_promise_rejection: TypedFunc<(i32, i32, i32), ()>,
    host_module_normalize: TypedFunc<(i32, i32), i32>,
    host_module_load: TypedFunc<(i32, i32), i32>,
    host_call: TypedFunc<(i32, i32, i32, i32, i32), i32>,
    free_count: TypedFunc<(), i32>,
    wasm_free_count: TypedFunc<(), i32>,
    binary_get_count: TypedFunc<(), i32>,
    cstring_free_count: TypedFunc<(), i32>,
}

fn env_import_harness(config: QuickJsHostConfig) -> Result<EnvImportHarness> {
    let engine = Engine::default();
    let module = Module::new(&engine, ENV_IMPORT_WAT)?;
    let mut linker = Linker::<HostState>::new(&engine);
    define_env_imports(&mut linker)?;
    let mut store = Store::new(&engine, HostState::new(config));
    let instance = linker.instantiate(&mut store, &module)?;
    let memory = instance
        .get_memory(&mut store, "memory")
        .context("test module should export memory")?;

    Ok(EnvImportHarness {
        memory,
        host_get_timezone_offset: instance
            .get_typed_func(&mut store, "host_get_timezone_offset")?,
        host_interrupt: instance.get_typed_func(&mut store, "host_interrupt")?,
        host_promise_rejection: instance.get_typed_func(&mut store, "host_promise_rejection")?,
        host_module_normalize: instance.get_typed_func(&mut store, "host_module_normalize")?,
        host_module_load: instance.get_typed_func(&mut store, "host_module_load")?,
        host_call: instance.get_typed_func(&mut store, "host_call")?,
        free_count: instance.get_typed_func(&mut store, "free_count")?,
        wasm_free_count: instance.get_typed_func(&mut store, "wasm_free_count")?,
        binary_get_count: instance.get_typed_func(&mut store, "binary_get_count")?,
        cstring_free_count: instance.get_typed_func(&mut store, "cstring_free_count")?,
        store,
    })
}

impl EnvImportHarness {
    fn attach_memory(&mut self) {
        self.store.data_mut().set_memory(self.memory);
    }
}

#[test]
fn env_imports_return_configured_timezone_and_neutral_statuses() -> Result<()> {
    let mut harness =
        env_import_harness(QuickJsHostConfig::new().with_timezone_offset_seconds(-19_800))?;

    assert_eq!(
        harness
            .host_get_timezone_offset
            .call(&mut harness.store, (123, -456))?,
        -19_800
    );
    assert_eq!(harness.host_interrupt.call(&mut harness.store, ())?, 0);
    assert_eq!(
        harness
            .host_module_normalize
            .call(&mut harness.store, (11, 22))?,
        0
    );
    assert_eq!(
        harness
            .host_module_load
            .call(&mut harness.store, (33, 44))?,
        0
    );
    assert_eq!(
        harness
            .host_call
            .call(&mut harness.store, (55, 66, 77, 88, 99))?,
        0
    );
    Ok(())
}

#[test]
fn host_promise_rejection_without_handler_frees_values_without_stringifying() -> Result<()> {
    let mut harness = env_import_harness(QuickJsHostConfig::new())?;

    harness
        .host_promise_rejection
        .call(&mut harness.store, (101, 202, 1))?;
    assert_eq!(harness.free_count.call(&mut harness.store, ())?, 2);
    assert_eq!(harness.cstring_free_count.call(&mut harness.store, ())?, 0);
    Ok(())
}

#[test]
fn host_promise_rejection_dispatches_handler_and_frees_values() -> Result<()> {
    let mut harness = env_import_harness(QuickJsHostConfig::new())?;
    harness.attach_memory();
    let events = Arc::new(Mutex::new(Vec::new()));
    let events_for_handler = Arc::clone(&events);

    harness
        .store
        .data_mut()
        .set_promise_rejection_handler(Box::new(move |event| {
            events_for_handler
                .lock()
                .unwrap()
                .push((event.reason().to_string(), event.is_handled()));
        }));

    harness
        .host_promise_rejection
        .call(&mut harness.store, (101, 202, 1))?;

    assert_eq!(
        events.lock().unwrap().as_slice(),
        &[("reason text".to_string(), true)]
    );
    assert_eq!(harness.store.data().promise_rejection_handler_depth(), 0);
    assert_eq!(harness.free_count.call(&mut harness.store, ())?, 2);
    assert_eq!(harness.cstring_free_count.call(&mut harness.store, ())?, 1);
    Ok(())
}

#[test]
fn host_promise_rejection_handler_panics_are_caught_and_cleanup_still_runs() -> Result<()> {
    let mut harness = env_import_harness(QuickJsHostConfig::new())?;
    harness.attach_memory();

    harness
        .store
        .data_mut()
        .set_promise_rejection_handler(Box::new(|_event| {
            panic!("panic from promise rejection handler test");
        }));

    harness
        .host_promise_rejection
        .call(&mut harness.store, (101, 202, 0))?;

    assert_eq!(harness.store.data().promise_rejection_handler_depth(), 0);
    assert_eq!(harness.free_count.call(&mut harness.store, ())?, 2);
    assert_eq!(harness.cstring_free_count.call(&mut harness.store, ())?, 1);
    Ok(())
}

#[test]
fn host_call_copies_array_buffer_argument_and_frees_len_slot() -> Result<()> {
    let mut harness = env_import_harness(QuickJsHostConfig::new())?;
    harness.attach_memory();

    harness.store.data_mut().insert_host_callback(
        "hostBytes".to_string(),
        HostCallbackEntry::new(
            Box::new(|args| {
                assert_eq!(
                    args,
                    &[QuickJsCallbackValue::Binary(
                        QuickJsBinaryValue::ArrayBuffer(vec![10, 20, 30])
                    )]
                );
                Ok(QuickJsCallbackValue::Scalar(QuickJsValue::Undefined))
            }),
            HostCallbackMode::BinaryCapable,
        ),
    )?;

    assert_eq!(
        harness
            .host_call
            .call(&mut harness.store, (1040, 9, 0, 1, 1080))?,
        555
    );
    assert_eq!(harness.store.data().host_callback_depth(), 0);
    assert_eq!(harness.wasm_free_count.call(&mut harness.store, ())?, 1);
    assert_eq!(harness.binary_get_count.call(&mut harness.store, ())?, 1);
    Ok(())
}

#[test]
fn host_call_copies_typed_array_argument_and_frees_scratch_handles() -> Result<()> {
    let mut harness = env_import_harness(QuickJsHostConfig::new())?;
    harness.attach_memory();
    harness
        .memory
        .write(&mut harness.store, 1080, &3456_i32.to_le_bytes())?;

    let seen_args = Arc::new(Mutex::new(None));
    let seen_args_for_callback = Arc::clone(&seen_args);
    harness.store.data_mut().insert_host_callback(
        "hostBytes".to_string(),
        HostCallbackEntry::new(
            Box::new(move |args| {
                *seen_args_for_callback.lock().unwrap() = Some(args.to_vec());
                Ok(QuickJsCallbackValue::Scalar(QuickJsValue::Undefined))
            }),
            HostCallbackMode::BinaryCapable,
        ),
    )?;

    let result = harness
        .host_call
        .call(&mut harness.store, (1040, 9, 0, 1, 1080))?;
    if result != 555 {
        let memory = harness.memory.data(&harness.store);
        let message = &memory[4096..4600];
        let end = message
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(message.len());
        anyhow::bail!(
            "host_call returned {result}: {}",
            String::from_utf8_lossy(&message[..end])
        );
    }
    assert_eq!(
        seen_args.lock().unwrap().as_ref(),
        Some(&vec![QuickJsCallbackValue::Binary(
            QuickJsBinaryValue::TypedArray {
                kind: QuickJsTypedArrayKind::Int16,
                bytes: vec![20, 30],
            }
        )])
    );
    assert_eq!(harness.store.data().host_callback_depth(), 0);
    assert_eq!(harness.wasm_free_count.call(&mut harness.store, ())?, 2);
    assert_eq!(harness.binary_get_count.call(&mut harness.store, ())?, 1);
    assert_eq!(harness.free_count.call(&mut harness.store, ())?, 1);
    Ok(())
}

#[test]
fn host_call_copies_data_view_argument_and_frees_scratch_handles() -> Result<()> {
    let mut harness = env_import_harness(QuickJsHostConfig::new())?;
    harness.attach_memory();
    harness
        .memory
        .write(&mut harness.store, 1080, &5678_i32.to_le_bytes())?;

    let seen_args = Arc::new(Mutex::new(None));
    let seen_args_for_callback = Arc::clone(&seen_args);
    harness.store.data_mut().insert_host_callback(
        "hostBytes".to_string(),
        HostCallbackEntry::new(
            Box::new(move |args| {
                *seen_args_for_callback.lock().unwrap() = Some(args.to_vec());
                Ok(QuickJsCallbackValue::Scalar(QuickJsValue::Undefined))
            }),
            HostCallbackMode::BinaryCapable,
        ),
    )?;

    assert_eq!(
        harness
            .host_call
            .call(&mut harness.store, (1040, 9, 0, 1, 1080))?,
        555
    );
    assert_eq!(
        seen_args.lock().unwrap().as_ref(),
        Some(&vec![QuickJsCallbackValue::Binary(
            QuickJsBinaryValue::DataView(vec![20, 30])
        )])
    );
    assert_eq!(harness.store.data().host_callback_depth(), 0);
    assert_eq!(harness.wasm_free_count.call(&mut harness.store, ())?, 2);
    assert_eq!(harness.binary_get_count.call(&mut harness.store, ())?, 1);
    assert_eq!(harness.free_count.call(&mut harness.store, ())?, 1);
    Ok(())
}

#[test]
fn host_call_copies_bigint_argument_and_frees_output_slot() -> Result<()> {
    let mut harness = env_import_harness(QuickJsHostConfig::new())?;
    harness.attach_memory();
    harness
        .memory
        .write(&mut harness.store, 1080, &6789_i32.to_le_bytes())?;

    let seen_args = Arc::new(Mutex::new(None));
    let seen_args_for_callback = Arc::clone(&seen_args);
    harness.store.data_mut().insert_host_callback(
        "hostBytes".to_string(),
        HostCallbackEntry::new(
            Box::new(move |args| {
                *seen_args_for_callback.lock().unwrap() = Some(args.to_vec());
                Ok(QuickJsCallbackValue::Scalar(QuickJsValue::BigIntI64(-7)))
            }),
            HostCallbackMode::Scalar,
        ),
    )?;

    assert_eq!(
        harness
            .host_call
            .call(&mut harness.store, (1040, 9, 0, 1, 1080))?,
        7777
    );
    assert_eq!(
        seen_args.lock().unwrap().as_ref(),
        Some(&vec![QuickJsCallbackValue::Scalar(
            QuickJsValue::BigIntI64((1_i64 << 32) + 42)
        )])
    );
    assert_eq!(harness.store.data().host_callback_depth(), 0);
    assert_eq!(harness.wasm_free_count.call(&mut harness.store, ())?, 1);
    assert_eq!(harness.free_count.call(&mut harness.store, ())?, 0);
    Ok(())
}

#[test]
fn scalar_host_call_rejects_binary_argument_without_calling_binary_getter() -> Result<()> {
    let mut harness = env_import_harness(QuickJsHostConfig::new())?;
    harness.attach_memory();

    harness.store.data_mut().insert_host_callback(
        "hostBytes".to_string(),
        HostCallbackEntry::new(
            Box::new(|_args| Ok(QuickJsCallbackValue::Scalar(QuickJsValue::Undefined))),
            HostCallbackMode::Scalar,
        ),
    )?;

    assert_eq!(
        harness
            .host_call
            .call(&mut harness.store, (1040, 9, 0, 1, 1080))?,
        7000
    );
    assert_eq!(harness.store.data().host_callback_depth(), 0);
    assert_eq!(harness.binary_get_count.call(&mut harness.store, ())?, 0);
    assert_eq!(harness.wasm_free_count.call(&mut harness.store, ())?, 1);
    assert_eq!(harness.free_count.call(&mut harness.store, ())?, 1);
    Ok(())
}

#[test]
fn host_call_frees_len_slot_when_binary_getter_returns_null() -> Result<()> {
    let mut harness = env_import_harness(QuickJsHostConfig::new())?;
    harness.attach_memory();

    harness.store.data_mut().insert_host_callback(
        "hostBytes".to_string(),
        HostCallbackEntry::new(
            Box::new(|_args| Ok(QuickJsCallbackValue::Scalar(QuickJsValue::Undefined))),
            HostCallbackMode::BinaryCapable,
        ),
    )?;

    assert_eq!(
        harness
            .host_call
            .call(&mut harness.store, (1040, 9, 0, 1, 1088))?,
        7000
    );
    assert_eq!(harness.store.data().host_callback_depth(), 0);
    assert_eq!(harness.wasm_free_count.call(&mut harness.store, ())?, 2);
    assert_eq!(harness.free_count.call(&mut harness.store, ())?, 2);
    assert_eq!(harness.cstring_free_count.call(&mut harness.store, ())?, 1);
    Ok(())
}

#[test]
fn host_interrupt_dispatches_handler_and_clear_restores_neutral_result() -> Result<()> {
    let mut harness = env_import_harness(QuickJsHostConfig::new())?;
    let mut polls = 0usize;

    harness
        .store
        .data_mut()
        .set_interrupt_handler(Box::new(move || {
            polls += 1;
            polls >= 2
        }));

    assert_eq!(harness.host_interrupt.call(&mut harness.store, ())?, 0);
    assert_eq!(harness.host_interrupt.call(&mut harness.store, ())?, 1);
    assert_eq!(harness.store.data().interrupt_handler_depth(), 0);

    harness.store.data_mut().clear_interrupt_handler();
    assert_eq!(harness.host_interrupt.call(&mut harness.store, ())?, 0);
    Ok(())
}

#[test]
fn host_interrupt_panics_request_interrupt_and_reset_depth() -> Result<()> {
    let mut harness = env_import_harness(QuickJsHostConfig::new())?;

    harness
        .store
        .data_mut()
        .set_interrupt_handler(Box::new(|| -> bool {
            panic!("panic from host interrupt test");
        }));

    assert_eq!(harness.host_interrupt.call(&mut harness.store, ())?, 1);
    assert_eq!(harness.store.data().interrupt_handler_depth(), 0);
    Ok(())
}
