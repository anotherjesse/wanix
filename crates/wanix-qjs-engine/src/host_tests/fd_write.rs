use super::*;

const ERRNO_BADF: i32 = 8;
const NWRITTEN_SENTINEL: u32 = u32::MAX;
const WASI_U32_SIZE: usize = 4;
const WASI_IOV_SIZE: usize = 2 * WASI_U32_SIZE;

#[test]
fn fd_write_rejects_bad_fd_before_memory_or_capture() -> Result<()> {
    let mut harness = host_import_harness(
        QuickJsHostConfig::new()
            .with_stdout_capture(true)
            .with_stderr_capture(true),
    )?;
    let memory_len = harness.memory.data_size(&harness.store);
    let nwritten_ptr = 64;
    write_nwritten_sentinel(&mut harness, nwritten_ptr)?;

    let errno = harness.call_fd_write(99, memory_len - 4, 1, nwritten_ptr)?;

    assert_eq!(errno, ERRNO_BADF);
    assert_nwritten(&harness, nwritten_ptr, NWRITTEN_SENTINEL)?;
    assert_no_captured_stdio(&harness);
    Ok(())
}

#[test]
fn fd_write_allows_empty_buffer_at_memory_end() -> Result<()> {
    let mut harness = host_import_harness(QuickJsHostConfig::new())?;
    let memory_len = harness.memory.data_size(&harness.store);
    let iovs_ptr = 16;
    let nwritten_ptr = 64;
    write_iov(&mut harness, iovs_ptr, memory_len, 0)?;
    write_nwritten_sentinel(&mut harness, nwritten_ptr)?;

    let errno = harness.call_fd_write(1, iovs_ptr, 1, nwritten_ptr)?;

    assert_eq!(errno, 0);
    assert_nwritten(&harness, nwritten_ptr, 0)?;
    Ok(())
}

#[test]
fn fd_write_allows_zero_iovs_without_reading_iovs_pointer() -> Result<()> {
    let mut harness = host_import_harness(QuickJsHostConfig::new().with_stdout_capture(true))?;
    let memory_len = harness.memory.data_size(&harness.store);
    let nwritten_ptr = 64;
    write_nwritten_sentinel(&mut harness, nwritten_ptr)?;

    let errno = harness.call_fd_write(1, memory_len, 0, nwritten_ptr)?;

    assert_eq!(errno, 0);
    assert_nwritten(&harness, nwritten_ptr, 0)?;
    assert_captured_stdout(&harness, b"");
    Ok(())
}

#[test]
fn fd_write_captures_stdout_iovs_when_configured() -> Result<()> {
    let mut harness = host_import_harness(QuickJsHostConfig::new().with_stdout_capture(true))?;
    let iovs_ptr = 16;
    let nwritten_ptr = 64;
    let first_ptr = 128;
    let second_ptr = 160;
    let first = b"hello ";
    let second = b"stdout";
    harness.memory.write(&mut harness.store, first_ptr, first)?;
    harness
        .memory
        .write(&mut harness.store, second_ptr, second)?;
    write_iov(&mut harness, iovs_ptr, first_ptr, first.len())?;
    write_iov(
        &mut harness,
        iovs_ptr + WASI_IOV_SIZE,
        second_ptr,
        second.len(),
    )?;
    write_nwritten_sentinel(&mut harness, nwritten_ptr)?;

    let errno = harness.call_fd_write(1, iovs_ptr, 2, nwritten_ptr)?;

    assert_eq!(errno, 0);
    assert_nwritten(
        &harness,
        nwritten_ptr,
        output_len_u32(first.len() + second.len())?,
    )?;
    assert_captured_stdout(&harness, b"hello stdout");
    assert_captured_stderr(&harness, b"");

    let captured = harness.store.data_mut().take_captured_stdout();
    assert_eq!(captured, b"hello stdout");
    assert_captured_stdout(&harness, b"");
    Ok(())
}

#[test]
fn fd_write_captures_large_stdout_when_configured() -> Result<()> {
    let mut harness = host_import_harness(QuickJsHostConfig::new().with_stdout_capture(true))?;
    let iovs_ptr = 16;
    let nwritten_ptr = 64;
    let buf_ptr = 1024;
    let len = 20 * 1024;
    let mut bytes = Vec::with_capacity(len);
    for index in 0..len {
        bytes.push(b'a' + u8::try_from(index % 26).context("test byte should fit in u8")?);
    }
    harness.memory.write(&mut harness.store, buf_ptr, &bytes)?;
    write_iov(&mut harness, iovs_ptr, buf_ptr, bytes.len())?;
    write_nwritten_sentinel(&mut harness, nwritten_ptr)?;

    let errno = harness.call_fd_write(1, iovs_ptr, 1, nwritten_ptr)?;

    assert_eq!(errno, 0);
    assert_nwritten(&harness, nwritten_ptr, output_len_u32(bytes.len())?)?;
    assert_captured_stdout(&harness, bytes.as_slice());
    assert_captured_stderr(&harness, b"");
    Ok(())
}

#[test]
fn fd_write_preflights_stdout_iovs_before_capture() -> Result<()> {
    let mut harness = host_import_harness(QuickJsHostConfig::new().with_stdout_capture(true))?;
    let memory_len = harness.memory.data_size(&harness.store);
    let iovs_ptr = 16;
    let nwritten_ptr = 64;
    let valid_ptr = 128;
    let bytes = b"partial stdout";
    harness.memory.write(&mut harness.store, valid_ptr, bytes)?;
    write_iov(&mut harness, iovs_ptr, valid_ptr, bytes.len())?;
    write_iov(
        &mut harness,
        iovs_ptr + WASI_IOV_SIZE,
        memory_len - WASI_U32_SIZE,
        WASI_IOV_SIZE,
    )?;
    write_nwritten_sentinel(&mut harness, nwritten_ptr)?;

    let err = harness
        .call_fd_write(1, iovs_ptr, 2, nwritten_ptr)
        .expect_err("fd_write should reject invalid iov before capturing output");

    assert!(format!("{err:#}").contains("guest memory range"));
    assert_captured_stdout(&harness, b"");
    assert_nwritten(&harness, nwritten_ptr, NWRITTEN_SENTINEL)?;
    Ok(())
}

#[test]
fn fd_write_preflights_stderr_iovs_before_capture() -> Result<()> {
    let mut harness = host_import_harness(QuickJsHostConfig::new().with_stderr_capture(true))?;
    let memory_len = harness.memory.data_size(&harness.store);
    let iovs_ptr = 16;
    let nwritten_ptr = 64;
    let valid_ptr = 128;
    let bytes = b"partial stderr";
    harness.memory.write(&mut harness.store, valid_ptr, bytes)?;
    write_iov(&mut harness, iovs_ptr, valid_ptr, bytes.len())?;
    write_iov(
        &mut harness,
        iovs_ptr + WASI_IOV_SIZE,
        memory_len - WASI_U32_SIZE,
        WASI_IOV_SIZE,
    )?;
    write_nwritten_sentinel(&mut harness, nwritten_ptr)?;

    let err = harness
        .call_fd_write(2, iovs_ptr, 2, nwritten_ptr)
        .expect_err("fd_write should reject invalid iov before capturing stderr");

    assert!(format!("{err:#}").contains("guest memory range"));
    assert_no_captured_stdio(&harness);
    assert_nwritten(&harness, nwritten_ptr, NWRITTEN_SENTINEL)?;
    Ok(())
}

#[test]
fn fd_write_rejects_oversized_iov_table_before_capture() -> Result<()> {
    let mut harness = host_import_harness(QuickJsHostConfig::new().with_stdout_capture(true))?;
    let memory_len = harness.memory.data_size(&harness.store);
    let nwritten_ptr = 64;
    write_nwritten_sentinel(&mut harness, nwritten_ptr)?;

    let err = harness
        .call_fd_write(1, 16, memory_len / WASI_IOV_SIZE, nwritten_ptr)
        .expect_err("fd_write should reject an oversized iov table before capture");

    assert!(format!("{err:#}").contains("guest memory range"));
    assert_captured_stdout(&harness, b"");
    assert_nwritten(&harness, nwritten_ptr, NWRITTEN_SENTINEL)?;
    Ok(())
}

#[test]
fn fd_write_rejects_iov_table_offset_overflow_before_capture() -> Result<()> {
    let mut harness = host_import_harness(QuickJsHostConfig::new().with_stdout_capture(true))?;
    let nwritten_ptr = 64;
    write_nwritten_sentinel(&mut harness, nwritten_ptr)?;
    let wrapping_iovs_ptr = -4;
    let crossing_iovs_len = 2;

    let err = harness
        .call_fd_write_raw(
            1,
            wrapping_iovs_ptr,
            crossing_iovs_len,
            test_guest_i32(nwritten_ptr, "fd_write nwritten pointer")?,
        )
        .expect_err("fd_write should reject iov table offset overflow before capture");

    assert!(format!("{err:#}").contains("guest pointer offset overflow"));
    assert_captured_stdout(&harness, b"");
    assert_nwritten(&harness, nwritten_ptr, NWRITTEN_SENTINEL)?;
    Ok(())
}

#[test]
fn fd_write_rejects_total_byte_overflow_before_capture() -> Result<()> {
    let mut harness = host_import_harness(QuickJsHostConfig::new().with_stdout_capture(true))?;
    let target_pages = 256;
    let current_pages = harness.memory.size(&harness.store);
    if current_pages < target_pages {
        harness
            .memory
            .grow(&mut harness.store, target_pages - current_pages)?;
    }

    let memory_len = harness.memory.data_size(&harness.store);
    let iovs_ptr = 1024;
    let nwritten_ptr = 64;
    let iovs_len = (u32::MAX as usize / memory_len) + 1;
    for index in 0..iovs_len {
        write_iov(
            &mut harness,
            iovs_ptr + (index * WASI_IOV_SIZE),
            0,
            memory_len,
        )?;
    }
    write_nwritten_sentinel(&mut harness, nwritten_ptr)?;

    let err = harness
        .call_fd_write(1, iovs_ptr, iovs_len, nwritten_ptr)
        .expect_err("fd_write should reject total byte-count overflow before capture");

    assert!(format!("{err:#}").contains("fd_write byte count overflow"));
    assert_captured_stdout(&harness, b"");
    assert_nwritten(&harness, nwritten_ptr, NWRITTEN_SENTINEL)?;
    Ok(())
}

#[test]
fn fd_write_preflights_nwritten_before_capture() -> Result<()> {
    let mut harness = host_import_harness(QuickJsHostConfig::new().with_stdout_capture(true))?;
    let memory_len = harness.memory.data_size(&harness.store);
    let iovs_ptr = 16;
    let nwritten_ptr = memory_len - (WASI_U32_SIZE / 2);
    let buf_ptr = 128;
    let bytes = b"no partial stdout";
    harness.memory.write(&mut harness.store, buf_ptr, bytes)?;
    write_iov(&mut harness, iovs_ptr, buf_ptr, bytes.len())?;

    let err = harness
        .call_fd_write(1, iovs_ptr, 1, nwritten_ptr)
        .expect_err("fd_write should reject invalid nwritten before capturing output");

    assert!(format!("{err:#}").contains("guest memory range"));
    assert_captured_stdout(&harness, b"");
    Ok(())
}

#[test]
fn fd_write_captures_stderr_when_configured() -> Result<()> {
    let mut harness = host_import_harness(QuickJsHostConfig::new().with_stderr_capture(true))?;
    let iovs_ptr = 16;
    let nwritten_ptr = 64;
    let buf_ptr = 128;
    let bytes = b"stderr bytes";
    harness.memory.write(&mut harness.store, buf_ptr, bytes)?;
    write_iov(&mut harness, iovs_ptr, buf_ptr, bytes.len())?;
    write_nwritten_sentinel(&mut harness, nwritten_ptr)?;

    let errno = harness.call_fd_write(2, iovs_ptr, 1, nwritten_ptr)?;

    assert_eq!(errno, 0);
    assert_nwritten(&harness, nwritten_ptr, output_len_u32(bytes.len())?)?;
    assert_captured_stdout(&harness, b"");
    assert_captured_stderr(&harness, b"stderr bytes");
    Ok(())
}

#[test]
fn fd_write_rejects_out_of_bounds_iov_buffer_before_nwritten() -> Result<()> {
    let mut harness = host_import_harness(QuickJsHostConfig::new())?;
    let memory_len = harness.memory.data_size(&harness.store);
    let iovs_ptr = 16;
    let nwritten_ptr = 64;
    write_iov(
        &mut harness,
        iovs_ptr,
        memory_len - WASI_U32_SIZE,
        WASI_IOV_SIZE,
    )?;
    write_nwritten_sentinel(&mut harness, nwritten_ptr)?;

    let err = harness
        .call_fd_write(1, iovs_ptr, 1, nwritten_ptr)
        .expect_err("out-of-bounds fd_write buffer should trap");

    assert!(format!("{err:#}").contains("guest memory range"));
    assert_nwritten(&harness, nwritten_ptr, NWRITTEN_SENTINEL)?;
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

fn assert_no_captured_stdio(harness: &HostImportHarness) {
    assert_captured_stdout(harness, b"");
    assert_captured_stderr(harness, b"");
}

fn assert_captured_stdout(harness: &HostImportHarness, expected: &[u8]) {
    assert_eq!(harness.store.data().captured_stdout(), expected);
}

fn assert_captured_stderr(harness: &HostImportHarness, expected: &[u8]) {
    assert_eq!(harness.store.data().captured_stderr(), expected);
}
