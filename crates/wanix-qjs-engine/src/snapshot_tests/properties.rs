use super::malformed::{MALFORMED_SNAPSHOT_MUTATIONS, MalformedSnapshotMutation, MutationInput};
use super::*;
use proptest::prelude::*;

fn valid_snapshot_strategy() -> impl Strategy<Value = Snapshot> {
    (1usize..=2, any::<[u8; 32]>(), any::<u8>())
        .prop_flat_map(|(page_count, wasm_sha256, fill)| {
            let memory_len = page_count * WASM_PAGE_SIZE;
            let memory_len_u32 = fixture_len_u32(memory_len);
            (
                Just(memory_len),
                Just(wasm_sha256),
                Just(fill),
                1u32..=memory_len_u32,
                1u32..memory_len_u32,
                1u32..memory_len_u32,
            )
        })
        .prop_map(
            |(memory_len, wasm_sha256, fill, stack_pointer, runtime_ptr, context_ptr)| {
                let mut memory = vec![fill; memory_len];
                memory[0] = fill.wrapping_add(1);
                memory[memory_len - 1] = fill.wrapping_sub(1);

                Snapshot {
                    format_version: SNAPSHOT_FORMAT_VERSION,
                    abi_version: QUICKJS_WASM_ABI_VERSION,
                    wasm_sha256,
                    memory,
                    stack_pointer,
                    runtime_ptr,
                    context_ptr,
                }
            },
        )
}

fn malformed_snapshot_mutation_strategy() -> impl Strategy<Value = MalformedSnapshotMutation> {
    proptest::sample::select(MALFORMED_SNAPSHOT_MUTATIONS)
}

#[test]
fn each_malformed_snapshot_mutation_is_rejected() {
    let snapshot = snapshot_fixture();
    let bytes = snapshot
        .try_to_bytes()
        .expect("fixture snapshot should serialize");

    for (index, mutation) in MALFORMED_SNAPSHOT_MUTATIONS.iter().copied().enumerate() {
        let mutated = mutation.mutated_bytes(
            &bytes,
            &snapshot,
            MutationInput {
                trailing_byte: index as u8,
                truncated_header_len: index,
            },
        );
        assert!(
            Snapshot::from_bytes(&mutated).is_err(),
            "mutation {mutation:?} should reject"
        );
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn valid_snapshots_roundtrip(snapshot in valid_snapshot_strategy()) {
        let bytes = snapshot
            .try_to_bytes()
            .expect("generated snapshot should serialize");
        let infallible_bytes = snapshot.to_bytes();
        prop_assert_eq!(infallible_bytes.as_slice(), bytes.as_slice());
        prop_assert_eq!(&bytes[..SNAPSHOT_MAGIC.len()], &SNAPSHOT_MAGIC);
        prop_assert_eq!(bytes.len(), SNAPSHOT_HEADER_LEN + snapshot.memory_len());

        let decoded = Snapshot::from_bytes(&bytes).expect("generated snapshot should decode");
        let decoded_bytes = decoded
            .try_to_bytes()
            .expect("decoded snapshot should serialize");
        prop_assert_eq!(decoded_bytes.as_slice(), bytes.as_slice());
        let decoded_infallible_bytes = decoded.to_bytes();
        prop_assert_eq!(decoded_infallible_bytes.as_slice(), bytes.as_slice());
        prop_assert_eq!(decoded, snapshot);
    }

    #[test]
    fn arbitrary_snapshot_bytes_do_not_panic(
        bytes in prop::collection::vec(
            any::<u8>(),
            0..=(SNAPSHOT_HEADER_LEN + (2 * WASM_PAGE_SIZE) + 16),
        )
    ) {
        let _ = Snapshot::from_bytes(&bytes);
    }

    #[test]
    fn arbitrary_snapshot_metadata_bytes_do_not_panic(
        bytes in prop::collection::vec(
            any::<u8>(),
            0..=(SNAPSHOT_HEADER_LEN + (2 * WASM_PAGE_SIZE) + 16),
        )
    ) {
        let _ = Snapshot::metadata_from_bytes(&bytes);
    }

    #[test]
    fn malformed_snapshot_envelopes_are_rejected(
        snapshot in valid_snapshot_strategy(),
        mutation in malformed_snapshot_mutation_strategy(),
        trailing_byte in any::<u8>(),
        truncated_header_len in 0usize..SNAPSHOT_HEADER_LEN,
    ) {
        let bytes = snapshot
            .try_to_bytes()
            .expect("generated snapshot should serialize");

        let bytes = mutation.mutated_bytes(
            &bytes,
            &snapshot,
            MutationInput {
                trailing_byte,
                truncated_header_len,
            },
        );

        match Snapshot::from_bytes(&bytes) {
            Ok(_) => prop_assert!(false, "mutation {mutation:?} should reject"),
            Err(err) => prop_assert!(
                err.to_string().contains(mutation.expected_error()),
                "mutation {mutation:?}: expected error to contain {:?}, got {err:#}",
                mutation.expected_error(),
            ),
        }
    }
}
