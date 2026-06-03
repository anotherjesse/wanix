use super::*;
use crate::SnapshotMetadata;

#[test]
fn snapshot_bytes_match_v1_layout_golden() {
    let wasm_sha256 = std::array::from_fn(|index| index as u8);
    let mut memory = vec![0; WASM_PAGE_SIZE];
    memory[..4].copy_from_slice(&[0xde, 0xad, 0xbe, 0xef]);
    memory[WASM_PAGE_SIZE - 4..].copy_from_slice(&[0xfa, 0xce, 0xb0, 0x0c]);

    let snapshot = Snapshot {
        format_version: SNAPSHOT_FORMAT_VERSION,
        abi_version: QUICKJS_WASM_ABI_VERSION,
        wasm_sha256,
        memory: memory.clone(),
        stack_pointer: 0x1234,
        runtime_ptr: 0x2345,
        context_ptr: 0x3456,
    };

    assert_eq!(SNAPSHOT_FORMAT_VERSION_OFFSET, 8);
    assert_eq!(SNAPSHOT_ABI_VERSION_OFFSET, 12);
    assert_eq!(SNAPSHOT_HEADER_LEN_OFFSET, 16);
    assert_eq!(SNAPSHOT_TOTAL_LEN_OFFSET, 20);
    assert_eq!(SNAPSHOT_MEMORY_LEN_OFFSET, 28);
    assert_eq!(SNAPSHOT_STACK_POINTER_OFFSET, 36);
    assert_eq!(SNAPSHOT_RUNTIME_PTR_OFFSET, 40);
    assert_eq!(SNAPSHOT_CONTEXT_PTR_OFFSET, 44);
    assert_eq!(SNAPSHOT_WASM_SHA256_OFFSET, 48);

    let mut expected_header = Vec::from([
        b'R', b'W', b'Q', b'S', b'N', b'A', b'P', 0x00, // magic
        0x01, 0x00, 0x00, 0x00, // snapshot format version
        0x01, 0x00, 0x00, 0x00, // QuickJS WASM ABI version
        0x50, 0x00, 0x00, 0x00, // header length
        0x50, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, // total length
        0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, // memory length
        0x34, 0x12, 0x00, 0x00, // stack pointer
        0x45, 0x23, 0x00, 0x00, // runtime pointer
        0x56, 0x34, 0x00, 0x00, // context pointer
    ]);
    expected_header.extend_from_slice(&wasm_sha256);
    assert_eq!(expected_header.len(), SNAPSHOT_HEADER_LEN);

    let bytes = snapshot
        .try_to_bytes()
        .expect("golden snapshot should serialize");
    assert_eq!(snapshot.to_bytes().as_slice(), bytes.as_slice());
    assert_eq!(&bytes[..SNAPSHOT_HEADER_LEN], expected_header.as_slice());
    assert_eq!(
        &bytes[SNAPSHOT_HEADER_LEN..SNAPSHOT_HEADER_LEN + 4],
        b"\xde\xad\xbe\xef"
    );
    assert_eq!(&bytes[bytes.len() - 4..], b"\xfa\xce\xb0\x0c");

    let mut hand_built = expected_header;
    hand_built.extend_from_slice(&memory);
    let metadata =
        Snapshot::metadata_from_bytes(&hand_built).expect("golden metadata should decode");
    let _: SnapshotMetadata = metadata;
    assert_eq!(metadata.format_version(), SNAPSHOT_FORMAT_VERSION);
    assert_eq!(metadata.abi_version(), QUICKJS_WASM_ABI_VERSION);
    assert_eq!(metadata.wasm_sha256(), wasm_sha256);
    assert_eq!(metadata.memory_len(), WASM_PAGE_SIZE);

    let decoded = Snapshot::from_bytes(&hand_built).expect("golden snapshot should decode");
    assert_eq!(decoded, snapshot);
    assert_eq!(decoded.metadata(), metadata);
    assert_eq!(
        decoded
            .try_to_bytes()
            .expect("decoded golden snapshot should serialize"),
        hand_built
    );
    assert_eq!(decoded.to_bytes().as_slice(), hand_built.as_slice());
}
