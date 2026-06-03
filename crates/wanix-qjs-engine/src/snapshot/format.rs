use anyhow::{Result, anyhow, bail};

pub(crate) const WASM_PAGE_SIZE: usize = 64 * 1024;
pub(crate) const SNAPSHOT_FORMAT_VERSION: u32 = 1;
pub(crate) const SNAPSHOT_MAGIC: [u8; 8] = *b"RWQSNAP\0";
pub(crate) const SNAPSHOT_FORMAT_VERSION_OFFSET: usize = SNAPSHOT_MAGIC.len();
pub(crate) const SNAPSHOT_ABI_VERSION_OFFSET: usize = SNAPSHOT_FORMAT_VERSION_OFFSET + 4;
pub(crate) const SNAPSHOT_HEADER_LEN_OFFSET: usize = SNAPSHOT_ABI_VERSION_OFFSET + 4;
pub(crate) const SNAPSHOT_TOTAL_LEN_OFFSET: usize = SNAPSHOT_HEADER_LEN_OFFSET + 4;
pub(crate) const SNAPSHOT_MEMORY_LEN_OFFSET: usize = SNAPSHOT_TOTAL_LEN_OFFSET + 8;
pub(crate) const SNAPSHOT_STACK_POINTER_OFFSET: usize = SNAPSHOT_MEMORY_LEN_OFFSET + 8;
pub(crate) const SNAPSHOT_RUNTIME_PTR_OFFSET: usize = SNAPSHOT_STACK_POINTER_OFFSET + 4;
pub(crate) const SNAPSHOT_CONTEXT_PTR_OFFSET: usize = SNAPSHOT_RUNTIME_PTR_OFFSET + 4;
pub(crate) const SNAPSHOT_WASM_SHA256_OFFSET: usize = SNAPSHOT_CONTEXT_PTR_OFFSET + 4;
pub(crate) const SNAPSHOT_HEADER_LEN: usize = SNAPSHOT_WASM_SHA256_OFFSET + 32;

pub(super) struct SnapshotReader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> SnapshotReader<'a> {
    pub(super) fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    pub(super) fn read_u32(&mut self, field: &str) -> Result<u32> {
        Ok(u32::from_le_bytes(self.read_array(field)?))
    }

    pub(super) fn read_u64(&mut self, field: &str) -> Result<u64> {
        Ok(u64::from_le_bytes(self.read_array(field)?))
    }

    pub(super) fn read_array<const N: usize>(&mut self, field: &str) -> Result<[u8; N]> {
        let bytes = self.read_bytes(N, field)?;
        let mut array = [0; N];
        array.copy_from_slice(bytes);
        Ok(array)
    }

    pub(super) fn read_bytes(&mut self, len: usize, field: &str) -> Result<&'a [u8]> {
        let end = self
            .offset
            .checked_add(len)
            .ok_or_else(|| anyhow!("snapshot {field} length overflows input offset"))?;
        if end > self.bytes.len() {
            bail!("snapshot is truncated while reading {field}");
        }
        let bytes = &self.bytes[self.offset..end];
        self.offset = end;
        Ok(bytes)
    }
}

pub(super) fn snapshot_header_len_u32() -> Result<u32> {
    u32::try_from(SNAPSHOT_HEADER_LEN)
        .map_err(|_| anyhow!("snapshot header length should fit in u32"))
}

pub(super) fn snapshot_len_u64(len: usize, field: &str) -> Result<u64> {
    u64::try_from(len).map_err(|_| anyhow!("snapshot {field} length should fit in u64"))
}

pub(super) fn snapshot_total_len(header_len: usize, memory_len: usize) -> Result<usize> {
    header_len
        .checked_add(memory_len)
        .ok_or_else(|| anyhow!("snapshot total length should fit in usize"))
}

pub(super) fn write_u32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

pub(super) fn write_u64(bytes: &mut Vec<u8>, value: u64) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_header_length_fits_serialized_field() -> Result<()> {
        assert_eq!(
            usize::try_from(snapshot_header_len_u32()?).ok(),
            Some(SNAPSHOT_HEADER_LEN)
        );
        Ok(())
    }

    #[test]
    fn snapshot_total_length_rejects_overflow() {
        let err = snapshot_total_len(SNAPSHOT_HEADER_LEN, usize::MAX)
            .expect_err("total length should reject usize overflow");
        assert!(err.to_string().contains("total length"));
    }
}
