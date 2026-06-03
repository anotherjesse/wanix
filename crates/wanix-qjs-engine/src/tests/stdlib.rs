use super::*;
use anyhow::Result;
use std::sync::{Arc, Mutex};

type WasiHostResult<T> = std::result::Result<T, QuickJsWasiErrno>;
type RecordedWrites = Arc<Mutex<Vec<(u32, Vec<u8>)>>>;

#[derive(Default)]
struct ExitWasiHost {
    exits: Arc<Mutex<Vec<u32>>>,
    writes: RecordedWrites,
}

impl QuickJsWasiHost for ExitWasiHost {
    fn proc_exit(&mut self, code: u32) -> WasiHostResult<()> {
        self.exits.lock().expect("test exit lock").push(code);
        Ok(())
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

    fn fd_write(&mut self, fd: u32, buf: &[u8]) -> WasiHostResult<usize> {
        self.writes
            .lock()
            .expect("test writes lock")
            .push((fd, buf.to_vec()));
        Ok(buf.len())
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
        Err(QuickJsWasiErrno::Nosys)
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
fn fixture_exposes_quickjs_std_and_os_modules() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;

    vm.eval_module_discard(
        r#"
        import * as std from "qjs:std";
        import * as os from "qjs:os";
        globalThis.qjsStdOutPuts = typeof std.out.puts;
        globalThis.qjsOsOpen = typeof os.open;
        "#,
        "stdlib-modules.mjs",
    )?;

    assert_eq!(vm.eval_string("qjsStdOutPuts")?, "function");
    assert_eq!(vm.eval_string("qjsOsOpen")?, "function");
    Ok(())
}

#[test]
fn quickjs_std_stdout_uses_wasi_capture() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let config = QuickJsHostConfig::new().with_stdout_capture(true);
    let mut vm = QuickJsRuntime::create_with_host_config(&engine, &module, config)?;

    vm.eval_module_discard(
        r#"
        import * as std from "qjs:std";
        std.out.puts("via qjs std\n");
        std.out.flush();
        "#,
        "stdlib-stdout.mjs",
    )?;

    assert_eq!(vm.take_captured_stdout(), b"via qjs std\n");
    Ok(())
}

#[test]
fn quickjs_std_exit_uses_live_wasi_host_proc_exit() -> Result<()> {
    let (_engine, module) = quickjs_fixture()?;
    let host = ExitWasiHost::default();
    let exits = Arc::clone(&host.exits);
    let writes = Arc::clone(&host.writes);
    let mut vm =
        module.create_runtime_with_options(QuickJsCreateOptions::new().with_wasi_host(host))?;

    let err = vm
        .eval_module_discard(
            r#"
            import * as std from "qjs:std";
            std.exit(7);
            globalThis.afterStdExit = true;
            "#,
            "stdlib-exit.mjs",
        )
        .expect_err("std.exit should trap through proc_exit");

    let message = format!("{err:#}");
    assert!(message.contains("WASI proc_exit(7)"), "{message}");
    assert_eq!(*exits.lock().expect("test exit lock"), [7]);
    let snapshot_err = vm
        .snapshot()
        .expect_err("process-exited runtime should not snapshot");
    assert!(
        snapshot_err
            .to_string()
            .contains("cannot snapshot after WASI proc_exit"),
        "{snapshot_err}"
    );
    drop(vm);
    assert!(writes.lock().expect("test writes lock").is_empty());
    Ok(())
}

#[test]
fn quickjs_std_stdout_reattaches_capture_after_restore() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;
    vm.eval_module_discard(
        r#"
        import * as std from "qjs:std";
        globalThis.stdReadyBeforeSnapshot = typeof std.out.puts;
        "#,
        "stdlib-before-snapshot.mjs",
    )?;
    assert_eq!(vm.eval_string("stdReadyBeforeSnapshot")?, "function");
    let snapshot = vm.snapshot()?;
    drop(vm);

    let config = QuickJsHostConfig::new().with_stdout_capture(true);
    let mut restored =
        QuickJsRuntime::restore_with_host_config(&engine, &module, &snapshot, config)?;
    restored.eval_module_discard(
        r#"
        import * as std from "qjs:std";
        std.out.puts("after restore\n");
        std.out.flush();
        "#,
        "stdlib-after-restore.mjs",
    )?;

    assert_eq!(restored.take_captured_stdout(), b"after restore\n");
    Ok(())
}

#[test]
fn quickjs_std_import_coexists_with_rust_module_loader() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;
    let normalized = Arc::new(Mutex::new(Vec::new()));
    let normalized_for_loader = Arc::clone(&normalized);

    vm.set_module_loader_with_normalizer(
        move |_base_name, specifier| {
            normalized_for_loader
                .lock()
                .expect("test normalizer lock")
                .push(specifier.to_owned());
            match specifier {
                "./lib.js" => Ok("lib.js".to_owned()),
                other => Ok(other.to_owned()),
            }
        },
        |name| match name {
            "lib.js" => Ok("export const message = 'from rust loader';".to_owned()),
            other => anyhow::bail!("unexpected module load {other}"),
        },
    )?;
    vm.eval_module_discard(
        r#"
        import * as std from "qjs:std";
        import { message } from "./lib.js";
        globalThis.loaderMessage = message;
        globalThis.loaderStdOutPuts = typeof std.out.puts;
        "#,
        "stdlib-loader.mjs",
    )?;

    assert_eq!(vm.eval_string("loaderMessage")?, "from rust loader");
    assert_eq!(vm.eval_string("loaderStdOutPuts")?, "function");
    assert_eq!(
        normalized.lock().expect("test normalizer lock").as_slice(),
        &["./lib.js".to_owned()]
    );
    Ok(())
}
