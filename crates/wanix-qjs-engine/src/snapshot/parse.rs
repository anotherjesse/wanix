use crate::QuickJsModule;
use anyhow::{Result, anyhow, bail};

use super::format::SnapshotReader;
use super::{SNAPSHOT_HEADER_LEN, SNAPSHOT_MAGIC, SnapshotMetadata, validate_snapshot_structure};

pub(super) struct ParsedSnapshotHeader {
    pub(super) metadata: SnapshotMetadata,
    pub(super) stack_pointer: u32,
    pub(super) runtime_ptr: u32,
    pub(super) context_ptr: u32,
}

pub(super) fn parse_snapshot_header(bytes: &[u8]) -> Result<ParsedSnapshotHeader> {
    let mut reader = SnapshotReader::new(bytes);
    let magic = reader.read_array::<8>("magic")?;
    if magic != SNAPSHOT_MAGIC {
        bail!("invalid snapshot magic");
    }
    let format_version = reader.read_u32("format version")?;
    let abi_version = reader.read_u32("ABI version")?;
    let header_len = usize::try_from(reader.read_u32("header length")?)
        .map_err(|_| anyhow!("snapshot header length does not fit in usize"))?;
    let total_len = usize::try_from(reader.read_u64("total length")?)
        .map_err(|_| anyhow!("snapshot total length does not fit in usize"))?;
    let memory_len = usize::try_from(reader.read_u64("memory length")?)
        .map_err(|_| anyhow!("snapshot memory length does not fit in usize"))?;
    let stack_pointer = reader.read_u32("stack pointer")?;
    let runtime_ptr = reader.read_u32("runtime pointer")?;
    let context_ptr = reader.read_u32("context pointer")?;
    let wasm_sha256 = reader.read_array::<32>("wasm SHA-256")?;

    if header_len != SNAPSHOT_HEADER_LEN {
        bail!("unsupported snapshot header length {header_len} (expected {SNAPSHOT_HEADER_LEN})");
    }
    if total_len != bytes.len() {
        bail!("snapshot total length does not match input length");
    }
    let expected_total = header_len
        .checked_add(memory_len)
        .ok_or_else(|| anyhow!("snapshot memory length overflows total length"))?;
    if expected_total != total_len {
        bail!("snapshot memory length does not match total length");
    }
    validate_snapshot_structure(
        format_version,
        abi_version,
        memory_len,
        stack_pointer,
        runtime_ptr,
        context_ptr,
    )?;

    Ok(ParsedSnapshotHeader {
        metadata: SnapshotMetadata {
            format_version,
            abi_version,
            wasm_sha256,
            memory_len,
        },
        stack_pointer,
        runtime_ptr,
        context_ptr,
    })
}

pub(super) fn validate_metadata_for_module(
    metadata: SnapshotMetadata,
    module: &QuickJsModule,
) -> Result<()> {
    let module_sha256 = module.wasm_sha256();
    if metadata.wasm_sha256 != module_sha256 {
        bail!(
            "snapshot was produced by a different QuickJS WASM module (snapshot SHA-256 {}, module SHA-256 {})",
            format_sha256_hex(&metadata.wasm_sha256),
            format_sha256_hex(&module_sha256)
        );
    }
    let minimum_memory_len = module.minimum_memory_len()?;
    if metadata.memory_len < minimum_memory_len {
        bail!(
            "snapshot memory length {} is smaller than QuickJS WASM module minimum memory length {minimum_memory_len}",
            metadata.memory_len
        );
    }
    Ok(())
}

fn format_sha256_hex(bytes: &[u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut hex = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        hex.push(char::from(HEX[usize::from(byte >> 4)]));
        hex.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    hex
}
