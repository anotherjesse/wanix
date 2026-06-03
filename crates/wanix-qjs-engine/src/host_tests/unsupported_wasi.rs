use super::*;
use wasmtime::{Engine, Linker, Module, Store, TypedFunc};

const ERRNO_BADF: i32 = 8;
const ERRNO_NOSYS: i32 = 52;

const UNSUPPORTED_WASI_WAT: &str = r#"
(module
  (import "wasi_snapshot_preview1" "fd_close" (func $fd_close (param i32) (result i32)))
  (import "wasi_snapshot_preview1" "fd_seek" (func $fd_seek (param i32 i64 i32 i32) (result i32)))

  (func (export "fd_close") (param i32) (result i32)
    local.get 0
    call $fd_close)
  (func (export "fd_seek") (param i32 i64 i32 i32) (result i32)
    local.get 0
    local.get 1
    local.get 2
    local.get 3
    call $fd_seek)
)
"#;

struct UnsupportedWasiHarness {
    store: Store<HostState>,
    fd_close: TypedFunc<i32, i32>,
    fd_seek: TypedFunc<(i32, i64, i32, i32), i32>,
}

fn unsupported_wasi_harness() -> Result<UnsupportedWasiHarness> {
    let engine = Engine::default();
    let module = Module::new(&engine, UNSUPPORTED_WASI_WAT)?;
    let mut linker = Linker::<HostState>::new(&engine);
    define_wasi_imports(&mut linker)?;
    let mut store = Store::new(&engine, HostState::new(QuickJsHostConfig::new()));
    let instance = linker.instantiate(&mut store, &module)?;

    Ok(UnsupportedWasiHarness {
        fd_close: instance.get_typed_func(&mut store, "fd_close")?,
        fd_seek: instance.get_typed_func(&mut store, "fd_seek")?,
        store,
    })
}

#[test]
fn fd_close_distinguishes_stdio_from_unknown_descriptors() -> Result<()> {
    let mut harness = unsupported_wasi_harness()?;

    assert_eq!(harness.fd_close.call(&mut harness.store, 0)?, ERRNO_BADF);
    assert_eq!(harness.fd_close.call(&mut harness.store, 1)?, ERRNO_NOSYS);
    assert_eq!(harness.fd_close.call(&mut harness.store, 2)?, ERRNO_NOSYS);
    assert_eq!(harness.fd_close.call(&mut harness.store, 99)?, ERRNO_BADF);
    Ok(())
}

#[test]
fn fd_seek_returns_badf_without_memory_access_for_unknown_descriptors() -> Result<()> {
    let mut harness = unsupported_wasi_harness()?;

    assert_eq!(
        harness
            .fd_seek
            .call(&mut harness.store, (1, -123, 2, 0x7fff))?,
        ERRNO_BADF
    );
    Ok(())
}
