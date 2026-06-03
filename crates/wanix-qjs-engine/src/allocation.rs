use anyhow::{Result, anyhow};

pub(crate) fn try_copy_bytes(bytes: &[u8], label: &str) -> Result<Vec<u8>> {
    let mut copy = try_reserve_bytes(bytes.len(), label)?;
    copy.extend_from_slice(bytes);
    Ok(copy)
}

pub(crate) fn try_reserve_bytes(capacity: usize, label: &str) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(capacity)
        .map_err(|err| anyhow!("{label} allocation failed: {err}"))?;
    Ok(bytes)
}

pub(crate) fn try_copy_str(value: &str, label: &str) -> Result<String> {
    let mut string = String::new();
    string
        .try_reserve_exact(value.len())
        .map_err(|err| anyhow!("{label} allocation failed: {err}"))?;
    string.push_str(value);
    Ok(string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn try_reserve_bytes_reports_allocation_label() {
        let err = try_reserve_bytes(usize::MAX, "test bytes")
            .expect_err("oversized byte reservation should fail");

        assert!(err.to_string().contains("test bytes allocation failed"));
    }

    #[test]
    fn try_copy_bytes_copies_input() {
        let copied = try_copy_bytes(b"abc", "test bytes").expect("small byte slice should copy");

        assert_eq!(copied, b"abc");
    }

    #[test]
    fn try_copy_str_copies_input() {
        let copied = try_copy_str("test", "test string").expect("small string should copy");

        assert_eq!(copied, "test");
    }
}
