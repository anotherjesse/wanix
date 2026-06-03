use super::*;
use anyhow::Result;
use std::sync::{Arc, Mutex};

type WasiHostResult<T> = std::result::Result<T, QuickJsWasiErrno>;
type RecordedWrites = Arc<Mutex<Vec<(u32, Vec<u8>)>>>;
const RIGHT_FD_READ: u64 = 1 << 1;
const RIGHT_FD_WRITE: u64 = 1 << 6;

#[derive(Default)]
struct ExitWasiHost {
    env: Vec<String>,
    exits: Arc<Mutex<Vec<u32>>>,
    writes: RecordedWrites,
}

impl QuickJsWasiHost for ExitWasiHost {
    fn env(&mut self) -> WasiHostResult<Vec<String>> {
        Ok(self.env.clone())
    }

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

#[derive(Default)]
struct ReadyFdWasiHost {
    stdin: Arc<Mutex<Vec<u8>>>,
    writes: RecordedWrites,
}

impl ReadyFdWasiHost {
    fn with_stdin(bytes: impl Into<Vec<u8>>) -> Self {
        Self {
            stdin: Arc::new(Mutex::new(bytes.into())),
            writes: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn writes(&self) -> RecordedWrites {
        Arc::clone(&self.writes)
    }
}

impl QuickJsWasiHost for ReadyFdWasiHost {
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

    fn fd_read(&mut self, fd: u32, buf: &mut [u8]) -> WasiHostResult<usize> {
        if fd != 0 {
            return Err(QuickJsWasiErrno::Badf);
        }
        let mut stdin = self.stdin.lock().expect("test stdin lock");
        let count = buf.len().min(stdin.len());
        buf[..count].copy_from_slice(&stdin[..count]);
        stdin.drain(..count);
        Ok(count)
    }

    fn fd_readdir(&mut self, _fd: u32) -> WasiHostResult<Vec<QuickJsWasiDirEntry>> {
        Err(QuickJsWasiErrno::Nosys)
    }

    fn fd_write(&mut self, fd: u32, buf: &[u8]) -> WasiHostResult<usize> {
        if fd != 1 {
            return Err(QuickJsWasiErrno::Badf);
        }
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

    fn fd_fdstat_get(&mut self, fd: u32) -> WasiHostResult<QuickJsWasiFdStat> {
        match fd {
            0 => Ok(QuickJsWasiFdStat::new(
                QuickJsWasiFileType::CharacterDevice,
                RIGHT_FD_READ,
                0,
            )),
            1 => Ok(QuickJsWasiFdStat::new(
                QuickJsWasiFileType::CharacterDevice,
                RIGHT_FD_WRITE,
                0,
            )),
            _ => Err(QuickJsWasiErrno::Badf),
        }
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
fn quickjs_std_env_uses_live_wasi_host_environ() -> Result<()> {
    let (_engine, module) = quickjs_fixture()?;
    let host = ExitWasiHost {
        env: vec!["MODE=test".to_owned(), "EMPTY=".to_owned()],
        ..ExitWasiHost::default()
    };
    let mut vm =
        module.create_runtime_with_options(QuickJsCreateOptions::new().with_wasi_host(host))?;

    vm.eval_module_discard(
        r#"
        import * as std from "qjs:std";
        const env = std.getenviron();
        globalThis.mode = std.getenv("MODE");
        globalThis.empty = env.EMPTY === "";
        globalThis.missing = String(std.getenv("MISSING"));
        "#,
        "stdlib-env.mjs",
    )?;

    assert_eq!(vm.eval_string("mode")?, "test");
    assert_eq!(vm.eval_string("String(empty)")?, "true");
    assert_eq!(vm.eval_string("missing")?, "undefined");
    Ok(())
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
        globalThis.qjsOsLstat = typeof os.lstat;
        globalThis.qjsOsReadlink = typeof os.readlink;
        globalThis.qjsOsSymlink = typeof os.symlink;
        globalThis.qjsOsFtruncate = typeof os.ftruncate;
        globalThis.qjsOsTruncate = typeof os.truncate;
        "#,
        "stdlib-modules.mjs",
    )?;

    assert_eq!(vm.eval_string("qjsStdOutPuts")?, "function");
    assert_eq!(vm.eval_string("qjsOsOpen")?, "function");
    assert_eq!(vm.eval_string("qjsOsLstat")?, "function");
    assert_eq!(vm.eval_string("qjsOsReadlink")?, "function");
    assert_eq!(vm.eval_string("qjsOsSymlink")?, "function");
    assert_eq!(vm.eval_string("qjsOsFtruncate")?, "function");
    assert_eq!(vm.eval_string("qjsOsTruncate")?, "function");
    Ok(())
}

#[test]
fn quickjs_os_sleep_uses_timer_poll_oneoff() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;

    vm.eval_module_discard(
        r#"
        import * as os from "qjs:os";
        globalThis.beforeSleep = true;
        os.sleep(0);
        globalThis.afterSleep = true;
        "#,
        "stdlib-sleep.mjs",
    )?;

    assert_eq!(vm.eval_string("String(beforeSleep && afterSleep)")?, "true");
    Ok(())
}

#[test]
fn quickjs_os_async_timers_need_event_loop_turns() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;

    vm.eval_module_discard(
        r#"
        import * as os from "qjs:os";
        globalThis.timerExports = [
          typeof os.sleepAsync,
          typeof os.setTimeout,
          typeof os.setInterval,
        ].join(",");
        globalThis.asyncTimerFired = false;
        if (typeof os.sleepAsync === "function") {
          os.sleepAsync(0).then(() => {
            globalThis.asyncTimerFired = true;
          });
        }
        if (typeof os.setTimeout === "function") {
          os.setTimeout(() => {
            globalThis.asyncTimerFired = true;
          }, 0);
        }
        "#,
        "stdlib-async-timers.mjs",
    )?;

    assert_eq!(
        vm.eval_string("timerExports")?,
        "function,function,function"
    );
    assert_eq!(
        vm.execute_pending_jobs_with_limit(4)?,
        0,
        "pending jobs alone should not complete async timers"
    );
    assert_eq!(vm.eval_string("String(asyncTimerFired)")?, "false");
    Ok(())
}

#[test]
fn quickjs_os_async_timers_run_on_immediate_event_loop_turns() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;

    vm.eval_module_discard(
        r#"
        import * as os from "qjs:os";
        globalThis.timerEvents = [];
        os.sleepAsync(0).then(() => {
          globalThis.timerEvents.push("sleep");
        });
        "#,
        "stdlib-immediate-timers.mjs",
    )?;

    assert_eq!(vm.eval_string("timerEvents.join(',')")?, "");
    assert!(vm.execute_immediate_event_loop_with_limit(8)? > 0);
    assert_eq!(vm.eval_string("timerEvents.join(',')")?, "sleep");
    assert_eq!(vm.execute_event_loop_once()?, QuickJsEventLoopStatus::Idle);
    Ok(())
}

#[test]
fn quickjs_os_future_timer_reports_wait_without_blocking() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;

    vm.eval_module_discard(
        r#"
        import * as os from "qjs:os";
        globalThis.futureTimerFired = false;
        os.setTimeout(() => {
          globalThis.futureTimerFired = true;
        }, 50);
        "#,
        "stdlib-future-timer.mjs",
    )?;

    match vm.execute_event_loop_once()? {
        QuickJsEventLoopStatus::Wait(delay) => {
            assert!(delay.as_millis() <= 50, "delay was {delay:?}");
        }
        status => bail!("expected future timer wait status, got {status:?}"),
    }
    assert_eq!(vm.eval_string("String(futureTimerFired)")?, "false");
    Ok(())
}

#[test]
fn quickjs_os_fd_handlers_run_on_ready_io_event_loop_turns() -> Result<()> {
    let (_engine, module) = quickjs_fixture()?;
    let host = ReadyFdWasiHost::with_stdin(b"ready input".to_vec());
    let writes = host.writes();
    let mut vm =
        module.create_runtime_with_options(QuickJsCreateOptions::new().with_wasi_host(host))?;

    vm.eval_module_discard(
        r#"
        import * as os from "qjs:os";
        import * as std from "qjs:std";

        globalThis.fdHandlerEvents = [];
        os.setReadHandler(0, () => {
          const bytes = new Uint8Array(32);
          const n = os.read(0, bytes.buffer, 0, bytes.length);
          const text = Array.from(bytes.slice(0, n)).map((byte) => String.fromCharCode(byte)).join("");
          globalThis.fdHandlerEvents.push("read:" + text);
          os.setReadHandler(0, null);
        });
        os.setWriteHandler(1, () => {
          std.out.puts("write handler\n");
          std.out.flush();
          globalThis.fdHandlerEvents.push("write");
          os.setWriteHandler(1, null);
        });
        "#,
        "stdlib-fd-handlers.mjs",
    )?;

    vm.execute_ready_io_event_loop_once()?;
    vm.execute_ready_io_event_loop_once()?;

    assert_eq!(
        vm.eval_string("fdHandlerEvents.sort().join('|')")?,
        "read:ready input|write"
    );
    assert!(
        writes
            .lock()
            .expect("test writes lock")
            .iter()
            .any(|write| write == &(1, b"write handler\n".to_vec()))
    );
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
