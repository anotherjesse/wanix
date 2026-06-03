use anyhow::Result;

use super::fault::CleanupFault::{
    JobQueueAlwaysPending, PendingJobFails, PendingJobLogsAndSucceeds,
};
use super::fixture::{
    CleanupEvent::{CStringFree, JobRun, ValueFree},
    CleanupValue::Exception,
    cleanup_runtime_with_faults, expect_cleanup_log,
};

#[test]
fn execute_pending_jobs_failure_frees_exception_resources() -> Result<()> {
    let mut vm = cleanup_runtime_with_faults(&[JobQueueAlwaysPending, PendingJobFails])?;

    let err = vm
        .execute_pending_jobs()
        .expect_err("failing pending job should surface error");

    let message = format!("{err:#}");
    assert!(message.contains("QuickJS pending job failed"));
    assert!(message.contains("ok"));
    expect_cleanup_log(&mut vm, &[CStringFree(1216), ValueFree(Exception)])?;
    Ok(())
}

#[test]
fn execute_pending_jobs_with_limit_stops_after_configured_jobs() -> Result<()> {
    let mut vm = cleanup_runtime_with_faults(&[JobQueueAlwaysPending, PendingJobLogsAndSucceeds])?;

    let err = vm
        .execute_pending_jobs_with_limit(3)
        .expect_err("permanently pending jobs should hit the configured limit");

    let message = format!("{err:#}");
    assert!(message.contains("QuickJS pending job limit reached after 3 jobs"));
    expect_cleanup_log(&mut vm, &[JobRun, JobRun, JobRun])?;
    Ok(())
}
