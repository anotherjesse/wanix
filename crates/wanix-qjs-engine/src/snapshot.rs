use crate::QuickJsModule;
use crate::allocation::try_copy_bytes;
use anyhow::{Result, anyhow};
use std::fmt;

mod format;
mod parse;
mod validation;

pub(crate) use crate::module::QUICKJS_WASM_ABI_VERSION;
#[cfg(test)]
pub(crate) use format::{
    SNAPSHOT_ABI_VERSION_OFFSET, SNAPSHOT_CONTEXT_PTR_OFFSET, SNAPSHOT_FORMAT_VERSION_OFFSET,
    SNAPSHOT_HEADER_LEN_OFFSET, SNAPSHOT_MEMORY_LEN_OFFSET, SNAPSHOT_RUNTIME_PTR_OFFSET,
    SNAPSHOT_STACK_POINTER_OFFSET, SNAPSHOT_TOTAL_LEN_OFFSET, SNAPSHOT_WASM_SHA256_OFFSET,
};
pub(crate) use format::{
    SNAPSHOT_FORMAT_VERSION, SNAPSHOT_HEADER_LEN, SNAPSHOT_MAGIC, WASM_PAGE_SIZE,
};
use format::{snapshot_header_len_u32, snapshot_len_u64, snapshot_total_len, write_u32, write_u64};
use parse::{parse_snapshot_header, validate_metadata_for_module};
pub(crate) use validation::snapshot_memory_page_count;
use validation::validate_snapshot_structure;

/// Compatibility metadata from a snapshot byte envelope.
///
/// Metadata can be parsed without copying the snapshot's WebAssembly memory
/// image, which lets callers cheaply route or reject persisted snapshots before
/// attempting a full restore.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SnapshotMetadata {
    format_version: u32,
    abi_version: u32,
    wasm_sha256: [u8; 32],
    memory_len: usize,
}

impl SnapshotMetadata {
    /// Returns the snapshot byte-format version.
    #[must_use]
    pub fn format_version(&self) -> u32 {
        self.format_version
    }

    /// Returns the QuickJS WebAssembly ABI version.
    #[must_use]
    pub fn abi_version(&self) -> u32 {
        self.abi_version
    }

    /// Returns the SHA-256 identity of the wasm module that produced the snapshot.
    #[must_use]
    pub fn wasm_sha256(&self) -> [u8; 32] {
        self.wasm_sha256
    }

    /// Returns the captured WebAssembly linear memory length in bytes.
    #[must_use]
    pub fn memory_len(&self) -> usize {
        self.memory_len
    }
}

/// A serialized VM image for one QuickJS WebAssembly module build.
///
/// A snapshot contains the full WebAssembly linear memory plus the VM pointers
/// needed to reattach QuickJS after instantiating the same wasm module again.
/// Use
/// [`QuickJsModule::restore_runtime_from_bytes`](crate::QuickJsModule::restore_runtime_from_bytes)
/// when restoring directly from persisted bytes, or
/// [`Snapshot::from_bytes_for_module`] when you need to inspect a snapshot value
/// before restoring it.
#[derive(Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub(crate) format_version: u32,
    pub(crate) abi_version: u32,
    pub(crate) wasm_sha256: [u8; 32],
    pub(crate) memory: Vec<u8>,
    pub(crate) stack_pointer: u32,
    pub(crate) runtime_ptr: u32,
    pub(crate) context_ptr: u32,
}

impl Snapshot {
    /// Returns compatibility metadata for this already-decoded snapshot.
    #[must_use]
    pub fn metadata(&self) -> SnapshotMetadata {
        SnapshotMetadata {
            format_version: self.format_version,
            abi_version: self.abi_version,
            wasm_sha256: self.wasm_sha256,
            memory_len: self.memory.len(),
        }
    }

    /// Returns the captured WebAssembly linear memory length in bytes.
    #[must_use]
    pub fn memory_len(&self) -> usize {
        self.memory.len()
    }

    /// Returns the SHA-256 identity of the wasm module that produced this snapshot.
    #[must_use]
    pub fn wasm_sha256(&self) -> [u8; 32] {
        self.wasm_sha256
    }

    /// Parses and validates snapshot metadata without copying the WebAssembly memory image.
    ///
    /// This checks the complete restore header, including saved guest pointer
    /// fields, while returning only route-friendly metadata.
    ///
    /// # Errors
    ///
    /// Returns an error if the byte envelope is malformed, unsupported, or
    /// structurally invalid.
    pub fn metadata_from_bytes(bytes: &[u8]) -> Result<SnapshotMetadata> {
        Ok(parse_snapshot_header(bytes)?.metadata)
    }

    /// Serializes this snapshot to the canonical little-endian byte format.
    ///
    /// # Panics
    ///
    /// Panics if this snapshot's byte length cannot fit or allocate for the
    /// canonical byte format. Use [`Snapshot::try_to_bytes`] to handle those
    /// conditions as errors.
    #[must_use]
    #[allow(clippy::expect_used)]
    pub fn to_bytes(&self) -> Vec<u8> {
        self.try_to_bytes()
            .expect("snapshot lengths should fit the canonical byte format")
    }

    /// Serializes this snapshot to the canonical little-endian byte format.
    ///
    /// # Errors
    ///
    /// Returns an error if this snapshot's memory length or total byte length
    /// cannot fit the canonical byte format, or if the output buffer cannot be
    /// allocated.
    pub fn try_to_bytes(&self) -> Result<Vec<u8>> {
        let total_len = snapshot_total_len(SNAPSHOT_HEADER_LEN, self.memory.len())?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(total_len)
            .map_err(|err| anyhow!("snapshot byte buffer allocation failed: {err}"))?;
        bytes.extend_from_slice(&SNAPSHOT_MAGIC);
        write_u32(&mut bytes, self.format_version);
        write_u32(&mut bytes, self.abi_version);
        write_u32(&mut bytes, snapshot_header_len_u32()?);
        write_u64(&mut bytes, snapshot_len_u64(total_len, "total")?);
        write_u64(&mut bytes, snapshot_len_u64(self.memory.len(), "memory")?);
        write_u32(&mut bytes, self.stack_pointer);
        write_u32(&mut bytes, self.runtime_ptr);
        write_u32(&mut bytes, self.context_ptr);
        bytes.extend_from_slice(&self.wasm_sha256);
        bytes.extend_from_slice(&self.memory);
        Ok(bytes)
    }

    /// Parses a snapshot without checking it against a module.
    ///
    /// Prefer
    /// [`QuickJsModule::restore_runtime_from_bytes`](crate::QuickJsModule::restore_runtime_from_bytes)
    /// when restoring directly from persisted bytes. Use
    /// [`Snapshot::from_bytes_for_module`] when the target module is available
    /// and you need a validated snapshot value before restore.
    ///
    /// # Errors
    ///
    /// Returns an error if the byte envelope is malformed, unsupported,
    /// structurally invalid, or the memory copy cannot be allocated.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        let header = parse_snapshot_header(bytes)?;
        let metadata = header.metadata;
        let memory = try_copy_bytes(&bytes[SNAPSHOT_HEADER_LEN..], "snapshot memory")?;

        Ok(Self {
            format_version: metadata.format_version,
            abi_version: metadata.abi_version,
            wasm_sha256: metadata.wasm_sha256,
            memory,
            stack_pointer: header.stack_pointer,
            runtime_ptr: header.runtime_ptr,
            context_ptr: header.context_ptr,
        })
    }

    /// Parses a snapshot and validates it against a compiled module.
    ///
    /// # Errors
    ///
    /// Returns an error if parsing fails or the snapshot was produced by a
    /// different QuickJS wasm module, or if the snapshot memory is smaller
    /// than the module's exported memory minimum.
    pub fn from_bytes_for_module(bytes: &[u8], module: &QuickJsModule) -> Result<Self> {
        let metadata = Self::metadata_from_bytes(bytes)?;
        validate_metadata_for_module(metadata, module)?;
        let snapshot = Self::from_bytes(bytes)?;
        Ok(snapshot)
    }

    pub(crate) fn validate_structure(&self) -> Result<()> {
        validate_snapshot_structure(
            self.format_version,
            self.abi_version,
            self.memory.len(),
            self.stack_pointer,
            self.runtime_ptr,
            self.context_ptr,
        )
    }

    pub(crate) fn validate_for(&self, module: &QuickJsModule) -> Result<()> {
        self.validate_structure()?;
        validate_metadata_for_module(self.metadata(), module)
    }
}

impl fmt::Debug for Snapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Snapshot")
            .field("format_version", &self.format_version)
            .field("abi_version", &self.abi_version)
            .field("wasm_sha256", &self.wasm_sha256)
            .field("memory_len", &self.memory.len())
            .finish()
    }
}
