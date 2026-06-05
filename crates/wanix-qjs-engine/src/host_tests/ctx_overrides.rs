//! Tests that the engine's `define_ctx_wasi_overrides` win over the shared
//! `wanix_wasi_host::add_to_linker` base on the LIVE `WasiBacking::Ctx` path.
//!
//! The shared linker accepts every `clock_id` and fills `random_get` with a
//! fixed byte; the engine deliberately shadows both. These tests run the real
//! Ctx wiring (the same as production qjs runtimes) and assert the engine
//! behavior, so the override survives even after the old mock harness is gone.

use super::*;

const ERRNO_SUCCESS: i32 = 0;
const ERRNO_NOSYS: i32 = 52;

#[test]
fn ctx_path_clock_time_get_rejects_unsupported_clock() -> Result<()> {
    let time = 0x0011_2233_4455_6677;
    let mut harness = ctx_import_harness(QuickJsHostConfig::new().with_clock_time_ns(time))?;
    let result_ptr = 64;
    write_u32(&mut harness, result_ptr, u32::MAX)?;
    write_u32(&mut harness, result_ptr + 4, u32::MAX)?;

    // Engine override rejects clock_id 2 with NOSYS and leaves the result alone,
    // even though the shared linker base would have accepted it and written.
    let errno = harness.call_clock_time_get(2, 0, result_ptr)?;
    assert_eq!(errno, ERRNO_NOSYS);
    assert_eq!(read_u64(&harness, result_ptr)?, u64::MAX);

    // A supported clock on the same Ctx harness still writes the configured time.
    let ok = harness.call_clock_time_get(0, 0, result_ptr)?;
    assert_eq!(ok, ERRNO_SUCCESS);
    assert_eq!(read_u64(&harness, result_ptr)?, time);
    Ok(())
}

#[test]
fn ctx_path_random_get_uses_configured_byte_and_rejects_oob() -> Result<()> {
    let byte = 0x7a;
    let mut harness = ctx_import_harness(QuickJsHostConfig::new().with_random_byte(byte))?;
    let ptr = 64usize;
    let len = 8usize;
    write_byte(&mut harness, ptr - 1, 0x11)?;
    write_byte(&mut harness, ptr + len, 0x11)?;

    // Engine override fills with the CONFIGURED byte, not the shared linker's
    // fixed byte, and touches nothing outside the requested range.
    let errno = harness.call_random_get(ptr, len)?;
    assert_eq!(errno, ERRNO_SUCCESS);
    for i in 0..len {
        assert_eq!(read_byte(&harness, ptr + i)?, byte);
    }
    assert_eq!(read_byte(&harness, ptr - 1)?, 0x11);
    assert_eq!(read_byte(&harness, ptr + len)?, 0x11);

    // An out-of-bounds range is rejected before writing anything.
    let memory_len = harness.memory.data_size(&harness.store);
    let err = harness
        .call_random_get(memory_len - 4, 8)
        .expect_err("random_get should reject an out-of-bounds buffer");
    let message = format!("{err:#}").to_lowercase();
    assert!(
        message.contains("bound") || message.contains("outside") || message.contains("memory"),
        "unexpected error: {message}"
    );
    Ok(())
}
