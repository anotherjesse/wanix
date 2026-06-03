use super::*;
use anyhow::Result;
use std::sync::{Arc, Mutex};

const LIBC_REGULAR_FILE_READ_RIGHTS: u64 = (1 << 1)
    | (1 << 2)
    | (1 << 5)
    | (1 << 10)
    | (1 << 13)
    | (1 << 14)
    | (1 << 18)
    | (1 << 19)
    | (1 << 21);
const LIBC_REGULAR_FILE_INHERITING_RIGHTS: u64 = LIBC_REGULAR_FILE_READ_RIGHTS | (1 << 6);
const FILE_RIGHTS_READ_SEEK_STAT: u64 = (1 << 1) | (1 << 2) | (1 << 5) | (1 << 21);
const DIRECTORY_RIGHTS_BASE: u64 =
    (1 << 10) | (1 << 13) | (1 << 14) | (1 << 18) | (1 << 19) | (1 << 21);
const DIRECTORY_RIGHTS_INHERITING: u64 =
    DIRECTORY_RIGHTS_BASE | FILE_RIGHTS_READ_SEEK_STAT | (1 << 6);

#[derive(Clone)]
struct LiveReadHost {
    calls: Arc<Mutex<Vec<String>>>,
    bytes: Arc<[u8]>,
    offset: usize,
}

impl LiveReadHost {
    fn new(bytes: impl Into<Arc<[u8]>>) -> Self {
        Self {
            calls: Arc::new(Mutex::new(Vec::new())),
            bytes: bytes.into(),
            offset: 0,
        }
    }

    fn calls(&self) -> Arc<Mutex<Vec<String>>> {
        Arc::clone(&self.calls)
    }

    fn record(&self, call: impl Into<String>) {
        self.calls.lock().expect("test call lock").push(call.into());
    }
}

impl QuickJsWasiHost for LiveReadHost {
    fn fd_prestat_get(
        &mut self,
        fd: u32,
    ) -> std::result::Result<QuickJsWasiPrestat, QuickJsWasiErrno> {
        self.record(format!("prestat:{fd}"));
        match fd {
            3 => Ok(QuickJsWasiPrestat::new("/")),
            _ => Err(QuickJsWasiErrno::Badf),
        }
    }

    fn path_open(
        &mut self,
        dirfd: u32,
        dirflags: u32,
        path: &[u8],
        oflags: u16,
        rights_base: u64,
        rights_inheriting: u64,
        fdflags: u16,
    ) -> std::result::Result<u32, QuickJsWasiErrno> {
        self.record(format!(
            "open:{dirfd}:{dirflags}:{}:{oflags}:{rights_base}:{rights_inheriting}:{fdflags}",
            String::from_utf8_lossy(path)
        ));
        if dirfd == 3 && path == b"input.txt" && oflags == 0 && fdflags == 0 {
            self.offset = 0;
            Ok(4)
        } else {
            Err(QuickJsWasiErrno::Noent)
        }
    }

    fn fd_read(&mut self, fd: u32, buf: &mut [u8]) -> std::result::Result<usize, QuickJsWasiErrno> {
        self.record(format!("read:{fd}:{}", buf.len()));
        if fd != 4 {
            return Err(QuickJsWasiErrno::Badf);
        }
        let remaining = self.bytes.len().saturating_sub(self.offset);
        let count = remaining.min(buf.len());
        buf[..count].copy_from_slice(&self.bytes[self.offset..self.offset + count]);
        self.offset += count;
        Ok(count)
    }

    fn fd_readdir(
        &mut self,
        fd: u32,
    ) -> std::result::Result<Vec<QuickJsWasiDirEntry>, QuickJsWasiErrno> {
        self.record(format!("readdir:{fd}"));
        Err(QuickJsWasiErrno::Nosys)
    }

    fn fd_write(&mut self, fd: u32, buf: &[u8]) -> std::result::Result<usize, QuickJsWasiErrno> {
        self.record(format!("write:{fd}:{}", String::from_utf8_lossy(buf)));
        Ok(buf.len())
    }

    fn fd_seek(
        &mut self,
        fd: u32,
        offset: i64,
        whence: QuickJsWasiWhence,
    ) -> std::result::Result<u64, QuickJsWasiErrno> {
        if fd != 4 {
            return Err(QuickJsWasiErrno::Badf);
        }
        let base = match whence {
            QuickJsWasiWhence::Set => 0,
            QuickJsWasiWhence::Cur => {
                i64::try_from(self.offset).map_err(|_| QuickJsWasiErrno::Inval)?
            }
            QuickJsWasiWhence::End => {
                i64::try_from(self.bytes.len()).map_err(|_| QuickJsWasiErrno::Inval)?
            }
        };
        let next = base.checked_add(offset).ok_or(QuickJsWasiErrno::Inval)?;
        if next < 0 {
            return Err(QuickJsWasiErrno::Inval);
        }
        self.offset = usize::try_from(next).map_err(|_| QuickJsWasiErrno::Inval)?;
        self.record(format!("seek:{fd}:{offset}:{whence:?}"));
        u64::try_from(self.offset).map_err(|_| QuickJsWasiErrno::Inval)
    }

    fn fd_tell(&mut self, fd: u32) -> std::result::Result<u64, QuickJsWasiErrno> {
        if fd != 4 {
            return Err(QuickJsWasiErrno::Badf);
        }
        self.record(format!("tell:{fd}"));
        u64::try_from(self.offset).map_err(|_| QuickJsWasiErrno::Inval)
    }

    fn fd_close(&mut self, fd: u32) -> std::result::Result<(), QuickJsWasiErrno> {
        self.record(format!("close:{fd}"));
        if fd == 4 {
            Ok(())
        } else {
            Err(QuickJsWasiErrno::Badf)
        }
    }

    fn fd_fdstat_get(
        &mut self,
        fd: u32,
    ) -> std::result::Result<QuickJsWasiFdStat, QuickJsWasiErrno> {
        self.record(format!("fdstat:{fd}"));
        match fd {
            3 => Ok(QuickJsWasiFdStat::new(
                QuickJsWasiFileType::Directory,
                DIRECTORY_RIGHTS_BASE,
                DIRECTORY_RIGHTS_INHERITING,
            )),
            4 => Ok(QuickJsWasiFdStat::new(
                QuickJsWasiFileType::RegularFile,
                FILE_RIGHTS_READ_SEEK_STAT,
                0,
            )),
            _ => Err(QuickJsWasiErrno::Badf),
        }
    }

    fn fd_filestat_get(
        &mut self,
        fd: u32,
    ) -> std::result::Result<QuickJsWasiFileStat, QuickJsWasiErrno> {
        self.record(format!("filestat:{fd}"));
        if fd == 4 {
            Ok(QuickJsWasiFileStat::new(
                QuickJsWasiFileType::RegularFile,
                u64::try_from(self.bytes.len()).map_err(|_| QuickJsWasiErrno::Inval)?,
            ))
        } else {
            Err(QuickJsWasiErrno::Badf)
        }
    }

    fn path_filestat_get(
        &mut self,
        dirfd: u32,
        flags: u32,
        path: &[u8],
    ) -> std::result::Result<QuickJsWasiFileStat, QuickJsWasiErrno> {
        self.record(format!(
            "pathstat:{dirfd}:{flags}:{}",
            String::from_utf8_lossy(path)
        ));
        if dirfd == 3 && path == b"input.txt" {
            Ok(QuickJsWasiFileStat::new(
                QuickJsWasiFileType::RegularFile,
                u64::try_from(self.bytes.len()).map_err(|_| QuickJsWasiErrno::Inval)?,
            ))
        } else {
            Err(QuickJsWasiErrno::Noent)
        }
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
fn quickjs_std_load_file_uses_live_wasi_host() -> Result<()> {
    let (_engine, module) = quickjs_fixture()?;
    let host = LiveReadHost::new(&b"from live host"[..]);
    let calls = host.calls();
    let options = QuickJsCreateOptions::new().with_wasi_host(host);
    let mut vm = module.create_runtime_with_options(options)?;

    vm.eval_module_discard(
        r#"
        import * as std from "qjs:std";
        globalThis.loadedText = std.loadFile("input.txt");
        "#,
        "stdlib-load-file.mjs",
    )?;

    assert_eq!(vm.eval_string("loadedText")?, "from live host");
    let calls = calls.lock().expect("test call lock");
    assert!(calls.iter().any(|call| {
        call == &format!(
            "open:3:1:input.txt:0:{LIBC_REGULAR_FILE_READ_RIGHTS}:{LIBC_REGULAR_FILE_INHERITING_RIGHTS}:0"
        )
    }));
    assert!(calls.iter().any(|call| call.starts_with("read:4:")));
    assert!(calls.iter().any(|call| call == "close:4"));
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
