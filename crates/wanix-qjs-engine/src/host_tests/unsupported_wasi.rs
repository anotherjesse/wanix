use super::*;
use wasmtime::{Engine, Linker, Memory, Module, Store, TypedFunc};

const ERRNO_BADF: i32 = 8;
const ERRNO_INVAL: i32 = 28;
const ERRNO_NOSYS: i32 = 52;
const ERRNO_SUCCESS: i32 = 0;
const SUBSCRIPTION_SIZE: usize = 48;
const SUBSCRIPTION_USERDATA_OFFSET: usize = 0;
const SUBSCRIPTION_TAG_OFFSET: usize = 8;
const SUBSCRIPTION_CLOCK_ID_OFFSET: usize = 16;
const SUBSCRIPTION_CLOCK_TIMEOUT_OFFSET: usize = 24;
const SUBSCRIPTION_CLOCK_FLAGS_OFFSET: usize = 40;
const EVENT_SIZE: usize = 32;
const EVENT_USERDATA_OFFSET: usize = 0;
const EVENT_ERROR_OFFSET: usize = 8;
const EVENT_TYPE_OFFSET: usize = 10;
const EVENTTYPE_CLOCK: u8 = 0;
const EVENTTYPE_FD_READ: u8 = 1;
const EVENTTYPE_FD_WRITE: u8 = 2;
const CLOCKID_MONOTONIC: u32 = 1;
const CLOCKID_PROCESS_CPUTIME_ID: u32 = 2;
const SUBCLOCKFLAGS_ABSTIME: u16 = 1 << 0;

type PathFilestatSetTimesFunc = TypedFunc<(i32, i32, i32, i32, i64, i64, i32), i32>;

const UNSUPPORTED_WASI_WAT: &str = r#"
(module
  (import "wasi_snapshot_preview1" "fd_close" (func $fd_close (param i32) (result i32)))
  (import "wasi_snapshot_preview1" "fd_fdstat_set_flags" (func $fd_fdstat_set_flags (param i32 i32) (result i32)))
  (import "wasi_snapshot_preview1" "fd_seek" (func $fd_seek (param i32 i64 i32 i32) (result i32)))
  (import "wasi_snapshot_preview1" "path_create_directory" (func $path_create_directory (param i32 i32 i32) (result i32)))
  (import "wasi_snapshot_preview1" "path_filestat_set_times" (func $path_filestat_set_times (param i32 i32 i32 i32 i64 i64 i32) (result i32)))
  (import "wasi_snapshot_preview1" "path_remove_directory" (func $path_remove_directory (param i32 i32 i32) (result i32)))
  (import "wasi_snapshot_preview1" "path_rename" (func $path_rename (param i32 i32 i32 i32 i32 i32) (result i32)))
  (import "wasi_snapshot_preview1" "path_unlink_file" (func $path_unlink_file (param i32 i32 i32) (result i32)))
  (import "wasi_snapshot_preview1" "poll_oneoff" (func $poll_oneoff (param i32 i32 i32 i32) (result i32)))
  (memory (export "memory") 1)

  (func (export "fd_close") (param i32) (result i32)
    local.get 0
    call $fd_close)
  (func (export "fd_fdstat_set_flags") (param i32 i32) (result i32)
    local.get 0
    local.get 1
    call $fd_fdstat_set_flags)
  (func (export "fd_seek") (param i32 i64 i32 i32) (result i32)
    local.get 0
    local.get 1
    local.get 2
    local.get 3
    call $fd_seek)
  (func (export "path_create_directory") (param i32 i32 i32) (result i32)
    local.get 0
    local.get 1
    local.get 2
    call $path_create_directory)
  (func (export "path_filestat_set_times") (param i32 i32 i32 i32 i64 i64 i32) (result i32)
    local.get 0
    local.get 1
    local.get 2
    local.get 3
    local.get 4
    local.get 5
    local.get 6
    call $path_filestat_set_times)
  (func (export "path_remove_directory") (param i32 i32 i32) (result i32)
    local.get 0
    local.get 1
    local.get 2
    call $path_remove_directory)
  (func (export "path_rename") (param i32 i32 i32 i32 i32 i32) (result i32)
    local.get 0
    local.get 1
    local.get 2
    local.get 3
    local.get 4
    local.get 5
    call $path_rename)
  (func (export "path_unlink_file") (param i32 i32 i32) (result i32)
    local.get 0
    local.get 1
    local.get 2
    call $path_unlink_file)
  (func (export "poll_oneoff") (param i32 i32 i32 i32) (result i32)
    local.get 0
    local.get 1
    local.get 2
    local.get 3
    call $poll_oneoff)
)
"#;

struct UnsupportedWasiHarness {
    store: Store<HostState>,
    memory: Memory,
    fd_close: TypedFunc<i32, i32>,
    fd_fdstat_set_flags: TypedFunc<(i32, i32), i32>,
    fd_seek: TypedFunc<(i32, i64, i32, i32), i32>,
    path_create_directory: TypedFunc<(i32, i32, i32), i32>,
    path_filestat_set_times: PathFilestatSetTimesFunc,
    path_remove_directory: TypedFunc<(i32, i32, i32), i32>,
    path_rename: TypedFunc<(i32, i32, i32, i32, i32, i32), i32>,
    path_unlink_file: TypedFunc<(i32, i32, i32), i32>,
    poll_oneoff: TypedFunc<(i32, i32, i32, i32), i32>,
}

fn unsupported_wasi_harness(config: QuickJsHostConfig) -> Result<UnsupportedWasiHarness> {
    let engine = Engine::default();
    let module = Module::new(&engine, UNSUPPORTED_WASI_WAT)?;
    let mut linker = Linker::<HostState>::new(&engine);
    define_wasi_imports(&mut linker)?;
    let mut store = Store::new(&engine, HostState::new(config));
    let instance = linker.instantiate(&mut store, &module)?;
    let memory = instance
        .get_memory(&mut store, "memory")
        .expect("test module exports memory");
    store.data_mut().set_memory(memory);

    Ok(UnsupportedWasiHarness {
        fd_close: instance.get_typed_func(&mut store, "fd_close")?,
        fd_fdstat_set_flags: instance.get_typed_func(&mut store, "fd_fdstat_set_flags")?,
        fd_seek: instance.get_typed_func(&mut store, "fd_seek")?,
        path_create_directory: instance.get_typed_func(&mut store, "path_create_directory")?,
        path_filestat_set_times: instance.get_typed_func(&mut store, "path_filestat_set_times")?,
        path_remove_directory: instance.get_typed_func(&mut store, "path_remove_directory")?,
        path_rename: instance.get_typed_func(&mut store, "path_rename")?,
        path_unlink_file: instance.get_typed_func(&mut store, "path_unlink_file")?,
        poll_oneoff: instance.get_typed_func(&mut store, "poll_oneoff")?,
        memory,
        store,
    })
}

#[test]
fn fd_close_distinguishes_stdio_from_unknown_descriptors() -> Result<()> {
    let mut harness = unsupported_wasi_harness(QuickJsHostConfig::new())?;

    assert_eq!(harness.fd_close.call(&mut harness.store, 0)?, ERRNO_BADF);
    assert_eq!(harness.fd_close.call(&mut harness.store, 1)?, ERRNO_NOSYS);
    assert_eq!(harness.fd_close.call(&mut harness.store, 2)?, ERRNO_NOSYS);
    assert_eq!(harness.fd_close.call(&mut harness.store, 99)?, ERRNO_BADF);
    Ok(())
}

#[test]
fn fd_seek_returns_badf_without_memory_access_for_unknown_descriptors() -> Result<()> {
    let mut harness = unsupported_wasi_harness(QuickJsHostConfig::new())?;

    assert_eq!(
        harness
            .fd_seek
            .call(&mut harness.store, (1, -123, 2, 0x7fff))?,
        ERRNO_BADF
    );
    Ok(())
}

#[test]
fn fd_fdstat_set_flags_is_defined_but_unsupported() -> Result<()> {
    let mut harness = unsupported_wasi_harness(QuickJsHostConfig::new())?;

    assert_eq!(
        harness
            .fd_fdstat_set_flags
            .call(&mut harness.store, (1, 0))?,
        ERRNO_NOSYS
    );
    assert_eq!(
        harness
            .fd_fdstat_set_flags
            .call(&mut harness.store, (99, 0))?,
        ERRNO_BADF
    );
    Ok(())
}

#[test]
fn path_mutation_imports_are_defined_but_unsupported() -> Result<()> {
    let config = QuickJsHostConfig::new().with_read_only_virtual_file("/file.txt", b"data")?;
    let mut harness = unsupported_wasi_harness(config)?;
    harness.memory.write(&mut harness.store, 64, b"next")?;
    harness.memory.write(&mut harness.store, 96, b"other")?;

    assert_eq!(
        harness
            .path_create_directory
            .call(&mut harness.store, (3, 64, 4))?,
        ERRNO_NOSYS
    );
    assert_eq!(
        harness
            .path_remove_directory
            .call(&mut harness.store, (3, 64, 4))?,
        ERRNO_NOSYS
    );
    assert_eq!(
        harness
            .path_unlink_file
            .call(&mut harness.store, (3, 64, 4))?,
        ERRNO_NOSYS
    );
    assert_eq!(
        harness
            .path_filestat_set_times
            .call(&mut harness.store, (3, 0, 64, 4, 0, 0, 0))?,
        ERRNO_NOSYS
    );
    assert_eq!(
        harness
            .path_rename
            .call(&mut harness.store, (3, 64, 4, 3, 96, 5))?,
        ERRNO_NOSYS
    );
    Ok(())
}

#[test]
fn poll_oneoff_empty_subscription_set_is_invalid() -> Result<()> {
    let mut harness = unsupported_wasi_harness(QuickJsHostConfig::new())?;

    assert_eq!(
        harness
            .poll_oneoff
            .call(&mut harness.store, (0, 0, 0, 64))?,
        ERRNO_INVAL
    );
    Ok(())
}

#[test]
fn poll_oneoff_single_clock_subscription_reports_clock_event() -> Result<()> {
    let mut harness = unsupported_wasi_harness(QuickJsHostConfig::new())?;
    write_clock_subscription(
        &harness.memory,
        &mut harness.store,
        64,
        0x1122_3344_5566_7788,
        0,
    )?;
    harness
        .memory
        .write(&mut harness.store, 192, &u32::MAX.to_le_bytes())?;

    assert_eq!(
        harness
            .poll_oneoff
            .call(&mut harness.store, (64, 128, 1, 192))?,
        ERRNO_SUCCESS
    );

    let mut event = [0; EVENT_SIZE];
    harness.memory.read(&harness.store, 128, &mut event)?;
    let mut nevents = [0; 4];
    harness.memory.read(&harness.store, 192, &mut nevents)?;
    assert_eq!(u32::from_le_bytes(nevents), 1);
    assert_eq!(
        read_u64(&event, EVENT_USERDATA_OFFSET),
        0x1122_3344_5566_7788
    );
    assert_eq!(read_u16(&event, EVENT_ERROR_OFFSET), 0);
    assert_eq!(event[EVENT_TYPE_OFFSET], EVENTTYPE_CLOCK);
    Ok(())
}

#[test]
fn poll_oneoff_absolute_clock_subscription_reports_due_event() -> Result<()> {
    let mut harness = unsupported_wasi_harness(QuickJsHostConfig::new().with_clock_time_ns(100))?;
    write_clock_subscription_with(
        &harness.memory,
        &mut harness.store,
        64,
        ClockSubscription {
            userdata: 0xaabb_ccdd_eeff_0011,
            clock_id: CLOCKID_MONOTONIC,
            timeout_ns: 100,
            flags: SUBCLOCKFLAGS_ABSTIME,
        },
    )?;

    assert_eq!(
        harness
            .poll_oneoff
            .call(&mut harness.store, (64, 128, 1, 192))?,
        ERRNO_SUCCESS
    );

    let mut event = [0; EVENT_SIZE];
    harness.memory.read(&harness.store, 128, &mut event)?;
    assert_eq!(
        read_u64(&event, EVENT_USERDATA_OFFSET),
        0xaabb_ccdd_eeff_0011
    );
    assert_eq!(read_u16(&event, EVENT_ERROR_OFFSET), 0);
    assert_eq!(event[EVENT_TYPE_OFFSET], EVENTTYPE_CLOCK);
    Ok(())
}

#[test]
fn poll_oneoff_unknown_clock_flags_are_invalid() -> Result<()> {
    let mut harness = unsupported_wasi_harness(QuickJsHostConfig::new())?;
    write_clock_subscription_with(
        &harness.memory,
        &mut harness.store,
        64,
        ClockSubscription {
            userdata: 0,
            clock_id: CLOCKID_MONOTONIC,
            timeout_ns: 0,
            flags: 1 << 1,
        },
    )?;

    assert_eq!(
        harness
            .poll_oneoff
            .call(&mut harness.store, (64, 128, 1, 192))?,
        ERRNO_INVAL
    );
    Ok(())
}

#[test]
fn poll_oneoff_unsupported_clock_ids_remain_unsupported() -> Result<()> {
    let mut harness = unsupported_wasi_harness(QuickJsHostConfig::new())?;
    write_clock_subscription_with(
        &harness.memory,
        &mut harness.store,
        64,
        ClockSubscription {
            userdata: 0,
            clock_id: CLOCKID_PROCESS_CPUTIME_ID,
            timeout_ns: 0,
            flags: 0,
        },
    )?;

    assert_eq!(
        harness
            .poll_oneoff
            .call(&mut harness.store, (64, 128, 1, 192))?,
        ERRNO_NOSYS
    );
    Ok(())
}

#[test]
fn poll_oneoff_fd_subscriptions_remain_unsupported() -> Result<()> {
    let mut harness = unsupported_wasi_harness(QuickJsHostConfig::new())?;
    write_fd_subscription(&harness.memory, &mut harness.store, 64, EVENTTYPE_FD_READ)?;

    assert_eq!(
        harness
            .poll_oneoff
            .call(&mut harness.store, (64, 128, 1, 192))?,
        ERRNO_NOSYS
    );
    write_fd_subscription(&harness.memory, &mut harness.store, 64, EVENTTYPE_FD_WRITE)?;

    assert_eq!(
        harness
            .poll_oneoff
            .call(&mut harness.store, (64, 128, 1, 192))?,
        ERRNO_NOSYS
    );
    Ok(())
}

#[test]
fn poll_oneoff_multi_subscription_sets_remain_unsupported() -> Result<()> {
    let mut harness = unsupported_wasi_harness(QuickJsHostConfig::new())?;
    harness
        .memory
        .write(&mut harness.store, 64, &u32::MAX.to_le_bytes())?;

    assert_eq!(
        harness
            .poll_oneoff
            .call(&mut harness.store, (0, 0, 2, 64))?,
        ERRNO_NOSYS
    );
    Ok(())
}

struct ClockSubscription {
    userdata: u64,
    clock_id: u32,
    timeout_ns: u64,
    flags: u16,
}

fn write_clock_subscription(
    memory: &Memory,
    store: &mut Store<HostState>,
    ptr: usize,
    userdata: u64,
    timeout_ns: u64,
) -> Result<()> {
    write_clock_subscription_with(
        memory,
        store,
        ptr,
        ClockSubscription {
            userdata,
            clock_id: CLOCKID_MONOTONIC,
            timeout_ns,
            flags: 0,
        },
    )
}

fn write_clock_subscription_with(
    memory: &Memory,
    store: &mut Store<HostState>,
    ptr: usize,
    clock: ClockSubscription,
) -> Result<()> {
    let mut bytes = [0; SUBSCRIPTION_SIZE];
    bytes[SUBSCRIPTION_USERDATA_OFFSET..SUBSCRIPTION_USERDATA_OFFSET + 8]
        .copy_from_slice(&clock.userdata.to_le_bytes());
    bytes[SUBSCRIPTION_TAG_OFFSET] = EVENTTYPE_CLOCK;
    bytes[SUBSCRIPTION_CLOCK_ID_OFFSET..SUBSCRIPTION_CLOCK_ID_OFFSET + 4]
        .copy_from_slice(&clock.clock_id.to_le_bytes());
    bytes[SUBSCRIPTION_CLOCK_TIMEOUT_OFFSET..SUBSCRIPTION_CLOCK_TIMEOUT_OFFSET + 8]
        .copy_from_slice(&clock.timeout_ns.to_le_bytes());
    bytes[SUBSCRIPTION_CLOCK_FLAGS_OFFSET..SUBSCRIPTION_CLOCK_FLAGS_OFFSET + 2]
        .copy_from_slice(&clock.flags.to_le_bytes());
    memory.write(store, ptr, &bytes)?;
    Ok(())
}

fn write_fd_subscription(
    memory: &Memory,
    store: &mut Store<HostState>,
    ptr: usize,
    event_type: u8,
) -> Result<()> {
    let mut subscription = [0; SUBSCRIPTION_SIZE];
    subscription[SUBSCRIPTION_TAG_OFFSET] = event_type;
    memory.write(store, ptr, &subscription)?;
    Ok(())
}

fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

fn read_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
        bytes[offset + 4],
        bytes[offset + 5],
        bytes[offset + 6],
        bytes[offset + 7],
    ])
}
