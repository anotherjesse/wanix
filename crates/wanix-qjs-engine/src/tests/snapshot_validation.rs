use super::*;
use crate::snapshot::SNAPSHOT_WASM_SHA256_OFFSET;

#[test]
fn rejects_snapshot_with_wrong_module_identity() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;
    let mut snapshot = vm.snapshot()?;
    snapshot.wasm_sha256[0] ^= 0xff;
    let snapshot_sha256_hex = sha256_hex(&snapshot.wasm_sha256());
    let module_sha256_hex = sha256_hex(&module.wasm_sha256());
    drop(vm);

    let err = match QuickJsRuntime::restore(&engine, &module, &snapshot) {
        Ok(_) => bail!("restore should reject wrong module identity"),
        Err(err) => err,
    };
    let message = err.to_string();
    assert!(message.contains("different QuickJS WASM module"));
    assert!(message.contains("snapshot SHA-256"));
    assert!(message.contains("module SHA-256"));
    assert!(message.contains(&snapshot_sha256_hex));
    assert!(message.contains(&module_sha256_hex));
    Ok(())
}

#[test]
fn from_bytes_for_module_rejects_wrong_module_identity() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;
    let snapshot = vm.snapshot()?;
    drop(vm);

    let mut bytes = snapshot.try_to_bytes()?;
    bytes[SNAPSHOT_WASM_SHA256_OFFSET] ^= 0xff;
    let decoded = Snapshot::from_bytes(&bytes)?;
    assert_ne!(decoded.wasm_sha256(), module.wasm_sha256());
    let snapshot_sha256 = decoded.wasm_sha256();
    let module_sha256 = module.wasm_sha256();
    let snapshot_sha256_hex = sha256_hex(&snapshot_sha256);
    let module_sha256_hex = sha256_hex(&module_sha256);

    let err = match Snapshot::from_bytes_for_module(&bytes, &module) {
        Ok(_) => bail!("from_bytes_for_module should reject wrong module identity"),
        Err(err) => err,
    };
    let message = err.to_string();
    assert!(message.contains("different QuickJS WASM module"));
    assert!(message.contains(&snapshot_sha256_hex));
    assert!(message.contains(&module_sha256_hex));

    let err = match module.restore_runtime_from_bytes(&bytes) {
        Ok(_) => bail!("restore_runtime_from_bytes should reject wrong module identity"),
        Err(err) => err,
    };
    let message = err.to_string();
    assert!(message.contains(&snapshot_sha256_hex));
    assert!(message.contains(&module_sha256_hex));
    Ok(())
}

#[test]
fn rejects_structurally_invalid_snapshots() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;
    let snapshot = vm.snapshot()?;
    drop(vm);

    let mut unaligned = snapshot.clone();
    unaligned.memory.pop();
    let err = match QuickJsRuntime::restore(&engine, &module, &unaligned) {
        Ok(_) => bail!("restore should reject unaligned memory"),
        Err(err) => err,
    };
    assert!(err.to_string().contains("page aligned"));

    let mut null_runtime = snapshot.clone();
    null_runtime.runtime_ptr = 0;
    let err = match QuickJsRuntime::restore(&engine, &module, &null_runtime) {
        Ok(_) => bail!("restore should reject null runtime pointer"),
        Err(err) => err,
    };
    assert!(err.to_string().contains("runtime_ptr is null"));

    let mut bad_context = snapshot;
    bad_context.context_ptr = u32::try_from(bad_context.memory.len())?;
    let err = match QuickJsRuntime::restore(&engine, &module, &bad_context) {
        Ok(_) => bail!("restore should reject out-of-range context pointer"),
        Err(err) => err,
    };
    assert!(err.to_string().contains("context_ptr is outside"));
    Ok(())
}

#[test]
fn rejects_incompatible_snapshot_metadata() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;
    let snapshot = vm.snapshot()?;
    drop(vm);

    let mut bad_format = snapshot.clone();
    bad_format.format_version += 1;
    let err = match QuickJsRuntime::restore(&engine, &module, &bad_format) {
        Ok(_) => bail!("restore should reject unsupported format version"),
        Err(err) => err,
    };
    assert!(err.to_string().contains("snapshot format version"));

    let mut bad_abi = snapshot.clone();
    bad_abi.abi_version += 1;
    let err = match QuickJsRuntime::restore(&engine, &module, &bad_abi) {
        Ok(_) => bail!("restore should reject unsupported ABI version"),
        Err(err) => err,
    };
    assert!(err.to_string().contains("QuickJS WASM ABI version"));

    let mut bad_stack = snapshot;
    bad_stack.stack_pointer = 0;
    let err = match QuickJsRuntime::restore(&engine, &module, &bad_stack) {
        Ok(_) => bail!("restore should reject null stack pointer"),
        Err(err) => err,
    };
    assert!(err.to_string().contains("stack pointer"));
    Ok(())
}

fn sha256_hex(bytes: &[u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut hex = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        hex.push(char::from(HEX[usize::from(byte >> 4)]));
        hex.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    hex
}
