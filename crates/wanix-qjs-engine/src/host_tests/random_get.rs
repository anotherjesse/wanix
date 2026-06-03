use super::*;

#[test]
fn random_get_fills_large_buffer_without_touching_neighbors() -> Result<()> {
    let mut harness = host_import_harness(QuickJsHostConfig::new().with_random_byte(0x7a))?;
    let ptr = 1024;
    let len = 20 * 1024;
    harness.memory.write(&mut harness.store, ptr - 1, &[0xaa])?;
    harness
        .memory
        .write(&mut harness.store, ptr + len, &[0xbb])?;

    let errno = harness.call_random_get(ptr, len)?;
    assert_eq!(errno, 0);

    let mut before = [0];
    let mut after = [0];
    let mut bytes = vec![0; len];
    harness.memory.read(&harness.store, ptr - 1, &mut before)?;
    harness.memory.read(&harness.store, ptr, &mut bytes)?;
    harness.memory.read(&harness.store, ptr + len, &mut after)?;

    assert_eq!(before, [0xaa]);
    assert!(bytes.iter().all(|byte| *byte == 0x7a));
    assert_eq!(after, [0xbb]);
    Ok(())
}

#[test]
fn random_get_rejects_out_of_bounds_range_before_writing() -> Result<()> {
    let mut harness = host_import_harness(QuickJsHostConfig::new().with_random_byte(0x7a))?;
    let memory_len = harness.memory.data_size(&harness.store);
    let ptr = memory_len - 4;
    harness
        .memory
        .write(&mut harness.store, ptr, &[0xaa, 0xbb, 0xcc, 0xdd])?;

    let err = harness
        .call_random_get(ptr, 8)
        .expect_err("out-of-bounds random_get should trap");

    let mut bytes = [0; 4];
    harness.memory.read(&harness.store, ptr, &mut bytes)?;
    assert!(format!("{err:#}").contains("guest memory range"));
    assert_eq!(bytes, [0xaa, 0xbb, 0xcc, 0xdd]);
    Ok(())
}
