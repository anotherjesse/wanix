use super::*;
use crate::host::{
    QuickJsWasiDirEntry, QuickJsWasiErrno, QuickJsWasiFdStat, QuickJsWasiFileStat,
    QuickJsWasiFileType, QuickJsWasiPrestat, QuickJsWasiWhence,
};
use std::sync::{Arc, Mutex};

const ERRNO_SUCCESS: i32 = 0;
const ERRNO_INVAL: i32 = 28;

const PROCESS_WASI_WAT: &str = r#"
(module
  (import "wasi_snapshot_preview1" "args_sizes_get" (func $args_sizes_get (param i32 i32) (result i32)))
  (import "wasi_snapshot_preview1" "args_get" (func $args_get (param i32 i32) (result i32)))
  (import "wasi_snapshot_preview1" "environ_sizes_get" (func $environ_sizes_get (param i32 i32) (result i32)))
  (import "wasi_snapshot_preview1" "environ_get" (func $environ_get (param i32 i32) (result i32)))
  (memory (export "memory") 1)
  (func (export "args_sizes_get") (param i32 i32) (result i32)
    local.get 0 local.get 1 call $args_sizes_get)
  (func (export "args_get") (param i32 i32) (result i32)
    local.get 0 local.get 1 call $args_get)
  (func (export "environ_sizes_get") (param i32 i32) (result i32)
    local.get 0 local.get 1 call $environ_sizes_get)
  (func (export "environ_get") (param i32 i32) (result i32)
    local.get 0 local.get 1 call $environ_get)
)
"#;

struct ProcessWasiHarness {
    store: Store<HostState>,
    memory: Memory,
    args_sizes_get: TypedFunc<(i32, i32), i32>,
    args_get: TypedFunc<(i32, i32), i32>,
    environ_sizes_get: TypedFunc<(i32, i32), i32>,
    environ_get: TypedFunc<(i32, i32), i32>,
}

impl ProcessWasiHarness {
    fn new(wasi_host: Option<Box<dyn QuickJsWasiHost>>) -> Result<Self> {
        let engine = Engine::default();
        let module = Module::new(&engine, PROCESS_WASI_WAT)?;
        let mut linker = Linker::<HostState>::new(&engine);
        define_wasi_imports(&mut linker)?;
        let wasi_host = wasi_host.map(|host| Arc::new(Mutex::new(host)));
        let mut store = Store::new(
            &engine,
            HostState::new_with_wasi_host(QuickJsHostConfig::new(), wasi_host),
        );
        let instance = linker.instantiate(&mut store, &module)?;
        let memory = instance
            .get_memory(&mut store, "memory")
            .context("test module should export memory")?;
        store.data_mut().set_memory(memory);

        Ok(Self {
            args_sizes_get: instance.get_typed_func(&mut store, "args_sizes_get")?,
            args_get: instance.get_typed_func(&mut store, "args_get")?,
            environ_sizes_get: instance.get_typed_func(&mut store, "environ_sizes_get")?,
            environ_get: instance.get_typed_func(&mut store, "environ_get")?,
            memory,
            store,
        })
    }

    fn read_u32(&self, offset: usize) -> Result<u32> {
        let mut bytes = [0; 4];
        self.memory.read(&self.store, offset, &mut bytes)?;
        Ok(u32::from_le_bytes(bytes))
    }

    fn read_bytes(&self, offset: usize, len: usize) -> Result<Vec<u8>> {
        let mut bytes = vec![0; len];
        self.memory.read(&self.store, offset, &mut bytes)?;
        Ok(bytes)
    }
}

type WasiHostResult<T> = std::result::Result<T, QuickJsWasiErrno>;

#[derive(Default)]
struct ProcessWasiHost {
    args: Vec<String>,
    env: Vec<String>,
}

impl QuickJsWasiHost for ProcessWasiHost {
    fn args(&mut self) -> WasiHostResult<Vec<String>> {
        Ok(self.args.clone())
    }

    fn env(&mut self) -> WasiHostResult<Vec<String>> {
        Ok(self.env.clone())
    }

    fn fd_prestat_get(&mut self, _fd: u32) -> WasiHostResult<QuickJsWasiPrestat> {
        Err(QuickJsWasiErrno::Nosys)
    }

    fn path_open(
        &mut self,
        _dirfd: u32,
        _dirflags: u32,
        _path: &[u8],
        _oflags: u16,
        _rights_base: u64,
        _rights_inheriting: u64,
        _fdflags: u16,
    ) -> WasiHostResult<u32> {
        Err(QuickJsWasiErrno::Nosys)
    }

    fn fd_read(&mut self, _fd: u32, _buf: &mut [u8]) -> WasiHostResult<usize> {
        Err(QuickJsWasiErrno::Nosys)
    }

    fn fd_readdir(&mut self, _fd: u32) -> WasiHostResult<Vec<QuickJsWasiDirEntry>> {
        Err(QuickJsWasiErrno::Nosys)
    }

    fn fd_write(&mut self, _fd: u32, _buf: &[u8]) -> WasiHostResult<usize> {
        Err(QuickJsWasiErrno::Nosys)
    }

    fn fd_seek(
        &mut self,
        _fd: u32,
        _offset: i64,
        _whence: QuickJsWasiWhence,
    ) -> WasiHostResult<u64> {
        Err(QuickJsWasiErrno::Nosys)
    }

    fn fd_close(&mut self, _fd: u32) -> WasiHostResult<()> {
        Err(QuickJsWasiErrno::Nosys)
    }

    fn fd_fdstat_get(&mut self, _fd: u32) -> WasiHostResult<QuickJsWasiFdStat> {
        Ok(QuickJsWasiFdStat::new(
            QuickJsWasiFileType::CharacterDevice,
            0,
            0,
        ))
    }

    fn fd_filestat_get(&mut self, _fd: u32) -> WasiHostResult<QuickJsWasiFileStat> {
        Err(QuickJsWasiErrno::Nosys)
    }

    fn path_filestat_get(
        &mut self,
        _dirfd: u32,
        _flags: u32,
        _path: &[u8],
    ) -> WasiHostResult<QuickJsWasiFileStat> {
        Err(QuickJsWasiErrno::Nosys)
    }
}

#[test]
fn process_imports_report_empty_args_and_env_without_live_host() -> Result<()> {
    let mut harness = ProcessWasiHarness::new(None)?;

    assert_eq!(
        harness.args_sizes_get.call(&mut harness.store, (16, 20))?,
        ERRNO_SUCCESS
    );
    assert_eq!(harness.read_u32(16)?, 0);
    assert_eq!(harness.read_u32(20)?, 0);

    assert_eq!(
        harness
            .environ_sizes_get
            .call(&mut harness.store, (24, 28))?,
        ERRNO_SUCCESS
    );
    assert_eq!(harness.read_u32(24)?, 0);
    assert_eq!(harness.read_u32(28)?, 0);
    Ok(())
}

#[test]
fn live_wasi_host_supplies_args_and_env_preview1_buffers() -> Result<()> {
    let host = ProcessWasiHost {
        args: vec!["main.js".to_owned(), "--flag".to_owned()],
        env: vec!["MODE=test".to_owned(), "EMPTY=".to_owned()],
    };
    let mut harness = ProcessWasiHarness::new(Some(Box::new(host)))?;

    assert_eq!(
        harness.args_sizes_get.call(&mut harness.store, (16, 20))?,
        ERRNO_SUCCESS
    );
    assert_eq!(harness.read_u32(16)?, 2);
    assert_eq!(harness.read_u32(20)?, 15);

    assert_eq!(
        harness.args_get.call(&mut harness.store, (100, 200))?,
        ERRNO_SUCCESS
    );
    assert_eq!(harness.read_u32(100)?, 200);
    assert_eq!(harness.read_u32(104)?, 208);
    assert_eq!(harness.read_bytes(200, 15)?, b"main.js\0--flag\0");

    assert_eq!(
        harness
            .environ_sizes_get
            .call(&mut harness.store, (24, 28))?,
        ERRNO_SUCCESS
    );
    assert_eq!(harness.read_u32(24)?, 2);
    assert_eq!(harness.read_u32(28)?, 17);

    assert_eq!(
        harness.environ_get.call(&mut harness.store, (120, 240))?,
        ERRNO_SUCCESS
    );
    assert_eq!(harness.read_u32(120)?, 240);
    assert_eq!(harness.read_u32(124)?, 250);
    assert_eq!(harness.read_bytes(240, 17)?, b"MODE=test\0EMPTY=\0");
    Ok(())
}

#[test]
fn process_imports_reject_strings_with_nul_bytes() -> Result<()> {
    let host = ProcessWasiHost {
        args: vec!["main\0.js".to_owned()],
        env: Vec::new(),
    };
    let mut harness = ProcessWasiHarness::new(Some(Box::new(host)))?;

    assert_eq!(
        harness.args_sizes_get.call(&mut harness.store, (16, 20))?,
        ERRNO_INVAL
    );
    assert_eq!(
        harness.args_get.call(&mut harness.store, (100, 200))?,
        ERRNO_INVAL
    );
    Ok(())
}
