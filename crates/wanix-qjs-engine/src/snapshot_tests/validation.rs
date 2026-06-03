use super::malformed::{MALFORMED_SNAPSHOT_MUTATIONS, MalformedSnapshotMutation, MutationInput};
use super::*;

#[test]
fn rejects_malformed_snapshot_bytes() {
    let snapshot = snapshot_fixture();
    let bytes = snapshot
        .try_to_bytes()
        .expect("fixture snapshot should serialize");

    for mutation in MALFORMED_SNAPSHOT_MUTATIONS {
        let mutated = mutation.mutated_bytes(&bytes, &snapshot, MutationInput::default());
        expect_snapshot_decode_error_for_mutation(*mutation, &mutated);
        expect_snapshot_metadata_decode_error_for_mutation(*mutation, &mutated);
    }
}

#[test]
fn snapshot_debug_hides_guest_capability_pointers() {
    let debug = format!("{:?}", snapshot_fixture());

    assert!(debug.contains("Snapshot"));
    assert!(debug.contains("memory_len"));
    assert!(!debug.contains("stack_pointer"));
    assert!(!debug.contains("runtime_ptr"));
    assert!(!debug.contains("context_ptr"));
}

#[test]
fn snapshot_pointer_boundaries_distinguish_objects_from_stack() {
    let snapshot = snapshot_fixture();
    let bytes = snapshot
        .try_to_bytes()
        .expect("fixture snapshot should serialize");
    let memory_len = fixture_len_u32(snapshot.memory_len());

    let mut runtime_at_end = bytes.clone();
    overwrite_u32(&mut runtime_at_end, SNAPSHOT_RUNTIME_PTR_OFFSET, memory_len);
    expect_snapshot_decode_error(&runtime_at_end, "runtime_ptr is outside");

    let mut context_at_end = bytes.clone();
    overwrite_u32(&mut context_at_end, SNAPSHOT_CONTEXT_PTR_OFFSET, memory_len);
    expect_snapshot_decode_error(&context_at_end, "context_ptr is outside");

    let mut high_runtime = bytes.clone();
    overwrite_u32(&mut high_runtime, SNAPSHOT_RUNTIME_PTR_OFFSET, u32::MAX);
    expect_snapshot_decode_error(&high_runtime, "runtime_ptr is outside");

    let mut stack_at_end = bytes.clone();
    overwrite_u32(&mut stack_at_end, SNAPSHOT_STACK_POINTER_OFFSET, memory_len);
    let decoded = Snapshot::from_bytes(&stack_at_end).expect("stack may point at memory end");
    assert_eq!(decoded.stack_pointer, memory_len);

    let mut stack_after_end = bytes;
    overwrite_u32(
        &mut stack_after_end,
        SNAPSHOT_STACK_POINTER_OFFSET,
        memory_len + 1,
    );
    expect_snapshot_decode_error(&stack_after_end, "stack pointer is outside");
}

fn expect_snapshot_decode_error_for_mutation(mutation: MalformedSnapshotMutation, bytes: &[u8]) {
    let expected = mutation.expected_error();
    match Snapshot::from_bytes(bytes) {
        Ok(_) => panic!("mutation {mutation:?}: snapshot bytes should be rejected"),
        Err(err) => assert!(
            err.to_string().contains(expected),
            "mutation {mutation:?}: expected error to contain {expected:?}, got {err:#}"
        ),
    }
}

fn expect_snapshot_metadata_decode_error_for_mutation(
    mutation: MalformedSnapshotMutation,
    bytes: &[u8],
) {
    let expected = mutation.expected_error();
    match Snapshot::metadata_from_bytes(bytes) {
        Ok(_) => panic!("mutation {mutation:?}: snapshot metadata should be rejected"),
        Err(err) => assert!(
            err.to_string().contains(expected),
            "mutation {mutation:?}: expected metadata error to contain {expected:?}, got {err:#}"
        ),
    }
}
