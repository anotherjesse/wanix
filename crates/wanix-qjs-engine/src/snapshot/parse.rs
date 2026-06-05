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

struct SnapshotHeaderFields {
    format_version: u32,
    abi_version: u32,
    header_len: usize,
    total_len: usize,
    memory_len: usize,
    stack_pointer: u32,
    runtime_ptr: u32,
    context_ptr: u32,
    wasm_sha256: [u8; 32],
}

pub(super) fn parse_snapshot_header(bytes: &[u8]) -> Result<ParsedSnapshotHeader> {
    let fields = read_snapshot_header_fields(bytes)?;

    validate_snapshot_envelope_lengths(
        bytes.len(),
        fields.header_len,
        fields.total_len,
        fields.memory_len,
    )?;
    validate_snapshot_structure(
        fields.format_version,
        fields.abi_version,
        fields.memory_len,
        fields.stack_pointer,
        fields.runtime_ptr,
        fields.context_ptr,
    )?;

    Ok(ParsedSnapshotHeader {
        metadata: SnapshotMetadata {
            format_version: fields.format_version,
            abi_version: fields.abi_version,
            wasm_sha256: fields.wasm_sha256,
            memory_len: fields.memory_len,
        },
        stack_pointer: fields.stack_pointer,
        runtime_ptr: fields.runtime_ptr,
        context_ptr: fields.context_ptr,
    })
}

fn read_snapshot_header_fields(bytes: &[u8]) -> Result<SnapshotHeaderFields> {
    let mut reader = SnapshotReader::new(bytes);
    validate_snapshot_magic(reader.read_array::<8>("magic")?)?;
    Ok(SnapshotHeaderFields {
        format_version: reader.read_u32("format version")?,
        abi_version: reader.read_u32("ABI version")?,
        header_len: read_usize_u32(&mut reader, "header length")?,
        total_len: read_usize_u64(&mut reader, "total length")?,
        memory_len: read_usize_u64(&mut reader, "memory length")?,
        stack_pointer: reader.read_u32("stack pointer")?,
        runtime_ptr: reader.read_u32("runtime pointer")?,
        context_ptr: reader.read_u32("context pointer")?,
        wasm_sha256: reader.read_array::<32>("wasm SHA-256")?,
    })
}

fn validate_snapshot_magic(magic: [u8; 8]) -> Result<()> {
    if magic != SNAPSHOT_MAGIC {
        bail!("invalid snapshot magic");
    }
    Ok(())
}

fn read_usize_u32(reader: &mut SnapshotReader<'_>, field: &str) -> Result<usize> {
    usize::try_from(reader.read_u32(field)?)
        .map_err(|_| anyhow!("snapshot {field} does not fit in usize"))
}

fn read_usize_u64(reader: &mut SnapshotReader<'_>, field: &str) -> Result<usize> {
    usize::try_from(reader.read_u64(field)?)
        .map_err(|_| anyhow!("snapshot {field} does not fit in usize"))
}

fn validate_snapshot_envelope_lengths(
    input_len: usize,
    header_len: usize,
    total_len: usize,
    memory_len: usize,
) -> Result<()> {
    if header_len != SNAPSHOT_HEADER_LEN {
        bail!("unsupported snapshot header length {header_len} (expected {SNAPSHOT_HEADER_LEN})");
    }
    if total_len != input_len {
        bail!("snapshot total length does not match input length");
    }
    let expected_total = header_len
        .checked_add(memory_len)
        .ok_or_else(|| anyhow!("snapshot memory length overflows total length"))?;
    if expected_total != total_len {
        bail!("snapshot memory length does not match total length");
    }
    Ok(())
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
