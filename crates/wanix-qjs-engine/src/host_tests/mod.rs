use crate::host::{HostState, QuickJsHostConfig, QuickJsWasiHost, define_wasi_imports};
use anyhow::{Context, Result};
use std::sync::{Arc, Mutex};
use wasmtime::{Engine, Linker, Memory, Module, Store, TypedFunc};

mod clock_time_get;
mod env;
mod fd_fdstat_get;
mod fd_write;
mod fd_write_capture_limits;
mod fs;
mod module_loader;
mod random_get;
mod unsupported_wasi;

const HOST_IMPORT_WAT: &str = r#"
(module
  (import "wasi_snapshot_preview1" "random_get" (func $random_get (param i32 i32) (result i32)))
  (import "wasi_snapshot_preview1" "clock_time_get" (func $clock_time_get (param i32 i64 i32) (result i32)))
  (import "wasi_snapshot_preview1" "fd_fdstat_get" (func $fd_fdstat_get (param i32 i32) (result i32)))
  (import "wasi_snapshot_preview1" "fd_write" (func $fd_write (param i32 i32 i32 i32) (result i32)))
  (memory (export "memory") 1)
  (func (export "random_get") (param i32 i32) (result i32)
    local.get 0
    local.get 1
    call $random_get)
  (func (export "clock_time_get") (param i32 i64 i32) (result i32)
    local.get 0
    local.get 1
    local.get 2
    call $clock_time_get)
  (func (export "fd_fdstat_get") (param i32 i32) (result i32)
    local.get 0
    local.get 1
    call $fd_fdstat_get)
  (func (export "fd_write") (param i32 i32 i32 i32) (result i32)
    local.get 0
    local.get 1
    local.get 2
    local.get 3
    call $fd_write)
)
"#;

struct HostImportHarness {
    store: Store<HostState>,
    memory: Memory,
    random_get: TypedFunc<(i32, i32), i32>,
    clock_time_get: TypedFunc<(i32, i64, i32), i32>,
    fd_fdstat_get: TypedFunc<(i32, i32), i32>,
    fd_write: TypedFunc<(i32, i32, i32, i32), i32>,
}

impl HostImportHarness {
    fn call_random_get(&mut self, ptr: usize, len: usize) -> Result<i32> {
        let errno = self.random_get.call(
            &mut self.store,
            (
                test_guest_i32(ptr, "random_get pointer")?,
                test_guest_i32(len, "random_get length")?,
            ),
        )?;
        Ok(errno)
    }

    fn call_fd_write(
        &mut self,
        fd: i32,
        iovs_ptr: usize,
        iovs_len: usize,
        nwritten_ptr: usize,
    ) -> Result<i32> {
        let errno = self.fd_write.call(
            &mut self.store,
            (
                fd,
                test_guest_i32(iovs_ptr, "fd_write iovs pointer")?,
                test_guest_i32(iovs_len, "fd_write iovs length")?,
                test_guest_i32(nwritten_ptr, "fd_write nwritten pointer")?,
            ),
        )?;
        Ok(errno)
    }

    fn call_fd_write_raw(
        &mut self,
        fd: i32,
        iovs_ptr: i32,
        iovs_len: i32,
        nwritten_ptr: i32,
    ) -> Result<i32> {
        Ok(self
            .fd_write
            .call(&mut self.store, (fd, iovs_ptr, iovs_len, nwritten_ptr))?)
    }

    fn call_clock_time_get(
        &mut self,
        clock_id: i32,
        precision: i64,
        result_ptr: usize,
    ) -> Result<i32> {
        self.call_clock_time_get_raw(
            clock_id,
            precision,
            test_guest_i32(result_ptr, "clock_time_get result pointer")?,
        )
    }

    fn call_clock_time_get_raw(
        &mut self,
        clock_id: i32,
        precision: i64,
        result_ptr: i32,
    ) -> Result<i32> {
        Ok(self
            .clock_time_get
            .call(&mut self.store, (clock_id, precision, result_ptr))?)
    }

    fn call_fd_fdstat_get(&mut self, fd: i32, stat_ptr: usize) -> Result<i32> {
        self.call_fd_fdstat_get_raw(fd, test_guest_i32(stat_ptr, "fd_fdstat_get stat pointer")?)
    }

    fn call_fd_fdstat_get_raw(&mut self, fd: i32, stat_ptr: i32) -> Result<i32> {
        Ok(self.fd_fdstat_get.call(&mut self.store, (fd, stat_ptr))?)
    }
}

fn host_import_harness(config: QuickJsHostConfig) -> Result<HostImportHarness> {
    host_import_harness_with_wasi_host(config, None)
}

fn host_import_harness_with_wasi_host(
    config: QuickJsHostConfig,
    wasi_host: Option<Box<dyn QuickJsWasiHost>>,
) -> Result<HostImportHarness> {
    let engine = Engine::default();
    let module = Module::new(&engine, HOST_IMPORT_WAT)?;
    let mut linker = Linker::<HostState>::new(&engine);
    define_wasi_imports(&mut linker)?;
    let wasi_host = wasi_host.map(|host| Arc::new(Mutex::new(host)));
    let mut store = Store::new(&engine, HostState::new_with_wasi_host(config, wasi_host));
    let instance = linker.instantiate(&mut store, &module)?;
    let memory = instance
        .get_memory(&mut store, "memory")
        .context("test module should export memory")?;
    store.data_mut().set_memory(memory);
    let random_get = instance.get_typed_func(&mut store, "random_get")?;
    let clock_time_get = instance.get_typed_func(&mut store, "clock_time_get")?;
    let fd_fdstat_get = instance.get_typed_func(&mut store, "fd_fdstat_get")?;
    let fd_write = instance.get_typed_func(&mut store, "fd_write")?;

    Ok(HostImportHarness {
        store,
        memory,
        random_get,
        clock_time_get,
        fd_fdstat_get,
        fd_write,
    })
}

fn write_u32(harness: &mut HostImportHarness, offset: usize, value: u32) -> Result<()> {
    Ok(harness
        .memory
        .write(&mut harness.store, offset, &value.to_le_bytes())?)
}

fn write_iov(
    harness: &mut HostImportHarness,
    offset: usize,
    buf_ptr: usize,
    buf_len: usize,
) -> Result<()> {
    write_u32(
        harness,
        offset,
        u32::try_from(buf_ptr).context("test buffer pointer should fit in u32")?,
    )?;
    write_u32(
        harness,
        offset + 4,
        u32::try_from(buf_len).context("test buffer length should fit in u32")?,
    )
}

fn read_u32(harness: &HostImportHarness, offset: usize) -> Result<u32> {
    let mut bytes = [0; 4];
    harness.memory.read(&harness.store, offset, &mut bytes)?;
    Ok(u32::from_le_bytes(bytes))
}

fn read_u64(harness: &HostImportHarness, offset: usize) -> Result<u64> {
    let mut bytes = [0; 8];
    harness.memory.read(&harness.store, offset, &mut bytes)?;
    Ok(u64::from_le_bytes(bytes))
}

fn test_guest_i32(value: usize, field: &str) -> Result<i32> {
    i32::try_from(value).with_context(|| format!("test {field} should fit in i32"))
}
