use crate::Snapshot;
use crate::snapshot::{
    QUICKJS_WASM_ABI_VERSION, SNAPSHOT_ABI_VERSION_OFFSET, SNAPSHOT_CONTEXT_PTR_OFFSET,
    SNAPSHOT_FORMAT_VERSION, SNAPSHOT_FORMAT_VERSION_OFFSET, SNAPSHOT_HEADER_LEN,
    SNAPSHOT_HEADER_LEN_OFFSET, SNAPSHOT_MAGIC, SNAPSHOT_MEMORY_LEN_OFFSET,
    SNAPSHOT_RUNTIME_PTR_OFFSET, SNAPSHOT_STACK_POINTER_OFFSET, SNAPSHOT_TOTAL_LEN_OFFSET,
    SNAPSHOT_WASM_SHA256_OFFSET, WASM_PAGE_SIZE,
};

mod layout;
mod malformed;
mod properties;
mod validation;

fn snapshot_fixture() -> Snapshot {
    let mut memory = vec![0; WASM_PAGE_SIZE];
    memory[0] = 0xaa;
    memory[WASM_PAGE_SIZE - 1] = 0xbb;

    Snapshot {
        format_version: SNAPSHOT_FORMAT_VERSION,
        abi_version: QUICKJS_WASM_ABI_VERSION,
        wasm_sha256: [0x42; 32],
        memory,
        stack_pointer: 32,
        runtime_ptr: 16,
        context_ptr: 24,
    }
}

fn expect_snapshot_decode_error(bytes: &[u8], expected: &str) {
    let err = Snapshot::from_bytes(bytes).expect_err("snapshot bytes should be rejected");
    assert!(
        err.to_string().contains(expected),
        "expected error to contain {expected:?}, got {err:#}"
    );
}

fn overwrite_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn overwrite_u64(bytes: &mut [u8], offset: usize, value: u64) {
    bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

fn fixture_len_u32(value: usize) -> u32 {
    u32::try_from(value).expect("fixture length should fit in u32")
}

fn fixture_len_u64(value: usize) -> u64 {
    u64::try_from(value).expect("fixture length should fit in u64")
}
