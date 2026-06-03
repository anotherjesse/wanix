use super::*;
use anyhow::Result;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

const OFLAGS_CREATE_TRUNCATE: u16 = 9;
const LIBC_REGULAR_FILE_READ_RIGHTS: u64 = (1 << 1)
    | (1 << 2)
    | (1 << 5)
    | (1 << 10)
    | (1 << 13)
    | (1 << 14)
    | (1 << 18)
    | (1 << 19)
    | (1 << 21);
const LIBC_REGULAR_FILE_WRITE_RIGHTS: u64 =
    (1 << 2) | (1 << 5) | (1 << 6) | (1 << 10) | (1 << 13) | (1 << 18) | (1 << 19) | (1 << 21);
const LIBC_REGULAR_FILE_INHERITING_RIGHTS: u64 = LIBC_REGULAR_FILE_READ_RIGHTS | (1 << 6);
const FILE_RIGHTS_READ_WRITE_SEEK_STAT: u64 = (1 << 1) | (1 << 2) | (1 << 5) | (1 << 6) | (1 << 21);
const DIRECTORY_RIGHTS_BASE: u64 =
    (1 << 10) | (1 << 13) | (1 << 14) | (1 << 18) | (1 << 19) | (1 << 21);
const DIRECTORY_RIGHTS_INHERITING: u64 = DIRECTORY_RIGHTS_BASE | FILE_RIGHTS_READ_WRITE_SEEK_STAT;

#[derive(Debug, Clone)]
struct OpenFile {
    path: String,
    offset: usize,
    rights_base: u64,
}

#[derive(Clone)]
struct LiveFileHost {
    calls: Arc<Mutex<Vec<String>>>,
    files: Arc<Mutex<BTreeMap<String, Vec<u8>>>>,
    open: BTreeMap<u32, OpenFile>,
    next_fd: u32,
}

impl LiveFileHost {
    fn with_file(path: impl Into<String>, bytes: impl Into<Vec<u8>>) -> Self {
        let mut files = BTreeMap::new();
        files.insert(path.into(), bytes.into());
        Self {
            calls: Arc::new(Mutex::new(Vec::new())),
            files: Arc::new(Mutex::new(files)),
            open: BTreeMap::new(),
            next_fd: 4,
        }
    }

    fn empty() -> Self {
        Self {
            calls: Arc::new(Mutex::new(Vec::new())),
            files: Arc::new(Mutex::new(BTreeMap::new())),
            open: BTreeMap::new(),
            next_fd: 4,
        }
    }

    fn calls(&self) -> Arc<Mutex<Vec<String>>> {
        Arc::clone(&self.calls)
    }

    fn files(&self) -> Arc<Mutex<BTreeMap<String, Vec<u8>>>> {
        Arc::clone(&self.files)
    }

    fn record(&self, call: impl Into<String>) {
        self.calls.lock().expect("test call lock").push(call.into());
    }
}

impl QuickJsWasiHost for LiveFileHost {
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
        let path = String::from_utf8_lossy(path).to_string();
        self.record(format!(
            "open:{dirfd}:{dirflags}:{path}:{oflags}:{rights_base}:{rights_inheriting}:{fdflags}"
        ));
        if dirfd != 3 || fdflags != 0 {
            return Err(QuickJsWasiErrno::Notcapable);
        }
        let mut files = self.files.lock().expect("test files lock");
        if oflags & OFLAGS_CREATE_TRUNCATE != 0 {
            files.insert(path.clone(), Vec::new());
        } else if !files.contains_key(&path) {
            return Err(QuickJsWasiErrno::Noent);
        }
        drop(files);
        let fd = self.next_fd;
        self.next_fd += 1;
        self.open.insert(
            fd,
            OpenFile {
                path,
                offset: 0,
                rights_base,
            },
        );
        Ok(fd)
    }

    fn fd_read(&mut self, fd: u32, buf: &mut [u8]) -> std::result::Result<usize, QuickJsWasiErrno> {
        self.record(format!("read:{fd}:{}", buf.len()));
        let open = self.open.get_mut(&fd).ok_or(QuickJsWasiErrno::Badf)?;
        let files = self.files.lock().expect("test files lock");
        let bytes = files.get(&open.path).ok_or(QuickJsWasiErrno::Noent)?;
        let remaining = bytes.len().saturating_sub(open.offset);
        let count = remaining.min(buf.len());
        buf[..count].copy_from_slice(&bytes[open.offset..open.offset + count]);
        open.offset += count;
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
        let open = self.open.get_mut(&fd).ok_or(QuickJsWasiErrno::Badf)?;
        if open.rights_base & (1 << 6) == 0 {
            return Err(QuickJsWasiErrno::Notcapable);
        }
        let mut files = self.files.lock().expect("test files lock");
        let bytes = files.get_mut(&open.path).ok_or(QuickJsWasiErrno::Noent)?;
        let end = open
            .offset
            .checked_add(buf.len())
            .ok_or(QuickJsWasiErrno::Inval)?;
        if end > bytes.len() {
            bytes.resize(end, 0);
        }
        bytes[open.offset..end].copy_from_slice(buf);
        open.offset = end;
        Ok(buf.len())
    }

    fn fd_seek(
        &mut self,
        fd: u32,
        offset: i64,
        whence: QuickJsWasiWhence,
    ) -> std::result::Result<u64, QuickJsWasiErrno> {
        let open = self.open.get_mut(&fd).ok_or(QuickJsWasiErrno::Badf)?;
        let files = self.files.lock().expect("test files lock");
        let len = files.get(&open.path).ok_or(QuickJsWasiErrno::Noent)?.len();
        let base = match whence {
            QuickJsWasiWhence::Set => 0,
            QuickJsWasiWhence::Cur => {
                i64::try_from(open.offset).map_err(|_| QuickJsWasiErrno::Inval)?
            }
            QuickJsWasiWhence::End => i64::try_from(len).map_err(|_| QuickJsWasiErrno::Inval)?,
        };
        let next = base.checked_add(offset).ok_or(QuickJsWasiErrno::Inval)?;
        if next < 0 {
            return Err(QuickJsWasiErrno::Inval);
        }
        open.offset = usize::try_from(next).map_err(|_| QuickJsWasiErrno::Inval)?;
        let next_offset = open.offset;
        self.record(format!("seek:{fd}:{offset}:{whence:?}"));
        u64::try_from(next_offset).map_err(|_| QuickJsWasiErrno::Inval)
    }

    fn fd_tell(&mut self, fd: u32) -> std::result::Result<u64, QuickJsWasiErrno> {
        let offset = self.open.get(&fd).ok_or(QuickJsWasiErrno::Badf)?.offset;
        self.record(format!("tell:{fd}"));
        u64::try_from(offset).map_err(|_| QuickJsWasiErrno::Inval)
    }

    fn fd_close(&mut self, fd: u32) -> std::result::Result<(), QuickJsWasiErrno> {
        self.record(format!("close:{fd}"));
        self.open
            .remove(&fd)
            .map(|_| ())
            .ok_or(QuickJsWasiErrno::Badf)
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
            fd => {
                let rights = self
                    .open
                    .get(&fd)
                    .ok_or(QuickJsWasiErrno::Badf)?
                    .rights_base
                    & FILE_RIGHTS_READ_WRITE_SEEK_STAT;
                Ok(QuickJsWasiFdStat::new(
                    QuickJsWasiFileType::RegularFile,
                    rights,
                    0,
                ))
            }
        }
    }

    fn fd_filestat_get(
        &mut self,
        fd: u32,
    ) -> std::result::Result<QuickJsWasiFileStat, QuickJsWasiErrno> {
        self.record(format!("filestat:{fd}"));
        let open = self.open.get(&fd).ok_or(QuickJsWasiErrno::Badf)?;
        let files = self.files.lock().expect("test files lock");
        file_stat(files.get(&open.path).ok_or(QuickJsWasiErrno::Noent)?)
    }

    fn path_filestat_get(
        &mut self,
        dirfd: u32,
        flags: u32,
        path: &[u8],
    ) -> std::result::Result<QuickJsWasiFileStat, QuickJsWasiErrno> {
        let path = String::from_utf8_lossy(path).to_string();
        self.record(format!("pathstat:{dirfd}:{flags}:{path}"));
        let files = self.files.lock().expect("test files lock");
        file_stat(files.get(&path).ok_or(QuickJsWasiErrno::Noent)?)
    }
}

fn file_stat(bytes: &[u8]) -> std::result::Result<QuickJsWasiFileStat, QuickJsWasiErrno> {
    Ok(QuickJsWasiFileStat::new(
        QuickJsWasiFileType::RegularFile,
        u64::try_from(bytes.len()).map_err(|_| QuickJsWasiErrno::Inval)?,
    ))
}

#[test]
fn quickjs_std_load_file_uses_live_wasi_host() -> Result<()> {
    let (_engine, module) = quickjs_fixture()?;
    let host = LiveFileHost::with_file("input.txt", b"from live host".to_vec());
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
fn quickjs_std_write_file_uses_live_wasi_host() -> Result<()> {
    let (_engine, module) = quickjs_fixture()?;
    let host = LiveFileHost::empty();
    let calls = host.calls();
    let files = host.files();
    let options = QuickJsCreateOptions::new().with_wasi_host(host);
    let mut vm = module.create_runtime_with_options(options)?;

    vm.eval_module_discard(
        r#"
        import * as std from "qjs:std";
        std.writeFile("created.txt", "from std write");
        "#,
        "stdlib-write-file.mjs",
    )?;

    let calls = calls.lock().expect("test call lock");
    assert!(calls.iter().any(|call| {
        call == &format!(
            "open:3:1:created.txt:{OFLAGS_CREATE_TRUNCATE}:{LIBC_REGULAR_FILE_WRITE_RIGHTS}:{LIBC_REGULAR_FILE_INHERITING_RIGHTS}:0"
        )
    }));
    assert!(calls.iter().any(|call| call == "write:4:from std write"));
    assert!(calls.iter().any(|call| call == "close:4"));
    drop(calls);
    assert_eq!(
        files.lock().expect("test files lock").get("created.txt"),
        Some(&b"from std write".to_vec())
    );
    Ok(())
}
