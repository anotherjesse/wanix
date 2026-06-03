use anyhow::Result;

use super::{
    fault::CleanupFault::QjsComputeMemoryUsageTrap,
    fixture::{
        CleanupEvent::GuestFree, cleanup_runtime, cleanup_runtime_with_fault, expect_cleanup_log,
    },
};

#[test]
fn memory_usage_frees_stats_buffer() -> Result<()> {
    let mut vm = cleanup_runtime()?;

    let usage = vm.memory_usage()?;
    assert_eq!(usage.malloc_size, 0);

    expect_cleanup_log(&mut vm, &[GuestFree(4096)])?;
    Ok(())
}

#[test]
fn memory_usage_trap_frees_stats_buffer() -> Result<()> {
    let mut vm = cleanup_runtime_with_fault(QjsComputeMemoryUsageTrap)?;

    let err = vm
        .memory_usage()
        .expect_err("trapping qjs_compute_memory_usage should fail");
    assert!(format!("{err:#}").contains("failed to call qjs_compute_memory_usage"));

    expect_cleanup_log(&mut vm, &[GuestFree(4096)])?;
    Ok(())
}
