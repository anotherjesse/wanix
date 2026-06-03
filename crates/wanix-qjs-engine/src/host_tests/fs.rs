use super::*;
use crate::host::{
    QuickJsWasiErrno, QuickJsWasiFdStat, QuickJsWasiFileStat, QuickJsWasiFileType, QuickJsWasiHost,
    QuickJsWasiPrestat, QuickJsWasiWhence,
};
use std::sync::{Arc, Mutex};
use wasmtime::{Engine, Linker, Module, Store, TypedFunc};

const ERRNO_SUCCESS: i32 = 0;
const ERRNO_BADF: i32 = 8;
const ERRNO_INVAL: i32 = 28;
const ERRNO_NAMETOOLONG: i32 = 37;
const ERRNO_NOENT: i32 = 44;
const ERRNO_NOSYS: i32 = 52;
const ERRNO_NOTCAPABLE: i32 = 76;

const RIGHT_FD_READ: i64 = 1 << 1;
const RIGHT_FD_SEEK: i64 = 1 << 2;
const RIGHT_FD_TELL: i64 = 1 << 5;
const RIGHT_PATH_OPEN: i64 = 1 << 13;
const RIGHT_PATH_FILESTAT_GET: i64 = 1 << 18;
const RIGHT_FD_FILESTAT_GET: i64 = 1 << 21;
const READ_SEEK_STAT_RIGHTS: i64 =
    RIGHT_FD_READ | RIGHT_FD_SEEK | RIGHT_FD_TELL | RIGHT_FD_FILESTAT_GET;

const PREOPEN_ROOT_FD: i32 = 3;
const FIRST_FILE_FD: i32 = 4;
const WASI_IOV_SIZE: usize = 8;
const WHENCE_SET: i32 = 0;
const FILETYPE_CHARACTER_DEVICE: u8 = 2;
const FILETYPE_DIRECTORY: u8 = 3;
const FILETYPE_REGULAR_FILE: u8 = 4;

const PATH_PTR: usize = 128;
const MAX_VIRTUAL_FILE_PATH_BYTES: usize = 4096;

type PathOpenFunc = TypedFunc<(i32, i32, i32, i32, i32, i64, i64, i32, i32), i32>;

const VIRTUAL_FS_WAT: &str = r#"
(module
  (import "wasi_snapshot_preview1" "fd_prestat_get" (func $fd_prestat_get (param i32 i32) (result i32)))
  (import "wasi_snapshot_preview1" "fd_prestat_dir_name" (func $fd_prestat_dir_name (param i32 i32 i32) (result i32)))
  (import "wasi_snapshot_preview1" "path_open" (func $path_open (param i32 i32 i32 i32 i32 i64 i64 i32 i32) (result i32)))
  (import "wasi_snapshot_preview1" "fd_read" (func $fd_read (param i32 i32 i32 i32) (result i32)))
  (import "wasi_snapshot_preview1" "fd_seek" (func $fd_seek (param i32 i64 i32 i32) (result i32)))
  (import "wasi_snapshot_preview1" "fd_close" (func $fd_close (param i32) (result i32)))
  (import "wasi_snapshot_preview1" "fd_fdstat_get" (func $fd_fdstat_get (param i32 i32) (result i32)))
  (import "wasi_snapshot_preview1" "fd_filestat_get" (func $fd_filestat_get (param i32 i32) (result i32)))
  (import "wasi_snapshot_preview1" "path_filestat_get" (func $path_filestat_get (param i32 i32 i32 i32 i32) (result i32)))
  (memory (export "memory") 1)
  (func (export "fd_prestat_get") (param i32 i32) (result i32)
    local.get 0 local.get 1 call $fd_prestat_get)
  (func (export "fd_prestat_dir_name") (param i32 i32 i32) (result i32)
    local.get 0 local.get 1 local.get 2 call $fd_prestat_dir_name)
  (func (export "path_open") (param i32 i32 i32 i32 i32 i64 i64 i32 i32) (result i32)
    local.get 0 local.get 1 local.get 2 local.get 3 local.get 4
    local.get 5 local.get 6 local.get 7 local.get 8
    call $path_open)
  (func (export "fd_read") (param i32 i32 i32 i32) (result i32)
    local.get 0 local.get 1 local.get 2 local.get 3 call $fd_read)
  (func (export "fd_seek") (param i32 i64 i32 i32) (result i32)
    local.get 0 local.get 1 local.get 2 local.get 3 call $fd_seek)
  (func (export "fd_close") (param i32) (result i32)
    local.get 0 call $fd_close)
  (func (export "fd_fdstat_get") (param i32 i32) (result i32)
    local.get 0 local.get 1 call $fd_fdstat_get)
  (func (export "fd_filestat_get") (param i32 i32) (result i32)
    local.get 0 local.get 1 call $fd_filestat_get)
  (func (export "path_filestat_get") (param i32 i32 i32 i32 i32) (result i32)
    local.get 0 local.get 1 local.get 2 local.get 3 local.get 4
    call $path_filestat_get)
)
"#;

struct VirtualFsHarness {
    store: Store<HostState>,
    memory: wasmtime::Memory,
    fd_prestat_get: TypedFunc<(i32, i32), i32>,
    fd_prestat_dir_name: TypedFunc<(i32, i32, i32), i32>,
    path_open: PathOpenFunc,
    fd_read: TypedFunc<(i32, i32, i32, i32), i32>,
    fd_seek: TypedFunc<(i32, i64, i32, i32), i32>,
    fd_close: TypedFunc<i32, i32>,
    fd_fdstat_get: TypedFunc<(i32, i32), i32>,
    fd_filestat_get: TypedFunc<(i32, i32), i32>,
    path_filestat_get: TypedFunc<(i32, i32, i32, i32, i32), i32>,
}

impl VirtualFsHarness {
    fn new(config: QuickJsHostConfig) -> Result<Self> {
        Self::new_with_wasi_host(config, None)
    }

    fn new_with_wasi_host(
        config: QuickJsHostConfig,
        wasi_host: Option<Box<dyn QuickJsWasiHost>>,
    ) -> Result<Self> {
        let engine = Engine::default();
        let module = Module::new(&engine, VIRTUAL_FS_WAT)?;
        let mut linker = Linker::<HostState>::new(&engine);
        define_wasi_imports(&mut linker)?;
        let wasi_host = wasi_host.map(|host| Arc::new(Mutex::new(host)));
        let mut store = Store::new(&engine, HostState::new_with_wasi_host(config, wasi_host));
        let instance = linker.instantiate(&mut store, &module)?;
        let memory = instance
            .get_memory(&mut store, "memory")
            .context("test module should export memory")?;
        store.data_mut().set_memory(memory);

        Ok(Self {
            fd_prestat_get: instance.get_typed_func(&mut store, "fd_prestat_get")?,
            fd_prestat_dir_name: instance.get_typed_func(&mut store, "fd_prestat_dir_name")?,
            path_open: instance.get_typed_func(&mut store, "path_open")?,
            fd_read: instance.get_typed_func(&mut store, "fd_read")?,
            fd_seek: instance.get_typed_func(&mut store, "fd_seek")?,
            fd_close: instance.get_typed_func(&mut store, "fd_close")?,
            fd_fdstat_get: instance.get_typed_func(&mut store, "fd_fdstat_get")?,
            fd_filestat_get: instance.get_typed_func(&mut store, "fd_filestat_get")?,
            path_filestat_get: instance.get_typed_func(&mut store, "path_filestat_get")?,
            memory,
            store,
        })
    }

    fn write_bytes(&mut self, ptr: usize, bytes: &[u8]) -> Result<()> {
        Ok(self.memory.write(&mut self.store, ptr, bytes)?)
    }

    fn read_bytes(&self, ptr: usize, len: usize) -> Result<Vec<u8>> {
        let mut bytes = vec![0; len];
        self.memory.read(&self.store, ptr, &mut bytes)?;
        Ok(bytes)
    }

    fn write_iov(&mut self, offset: usize, buf_ptr: usize, buf_len: usize) -> Result<()> {
        self.write_u32(
            offset,
            u32::try_from(buf_ptr).context("iov ptr should fit u32")?,
        )?;
        self.write_u32(
            offset + 4,
            u32::try_from(buf_len).context("iov len should fit u32")?,
        )
    }

    fn write_u32(&mut self, offset: usize, value: u32) -> Result<()> {
        Ok(self
            .memory
            .write(&mut self.store, offset, &value.to_le_bytes())?)
    }

    fn read_u8(&self, offset: usize) -> Result<u8> {
        let mut byte = [0; 1];
        self.memory.read(&self.store, offset, &mut byte)?;
        Ok(byte[0])
    }

    fn read_u32(&self, offset: usize) -> Result<u32> {
        let mut bytes = [0; 4];
        self.memory.read(&self.store, offset, &mut bytes)?;
        Ok(u32::from_le_bytes(bytes))
    }

    fn read_u64(&self, offset: usize) -> Result<u64> {
        let mut bytes = [0; 8];
        self.memory.read(&self.store, offset, &mut bytes)?;
        Ok(u64::from_le_bytes(bytes))
    }

    fn open_path(&mut self, path: &str, opened_fd_ptr: usize) -> Result<i32> {
        self.open_path_with(path, READ_SEEK_STAT_RIGHTS, 0, 0, 0, opened_fd_ptr)
    }

    fn open_path_with(
        &mut self,
        path: &str,
        rights_base: i64,
        dirflags: i32,
        oflags: i32,
        fdflags: i32,
        opened_fd_ptr: usize,
    ) -> Result<i32> {
        self.write_bytes(PATH_PTR, path.as_bytes())?;
        Ok(self.path_open.call(
            &mut self.store,
            (
                PREOPEN_ROOT_FD,
                dirflags,
                test_guest_i32(PATH_PTR, "path pointer")?,
                test_guest_i32(path.len(), "path length")?,
                oflags,
                rights_base,
                0,
                fdflags,
                test_guest_i32(opened_fd_ptr, "opened fd pointer")?,
            ),
        )?)
    }
}

fn virtual_fs_config() -> Result<QuickJsHostConfig> {
    QuickJsHostConfig::new()
        .with_read_only_virtual_file("/app/config.txt", b"hello virtual fs")?
        .with_read_only_virtual_file("/app/empty.txt", b"")
}

type WasiHostResult<T> = std::result::Result<T, QuickJsWasiErrno>;

#[derive(Clone, Default)]
struct MetadataWasiHost {
    calls: Arc<Mutex<Vec<String>>>,
}

impl MetadataWasiHost {
    fn record(&self, call: impl Into<String>) {
        self.calls.lock().expect("test call lock").push(call.into());
    }
}

impl QuickJsWasiHost for MetadataWasiHost {
    fn fd_prestat_get(&mut self, fd: u32) -> WasiHostResult<QuickJsWasiPrestat> {
        self.record(format!("prestat:{fd}"));
        Ok(QuickJsWasiPrestat::new("/mnt"))
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

    fn fd_write(&mut self, _fd: u32, _buf: &[u8]) -> WasiHostResult<usize> {
        Err(QuickJsWasiErrno::Nosys)
    }

    fn fd_seek(&mut self, fd: u32, offset: i64, whence: QuickJsWasiWhence) -> WasiHostResult<u64> {
        self.record(format!("seek:{fd}:{offset}:{whence:?}"));
        Ok(123)
    }

    fn fd_close(&mut self, fd: u32) -> WasiHostResult<()> {
        self.record(format!("close:{fd}"));
        Ok(())
    }

    fn fd_fdstat_get(&mut self, fd: u32) -> WasiHostResult<QuickJsWasiFdStat> {
        self.record(format!("fdstat:{fd}"));
        Ok(QuickJsWasiFdStat::new(
            QuickJsWasiFileType::Directory,
            RIGHT_PATH_OPEN.cast_unsigned(),
            READ_SEEK_STAT_RIGHTS.cast_unsigned(),
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
fn live_wasi_host_supplies_prestat_fdstat_seek_and_close() -> Result<()> {
    let host = MetadataWasiHost::default();
    let calls = Arc::clone(&host.calls);
    let mut harness =
        VirtualFsHarness::new_with_wasi_host(QuickJsHostConfig::new(), Some(Box::new(host)))?;

    assert_eq!(
        harness.fd_prestat_get.call(&mut harness.store, (9, 64))?,
        ERRNO_SUCCESS
    );
    assert_eq!(harness.read_u32(68)?, 4);

    assert_eq!(
        harness
            .fd_prestat_dir_name
            .call(&mut harness.store, (9, 80, 4))?,
        ERRNO_SUCCESS
    );
    assert_eq!(harness.read_bytes(80, 4)?, b"/mnt");

    assert_eq!(
        harness.fd_fdstat_get.call(&mut harness.store, (9, 96))?,
        ERRNO_SUCCESS
    );
    assert_eq!(harness.read_u8(96)?, FILETYPE_DIRECTORY);
    assert_eq!(harness.read_u64(104)?, RIGHT_PATH_OPEN.cast_unsigned());
    assert_eq!(
        harness.read_u64(112)?,
        READ_SEEK_STAT_RIGHTS.cast_unsigned()
    );

    assert_eq!(
        harness
            .fd_seek
            .call(&mut harness.store, (5, 7, WHENCE_SET, 128))?,
        ERRNO_SUCCESS
    );
    assert_eq!(harness.read_u64(128)?, 123);

    assert_eq!(harness.fd_close.call(&mut harness.store, 5)?, ERRNO_SUCCESS);
    assert_eq!(
        calls.lock().expect("test call lock").as_slice(),
        &[
            "prestat:9".to_owned(),
            "prestat:9".to_owned(),
            "fdstat:9".to_owned(),
            "seek:5:7:Set".to_owned(),
            "close:5".to_owned(),
        ]
    );
    Ok(())
}

#[test]
fn virtual_directory_detection_uses_path_component_prefixes() -> Result<()> {
    let config = QuickJsHostConfig::new()
        .with_read_only_virtual_file("/app/config.txt", b"data")?
        .with_read_only_virtual_file("/apple/config.txt", b"other")?;

    assert!(config.has_read_only_virtual_directory(b"/"));
    assert!(config.has_read_only_virtual_directory(b"/app"));
    assert!(config.has_read_only_virtual_directory(b"/apple"));
    assert!(!config.has_read_only_virtual_directory(b"/ap"));
    assert!(!config.has_read_only_virtual_directory(b"/app/config.txt"));
    Ok(())
}

#[test]
fn config_rejects_surprising_virtual_file_paths() -> Result<()> {
    for path in [
        "relative.txt",
        "/",
        "/app//config.txt",
        "/app/./config.txt",
        "/app/../secret.txt",
        "/app\\config.txt",
        "/app/bad\0name",
    ] {
        let err = QuickJsHostConfig::new()
            .with_read_only_virtual_file(path, b"data")
            .expect_err("invalid virtual file path should be rejected");
        assert!(err.to_string().contains("read-only virtual file path"));
    }

    let config =
        QuickJsHostConfig::new().with_read_only_virtual_file("/app/config.txt", b"data")?;
    assert_eq!(config.read_only_virtual_file_count(), 1);
    Ok(())
}

#[test]
fn config_and_path_open_reject_too_long_virtual_file_paths() -> Result<()> {
    let absolute_too_long = format!("/{}", "a".repeat(MAX_VIRTUAL_FILE_PATH_BYTES));
    let err = QuickJsHostConfig::new()
        .with_read_only_virtual_file(&absolute_too_long, b"data")
        .expect_err("overlong configured path should be rejected");
    assert!(err.to_string().contains("at most"));

    let mut harness = VirtualFsHarness::new(virtual_fs_config()?)?;
    let relative_too_long = "a".repeat(MAX_VIRTUAL_FILE_PATH_BYTES);

    assert_eq!(
        harness.open_path(&relative_too_long, 60)?,
        ERRNO_NAMETOOLONG
    );
    Ok(())
}

#[test]
fn preopen_discovery_reports_root_when_virtual_files_are_configured() -> Result<()> {
    let mut harness = VirtualFsHarness::new(virtual_fs_config()?)?;

    assert_eq!(
        harness
            .fd_prestat_get
            .call(&mut harness.store, (PREOPEN_ROOT_FD, 64))?,
        ERRNO_SUCCESS
    );
    assert_eq!(harness.read_u32(64)?, 0);
    assert_eq!(harness.read_u32(68)?, 1);

    assert_eq!(
        harness
            .fd_prestat_dir_name
            .call(&mut harness.store, (PREOPEN_ROOT_FD, 80, 1))?,
        ERRNO_SUCCESS
    );
    assert_eq!(harness.read_bytes(80, 1)?, b"/");

    assert_eq!(
        harness.fd_prestat_get.call(&mut harness.store, (99, 64))?,
        ERRNO_BADF
    );

    let mut empty = VirtualFsHarness::new(QuickJsHostConfig::new())?;
    assert_eq!(
        empty
            .fd_prestat_get
            .call(&mut empty.store, (PREOPEN_ROOT_FD, 64))?,
        ERRNO_BADF
    );
    Ok(())
}

#[test]
fn virtual_file_open_read_seek_and_close_round_trip() -> Result<()> {
    let mut harness = VirtualFsHarness::new(virtual_fs_config()?)?;

    assert_eq!(harness.open_path("app/config.txt", 96)?, ERRNO_SUCCESS);
    let fd = i32::try_from(harness.read_u32(96)?).context("fd should fit i32")?;
    assert_eq!(fd, FIRST_FILE_FD);

    harness.write_iov(300, 500, 5)?;
    harness.write_iov(300 + WASI_IOV_SIZE, 600, 8)?;
    assert_eq!(
        harness.fd_read.call(&mut harness.store, (fd, 300, 2, 72))?,
        ERRNO_SUCCESS
    );
    assert_eq!(harness.read_u32(72)?, 13);
    assert_eq!(harness.read_bytes(500, 5)?, b"hello");
    assert_eq!(harness.read_bytes(600, 8)?, b" virtual");

    assert_eq!(
        harness.fd_seek.call(&mut harness.store, (fd, 6, 0, 88))?,
        ERRNO_SUCCESS
    );
    assert_eq!(harness.read_u64(88)?, 6);

    harness.write_iov(300, 700, 2)?;
    assert_eq!(
        harness.fd_read.call(&mut harness.store, (fd, 300, 1, 72))?,
        ERRNO_SUCCESS
    );
    assert_eq!(harness.read_u32(72)?, 2);
    assert_eq!(harness.read_bytes(700, 2)?, b"vi");

    assert_eq!(
        harness.fd_close.call(&mut harness.store, fd)?,
        ERRNO_SUCCESS
    );
    assert_eq!(
        harness.fd_read.call(&mut harness.store, (fd, 300, 1, 72))?,
        ERRNO_BADF
    );
    assert_eq!(harness.fd_close.call(&mut harness.store, fd)?, ERRNO_BADF);
    Ok(())
}

#[test]
fn filestat_reports_virtual_file_sizes_and_types() -> Result<()> {
    let mut harness = VirtualFsHarness::new(virtual_fs_config()?)?;

    assert_eq!(
        harness.fd_filestat_get.call(&mut harness.store, (1, 64))?,
        ERRNO_SUCCESS
    );
    assert_eq!(harness.read_u8(64 + 16)?, FILETYPE_CHARACTER_DEVICE);
    assert_eq!(harness.read_u64(64 + 32)?, 0);

    assert_eq!(
        harness
            .fd_filestat_get
            .call(&mut harness.store, (PREOPEN_ROOT_FD, 64))?,
        ERRNO_SUCCESS
    );
    assert_eq!(harness.read_u8(64 + 16)?, FILETYPE_DIRECTORY);
    assert_eq!(harness.read_u64(64 + 32)?, 0);

    assert_eq!(harness.open_path("app/config.txt", 96)?, ERRNO_SUCCESS);
    let fd = i32::try_from(harness.read_u32(96)?).context("fd should fit i32")?;
    assert_eq!(
        harness.fd_filestat_get.call(&mut harness.store, (fd, 64))?,
        ERRNO_SUCCESS
    );
    assert_eq!(harness.read_u8(64 + 16)?, FILETYPE_REGULAR_FILE);
    assert_eq!(harness.read_u64(64 + 32)?, 16);

    assert_eq!(
        harness.open_path_with("app/empty.txt", RIGHT_FD_FILESTAT_GET, 0, 0, 0, 100)?,
        ERRNO_SUCCESS
    );
    let empty_fd = i32::try_from(harness.read_u32(100)?).context("fd should fit i32")?;
    assert_eq!(
        harness
            .fd_filestat_get
            .call(&mut harness.store, (empty_fd, 64))?,
        ERRNO_SUCCESS
    );
    assert_eq!(harness.read_u8(64 + 16)?, FILETYPE_REGULAR_FILE);
    assert_eq!(harness.read_u64(64 + 32)?, 0);
    Ok(())
}

#[test]
fn path_filestat_reports_virtual_file_sizes_without_opening() -> Result<()> {
    let mut harness = VirtualFsHarness::new(virtual_fs_config()?)?;
    harness.write_bytes(PATH_PTR, b"app/config.txt")?;

    assert_eq!(
        harness.path_filestat_get.call(
            &mut harness.store,
            (
                PREOPEN_ROOT_FD,
                0,
                test_guest_i32(PATH_PTR, "path pointer")?,
                14,
                64,
            ),
        )?,
        ERRNO_SUCCESS
    );
    assert_eq!(harness.read_u8(64 + 16)?, FILETYPE_REGULAR_FILE);
    assert_eq!(harness.read_u64(64 + 32)?, 16);

    assert_eq!(
        harness.path_filestat_get.call(
            &mut harness.store,
            (
                PREOPEN_ROOT_FD,
                1,
                test_guest_i32(PATH_PTR, "path pointer")?,
                14,
                64,
            ),
        )?,
        ERRNO_SUCCESS
    );
    assert_eq!(harness.read_u8(64 + 16)?, FILETYPE_REGULAR_FILE);
    assert_eq!(harness.read_u64(64 + 32)?, 16);
    assert_eq!(harness.store.data().open_virtual_file_count(), 0);

    harness.write_bytes(PATH_PTR, b"app")?;
    assert_eq!(
        harness.path_filestat_get.call(
            &mut harness.store,
            (
                PREOPEN_ROOT_FD,
                0,
                test_guest_i32(PATH_PTR, "path pointer")?,
                3,
                64,
            ),
        )?,
        ERRNO_SUCCESS
    );
    assert_eq!(harness.read_u8(64 + 16)?, FILETYPE_DIRECTORY);
    assert_eq!(harness.read_u64(64 + 32)?, 0);

    Ok(())
}

#[test]
fn path_filestat_rejects_unsupported_paths_and_flags() -> Result<()> {
    let mut harness = VirtualFsHarness::new(virtual_fs_config()?)?;

    for (path, errno) in [
        ("app/missing.txt", ERRNO_NOENT),
        ("../app/config.txt", ERRNO_NOTCAPABLE),
        ("/app/config.txt", ERRNO_NOTCAPABLE),
        ("app//config.txt", ERRNO_NOTCAPABLE),
    ] {
        harness.write_bytes(PATH_PTR, path.as_bytes())?;
        assert_eq!(
            harness.path_filestat_get.call(
                &mut harness.store,
                (
                    PREOPEN_ROOT_FD,
                    0,
                    test_guest_i32(PATH_PTR, "path pointer")?,
                    test_guest_i32(path.len(), "path length")?,
                    64,
                ),
            )?,
            errno
        );
    }

    harness.write_bytes(PATH_PTR, b"app/config.txt")?;
    assert_eq!(
        harness.path_filestat_get.call(
            &mut harness.store,
            (
                PREOPEN_ROOT_FD,
                2,
                test_guest_i32(PATH_PTR, "path pointer")?,
                14,
                64,
            ),
        )?,
        ERRNO_NOTCAPABLE
    );
    assert_eq!(
        harness
            .path_filestat_get
            .call(&mut harness.store, (99, 0, 128, 14, 64))?,
        ERRNO_BADF
    );
    Ok(())
}

#[test]
fn path_filestat_preflights_stat_pointer_and_path_length() -> Result<()> {
    let mut harness = VirtualFsHarness::new(virtual_fs_config()?)?;
    harness.write_bytes(PATH_PTR, b"app/config.txt")?;
    let memory_len = harness.memory.data_size(&harness.store);

    let err = harness
        .path_filestat_get
        .call(
            &mut harness.store,
            (
                PREOPEN_ROOT_FD,
                0,
                test_guest_i32(PATH_PTR, "path pointer")?,
                14,
                test_guest_i32(memory_len - 4, "stat pointer")?,
            ),
        )
        .expect_err("path_filestat_get should preflight stat pointer");
    assert!(format!("{err:#}").contains("guest memory range"));

    let relative_too_long = "a".repeat(MAX_VIRTUAL_FILE_PATH_BYTES);
    harness.write_bytes(PATH_PTR, relative_too_long.as_bytes())?;
    assert_eq!(
        harness.path_filestat_get.call(
            &mut harness.store,
            (
                PREOPEN_ROOT_FD,
                0,
                test_guest_i32(PATH_PTR, "path pointer")?,
                test_guest_i32(relative_too_long.len(), "path length")?,
                64,
            ),
        )?,
        ERRNO_NAMETOOLONG
    );
    Ok(())
}

#[test]
fn fd_read_rejects_oversized_iov_table_before_nread_or_offset_change() -> Result<()> {
    let mut harness = VirtualFsHarness::new(virtual_fs_config()?)?;
    assert_eq!(harness.open_path("app/config.txt", 96)?, ERRNO_SUCCESS);
    let fd = i32::try_from(harness.read_u32(96)?).context("fd should fit i32")?;

    harness.write_u32(72, u32::MAX)?;
    let memory_len = harness.memory.data_size(&harness.store);
    let err = harness
        .fd_read
        .call(
            &mut harness.store,
            (
                fd,
                16,
                i32::try_from(memory_len / WASI_IOV_SIZE)
                    .context("test iov count should fit i32")?,
                72,
            ),
        )
        .expect_err("oversized iov table should trap before reading");

    assert!(format!("{err:#}").contains("guest memory range"));
    assert_eq!(harness.read_u32(72)?, u32::MAX);

    harness.write_iov(300, 500, 1)?;
    assert_eq!(
        harness.fd_read.call(&mut harness.store, (fd, 300, 1, 72))?,
        ERRNO_SUCCESS
    );
    assert_eq!(harness.read_u32(72)?, 1);
    assert_eq!(harness.read_bytes(500, 1)?, b"h");
    Ok(())
}

#[test]
fn fd_read_rejects_total_iov_len_overflow_before_writing() -> Result<()> {
    let mut harness = VirtualFsHarness::new(virtual_fs_config()?)?;
    assert_eq!(harness.open_path("app/config.txt", 96)?, ERRNO_SUCCESS);
    let fd = i32::try_from(harness.read_u32(96)?).context("fd should fit i32")?;

    let target_pages = 256;
    let current_pages = harness.memory.size(&harness.store);
    if current_pages < target_pages {
        harness
            .memory
            .grow(&mut harness.store, target_pages - current_pages)?;
    }

    let memory_len = harness.memory.data_size(&harness.store);
    let iovs_ptr = 1024;
    let iovs_len = (u32::MAX as usize / memory_len) + 1;
    for index in 0..iovs_len {
        harness.write_iov(iovs_ptr + (index * WASI_IOV_SIZE), 0, memory_len)?;
    }
    harness.write_bytes(0, b"Z")?;
    harness.write_u32(72, u32::MAX)?;

    let err = harness
        .fd_read
        .call(
            &mut harness.store,
            (
                fd,
                test_guest_i32(iovs_ptr, "fd_read iovs pointer")?,
                test_guest_i32(iovs_len, "fd_read iovs length")?,
                72,
            ),
        )
        .expect_err("iov byte-count overflow should trap before writes");

    assert!(format!("{err:#}").contains("WASI byte count overflow"));
    assert_eq!(harness.read_bytes(0, 1)?, b"Z");
    assert_eq!(harness.read_u32(72)?, u32::MAX);
    Ok(())
}

#[test]
fn virtual_file_open_rejects_missing_mutating_and_escaping_paths() -> Result<()> {
    let mut harness = VirtualFsHarness::new(virtual_fs_config()?)?;

    assert_eq!(harness.open_path("app/missing.txt", 60)?, ERRNO_NOENT);

    for path in ["../app/config.txt", "/app/config.txt", "app//config.txt"] {
        assert_eq!(harness.open_path(path, 60)?, ERRNO_NOTCAPABLE);
    }

    assert_eq!(
        harness.open_path_with("app/config.txt", READ_SEEK_STAT_RIGHTS, 1, 0, 0, 60)?,
        ERRNO_SUCCESS
    );

    for (dirflags, oflags, fdflags) in [(2, 0, 0), (0, 1, 0), (0, 0, 1)] {
        assert_eq!(
            harness.open_path_with(
                "app/config.txt",
                READ_SEEK_STAT_RIGHTS,
                dirflags,
                oflags,
                fdflags,
                60,
            )?,
            ERRNO_NOTCAPABLE
        );
    }

    assert_eq!(
        harness.open_path_with("app/config.txt", 1 << 6, 0, 0, 0, 60)?,
        ERRNO_NOTCAPABLE
    );
    assert_eq!(
        harness.path_open.call(
            &mut harness.store,
            (99, 0, 128, 14, 0, RIGHT_FD_READ, 0, 0, 60)
        )?,
        ERRNO_BADF
    );
    Ok(())
}

#[test]
fn virtual_file_rights_are_not_silently_upgraded() -> Result<()> {
    let mut harness = VirtualFsHarness::new(virtual_fs_config()?)?;

    assert_eq!(
        harness.open_path_with("app/config.txt", 0, 0, 0, 0, 96)?,
        ERRNO_SUCCESS
    );
    let fd = i32::try_from(harness.read_u32(96)?).context("fd should fit i32")?;

    harness.write_iov(300, 500, 5)?;
    assert_eq!(
        harness.fd_read.call(&mut harness.store, (fd, 300, 1, 72))?,
        ERRNO_NOTCAPABLE
    );
    assert_eq!(
        harness.fd_seek.call(&mut harness.store, (fd, 0, 0, 88))?,
        ERRNO_NOTCAPABLE
    );
    assert_eq!(
        harness.fd_filestat_get.call(&mut harness.store, (fd, 88))?,
        ERRNO_NOTCAPABLE
    );
    Ok(())
}

#[test]
fn virtual_file_seek_rejects_invalid_whence_and_negative_offsets() -> Result<()> {
    let mut harness = VirtualFsHarness::new(virtual_fs_config()?)?;

    assert_eq!(harness.open_path("app/config.txt", 96)?, ERRNO_SUCCESS);
    let fd = i32::try_from(harness.read_u32(96)?).context("fd should fit i32")?;

    assert_eq!(
        harness.fd_seek.call(&mut harness.store, (fd, -1, 0, 88))?,
        ERRNO_INVAL
    );
    assert_eq!(
        harness.fd_seek.call(&mut harness.store, (fd, 0, 99, 88))?,
        ERRNO_INVAL
    );
    assert_eq!(
        harness
            .fd_seek
            .call(&mut harness.store, (1, 0, 0, 0x7fff))?,
        ERRNO_BADF
    );
    Ok(())
}

#[test]
fn fd_close_reports_stdio_and_preopen_as_unsupported() -> Result<()> {
    let mut harness = VirtualFsHarness::new(virtual_fs_config()?)?;

    assert_eq!(harness.fd_close.call(&mut harness.store, 1)?, ERRNO_NOSYS);
    assert_eq!(harness.fd_close.call(&mut harness.store, 2)?, ERRNO_NOSYS);
    assert_eq!(
        harness.fd_close.call(&mut harness.store, PREOPEN_ROOT_FD)?,
        ERRNO_NOSYS
    );
    assert_eq!(harness.fd_close.call(&mut harness.store, 99)?, ERRNO_BADF);
    Ok(())
}

#[test]
fn fd_fdstat_reports_stdio_preopen_and_regular_file_types() -> Result<()> {
    let mut harness = VirtualFsHarness::new(virtual_fs_config()?)?;

    assert_eq!(
        harness.fd_fdstat_get.call(&mut harness.store, (1, 64))?,
        ERRNO_SUCCESS
    );
    assert_eq!(harness.read_u8(64)?, FILETYPE_CHARACTER_DEVICE);

    assert_eq!(
        harness
            .fd_fdstat_get
            .call(&mut harness.store, (PREOPEN_ROOT_FD, 64))?,
        ERRNO_SUCCESS
    );
    assert_eq!(harness.read_u8(64)?, FILETYPE_DIRECTORY);
    assert_eq!(
        harness.read_u64(72)? & (RIGHT_PATH_OPEN as u64),
        RIGHT_PATH_OPEN as u64
    );
    assert_eq!(
        harness.read_u64(72)? & (RIGHT_PATH_FILESTAT_GET as u64),
        RIGHT_PATH_FILESTAT_GET as u64
    );
    assert_eq!(
        harness.read_u64(80)? & (RIGHT_FD_READ as u64),
        RIGHT_FD_READ as u64
    );

    assert_eq!(harness.open_path("app/config.txt", 96)?, ERRNO_SUCCESS);
    let fd = i32::try_from(harness.read_u32(96)?).context("fd should fit i32")?;
    assert_eq!(
        harness.fd_fdstat_get.call(&mut harness.store, (fd, 64))?,
        ERRNO_SUCCESS
    );
    assert_eq!(harness.read_u8(64)?, FILETYPE_REGULAR_FILE);
    assert_eq!(
        harness.read_u64(72)? & (RIGHT_FD_READ as u64),
        RIGHT_FD_READ as u64
    );
    assert_eq!(harness.read_u64(80)?, 0);
    Ok(())
}
