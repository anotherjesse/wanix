use super::*;

const ERRNO_NOSYS: i32 = 52;

#[test]
fn clock_time_get_writes_configured_time_for_supported_clocks() -> Result<()> {
    let time = 0x0102_0304_0506_0708;
    let mut harness = host_import_harness(QuickJsHostConfig::new().with_clock_time_ns(time))?;
    let first_ptr = 64;
    let second_ptr = 96;
    write_u32(&mut harness, first_ptr, u32::MAX)?;
    write_u32(&mut harness, first_ptr + 4, u32::MAX)?;
    write_u32(&mut harness, second_ptr, u32::MAX)?;
    write_u32(&mut harness, second_ptr + 4, u32::MAX)?;

    let realtime_errno = harness.call_clock_time_get(0, 123, first_ptr)?;
    let monotonic_errno = harness.call_clock_time_get(1, 456, second_ptr)?;

    assert_eq!(realtime_errno, 0);
    assert_eq!(monotonic_errno, 0);
    assert_eq!(read_u64(&harness, first_ptr)?, time);
    assert_eq!(read_u64(&harness, second_ptr)?, time);
    Ok(())
}

#[test]
fn clock_time_get_rejects_unsupported_clock_before_memory() -> Result<()> {
    let mut harness = host_import_harness(QuickJsHostConfig::new())?;
    let memory_len = harness.memory.data_size(&harness.store);
    let result_ptr = memory_len - 4;
    write_u32(&mut harness, result_ptr, u32::MAX)?;

    let errno = harness.call_clock_time_get(2, 0, result_ptr)?;

    assert_eq!(errno, ERRNO_NOSYS);
    assert_eq!(read_u32(&harness, result_ptr)?, u32::MAX);
    Ok(())
}

#[test]
fn clock_time_get_traps_on_out_of_bounds_result_pointer() -> Result<()> {
    let mut harness = host_import_harness(QuickJsHostConfig::new())?;
    let memory_len = harness.memory.data_size(&harness.store);

    let err = harness
        .call_clock_time_get(0, 0, memory_len - 4)
        .expect_err("clock_time_get should trap when result pointer cannot hold u64");

    assert!(format!("{err:#}").contains("out of bounds"));
    Ok(())
}
