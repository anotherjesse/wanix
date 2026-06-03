use super::*;
use crate::snapshot::WASM_PAGE_SIZE;
use anyhow::{Result, bail};
use std::sync::{Arc, Mutex};

mod fixture;

use fixture::{
    STDIO_RUNTIME_WAT, expect_create_error, expect_restore_error, runtime_fixture,
    runtime_fixture_with_replacement, stdio_runtime_fixture, synthetic_snapshot,
};

#[test]
fn restore_starts_with_fresh_stdio_capture_buffers() -> Result<()> {
    let (engine, module) = stdio_runtime_fixture()?;
    let config = QuickJsHostConfig::new().with_stdout_capture(true);

    let mut vm = QuickJsRuntime::create_with_host_config(&engine, &module, config)?;
    vm.eval_discard("emit()")?;
    assert_eq!(vm.captured_stdout(), b"stdio event\n");
    assert_eq!(vm.take_captured_stdout(), b"stdio event\n".to_vec());
    assert_eq!(vm.captured_stdout(), b"");

    vm.eval_discard("emit()")?;
    assert_eq!(vm.captured_stdout(), b"stdio event\n");

    let snapshot = vm.snapshot()?;
    drop(vm);

    let mut restored = QuickJsRuntime::restore_with_host_config(
        &engine,
        &module,
        &snapshot,
        QuickJsHostConfig::new().with_stdout_capture(true),
    )?;
    assert_eq!(restored.captured_stdout(), b"");

    restored.eval_discard("emit()")?;
    assert_eq!(restored.captured_stdout(), b"stdio event\n");
    Ok(())
}

#[test]
fn restore_reattaches_stdio_capture_limit_from_restore_config() -> Result<()> {
    let (engine, module) = stdio_runtime_fixture()?;

    let mut vm = QuickJsRuntime::create_with_host_config(
        &engine,
        &module,
        QuickJsHostConfig::new().with_stdout_capture(true),
    )?;
    let snapshot = vm.snapshot()?;
    drop(vm);

    let mut restored = QuickJsRuntime::restore_with_host_config(
        &engine,
        &module,
        &snapshot,
        QuickJsHostConfig::new().with_limited_stdout_capture(0),
    )?;
    assert_eq!(restored.captured_stdout(), b"");

    let err = restored
        .eval_discard("emit()")
        .expect_err("restored runtime should enforce restore-time stdout capture limit");

    assert!(format!("{err:#}").contains("captured stdout byte limit exceeded"));
    assert_eq!(restored.captured_stdout(), b"");
    Ok(())
}

#[test]
fn module_byte_restore_reattaches_stdio_capture_limit_from_restore_config() -> Result<()> {
    let (_engine, module) = stdio_runtime_fixture()?;

    let mut vm = module.create_runtime_with_host_config(
        QuickJsHostConfig::new().with_limited_stdout_capture(1024),
    )?;
    let bytes = vm.snapshot()?.try_to_bytes()?;
    drop(vm);

    let mut restored = module.restore_runtime_from_bytes_with_host_config(
        &bytes,
        QuickJsHostConfig::new().with_limited_stdout_capture(0),
    )?;

    let err = restored
        .eval_discard("emit()")
        .expect_err("module-owned byte restore should enforce restore-time stdout capture limit");

    assert!(format!("{err:#}").contains("captured stdout byte limit exceeded"));
    assert_eq!(restored.captured_stdout(), b"");
    Ok(())
}

type WasiHostResult<T> = std::result::Result<T, QuickJsWasiErrno>;
type WriteLog = Arc<Mutex<Vec<(u32, Vec<u8>)>>>;
type SnapshotBlockers = Arc<Mutex<Vec<String>>>;

#[derive(Clone, Default)]
struct RestoreRecordingWasiHost {
    writes: WriteLog,
    snapshot_blockers: SnapshotBlockers,
}

impl QuickJsWasiHost for RestoreRecordingWasiHost {
    fn snapshot_blockers(&mut self) -> WasiHostResult<Vec<String>> {
        self.snapshot_blockers
            .lock()
            .map_err(|_| QuickJsWasiErrno::Io)
            .map(|blockers| blockers.clone())
    }

    fn fd_prestat_get(&mut self, _fd: u32) -> WasiHostResult<QuickJsWasiPrestat> {
        Err(QuickJsWasiErrno::Nosys)
    }

    fn path_open(
        &mut self,
        _dirfd: u32,
        _dirflags: u32,
        _path: &[u8],
        _oflags: u16,
        _rights_base: u64,
        _rights_inheriting: u64,
        _fdflags: u16,
    ) -> WasiHostResult<u32> {
        Err(QuickJsWasiErrno::Nosys)
    }

    fn fd_read(&mut self, _fd: u32, _buf: &mut [u8]) -> WasiHostResult<usize> {
        Err(QuickJsWasiErrno::Nosys)
    }

    fn fd_readdir(&mut self, _fd: u32) -> WasiHostResult<Vec<QuickJsWasiDirEntry>> {
        Err(QuickJsWasiErrno::Nosys)
    }

    fn fd_write(&mut self, fd: u32, buf: &[u8]) -> WasiHostResult<usize> {
        self.writes
            .lock()
            .expect("test writes lock")
            .push((fd, buf.to_vec()));
        Ok(buf.len())
    }

    fn fd_seek(
        &mut self,
        _fd: u32,
        _offset: i64,
        _whence: QuickJsWasiWhence,
    ) -> WasiHostResult<u64> {
        Err(QuickJsWasiErrno::Nosys)
    }

    fn fd_close(&mut self, _fd: u32) -> WasiHostResult<()> {
        Err(QuickJsWasiErrno::Nosys)
    }

    fn fd_fdstat_get(&mut self, _fd: u32) -> WasiHostResult<QuickJsWasiFdStat> {
        Ok(QuickJsWasiFdStat::new(
            QuickJsWasiFileType::CharacterDevice,
            0,
            0,
        ))
    }

    fn fd_filestat_get(&mut self, _fd: u32) -> WasiHostResult<QuickJsWasiFileStat> {
        Err(QuickJsWasiErrno::Nosys)
    }

    fn path_filestat_get(
        &mut self,
        _dirfd: u32,
        _flags: u32,
        _path: &[u8],
    ) -> WasiHostResult<QuickJsWasiFileStat> {
        Err(QuickJsWasiErrno::Nosys)
    }
}

#[test]
fn restore_reattaches_live_wasi_host_from_restore_options() -> Result<()> {
    let (engine, module) = stdio_runtime_fixture()?;
    let mut vm = QuickJsRuntime::create_with_host_config(
        &engine,
        &module,
        QuickJsHostConfig::new().with_stdout_capture(true),
    )?;
    let snapshot = vm.snapshot()?;
    drop(vm);

    let host = RestoreRecordingWasiHost::default();
    let writes = Arc::clone(&host.writes);
    let mut restored = QuickJsRuntime::restore_with_options(
        &engine,
        &module,
        &snapshot,
        QuickJsRestoreOptions::new()
            .with_host_config(QuickJsHostConfig::new().with_stdout_capture(true))
            .with_wasi_host(host),
    )?;

    restored.eval_discard("emit()")?;

    assert_eq!(restored.captured_stdout(), b"");
    assert_eq!(
        &*writes.lock().expect("test writes lock"),
        &[(1, b"stdio event\n".to_vec())]
    );
    Ok(())
}

#[test]
fn module_byte_restore_reattaches_live_wasi_host_from_restore_options() -> Result<()> {
    let (_engine, module) = stdio_runtime_fixture()?;
    let mut vm = module
        .create_runtime_with_host_config(QuickJsHostConfig::new().with_stdout_capture(true))?;
    let bytes = vm.snapshot()?.try_to_bytes()?;
    drop(vm);

    let host = RestoreRecordingWasiHost::default();
    let writes = Arc::clone(&host.writes);
    let mut restored = module.restore_runtime_from_bytes_with_options(
        &bytes,
        QuickJsRestoreOptions::new()
            .with_host_config(QuickJsHostConfig::new().with_stdout_capture(true))
            .with_wasi_host(host),
    )?;

    restored.eval_discard("emit()")?;

    assert_eq!(restored.captured_stdout(), b"");
    assert_eq!(
        &*writes.lock().expect("test writes lock"),
        &[(1, b"stdio event\n".to_vec())]
    );
    Ok(())
}

#[test]
fn snapshot_rejects_live_wasi_host_blockers() -> Result<()> {
    let (engine, module) = stdio_runtime_fixture()?;
    let host = RestoreRecordingWasiHost::default();
    host.snapshot_blockers
        .lock()
        .expect("test snapshot blockers lock")
        .push("open dynamic WASI fd 4".to_owned());
    let mut vm = QuickJsRuntime::create_with_options(
        &engine,
        &module,
        QuickJsCreateOptions::new().with_wasi_host(host),
    )?;

    let err = vm
        .snapshot()
        .expect_err("snapshot should reject live WASI host blockers");
    let message = format!("{err:#}");

    assert!(message.contains("live WASI host resources"));
    assert!(message.contains("open dynamic WASI fd 4"));
    Ok(())
}

#[test]
fn runtime_limit_apis_call_expected_optional_exports() -> Result<()> {
    let wat = runtime_fixture_with_extra_limits()?;
    let (engine, module) = runtime_fixture(&wat)?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;

    vm.set_memory_limit(123_456)?;
    vm.set_max_stack_size(654_321)?;
    vm.set_gc_threshold(222_333)?;
    assert_eq!(vm.gc_threshold()?, 222_333);
    vm.run_gc()?;
    let usage = vm.memory_usage()?;
    assert_eq!(usage.malloc_size, 777);
    assert_eq!(usage.malloc_limit, 123_456);
    assert_eq!(usage.memory_used_size, 888);
    vm.eval_discard("check limits")?;

    vm.clear_memory_limit()?;
    vm.clear_max_stack_size()?;
    vm.disable_automatic_gc()?;
    let err = vm
        .eval_discard("check cleared limits")
        .expect_err("synthetic eval should trap after limits are cleared");
    assert!(format!("{err:#}").contains("error while executing"));
    Ok(())
}

#[test]
fn legacy_runtime_apis_reject_mismatched_engine() -> Result<()> {
    let (_engine, module) = stdio_runtime_fixture()?;
    let other_engine = wasmtime::Engine::default();
    let snapshot = synthetic_snapshot(&module, WASM_PAGE_SIZE);

    let create_err = match QuickJsRuntime::create(&other_engine, &module) {
        Ok(_) => bail!("create should reject an engine that did not compile the module"),
        Err(err) => err,
    };
    assert!(format!("{create_err:#}").contains("different Wasmtime engine"));

    let restore_err = match QuickJsRuntime::restore(&other_engine, &module, &snapshot) {
        Ok(_) => bail!("restore should reject an engine that did not compile the module"),
        Err(err) => err,
    };
    assert!(format!("{restore_err:#}").contains("different Wasmtime engine"));
    Ok(())
}

fn runtime_fixture_with_extra_limits() -> Result<String> {
    let eval_replacement = r#"  (func (export "qjs_eval")
    (param $code i32)
    (param $code_len i32)
    (param $filename i32)
    (param $flags i32)
    (result i32)
    global.get $last_memory_limit
    i32.const 123456
    i32.ne
    if
      unreachable
    end
    global.get $last_stack_size
    i32.const 654321
    i32.ne
    if
      unreachable
    end
    global.get $last_gc_threshold
    i32.const 222333
    i32.ne
    if
      unreachable
    end
    global.get $gc_ran
    i32.const 1
    i32.ne
    if
      unreachable
    end
    i32.const 4)"#;
    let wat = runtime_fixture_with_replacement(
        r#"  (func (export "qjs_eval")
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
    i32.const 4)"#,
        eval_replacement,
    );
    let prefix = wat
        .trim_end()
        .strip_suffix(')')
        .ok_or_else(|| anyhow::anyhow!("test fixture module should end with ')'"))?;
    Ok(format!(
        r#"{prefix}
  (global $last_memory_limit (mut i32) (i32.const -1))
  (global $last_stack_size (mut i32) (i32.const -1))
  (global $last_gc_threshold (mut i32) (i32.const -1))
  (global $gc_ran (mut i32) (i32.const 0))
  (func (export "qjs_set_memory_limit") (param $limit i32)
    local.get $limit
    global.set $last_memory_limit)
  (func (export "qjs_set_max_stack_size") (param $size i32)
    local.get $size
    global.set $last_stack_size)
  (func (export "qjs_run_gc")
    i32.const 1
    global.set $gc_ran)
  (func (export "qjs_set_gc_threshold") (param $threshold i32)
    local.get $threshold
    global.set $last_gc_threshold)
  (func (export "qjs_get_gc_threshold") (result i32)
    global.get $last_gc_threshold)
  (func (export "qjs_compute_memory_usage") (param $out i32)
    local.get $out
    i64.const 777
    i64.store
    local.get $out
    i32.const 8
    i32.add
    global.get $last_memory_limit
    i64.extend_i32_s
    i64.store
    local.get $out
    i32.const 16
    i32.add
    i64.const 888
    i64.store)
)
"#
    ))
}

#[test]
fn restore_does_not_call_initialize_or_qjs_init() -> Result<()> {
    let wat = runtime_fixture_with_replacement(
        r#"  (func (export "_initialize"))"#,
        r#"  (func (export "_initialize")
    unreachable)"#,
    );
    let with_trapping_initialize = wat;
    let wat = with_trapping_initialize.replacen(
        r#"  (func (export "qjs_init") (result i32)
    i32.const 0)"#,
        r#"  (func (export "qjs_init") (result i32)
    unreachable)"#,
        1,
    );
    assert_ne!(
        wat, with_trapping_initialize,
        "fixture replacement should match"
    );
    let (engine, module) = runtime_fixture(&wat)?;
    let snapshot = synthetic_snapshot(&module, WASM_PAGE_SIZE);

    let _restored = QuickJsRuntime::restore(&engine, &module, &snapshot)?;
    Ok(())
}

#[test]
fn restore_reports_memory_grow_failure_context() -> Result<()> {
    let wat = runtime_fixture_with_replacement(
        r#"  (memory (export "memory") 1)"#,
        r#"  (memory (export "memory") 1 1)"#,
    );
    let (engine, module) = runtime_fixture(&wat)?;
    let snapshot = synthetic_snapshot(&module, WASM_PAGE_SIZE * 2);

    expect_restore_error(
        &engine,
        &module,
        &snapshot,
        "failed to grow memory for snapshot restore",
    )
}

#[test]
fn restore_rejects_snapshot_smaller_than_module_initial_memory() -> Result<()> {
    let wat = runtime_fixture_with_replacement(
        r#"  (memory (export "memory") 1)"#,
        r#"  (memory (export "memory") 2)"#,
    );
    let (engine, module) = runtime_fixture(&wat)?;
    let snapshot = synthetic_snapshot(&module, WASM_PAGE_SIZE);
    let expected = "smaller than QuickJS WASM module minimum memory length";

    expect_restore_error(&engine, &module, &snapshot, expected)?;

    let bytes = snapshot.try_to_bytes()?;
    let err = match module.restore_runtime_from_bytes(&bytes) {
        Ok(_) => bail!("runtime restore from bytes should reject a short memory image"),
        Err(err) => err,
    };
    assert!(format!("{err:#}").contains(expected));
    Ok(())
}

#[test]
fn restore_reports_runtime_context_reattach_failure() -> Result<()> {
    let wat = runtime_fixture_with_replacement(
        r#"  (func (export "qjs_set_runtime_and_context") (param $runtime i32) (param $context i32))"#,
        r#"  (func (export "qjs_set_runtime_and_context") (param $runtime i32) (param $context i32)
    unreachable)"#,
    );
    let (engine, module) = runtime_fixture(&wat)?;
    let snapshot = synthetic_snapshot(&module, WASM_PAGE_SIZE);

    expect_restore_error(
        &engine,
        &module,
        &snapshot,
        "failed to restore QuickJS runtime/context pointers",
    )
}

#[test]
fn restore_rejects_runtime_pointer_mismatch_after_reattach() -> Result<()> {
    let (engine, module) = stdio_runtime_fixture()?;
    let mut snapshot = synthetic_snapshot(&module, WASM_PAGE_SIZE);
    // The synthetic setter is intentionally a no-op; changing the expected
    // pointer proves restore rejects a reattach that did not stick.
    snapshot.runtime_ptr = 1024;

    expect_restore_error(
        &engine,
        &module,
        &snapshot,
        "restored JSRuntime pointer 256 does not match snapshot runtime_ptr 1024",
    )
}

#[test]
fn restore_rejects_context_pointer_mismatch_after_reattach() -> Result<()> {
    let (engine, module) = stdio_runtime_fixture()?;
    let mut snapshot = synthetic_snapshot(&module, WASM_PAGE_SIZE);
    // The synthetic setter is intentionally a no-op; changing the expected
    // pointer proves restore rejects a reattach that did not stick.
    snapshot.context_ptr = 1024;

    expect_restore_error(
        &engine,
        &module,
        &snapshot,
        "restored JSContext pointer 512 does not match snapshot context_ptr 1024",
    )
}

#[test]
fn restore_verifies_runtime_context_before_restoring_stack_pointer() -> Result<()> {
    let wat = runtime_fixture_with_replacement(
        r#"  (func (export "qjs_get_context_ptr") (result i32)
    i32.const 512)"#,
        r#"  (func (export "qjs_get_context_ptr") (result i32)
    global.get $__stack_pointer
    i32.const 65536
    i32.ne
    if
      unreachable
    end
    i32.const 512)"#,
    );
    let (engine, module) = runtime_fixture(&wat)?;
    let mut snapshot = synthetic_snapshot(&module, WASM_PAGE_SIZE);
    snapshot.stack_pointer = 32768;

    let _restored = QuickJsRuntime::restore(&engine, &module, &snapshot)?;
    Ok(())
}

#[test]
fn restore_reports_runtime_pointer_verification_failure_context() -> Result<()> {
    let wat = runtime_fixture_with_replacement(
        r#"  (func (export "qjs_get_runtime_ptr") (result i32)
    i32.const 256)"#,
        r#"  (func (export "qjs_get_runtime_ptr") (result i32)
    unreachable)"#,
    );
    let (engine, module) = runtime_fixture(&wat)?;
    let snapshot = synthetic_snapshot(&module, WASM_PAGE_SIZE);

    expect_restore_error(
        &engine,
        &module,
        &snapshot,
        "failed to verify restored JSRuntime pointer",
    )
}

#[test]
fn restore_reports_context_pointer_verification_failure_context() -> Result<()> {
    let wat = runtime_fixture_with_replacement(
        r#"  (func (export "qjs_get_context_ptr") (result i32)
    i32.const 512)"#,
        r#"  (func (export "qjs_get_context_ptr") (result i32)
    unreachable)"#,
    );
    let (engine, module) = runtime_fixture(&wat)?;
    let snapshot = synthetic_snapshot(&module, WASM_PAGE_SIZE);

    expect_restore_error(
        &engine,
        &module,
        &snapshot,
        "failed to verify restored JSContext pointer",
    )
}

#[test]
fn snapshot_rejects_invalid_live_runtime_pointer() -> Result<()> {
    let wat = STDIO_RUNTIME_WAT.replace(
        "(func (export \"qjs_get_runtime_ptr\") (result i32)\n    i32.const 256)",
        "(func (export \"qjs_get_runtime_ptr\") (result i32)\n    i32.const 0)",
    );
    assert_ne!(wat, STDIO_RUNTIME_WAT);
    let (engine, module) = runtime_fixture(&wat)?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;

    let err = vm
        .snapshot()
        .expect_err("snapshot should reject an invalid live runtime pointer");

    assert!(format!("{err:#}").contains("runtime_ptr is null"));
    Ok(())
}

#[test]
fn snapshot_rejects_open_virtual_file_descriptors() -> Result<()> {
    let wat = STDIO_RUNTIME_WAT.replacen(
        r#"  (import "wasi_snapshot_preview1" "fd_write" (func $fd_write (param i32 i32 i32 i32) (result i32)))"#,
        r#"  (import "wasi_snapshot_preview1" "fd_write" (func $fd_write (param i32 i32 i32 i32) (result i32)))
  (import "wasi_snapshot_preview1" "path_open" (func $path_open (param i32 i32 i32 i32 i32 i64 i64 i32 i32) (result i32)))"#,
        1,
    );
    assert_ne!(
        wat, STDIO_RUNTIME_WAT,
        "fixture import replacement should match"
    );
    let wat = wat.replacen(
        r#"  (data (i32.const 1024) "stdio event\n")"#,
        r#"  (data (i32.const 1024) "app/config.txt")"#,
        1,
    );
    let wat = wat.replacen(
        r#"  (func (export "qjs_eval")
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
    i32.const 4)"#,
        r#"  (func (export "qjs_eval")
    (param $code i32)
    (param $code_len i32)
    (param $filename i32)
    (param $flags i32)
    (result i32)
    i32.const 3
    i32.const 0
    i32.const 1024
    i32.const 14
    i32.const 0
    i64.const 6
    i64.const 0
    i32.const 0
    i32.const 96
    call $path_open
    drop
    i32.const 4)"#,
        1,
    );
    assert_ne!(
        wat, STDIO_RUNTIME_WAT,
        "fixture eval replacement should match"
    );
    let (engine, module) = runtime_fixture(&wat)?;
    let config =
        QuickJsHostConfig::new().with_read_only_virtual_file("/app/config.txt", b"data")?;
    let mut vm = QuickJsRuntime::create_with_host_config(&engine, &module, config)?;

    vm.eval_discard("open virtual file")?;
    let err = vm
        .snapshot()
        .expect_err("snapshot should reject live virtual file descriptors");

    assert!(format!("{err:#}").contains("virtual file descriptor"));
    Ok(())
}

#[test]
fn drop_ignores_qjs_destroy_trap() -> Result<()> {
    let wat = runtime_fixture_with_replacement(
        r#"  (func (export "qjs_destroy"))"#,
        r#"  (func (export "qjs_destroy")
    unreachable)"#,
    );
    let (engine, module) = runtime_fixture(&wat)?;
    let vm = QuickJsRuntime::create(&engine, &module)?;

    drop(vm);
    Ok(())
}

#[test]
fn create_reports_initialize_trap_context() -> Result<()> {
    let wat = runtime_fixture_with_replacement(
        r#"  (func (export "_initialize"))"#,
        r#"  (func (export "_initialize")
    unreachable)"#,
    );

    expect_create_error(&wat, "failed to initialize WASI reactor")
}

#[test]
fn create_reports_qjs_init_trap_context() -> Result<()> {
    let wat = runtime_fixture_with_replacement(
        r#"  (func (export "qjs_init") (result i32)
    i32.const 0)"#,
        r#"  (func (export "qjs_init") (result i32)
    unreachable)"#,
    );

    expect_create_error(&wat, "failed to call qjs_init")
}

#[test]
fn create_rejects_nonzero_qjs_init_result() -> Result<()> {
    let wat = runtime_fixture_with_replacement(
        r#"  (func (export "qjs_init") (result i32)
    i32.const 0)"#,
        r#"  (func (export "qjs_init") (result i32)
    i32.const 7)"#,
    );

    expect_create_error(&wat, "QuickJS runtime initialization failed with code 7")
}
