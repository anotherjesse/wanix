use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[test]
fn interrupt_handler_stops_infinite_loop_and_runtime_recovers() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;
    let polls = Arc::new(AtomicUsize::new(0));
    let polls_for_handler = Arc::clone(&polls);

    vm.set_interrupt_handler(move || polls_for_handler.fetch_add(1, Ordering::SeqCst) > 0)?;

    let err = vm
        .eval_discard("while (true) {}")
        .expect_err("interrupt handler should stop an infinite loop");
    message_assert_contains(&err, "QuickJS exception");
    message_assert_contains(&err, "interrupted");
    assert!(polls.load(Ordering::SeqCst) > 0);
    assert_eq!(vm.eval_number("6 * 7")?, 42.0);
    Ok(())
}

#[test]
fn clear_interrupt_handler_disables_interrupt_dispatch() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;

    vm.set_interrupt_handler(|| true)?;
    vm.clear_interrupt_handler()?;

    assert_eq!(vm.eval_number("21 * 2")?, 42.0);
    Ok(())
}

#[test]
fn interrupt_handler_panics_interrupt_and_runtime_recovers() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;

    vm.set_interrupt_handler(|| -> bool {
        panic!("panic from Rust interrupt handler");
    })?;

    let err = vm
        .eval_discard("while (true) {}")
        .expect_err("interrupt handler panic should interrupt execution");
    message_assert_contains(&err, "QuickJS exception");
    message_assert_contains(&err, "interrupted");
    assert_eq!(vm.eval_number("6 * 7")?, 42.0);
    Ok(())
}

#[test]
fn memory_limit_surfaces_allocation_failure_and_can_clear() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;

    vm.set_memory_limit(1)?;
    let err = vm
        .eval_discard("globalThis.tooBig = new Array(1000).fill('x')")
        .expect_err("small memory limit should reject allocation");
    message_assert_contains(&err, "QuickJS exception");

    vm.clear_memory_limit()?;
    assert_eq!(vm.eval_number("6 * 7")?, 42.0);
    Ok(())
}

#[test]
fn gc_threshold_round_trips_and_can_clear() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;

    vm.set_gc_threshold(256 * 1024)?;
    assert_eq!(vm.gc_threshold()?, 256 * 1024);
    vm.disable_automatic_gc()?;
    assert_eq!(vm.gc_threshold()?, u32::MAX);
    Ok(())
}

#[test]
fn memory_usage_reports_heap_limit_and_runtime_recovers_after_gc() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;

    vm.set_memory_limit(1024 * 1024)?;
    let limited = vm.memory_usage()?;
    assert_eq!(limited.malloc_limit, 1024 * 1024);
    assert!(limited.memory_used_size > 0);

    vm.clear_memory_limit()?;
    let unlimited = vm.memory_usage()?;
    assert_eq!(unlimited.malloc_limit, 0);

    vm.run_gc()?;
    assert_eq!(vm.eval_number("6 * 7")?, 42.0);
    Ok(())
}

#[test]
fn run_gc_can_collect_unreachable_objects() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;

    vm.eval_discard(
        r#"
        globalThis.gcPayload = Array.from(
            { length: 2000 },
            (_, index) => ({ index, text: "x".repeat(64) })
        );
        "#,
    )?;
    let live = vm.memory_usage()?;

    vm.eval_discard("globalThis.gcPayload = undefined")?;
    vm.run_gc()?;
    let collected = vm.memory_usage()?;

    assert!(
        collected.obj_count < live.obj_count,
        "expected GC to collect unreachable objects: live={}, collected={}",
        live.obj_count,
        collected.obj_count
    );
    Ok(())
}

#[test]
fn run_gc_does_not_shrink_snapshot_linear_memory() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;
    let initial_len = vm.snapshot()?.memory_len();

    vm.eval_discard("globalThis.largeBuffer = new ArrayBuffer(8 * 1024 * 1024)")?;
    let grown_len = vm.snapshot()?.memory_len();
    assert!(
        grown_len > initial_len,
        "expected ArrayBuffer allocation to grow wasm memory: initial={initial_len}, grown={grown_len}"
    );

    vm.eval_discard("globalThis.largeBuffer = undefined")?;
    vm.run_gc()?;
    let after_gc_len = vm.snapshot()?.memory_len();

    assert_eq!(after_gc_len, grown_len);
    Ok(())
}

#[test]
fn stack_limit_can_be_set_and_cleared_for_finite_eval() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;

    vm.set_max_stack_size(1024 * 1024)?;
    assert_eq!(vm.eval_number("21 * 2")?, 42.0);
    vm.clear_max_stack_size()?;
    assert_eq!(vm.eval_number("21 * 2")?, 42.0);
    Ok(())
}

#[test]
fn restored_runtime_can_reattach_interrupt_handler() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;

    vm.eval_discard("globalThis.limitState = 'snapshotted'")?;
    vm.set_interrupt_handler(|| true)?;
    let snapshot_bytes = vm.snapshot()?.try_to_bytes()?;
    drop(vm);

    let mut restored = module.restore_runtime_from_bytes(&snapshot_bytes)?;
    assert_eq!(restored.eval_string("limitState")?, "snapshotted");
    assert_eq!(restored.eval_number("21 * 2")?, 42.0);
    restored.set_interrupt_handler(|| true)?;

    let err = restored
        .eval_discard("while (true) {}")
        .expect_err("reattached interrupt handler should stop restored runtime");
    message_assert_contains(&err, "interrupted");
    assert_eq!(restored.eval_string("limitState")?, "snapshotted");
    Ok(())
}

#[test]
fn restored_runtime_can_reapply_memory_and_stack_limits() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;

    vm.eval_discard("globalThis.limitState = 'snapshotted'")?;
    let snapshot_bytes = vm.snapshot()?.try_to_bytes()?;
    drop(vm);

    let mut restored = module.restore_runtime_from_bytes(&snapshot_bytes)?;
    assert_eq!(restored.eval_string("limitState")?, "snapshotted");

    restored.set_memory_limit(1)?;
    let err = restored
        .eval_discard("globalThis.tooBigAfterRestore = new Array(1000).fill('x')")
        .expect_err("restored runtime should enforce memory limits after reattachment");
    message_assert_contains(&err, "QuickJS exception");
    restored.clear_memory_limit()?;

    restored.set_max_stack_size(1024 * 1024)?;
    assert_eq!(restored.eval_number("6 * 7")?, 42.0);
    restored.clear_max_stack_size()?;
    assert_eq!(restored.eval_number("21 * 2")?, 42.0);
    Ok(())
}

#[test]
fn restored_runtime_can_report_memory_usage_and_run_gc() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;

    vm.eval_discard("globalThis.limitState = 'snapshotted'")?;
    vm.set_gc_threshold(300 * 1024)?;
    let snapshot_bytes = vm.snapshot()?.try_to_bytes()?;
    drop(vm);

    let mut restored = module.restore_runtime_from_bytes(&snapshot_bytes)?;
    assert_eq!(restored.eval_string("limitState")?, "snapshotted");
    assert_eq!(restored.gc_threshold()?, 300 * 1024);
    let usage = restored.memory_usage()?;
    assert!(usage.memory_used_size > 0);

    restored.set_gc_threshold(512 * 1024)?;
    assert_eq!(restored.gc_threshold()?, 512 * 1024);
    restored.run_gc()?;
    assert_eq!(restored.eval_number("21 * 2")?, 42.0);
    Ok(())
}

fn message_assert_contains(err: &anyhow::Error, expected: &str) {
    message_assert_contains_str(&format!("{err:#}"), expected);
}

fn message_assert_contains_str(message: &str, expected: &str) {
    assert!(
        message.contains(expected),
        "expected error to contain {expected:?}, got {message:?}"
    );
}
