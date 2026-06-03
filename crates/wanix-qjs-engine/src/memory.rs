use anyhow::{Result, bail};

pub(crate) const QUICKJS_MEMORY_USAGE_FIELD_COUNT: usize = 26;
pub(crate) const QUICKJS_MEMORY_USAGE_BYTE_LEN: u32 = (QUICKJS_MEMORY_USAGE_FIELD_COUNT * 8) as u32;

/// Copied QuickJS runtime memory usage counters.
///
/// These fields mirror the reference adapter's flattened `JSMemoryUsage` field
/// order as signed 64-bit diagnostics. Values are expected to be non-negative
/// for normal QuickJS runtimes, but the API preserves the C ABI representation
/// instead of reinterpreting counters as Rust-native sizes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct QuickJsMemoryUsage {
    /// Total bytes currently allocated through QuickJS malloc accounting.
    pub malloc_size: i64,
    /// Current QuickJS malloc limit in bytes, or `0` when unlimited.
    pub malloc_limit: i64,
    /// Estimated live QuickJS memory usage in bytes.
    pub memory_used_size: i64,
    /// Number of active QuickJS malloc allocations.
    pub malloc_count: i64,
    /// Number of live QuickJS memory-using entities.
    pub memory_used_count: i64,
    /// Number of interned atoms.
    pub atom_count: i64,
    /// Bytes used by interned atoms.
    pub atom_size: i64,
    /// Number of strings.
    pub str_count: i64,
    /// Bytes used by strings.
    pub str_size: i64,
    /// Number of JavaScript objects.
    pub obj_count: i64,
    /// Bytes used by JavaScript objects.
    pub obj_size: i64,
    /// Number of object properties.
    pub prop_count: i64,
    /// Bytes used by object properties.
    pub prop_size: i64,
    /// Number of object shapes.
    pub shape_count: i64,
    /// Bytes used by object shapes.
    pub shape_size: i64,
    /// Number of JavaScript function bytecode records.
    pub js_func_count: i64,
    /// Bytes used by JavaScript function records.
    pub js_func_size: i64,
    /// Bytes used by JavaScript function bytecode.
    pub js_func_code_size: i64,
    /// Number of JavaScript function pc-to-line records.
    pub js_func_pc2line_count: i64,
    /// Bytes used by JavaScript function pc-to-line records.
    pub js_func_pc2line_size: i64,
    /// Number of C function objects.
    pub c_func_count: i64,
    /// Number of arrays.
    pub array_count: i64,
    /// Number of arrays using QuickJS's fast-array representation.
    pub fast_array_count: i64,
    /// Number of elements stored in fast arrays.
    pub fast_array_elements: i64,
    /// Number of binary objects.
    pub binary_object_count: i64,
    /// Bytes used by binary objects.
    pub binary_object_size: i64,
}

impl QuickJsMemoryUsage {
    pub(crate) fn from_le_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.len() != QUICKJS_MEMORY_USAGE_BYTE_LEN as usize {
            bail!(
                "QuickJS memory usage buffer has {} bytes, expected {}",
                bytes.len(),
                QUICKJS_MEMORY_USAGE_BYTE_LEN
            );
        }

        let mut fields = [0i64; QUICKJS_MEMORY_USAGE_FIELD_COUNT];
        for (field, chunk) in fields.iter_mut().zip(bytes.chunks_exact(8)) {
            let mut field_bytes = [0u8; 8];
            field_bytes.copy_from_slice(chunk);
            *field = i64::from_le_bytes(field_bytes);
        }
        Ok(Self::from_fields(fields))
    }

    fn from_fields(fields: [i64; QUICKJS_MEMORY_USAGE_FIELD_COUNT]) -> Self {
        Self {
            malloc_size: fields[0],
            malloc_limit: fields[1],
            memory_used_size: fields[2],
            malloc_count: fields[3],
            memory_used_count: fields[4],
            atom_count: fields[5],
            atom_size: fields[6],
            str_count: fields[7],
            str_size: fields[8],
            obj_count: fields[9],
            obj_size: fields[10],
            prop_count: fields[11],
            prop_size: fields[12],
            shape_count: fields[13],
            shape_size: fields[14],
            js_func_count: fields[15],
            js_func_size: fields[16],
            js_func_code_size: fields[17],
            js_func_pc2line_count: fields[18],
            js_func_pc2line_size: fields[19],
            c_func_count: fields[20],
            array_count: fields[21],
            fast_array_count: fields[22],
            fast_array_elements: fields[23],
            binary_object_count: fields[24],
            binary_object_size: fields[25],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_usage_fields_preserve_signed_i64_values() {
        let mut bytes = vec![0u8; QUICKJS_MEMORY_USAGE_BYTE_LEN as usize];
        bytes[0..8].copy_from_slice(&777i64.to_le_bytes());
        bytes[8..16].copy_from_slice(&(-1i64).to_le_bytes());
        bytes[16..24].copy_from_slice(&888i64.to_le_bytes());

        let usage = QuickJsMemoryUsage::from_le_bytes(&bytes).unwrap();

        assert_eq!(usage.malloc_size, 777);
        assert_eq!(usage.malloc_limit, -1);
        assert_eq!(usage.memory_used_size, 888);
    }
}
