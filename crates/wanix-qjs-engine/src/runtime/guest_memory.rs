use super::QuickJsRuntime;
use super::cleanup::finish_with_cleanup;
use crate::allocation::{try_copy_bytes, try_copy_str};
use crate::guest::{guest_offset, guest_u32};
use anyhow::{Result, anyhow, bail};
use wasmtime::error::Context as _;

#[derive(Debug)]
pub(super) struct GuestString {
    pub(super) ptr: i32,
    pub(super) len: u32,
}

#[derive(Debug)]
pub(super) struct GuestBytes {
    pub(super) ptr: i32,
    pub(super) len: u32,
}

impl QuickJsRuntime {
    pub(super) fn write_c_string(&mut self, value: &str) -> Result<GuestString> {
        let bytes = value.as_bytes();
        let len = u32::try_from(bytes.len()).context("guest string is too large")?;
        let alloc_len = len
            .checked_add(1)
            .ok_or_else(|| anyhow!("guest string allocation size overflowed"))?;
        let ptr = self.guest_malloc(alloc_len)?;
        let terminator = guest_offset(ptr)
            .checked_add(bytes.len())
            .ok_or_else(|| anyhow!("guest string pointer offset overflowed"))?;
        let write = self
            .memory
            .write(&mut self.store, guest_offset(ptr), bytes)
            .context("failed to write string into guest memory")
            .and_then(|_| {
                self.memory
                    .write(&mut self.store, terminator, &[0])
                    .context("failed to write string terminator into guest memory")
            });
        if let Err(err) = write {
            let cleanup = self.guest_free(ptr);
            return finish_with_cleanup(Err(err), cleanup.err());
        }
        Ok(GuestString { ptr, len })
    }

    pub(super) fn write_guest_bytes(&mut self, bytes: &[u8], label: &str) -> Result<GuestBytes> {
        let len = u32::try_from(bytes.len()).with_context(|| format!("{label} is too large"))?;
        let alloc_len = len.max(1);
        let ptr = self.guest_malloc(alloc_len)?;
        let write = if bytes.is_empty() {
            Ok(())
        } else {
            self.memory
                .write(&mut self.store, guest_offset(ptr), bytes)
                .with_context(|| format!("failed to write {label} into guest memory"))
        };
        if let Err(err) = write {
            let cleanup = self.guest_free(ptr);
            return finish_with_cleanup(Err(err), cleanup.err());
        }
        Ok(GuestBytes { ptr, len })
    }

    pub(super) fn read_c_string(&self, ptr: i32) -> Result<String> {
        if ptr == 0 {
            bail!("null guest C string pointer");
        }
        let memory = self.memory.data(&self.store);
        let start = guest_offset(ptr);
        if start >= memory.len() {
            bail!(
                "guest C string pointer {} is outside memory",
                guest_u32(ptr)
            );
        }
        let mut end = start;
        while end < memory.len() && memory[end] != 0 {
            end += 1;
        }
        if end == memory.len() {
            bail!("unterminated C string at guest pointer {}", guest_u32(ptr));
        }
        let string =
            std::str::from_utf8(&memory[start..end]).context("guest string was not valid UTF-8")?;
        try_copy_str(string, "guest C string")
    }

    pub(super) fn read_guest_bytes(&self, ptr: i32, len: u32, label: &str) -> Result<Vec<u8>> {
        let memory = self.memory.data(&self.store);
        let start = guest_offset(ptr);
        let len = usize::try_from(len).context("guest byte length does not fit host usize")?;
        let end = start
            .checked_add(len)
            .ok_or_else(|| anyhow!("{label} pointer offset overflowed"))?;
        if end > memory.len() {
            bail!(
                "{label} range [{start}, {end}) is outside memory length {}",
                memory.len()
            );
        }
        try_copy_bytes(&memory[start..end], label)
    }

    pub(super) fn read_guest_u32(&self, ptr: i32, label: &str) -> Result<u32> {
        let memory = self.memory.data(&self.store);
        let start = guest_offset(ptr);
        let end = start
            .checked_add(4)
            .ok_or_else(|| anyhow!("{label} pointer offset overflowed"))?;
        if end > memory.len() {
            bail!(
                "{label} range [{start}, {end}) is outside memory length {}",
                memory.len()
            );
        }
        let mut bytes = [0; 4];
        bytes.copy_from_slice(&memory[start..end]);
        Ok(u32::from_le_bytes(bytes))
    }

    pub(super) fn read_and_free_quickjs_c_string(
        &mut self,
        ptr: i32,
        cleanup_context: &'static str,
    ) -> Result<String> {
        let string = self.read_c_string(ptr);
        let cleanup = self
            .qjs_free_cstring
            .call(&mut self.store, ptr)
            .context(cleanup_context);
        finish_with_cleanup(string, cleanup.err().map(Into::into))
    }

    pub(super) fn guest_malloc(&mut self, size: u32) -> Result<i32> {
        let host_size = i32::try_from(size).map_err(|_| {
            anyhow!("guest allocation size {size} exceeds supported host call range")
        })?;
        let ptr = self
            .wasm_malloc
            .call(&mut self.store, host_size)
            .context("wasm_malloc failed")?;
        if ptr == 0 {
            bail!("wasm_malloc returned NULL for {size} bytes");
        }
        Ok(ptr)
    }

    pub(super) fn guest_free(&mut self, ptr: i32) -> Result<()> {
        Ok(self
            .wasm_free
            .call(&mut self.store, ptr)
            .context("wasm_free failed")?)
    }
}
