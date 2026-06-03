use super::*;
use std::sync::{Arc, Mutex};

type PromiseRejectionEvents = Arc<Mutex<Vec<(String, bool)>>>;

fn capture_promise_rejections(vm: &mut QuickJsRuntime) -> Result<PromiseRejectionEvents> {
    let events = Arc::new(Mutex::new(Vec::new()));
    let events_for_handler = Arc::clone(&events);
    vm.set_promise_rejection_handler(move |event| {
        events_for_handler
            .lock()
            .unwrap()
            .push((event.reason().to_string(), event.is_handled()));
    })?;
    Ok(events)
}

fn captured_events(events: &PromiseRejectionEvents) -> Vec<(String, bool)> {
    events.lock().unwrap().clone()
}

#[test]
fn unhandled_promise_rejection_reports_reason() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;
    let events = capture_promise_rejections(&mut vm)?;

    vm.eval_discard(r#"Promise.reject("oops")"#)?;
    vm.execute_pending_jobs()?;

    assert!(
        captured_events(&events)
            .iter()
            .any(|(reason, is_handled)| reason == "oops" && !is_handled)
    );
    assert_eq!(vm.eval_number("6 * 7")?, 42.0);
    Ok(())
}

#[test]
fn promise_rejection_reports_error_objects_as_strings() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;
    let events = capture_promise_rejections(&mut vm)?;

    vm.eval_discard(r#"Promise.reject(new TypeError("bad type"))"#)?;
    vm.execute_pending_jobs()?;

    assert!(captured_events(&events).iter().any(|(reason, is_handled)| {
        reason.contains("TypeError") && reason.contains("bad type") && !is_handled
    }));
    Ok(())
}

#[test]
fn unstringifiable_promise_rejection_reason_uses_lossy_fallback() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;
    let events = capture_promise_rejections(&mut vm)?;

    vm.eval_discard(
        r#"
        Promise.reject({
          toString() {
            throw new Error("toString boom");
          }
        });
        "#,
    )?;
    vm.execute_pending_jobs()?;

    assert!(captured_events(&events).iter().any(|(reason, is_handled)| {
        reason == "[promise rejection reason could not be stringified]" && !is_handled
    }));
    assert_eq!(vm.eval_number("21 * 2")?, 42.0);
    Ok(())
}

#[test]
fn promise_rejection_reports_later_handled_notification() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;
    let events = capture_promise_rejections(&mut vm)?;

    vm.eval_discard(
        r#"
        const p = Promise.reject("later-caught");
        Promise.resolve().then(() => p.catch(() => {}));
        "#,
    )?;
    vm.execute_pending_jobs()?;

    let events = captured_events(&events);
    assert!(
        events
            .iter()
            .any(|(reason, is_handled)| reason == "later-caught" && !is_handled)
    );
    assert!(
        events
            .iter()
            .any(|(reason, is_handled)| reason == "later-caught" && *is_handled)
    );
    Ok(())
}

#[test]
fn clearing_promise_rejection_handler_disables_notifications() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;
    let events = capture_promise_rejections(&mut vm)?;

    vm.clear_promise_rejection_handler()?;
    vm.eval_discard(r#"Promise.reject("ignored")"#)?;
    vm.execute_pending_jobs()?;

    assert!(captured_events(&events).is_empty());
    assert_eq!(vm.eval_number("21 * 2")?, 42.0);
    Ok(())
}

#[test]
fn restored_runtime_can_reattach_promise_rejection_handler() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;

    vm.eval_discard("globalThis.resumeMarker = 'snapshotted'")?;
    let snapshot_bytes = vm.snapshot()?.try_to_bytes()?;
    drop(vm);

    let mut restored = module.restore_runtime_from_bytes(&snapshot_bytes)?;
    assert_eq!(restored.eval_string("resumeMarker")?, "snapshotted");
    let events = capture_promise_rejections(&mut restored)?;

    restored.eval_discard(r#"Promise.reject("post-restore")"#)?;
    restored.execute_pending_jobs()?;

    assert!(
        captured_events(&events)
            .iter()
            .any(|(reason, is_handled)| reason == "post-restore" && !is_handled)
    );
    Ok(())
}

#[test]
fn restored_runtime_drops_rejections_until_handler_is_reattached() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;
    let before_snapshot_events = capture_promise_rejections(&mut vm)?;

    vm.eval_discard("globalThis.resumeMarker = 'tracked before snapshot'")?;
    let snapshot_bytes = vm.snapshot()?.try_to_bytes()?;
    drop(vm);

    let mut restored = module.restore_runtime_from_bytes(&snapshot_bytes)?;
    assert_eq!(
        restored.eval_string("resumeMarker")?,
        "tracked before snapshot"
    );

    restored.eval_discard(r#"Promise.reject("dropped-before-reattach")"#)?;
    restored.execute_pending_jobs()?;
    assert!(captured_events(&before_snapshot_events).is_empty());
    assert_eq!(restored.eval_number("6 * 7")?, 42.0);

    let after_restore_events = capture_promise_rejections(&mut restored)?;
    restored.eval_discard(r#"Promise.reject("observed-after-reattach")"#)?;
    restored.execute_pending_jobs()?;
    assert!(
        captured_events(&after_restore_events)
            .iter()
            .any(|(reason, is_handled)| reason == "observed-after-reattach" && !is_handled)
    );
    Ok(())
}
