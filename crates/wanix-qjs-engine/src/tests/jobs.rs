use super::*;

#[test]
fn restores_pending_promise_and_job_queue() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;

    let mut vm = QuickJsRuntime::create(&engine, &module)?;
    vm.eval_discard(
        r#"
        globalThis.stepResult = "not yet";
        let __resolve;
        globalThis.pendingStep = new Promise(resolve => { __resolve = resolve; });
        globalThis.__resolveFunc = __resolve;
        globalThis.pendingStep.then(value => {
          globalThis.stepResult = "completed: " + value;
        });
        "#,
    )?;
    vm.execute_pending_jobs()?;
    assert_eq!(vm.eval_string("stepResult")?, "not yet");

    let snapshot = Snapshot::from_bytes_for_module(&vm.snapshot()?.try_to_bytes()?, &module)?;
    drop(vm);

    let mut restored = QuickJsRuntime::restore(&engine, &module, &snapshot)?;
    restored.call_global_function_with_string("__resolveFunc", "step-42-result")?;
    restored.execute_pending_jobs()?;

    assert_eq!(
        restored.eval_string("stepResult")?,
        "completed: step-42-result"
    );
    Ok(())
}

#[test]
fn pending_job_errors_surface_and_runtime_recovers() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;

    vm.eval_discard(
        r#"
        globalThis.jobStatus = "scheduled";
        queueMicrotask(() => {
          globalThis.jobStatus = "running";
          throw new Error("job boom");
        });
        "#,
    )?;

    let err = vm
        .execute_pending_jobs()
        .expect_err("throwing pending job should surface to Rust");
    let message = err.to_string();
    assert!(message.contains("QuickJS pending job failed"));
    assert!(message.contains("job boom"));

    assert_eq!(vm.eval_string("jobStatus")?, "running");
    assert_eq!(vm.eval_number("6 * 7")?, 42.0);
    Ok(())
}

#[test]
fn bounded_pending_job_drain_respects_limit() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;

    vm.eval_discard(
        r#"
        globalThis.jobCount = 0;
        queueMicrotask(() => {
          globalThis.jobCount += 1;
        });
        "#,
    )?;

    let err = vm
        .execute_pending_jobs_with_limit(0)
        .expect_err("zero limit should reject a non-empty queue");
    assert!(err.to_string().contains("pending job limit"));
    assert_eq!(vm.eval_number("jobCount")?, 0.0);

    assert_eq!(vm.execute_pending_jobs_with_limit(1)?, 1);
    assert_eq!(vm.eval_number("jobCount")?, 1.0);
    assert_eq!(vm.execute_pending_jobs_with_limit(0)?, 0);
    Ok(())
}
