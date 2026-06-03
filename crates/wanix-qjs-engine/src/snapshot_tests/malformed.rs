use super::*;

#[derive(Clone, Copy, Debug)]
pub(super) struct MutationInput {
    pub(super) trailing_byte: u8,
    pub(super) truncated_header_len: usize,
}

impl Default for MutationInput {
    fn default() -> Self {
        Self {
            trailing_byte: 0,
            truncated_header_len: SNAPSHOT_HEADER_LEN - 1,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) enum MalformedSnapshotMutation {
    BadMagic,
    TruncatedHeader,
    UnsupportedFormatVersion,
    UnsupportedAbiVersion,
    HeaderLengthTooSmall,
    HeaderLengthTooLarge,
    TotalLengthTooSmall,
    TotalLengthTooLarge,
    MemoryLengthTooSmall,
    MemoryLengthTooLarge,
    AdjustedTrailingBytes,
    EmptyMemory,
    UnalignedMemory,
    HugeMemoryLength,
    RuntimePointerNull,
    RuntimePointerAtMemoryEnd,
    RuntimePointerTooLarge,
    ContextPointerNull,
    ContextPointerAtMemoryEnd,
    ContextPointerTooLarge,
    StackPointerNull,
    StackPointerAfterMemoryEnd,
    TrailingByte,
}

pub(super) const MALFORMED_SNAPSHOT_MUTATIONS: &[MalformedSnapshotMutation] = &[
    MalformedSnapshotMutation::BadMagic,
    MalformedSnapshotMutation::TruncatedHeader,
    MalformedSnapshotMutation::UnsupportedFormatVersion,
    MalformedSnapshotMutation::UnsupportedAbiVersion,
    MalformedSnapshotMutation::HeaderLengthTooSmall,
    MalformedSnapshotMutation::HeaderLengthTooLarge,
    MalformedSnapshotMutation::TotalLengthTooSmall,
    MalformedSnapshotMutation::TotalLengthTooLarge,
    MalformedSnapshotMutation::MemoryLengthTooSmall,
    MalformedSnapshotMutation::MemoryLengthTooLarge,
    MalformedSnapshotMutation::AdjustedTrailingBytes,
    MalformedSnapshotMutation::EmptyMemory,
    MalformedSnapshotMutation::UnalignedMemory,
    MalformedSnapshotMutation::HugeMemoryLength,
    MalformedSnapshotMutation::RuntimePointerNull,
    MalformedSnapshotMutation::RuntimePointerAtMemoryEnd,
    MalformedSnapshotMutation::RuntimePointerTooLarge,
    MalformedSnapshotMutation::ContextPointerNull,
    MalformedSnapshotMutation::ContextPointerAtMemoryEnd,
    MalformedSnapshotMutation::ContextPointerTooLarge,
    MalformedSnapshotMutation::StackPointerNull,
    MalformedSnapshotMutation::StackPointerAfterMemoryEnd,
    MalformedSnapshotMutation::TrailingByte,
];

impl MalformedSnapshotMutation {
    pub(super) fn mutated_bytes(
        self,
        bytes: &[u8],
        snapshot: &Snapshot,
        input: MutationInput,
    ) -> Vec<u8> {
        match self {
            Self::BadMagic => mutate(bytes, |bytes| bytes[0] ^= 0xff),
            Self::TruncatedHeader => bytes[..input.truncated_header_len].to_vec(),
            Self::UnsupportedFormatVersion => mutate(bytes, |bytes| {
                overwrite_u32(
                    bytes,
                    SNAPSHOT_FORMAT_VERSION_OFFSET,
                    SNAPSHOT_FORMAT_VERSION + 1,
                );
            }),
            Self::UnsupportedAbiVersion => mutate(bytes, |bytes| {
                overwrite_u32(
                    bytes,
                    SNAPSHOT_ABI_VERSION_OFFSET,
                    QUICKJS_WASM_ABI_VERSION + 1,
                );
            }),
            Self::HeaderLengthTooSmall => mutate(bytes, |bytes| {
                overwrite_u32(
                    bytes,
                    SNAPSHOT_HEADER_LEN_OFFSET,
                    fixture_len_u32(SNAPSHOT_HEADER_LEN - 1),
                );
            }),
            Self::HeaderLengthTooLarge => mutate(bytes, |bytes| {
                overwrite_u32(
                    bytes,
                    SNAPSHOT_HEADER_LEN_OFFSET,
                    fixture_len_u32(SNAPSHOT_HEADER_LEN + 4),
                );
            }),
            Self::TotalLengthTooSmall => mutate(bytes, |bytes| {
                let total_len = fixture_len_u64(bytes.len() - 1);
                overwrite_u64(bytes, SNAPSHOT_TOTAL_LEN_OFFSET, total_len);
            }),
            Self::TotalLengthTooLarge => mutate(bytes, |bytes| {
                let total_len = fixture_len_u64(bytes.len() + 1);
                overwrite_u64(bytes, SNAPSHOT_TOTAL_LEN_OFFSET, total_len);
            }),
            Self::MemoryLengthTooSmall => mutate(bytes, |bytes| {
                overwrite_u64(
                    bytes,
                    SNAPSHOT_MEMORY_LEN_OFFSET,
                    fixture_len_u64(snapshot.memory_len() - 1),
                );
            }),
            Self::MemoryLengthTooLarge => mutate(bytes, |bytes| {
                overwrite_u64(
                    bytes,
                    SNAPSHOT_MEMORY_LEN_OFFSET,
                    fixture_len_u64(snapshot.memory_len() + 1),
                );
            }),
            Self::AdjustedTrailingBytes => mutate(bytes, |bytes| {
                bytes.push(input.trailing_byte);
                let total_len = fixture_len_u64(bytes.len());
                overwrite_u64(bytes, SNAPSHOT_TOTAL_LEN_OFFSET, total_len);
            }),
            Self::EmptyMemory => mutate_header(bytes, |bytes| {
                overwrite_u64(
                    bytes,
                    SNAPSHOT_TOTAL_LEN_OFFSET,
                    fixture_len_u64(SNAPSHOT_HEADER_LEN),
                );
                overwrite_u64(bytes, SNAPSHOT_MEMORY_LEN_OFFSET, 0);
            }),
            Self::UnalignedMemory => mutate_header(bytes, |bytes| {
                bytes.push(input.trailing_byte);
                overwrite_u64(
                    bytes,
                    SNAPSHOT_TOTAL_LEN_OFFSET,
                    fixture_len_u64(SNAPSHOT_HEADER_LEN + 1),
                );
                overwrite_u64(bytes, SNAPSHOT_MEMORY_LEN_OFFSET, 1);
            }),
            Self::HugeMemoryLength => mutate(bytes, |bytes| {
                overwrite_u64(bytes, SNAPSHOT_MEMORY_LEN_OFFSET, u64::MAX);
            }),
            Self::RuntimePointerNull => mutate(bytes, |bytes| {
                overwrite_u32(bytes, SNAPSHOT_RUNTIME_PTR_OFFSET, 0);
            }),
            Self::RuntimePointerAtMemoryEnd => mutate(bytes, |bytes| {
                overwrite_u32(
                    bytes,
                    SNAPSHOT_RUNTIME_PTR_OFFSET,
                    fixture_len_u32(snapshot.memory_len()),
                );
            }),
            Self::RuntimePointerTooLarge => mutate(bytes, |bytes| {
                overwrite_u32(bytes, SNAPSHOT_RUNTIME_PTR_OFFSET, u32::MAX);
            }),
            Self::ContextPointerNull => mutate(bytes, |bytes| {
                overwrite_u32(bytes, SNAPSHOT_CONTEXT_PTR_OFFSET, 0);
            }),
            Self::ContextPointerAtMemoryEnd => mutate(bytes, |bytes| {
                overwrite_u32(
                    bytes,
                    SNAPSHOT_CONTEXT_PTR_OFFSET,
                    fixture_len_u32(snapshot.memory_len()),
                );
            }),
            Self::ContextPointerTooLarge => mutate(bytes, |bytes| {
                overwrite_u32(bytes, SNAPSHOT_CONTEXT_PTR_OFFSET, u32::MAX);
            }),
            Self::StackPointerNull => mutate(bytes, |bytes| {
                overwrite_u32(bytes, SNAPSHOT_STACK_POINTER_OFFSET, 0);
            }),
            Self::StackPointerAfterMemoryEnd => mutate(bytes, |bytes| {
                overwrite_u32(
                    bytes,
                    SNAPSHOT_STACK_POINTER_OFFSET,
                    fixture_len_u32(snapshot.memory_len() + 1),
                );
            }),
            Self::TrailingByte => mutate(bytes, |bytes| bytes.push(input.trailing_byte)),
        }
    }

    pub(super) fn expected_error(self) -> &'static str {
        match self {
            Self::BadMagic => "snapshot magic",
            Self::TruncatedHeader => "truncated",
            Self::UnsupportedFormatVersion => "snapshot format version",
            Self::UnsupportedAbiVersion => "QuickJS WASM ABI version",
            Self::HeaderLengthTooSmall | Self::HeaderLengthTooLarge => "snapshot header length",
            Self::TotalLengthTooSmall | Self::TotalLengthTooLarge | Self::TrailingByte => {
                "snapshot total length"
            }
            Self::MemoryLengthTooSmall
            | Self::MemoryLengthTooLarge
            | Self::AdjustedTrailingBytes
            | Self::HugeMemoryLength => "snapshot memory length",
            Self::EmptyMemory => "snapshot memory is empty",
            Self::UnalignedMemory => "page aligned",
            Self::RuntimePointerNull => "runtime_ptr is null",
            Self::RuntimePointerAtMemoryEnd | Self::RuntimePointerTooLarge => {
                "runtime_ptr is outside"
            }
            Self::ContextPointerNull => "context_ptr is null",
            Self::ContextPointerAtMemoryEnd | Self::ContextPointerTooLarge => {
                "context_ptr is outside"
            }
            Self::StackPointerNull => "stack pointer",
            Self::StackPointerAfterMemoryEnd => "stack pointer is outside",
        }
    }
}

fn mutate(bytes: &[u8], mutation: impl FnOnce(&mut Vec<u8>)) -> Vec<u8> {
    let mut mutated = bytes.to_vec();
    mutation(&mut mutated);
    mutated
}

fn mutate_header(bytes: &[u8], mutation: impl FnOnce(&mut Vec<u8>)) -> Vec<u8> {
    let mut mutated = bytes[..SNAPSHOT_HEADER_LEN].to_vec();
    mutation(&mut mutated);
    mutated
}
