/// Preserves the raw bits of a Wasmtime `i32` ABI value as a guest `u32`.
pub(crate) fn guest_u32(value: i32) -> u32 {
    value.cast_unsigned()
}

/// Preserves the raw bits of a guest `u32` as a Wasmtime `i32` ABI value.
pub(crate) fn guest_i32(value: u32) -> i32 {
    value.cast_signed()
}

/// Adds a byte offset to a guest pointer and preserves the result as a wasm ABI value.
pub(crate) fn guest_i32_add(ptr: i32, offset: u32) -> Option<i32> {
    guest_u32(ptr).checked_add(offset).map(guest_i32)
}

/// Splits a signed 64-bit value into low/high 32-bit wasm ABI words.
pub(crate) fn i64_to_guest_i32_halves(value: i64) -> (i32, i32) {
    let bits = value as u64;
    (guest_i32(bits as u32), guest_i32((bits >> 32) as u32))
}

/// Reconstructs a signed 64-bit value from low/high guest words.
pub(crate) fn i64_from_guest_u32_halves(lo: u32, hi: u32) -> i64 {
    (((hi as u64) << 32) | lo as u64) as i64
}

/// Converts a guest pointer into a host memory offset without sign extension.
pub(crate) fn guest_offset(ptr: i32) -> usize {
    guest_u32(ptr) as usize
}

/// Adds an indexed byte stride to a guest pointer, preserving 32-bit pointer bounds.
pub(crate) fn guest_offset_at(base: i32, index: usize, stride: usize) -> Option<usize> {
    let relative = index.checked_mul(stride)?;
    let offset = guest_offset(base).checked_add(relative)?;
    if offset > u32::MAX as usize {
        return None;
    }
    Some(offset)
}

/// Converts a non-negative guest length into a host length.
pub(crate) fn guest_len(value: i32) -> Option<usize> {
    u32::try_from(value).ok().map(|value| value as usize)
}

/// Converts a host-created guest byte length into a signed wasm ABI value.
pub(crate) fn host_len_i32(value: u32) -> Option<i32> {
    i32::try_from(value).ok()
}

/// Converts a host-created guest item count into a signed wasm ABI value.
pub(crate) fn host_count_i32(value: usize) -> Option<i32> {
    i32::try_from(value).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guest_offsets_preserve_wasm_pointer_bits() {
        assert_eq!(guest_offset(0), 0);
        assert_eq!(guest_offset(i32::MAX), 0x7fff_ffffusize);
        assert_eq!(guest_offset(i32::MIN), 0x8000_0000usize);
        assert_eq!(guest_offset(-1), u32::MAX as usize);
    }

    #[test]
    fn guest_scalar_conversions_preserve_bits() {
        assert_eq!(guest_u32(0), 0);
        assert_eq!(guest_u32(i32::MIN), 0x8000_0000);
        assert_eq!(guest_u32(-1), u32::MAX);
        assert_eq!(guest_i32(0), 0);
        assert_eq!(guest_i32(0x8000_0000), i32::MIN);
        assert_eq!(guest_i32(u32::MAX), -1);
    }

    #[test]
    fn guest_i32_add_preserves_wasm_pointer_bits() {
        assert_eq!(guest_i32_add(0, 8), Some(8));
        assert_eq!(guest_i32_add(i32::MAX, 1), Some(i32::MIN));
        assert_eq!(guest_i32_add(i32::MIN, 8), Some(0x8000_0008u32 as i32));
        assert_eq!(guest_i32_add(-4, 3), Some(-1));
        assert_eq!(guest_i32_add(-4, 4), None);
    }

    #[test]
    fn i64_guest_halves_preserve_bits() {
        for value in [
            0,
            1,
            -1,
            i64::MIN,
            i64::MAX,
            0x1234_5678_9abc_def0u64 as i64,
        ] {
            let (lo, hi) = i64_to_guest_i32_halves(value);
            assert_eq!(
                i64_from_guest_u32_halves(guest_u32(lo), guest_u32(hi)),
                value
            );
        }
    }

    #[test]
    fn indexed_guest_offsets_check_overflow() {
        assert_eq!(guest_offset_at(-8, 1, 4).unwrap(), u32::MAX as usize - 3);
        assert!(guest_offset_at(-4, 1, 8).is_none());
        assert!(guest_offset_at(0, usize::MAX, 8).is_none());
    }

    #[test]
    fn guest_lengths_reject_negative_values() {
        assert_eq!(guest_len(-1), None);
        assert_eq!(guest_len(0), Some(0));
        assert_eq!(guest_len(i32::MAX), Some(i32::MAX as usize));
    }

    #[test]
    fn host_lengths_and_counts_reject_values_outside_guest_call_range() {
        assert_eq!(host_len_i32(0), Some(0));
        assert_eq!(host_len_i32(i32::MAX as u32), Some(i32::MAX));
        assert_eq!(host_len_i32(i32::MAX as u32 + 1), None);

        assert_eq!(host_count_i32(0), Some(0));
        assert_eq!(host_count_i32(i32::MAX as usize), Some(i32::MAX));
        assert_eq!(host_count_i32(i32::MAX as usize + 1), None);
    }
}
