use super::*;

const NWRITTEN_SENTINEL: u32 = u32::MAX;
const WASI_U32_SIZE: usize = 4;
const WASI_IOV_SIZE: usize = 2 * WASI_U32_SIZE;

#[test]
fn fd_write_allows_stdout_capture_at_byte_limit() -> Result<()> {
    let bytes = b"limit";
    let mut harness =
        host_import_harness(QuickJsHostConfig::new().with_limited_stdout_capture(bytes.len()))?;
    let iovs_ptr = 16;
    let nwritten_ptr = 64;
    write_iov_bytes(&mut harness, iovs_ptr, 128, bytes)?;
    write_nwritten_sentinel(&mut harness, nwritten_ptr)?;

    let errno = harness.call_fd_write(1, iovs_ptr, 1, nwritten_ptr)?;

    assert_eq!(errno, 0);
    assert_nwritten(&harness, nwritten_ptr, output_len_u32(bytes.len())?)?;
    assert_captured_stdout(&harness, bytes);
    Ok(())
}

#[test]
fn fd_write_rejects_stdout_capture_over_byte_limit_without_mutation() -> Result<()> {
    let mut harness = host_import_harness(QuickJsHostConfig::new().with_limited_stdout_capture(8))?;
    let iovs_ptr = 16;
    let nwritten_ptr = 64;
    let kept = b"kept";
    write_iov_bytes(&mut harness, iovs_ptr, 128, kept)?;
    write_nwritten_sentinel(&mut harness, nwritten_ptr)?;
    assert_eq!(harness.call_fd_write(1, iovs_ptr, 1, nwritten_ptr)?, 0);
    assert_captured_stdout(&harness, kept);

    let overflow = b"this write is too large";
    write_iov_bytes(&mut harness, iovs_ptr, 160, overflow)?;
    write_nwritten_sentinel(&mut harness, nwritten_ptr)?;

    let err = harness
        .call_fd_write(1, iovs_ptr, 1, nwritten_ptr)
        .expect_err("fd_write should reject stdout capture past configured limit");

    assert!(format!("{err:#}").contains("captured stdout byte limit exceeded"));
    assert_captured_stdout(&harness, kept);
    assert_nwritten(&harness, nwritten_ptr, NWRITTEN_SENTINEL)?;
    Ok(())
}

#[test]
fn fd_write_allows_stdout_capture_that_exactly_fills_remaining_limit() -> Result<()> {
    let mut harness = host_import_harness(QuickJsHostConfig::new().with_limited_stdout_capture(8))?;
    let iovs_ptr = 16;
    let nwritten_ptr = 64;
    let first = b"kept";
    let second = b"tail";
    write_iov_bytes(&mut harness, iovs_ptr, 128, first)?;
    write_nwritten_sentinel(&mut harness, nwritten_ptr)?;
    assert_eq!(harness.call_fd_write(1, iovs_ptr, 1, nwritten_ptr)?, 0);

    write_iov_bytes(&mut harness, iovs_ptr, 160, second)?;
    write_nwritten_sentinel(&mut harness, nwritten_ptr)?;
    let errno = harness.call_fd_write(1, iovs_ptr, 1, nwritten_ptr)?;

    assert_eq!(errno, 0);
    assert_nwritten(&harness, nwritten_ptr, output_len_u32(second.len())?)?;
    assert_captured_stdout(&harness, b"kepttail");
    Ok(())
}

#[test]
fn fd_write_rejects_multi_iov_capture_over_byte_limit_without_partial_capture() -> Result<()> {
    let first = b"12345";
    let second = b"67890";
    let mut harness =
        host_import_harness(QuickJsHostConfig::new().with_limited_stdout_capture(first.len()))?;
    let iovs_ptr = 16;
    let nwritten_ptr = 64;
    write_iov_bytes(&mut harness, iovs_ptr, 128, first)?;
    write_iov_bytes(&mut harness, iovs_ptr + WASI_IOV_SIZE, 160, second)?;
    write_nwritten_sentinel(&mut harness, nwritten_ptr)?;

    let err = harness
        .call_fd_write(1, iovs_ptr, 2, nwritten_ptr)
        .expect_err("fd_write should reject total stdout capture past configured limit");

    assert!(format!("{err:#}").contains("captured stdout byte limit exceeded"));
    assert_captured_stdout(&harness, b"");
    assert_nwritten(&harness, nwritten_ptr, NWRITTEN_SENTINEL)?;
    Ok(())
}

#[test]
fn fd_write_zero_stdout_capture_limit_allows_only_empty_writes() -> Result<()> {
    let mut harness = host_import_harness(QuickJsHostConfig::new().with_limited_stdout_capture(0))?;
    let memory_len = harness.memory.data_size(&harness.store);
    let iovs_ptr = 16;
    let nwritten_ptr = 64;
    write_iov(&mut harness, iovs_ptr, memory_len, 0)?;
    write_nwritten_sentinel(&mut harness, nwritten_ptr)?;

    let errno = harness.call_fd_write(1, iovs_ptr, 1, nwritten_ptr)?;

    assert_eq!(errno, 0);
    assert_nwritten(&harness, nwritten_ptr, 0)?;
    assert_captured_stdout(&harness, b"");

    write_iov_bytes(&mut harness, iovs_ptr, 128, b"x")?;
    write_nwritten_sentinel(&mut harness, nwritten_ptr)?;
    let err = harness
        .call_fd_write(1, iovs_ptr, 1, nwritten_ptr)
        .expect_err("fd_write should reject non-empty stdout capture past zero-byte limit");

    assert!(format!("{err:#}").contains("captured stdout byte limit exceeded"));
    assert_captured_stdout(&harness, b"");
    assert_nwritten(&harness, nwritten_ptr, NWRITTEN_SENTINEL)?;
    Ok(())
}

#[test]
fn fd_write_enforces_stderr_capture_limit_independently() -> Result<()> {
    let stderr = b"stderr";
    let mut harness = host_import_harness(
        QuickJsHostConfig::new()
            .with_limited_stdout_capture(0)
            .with_limited_stderr_capture(stderr.len()),
    )?;
    let iovs_ptr = 16;
    let nwritten_ptr = 64;
    write_iov_bytes(&mut harness, iovs_ptr, 128, stderr)?;
    write_nwritten_sentinel(&mut harness, nwritten_ptr)?;

    let errno = harness.call_fd_write(2, iovs_ptr, 1, nwritten_ptr)?;

    assert_eq!(errno, 0);
    assert_captured_stdout(&harness, b"");
    assert_captured_stderr(&harness, stderr);

    write_iov_bytes(&mut harness, iovs_ptr, 160, b"x")?;
    write_nwritten_sentinel(&mut harness, nwritten_ptr)?;
    let err = harness
        .call_fd_write(1, iovs_ptr, 1, nwritten_ptr)
        .expect_err("fd_write should reject stdout capture while stderr remains under limit");

    assert!(format!("{err:#}").contains("captured stdout byte limit exceeded"));
    assert_captured_stdout(&harness, b"");
    assert_captured_stderr(&harness, stderr);
    assert_nwritten(&harness, nwritten_ptr, NWRITTEN_SENTINEL)?;
    Ok(())
}

#[test]
fn fd_write_capture_limit_counts_retained_bytes_after_take() -> Result<()> {
    let bytes = b"drain";
    let mut harness =
        host_import_harness(QuickJsHostConfig::new().with_limited_stdout_capture(bytes.len()))?;
    let iovs_ptr = 16;
    let nwritten_ptr = 64;
    write_iov_bytes(&mut harness, iovs_ptr, 128, bytes)?;
    write_nwritten_sentinel(&mut harness, nwritten_ptr)?;
    assert_eq!(harness.call_fd_write(1, iovs_ptr, 1, nwritten_ptr)?, 0);
    assert_eq!(harness.store.data_mut().take_captured_stdout(), bytes);
    assert_captured_stdout(&harness, b"");

    write_nwritten_sentinel(&mut harness, nwritten_ptr)?;
    let errno = harness.call_fd_write(1, iovs_ptr, 1, nwritten_ptr)?;

    assert_eq!(errno, 0);
    assert_captured_stdout(&harness, bytes);
    Ok(())
}

fn write_nwritten_sentinel(harness: &mut HostImportHarness, nwritten_ptr: usize) -> Result<()> {
    write_u32(harness, nwritten_ptr, NWRITTEN_SENTINEL)
}

fn assert_nwritten(harness: &HostImportHarness, nwritten_ptr: usize, expected: u32) -> Result<()> {
    assert_eq!(read_u32(harness, nwritten_ptr)?, expected);
    Ok(())
}

fn output_len_u32(len: usize) -> Result<u32> {
    u32::try_from(len).context("test output length should fit in u32")
}

fn write_iov_bytes(
    harness: &mut HostImportHarness,
    iovs_ptr: usize,
    buf_ptr: usize,
    bytes: &[u8],
) -> Result<()> {
    harness.memory.write(&mut harness.store, buf_ptr, bytes)?;
    write_iov(harness, iovs_ptr, buf_ptr, bytes.len())
}

fn assert_captured_stdout(harness: &HostImportHarness, expected: &[u8]) {
    assert_eq!(harness.store.data().captured_stdout(), expected);
}

fn assert_captured_stderr(harness: &HostImportHarness, expected: &[u8]) {
    assert_eq!(harness.store.data().captured_stderr(), expected);
}
