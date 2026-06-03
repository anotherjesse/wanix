use super::*;

const ERRNO_BADF: i32 = 8;

#[test]
fn fd_fdstat_get_writes_character_device_stat_for_stdio() -> Result<()> {
    let mut harness = host_import_harness(QuickJsHostConfig::new())?;
    let stdout_stat_ptr = 64;
    let stderr_stat_ptr = 128;
    fill_stat(&mut harness, stdout_stat_ptr, 0xaa)?;
    fill_stat(&mut harness, stderr_stat_ptr, 0xbb)?;

    let stdout_errno = harness.call_fd_fdstat_get(1, stdout_stat_ptr)?;
    let stderr_errno = harness.call_fd_fdstat_get(2, stderr_stat_ptr)?;

    assert_eq!(stdout_errno, 0);
    assert_eq!(stderr_errno, 0);
    assert_eq!(read_stat(&harness, stdout_stat_ptr)?, expected_fdstat());
    assert_eq!(read_stat(&harness, stderr_stat_ptr)?, expected_fdstat());
    Ok(())
}

#[test]
fn fd_fdstat_get_rejects_bad_fd_before_memory() -> Result<()> {
    let mut harness = host_import_harness(QuickJsHostConfig::new())?;
    let memory_len = harness.memory.data_size(&harness.store);
    let stat_ptr = memory_len - 4;
    write_u32(&mut harness, stat_ptr, u32::MAX)?;

    let errno = harness.call_fd_fdstat_get(99, stat_ptr)?;

    assert_eq!(errno, ERRNO_BADF);
    assert_eq!(read_u32(&harness, stat_ptr)?, u32::MAX);
    Ok(())
}

#[test]
fn fd_fdstat_get_traps_on_out_of_bounds_stat_pointer() -> Result<()> {
    let mut harness = host_import_harness(QuickJsHostConfig::new())?;
    let memory_len = harness.memory.data_size(&harness.store);

    let err = harness
        .call_fd_fdstat_get(1, memory_len - 4)
        .expect_err("fd_fdstat_get should trap when stat pointer cannot hold fdstat");

    assert!(format!("{err:#}").contains("out of bounds"));
    Ok(())
}

fn fill_stat(harness: &mut HostImportHarness, offset: usize, byte: u8) -> Result<()> {
    harness
        .memory
        .write(&mut harness.store, offset, &[byte; 24])?;
    Ok(())
}

fn read_stat(harness: &HostImportHarness, offset: usize) -> Result<[u8; 24]> {
    let mut bytes = [0; 24];
    harness.memory.read(&harness.store, offset, &mut bytes)?;
    Ok(bytes)
}

fn expected_fdstat() -> [u8; 24] {
    let mut stat = [0; 24];
    stat[0] = 2;
    stat
}
