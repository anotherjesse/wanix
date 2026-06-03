use super::{QUICKJS_WASM_ABI_VERSION, SNAPSHOT_FORMAT_VERSION, WASM_PAGE_SIZE};
use anyhow::{Result, anyhow, bail};

pub(super) fn validate_snapshot_structure(
    format_version: u32,
    abi_version: u32,
    memory_len: usize,
    stack_pointer: u32,
    runtime_ptr: u32,
    context_ptr: u32,
) -> Result<()> {
    if format_version != SNAPSHOT_FORMAT_VERSION {
        bail!(
            "unsupported snapshot format version {format_version} (expected {SNAPSHOT_FORMAT_VERSION})"
        );
    }
    if abi_version != QUICKJS_WASM_ABI_VERSION {
        bail!(
            "unsupported QuickJS WASM ABI version {abi_version} (expected {QUICKJS_WASM_ABI_VERSION})"
        );
    }
    if memory_len == 0 {
        bail!("snapshot memory is empty");
    }
    if !memory_len.is_multiple_of(WASM_PAGE_SIZE) {
        bail!("snapshot memory length is not WebAssembly page aligned");
    }
    validate_snapshot_pointer(runtime_ptr, memory_len, "runtime_ptr")?;
    validate_snapshot_pointer(context_ptr, memory_len, "context_ptr")?;
    validate_snapshot_stack_pointer(stack_pointer, memory_len)?;
    Ok(())
}

pub(crate) fn snapshot_memory_page_count(memory_len: usize) -> Result<u64> {
    let page_count = memory_len.div_ceil(WASM_PAGE_SIZE);
    u64::try_from(page_count).map_err(|_| anyhow!("snapshot memory page count does not fit in u64"))
}

fn validate_snapshot_pointer(ptr: u32, memory_len: usize, name: &str) -> Result<()> {
    let ptr = snapshot_pointer_offset(ptr, name)?;
    if ptr >= memory_len {
        bail!("snapshot {name} is outside restored memory");
    }
    Ok(())
}

fn validate_snapshot_stack_pointer(stack_pointer: u32, memory_len: usize) -> Result<()> {
    let stack_pointer = snapshot_pointer_offset(stack_pointer, "stack pointer")?;
    if stack_pointer > memory_len {
        bail!("snapshot stack pointer is outside restored memory");
    }
    Ok(())
}

fn snapshot_pointer_offset(ptr: u32, name: &str) -> Result<usize> {
    if ptr == 0 {
        bail!("snapshot {name} is null");
    }
    usize::try_from(ptr).map_err(|_| anyhow!("snapshot {name} does not fit host pointer size"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_page_count_uses_wasm_page_units() {
        assert_eq!(snapshot_memory_page_count(1).unwrap(), 1);
        assert_eq!(snapshot_memory_page_count(WASM_PAGE_SIZE).unwrap(), 1);
        assert_eq!(snapshot_memory_page_count(WASM_PAGE_SIZE + 1).unwrap(), 2);
    }
}
