use super::*;
use anyhow::Result;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

const OFLAGS_CREATE_TRUNCATE: u16 = 9;
const OFLAGS_DIRECTORY: u16 = 2;
const FDFLAGS_NONBLOCK: u16 = 4;
const PATH_LOOKUP_SYMLINK_FOLLOW: u32 = 1;
const FD_WRITE_RIGHT: u64 = 1 << 6;
const FD_FILESTAT_SET_SIZE_RIGHT: u64 = 1 << 22;
const LIBC_REGULAR_FILE_READ_RIGHTS: u64 = (1 << 1)
    | (1 << 2)
    | (1 << 5)
    | (1 << 10)
    | (1 << 13)
    | (1 << 14)
    | (1 << 18)
    | (1 << 19)
    | (1 << 21);
const LIBC_REGULAR_FILE_WRITE_RIGHTS: u64 = (1 << 2)
    | (1 << 5)
    | FD_WRITE_RIGHT
    | (1 << 10)
    | (1 << 13)
    | (1 << 18)
    | (1 << 19)
    | (1 << 21);
const LIBC_REGULAR_FILE_INHERITING_RIGHTS: u64 = LIBC_REGULAR_FILE_READ_RIGHTS | (1 << 6);
const FILE_RIGHTS_READ_WRITE_SEEK_STAT: u64 =
    (1 << 1) | (1 << 2) | (1 << 5) | FD_WRITE_RIGHT | (1 << 21);
const FILE_FDSTAT_RIGHTS: u64 = FILE_RIGHTS_READ_WRITE_SEEK_STAT | FD_FILESTAT_SET_SIZE_RIGHT;
const DIRECTORY_RIGHTS_BASE: u64 =
    (1 << 10) | (1 << 13) | (1 << 14) | (1 << 18) | (1 << 19) | (1 << 21);
const DIRECTORY_RIGHTS_INHERITING: u64 = DIRECTORY_RIGHTS_BASE | FILE_RIGHTS_READ_WRITE_SEEK_STAT;
const LIBC_DIRECTORY_RIGHTS_BASE: u64 = DIRECTORY_RIGHTS_INHERITING & !(1 << 6);

#[derive(Debug, Clone)]
struct OpenFile {
    path: String,
    offset: usize,
    rights_base: u64,
    kind: OpenKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OpenKind {
    File,
    Directory,
}

struct PathOpenRequest {
    dirfd: u32,
    dirflags: u32,
    path: String,
    oflags: u16,
    rights_base: u64,
    rights_inheriting: u64,
    fdflags: u16,
}

#[derive(Clone)]
struct LiveFileHost {
    calls: Arc<Mutex<Vec<String>>>,
    files: Arc<Mutex<BTreeMap<String, Vec<u8>>>>,
    symlinks: Arc<Mutex<BTreeMap<String, Vec<u8>>>>,
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
            symlinks: Arc::new(Mutex::new(BTreeMap::new())),
            open: BTreeMap::new(),
            next_fd: 4,
        }
    }

    fn empty() -> Self {
        Self {
            calls: Arc::new(Mutex::new(Vec::new())),
            files: Arc::new(Mutex::new(BTreeMap::new())),
            symlinks: Arc::new(Mutex::new(BTreeMap::new())),
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

    fn symlinks(&self) -> Arc<Mutex<BTreeMap<String, Vec<u8>>>> {
        Arc::clone(&self.symlinks)
    }

    fn record(&self, call: impl Into<String>) {
        self.calls.lock().expect("test call lock").push(call.into());
    }

    fn resolve_dir_path(
        &self,
        dirfd: u32,
        path: &str,
    ) -> std::result::Result<String, QuickJsWasiErrno> {
        if dirfd == 3 {
            return Ok(path.to_owned());
        }
        let open = self.open.get(&dirfd).ok_or(QuickJsWasiErrno::Badf)?;
        if open.kind != OpenKind::Directory {
            return Err(QuickJsWasiErrno::Notdir);
        }
        if open.path == "." {
            Ok(path.to_owned())
        } else {
            Ok(format!("{}/{path}", open.path))
        }
    }

    fn kind_for_path_open(
        &self,
        request: &PathOpenRequest,
    ) -> std::result::Result<OpenKind, QuickJsWasiErrno> {
        if path_open_creates_file(request) {
            self.create_open_file(request)
        } else {
            self.existing_open_kind(&request.path)
        }
    }

    fn create_open_file(
        &self,
        request: &PathOpenRequest,
    ) -> std::result::Result<OpenKind, QuickJsWasiErrno> {
        let mut files = self.files.lock().expect("test files lock");
        let symlinks = self.symlinks.lock().expect("test symlink lock");
        if path_exists(&files, &symlinks, &request.path) {
            return Err(QuickJsWasiErrno::Isdir);
        }
        if path_open_directory_requested(request) {
            return Err(QuickJsWasiErrno::Notcapable);
        }
        drop(symlinks);
        files.insert(request.path.clone(), Vec::new());
        Ok(OpenKind::File)
    }

    fn existing_open_kind(&self, path: &str) -> std::result::Result<OpenKind, QuickJsWasiErrno> {
        let files = self.files.lock().expect("test files lock");
        let symlinks = self.symlinks.lock().expect("test symlink lock");
        open_path_kind(&files, &symlinks, path).ok_or(QuickJsWasiErrno::Noent)
    }

    fn insert_open_file(&mut self, request: PathOpenRequest, kind: OpenKind) -> u32 {
        let fd = self.next_fd;
        self.next_fd += 1;
        self.open.insert(
            fd,
            OpenFile {
                path: request.path,
                offset: 0,
                rights_base: request.rights_base,
                kind,
            },
        );
        fd
    }

    fn seek_target_offset(
        &self,
        fd: u32,
        offset: i64,
        whence: QuickJsWasiWhence,
    ) -> std::result::Result<usize, QuickJsWasiErrno> {
        let open = self.open.get(&fd).ok_or(QuickJsWasiErrno::Badf)?;
        if open.kind != OpenKind::File {
            return Err(QuickJsWasiErrno::Inval);
        }
        let len = self.open_file_len(open)?;
        let base = seek_base_offset(open.offset, len, whence)?;
        checked_seek_offset(base, offset)
    }

    fn open_file_len(&self, open: &OpenFile) -> std::result::Result<usize, QuickJsWasiErrno> {
        let files = self.files.lock().expect("test files lock");
        files
            .get(&open.path)
            .map(Vec::len)
            .ok_or(QuickJsWasiErrno::Noent)
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
        let path =
            self.resolve_dir_path(dirfd, &normalize_test_path(&String::from_utf8_lossy(path)))?;
        let request = PathOpenRequest {
            dirfd,
            dirflags,
            path,
            oflags,
            rights_base,
            rights_inheriting,
            fdflags,
        };
        self.record(path_open_record(&request));
        validate_path_open_flags(&request)?;
        let kind = self.kind_for_path_open(&request)?;
        validate_path_open_kind(&request, kind)?;
        Ok(self.insert_open_file(request, kind))
    }

    fn fd_read(&mut self, fd: u32, buf: &mut [u8]) -> std::result::Result<usize, QuickJsWasiErrno> {
        self.record(format!("read:{fd}:{}", buf.len()));
        let open = self.open.get_mut(&fd).ok_or(QuickJsWasiErrno::Badf)?;
        if open.kind != OpenKind::File {
            return Err(QuickJsWasiErrno::Isdir);
        }
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
        let open = self.open.get(&fd).ok_or(QuickJsWasiErrno::Badf)?;
        if open.kind != OpenKind::Directory {
            return Err(QuickJsWasiErrno::Notdir);
        }
        let files = self.files.lock().expect("test files lock");
        let symlinks = self.symlinks.lock().expect("test symlink lock");
        Ok(directory_entries(&files, &symlinks, &open.path))
    }

    fn fd_write(&mut self, fd: u32, buf: &[u8]) -> std::result::Result<usize, QuickJsWasiErrno> {
        self.record(format!("write:{fd}:{}", String::from_utf8_lossy(buf)));
        let open = self.open.get_mut(&fd).ok_or(QuickJsWasiErrno::Badf)?;
        if open.kind != OpenKind::File {
            return Err(QuickJsWasiErrno::Isdir);
        }
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
        let next_offset = self.seek_target_offset(fd, offset, whence)?;
        self.open.get_mut(&fd).ok_or(QuickJsWasiErrno::Badf)?.offset = next_offset;
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
                let open = self.open.get(&fd).ok_or(QuickJsWasiErrno::Badf)?;
                match open.kind {
                    OpenKind::File => {
                        let mut rights_base = open.rights_base & FILE_FDSTAT_RIGHTS;
                        if open.rights_base & FD_WRITE_RIGHT != 0 {
                            rights_base |= FD_FILESTAT_SET_SIZE_RIGHT;
                        }
                        Ok(QuickJsWasiFdStat::new(
                            QuickJsWasiFileType::RegularFile,
                            rights_base,
                            0,
                        ))
                    }
                    OpenKind::Directory => Ok(QuickJsWasiFdStat::new(
                        QuickJsWasiFileType::Directory,
                        DIRECTORY_RIGHTS_BASE,
                        DIRECTORY_RIGHTS_INHERITING,
                    )),
                }
            }
        }
    }

    fn fd_filestat_get(
        &mut self,
        fd: u32,
    ) -> std::result::Result<QuickJsWasiFileStat, QuickJsWasiErrno> {
        self.record(format!("filestat:{fd}"));
        let open = self.open.get(&fd).ok_or(QuickJsWasiErrno::Badf)?;
        if open.kind == OpenKind::Directory {
            return directory_stat();
        }
        let files = self.files.lock().expect("test files lock");
        file_stat(files.get(&open.path).ok_or(QuickJsWasiErrno::Noent)?)
    }

    fn fd_filestat_set_size(
        &mut self,
        fd: u32,
        size: u64,
    ) -> std::result::Result<(), QuickJsWasiErrno> {
        self.record(format!("set_size:{fd}:{size}"));
        let open = self.open.get(&fd).ok_or(QuickJsWasiErrno::Badf)?;
        if open.kind != OpenKind::File {
            return Err(QuickJsWasiErrno::Notcapable);
        }
        if open.rights_base & (FD_FILESTAT_SET_SIZE_RIGHT | FD_WRITE_RIGHT) == 0 {
            return Err(QuickJsWasiErrno::Notcapable);
        }
        let mut files = self.files.lock().expect("test files lock");
        let bytes = files.get_mut(&open.path).ok_or(QuickJsWasiErrno::Noent)?;
        bytes.resize(
            usize::try_from(size).map_err(|_| QuickJsWasiErrno::Inval)?,
            0,
        );
        Ok(())
    }

    fn path_filestat_get(
        &mut self,
        dirfd: u32,
        flags: u32,
        path: &[u8],
    ) -> std::result::Result<QuickJsWasiFileStat, QuickJsWasiErrno> {
        let path =
            self.resolve_dir_path(dirfd, &normalize_test_path(&String::from_utf8_lossy(path)))?;
        self.record(format!("pathstat:{dirfd}:{flags}:{path}"));
        let files = self.files.lock().expect("test files lock");
        let symlinks = self.symlinks.lock().expect("test symlink lock");
        path_stat(
            &files,
            &symlinks,
            &path,
            flags & PATH_LOOKUP_SYMLINK_FOLLOW != 0,
        )
    }

    fn path_readlink(
        &mut self,
        dirfd: u32,
        path: &[u8],
    ) -> std::result::Result<Vec<u8>, QuickJsWasiErrno> {
        let path =
            self.resolve_dir_path(dirfd, &normalize_test_path(&String::from_utf8_lossy(path)))?;
        self.record(format!("readlink:{dirfd}:{path}"));
        let symlinks = self.symlinks.lock().expect("test symlink lock");
        symlinks.get(&path).cloned().ok_or(QuickJsWasiErrno::Inval)
    }

    fn path_symlink(
        &mut self,
        target: &[u8],
        dirfd: u32,
        path: &[u8],
    ) -> std::result::Result<(), QuickJsWasiErrno> {
        let path =
            self.resolve_dir_path(dirfd, &normalize_test_path(&String::from_utf8_lossy(path)))?;
        self.record(format!(
            "symlink:{}:{dirfd}:{path}",
            String::from_utf8_lossy(target)
        ));
        if path == "." {
            return Err(QuickJsWasiErrno::Exist);
        }

        let files = self.files.lock().expect("test files lock");
        let mut symlinks = self.symlinks.lock().expect("test symlink lock");
        if path_exists(&files, &symlinks, &path) {
            return Err(QuickJsWasiErrno::Exist);
        }
        if let Some(parent) = parent_path(&path)
            && !directory_exists(&files, &symlinks, parent)
        {
            return Err(QuickJsWasiErrno::Noent);
        }
        symlinks.insert(path, target.to_vec());
        Ok(())
    }
}

fn file_stat(bytes: &[u8]) -> std::result::Result<QuickJsWasiFileStat, QuickJsWasiErrno> {
    Ok(QuickJsWasiFileStat::new(
        QuickJsWasiFileType::RegularFile,
        u64::try_from(bytes.len()).map_err(|_| QuickJsWasiErrno::Inval)?,
    ))
}

fn directory_stat() -> std::result::Result<QuickJsWasiFileStat, QuickJsWasiErrno> {
    Ok(QuickJsWasiFileStat::new(QuickJsWasiFileType::Directory, 0))
}

fn symlink_stat(target: &[u8]) -> std::result::Result<QuickJsWasiFileStat, QuickJsWasiErrno> {
    Ok(QuickJsWasiFileStat::new(
        QuickJsWasiFileType::SymbolicLink,
        u64::try_from(target.len()).map_err(|_| QuickJsWasiErrno::Inval)?,
    ))
}

fn path_open_record(request: &PathOpenRequest) -> String {
    let PathOpenRequest {
        dirfd,
        dirflags,
        path,
        oflags,
        rights_base,
        rights_inheriting,
        fdflags,
    } = request;
    format!("open:{dirfd}:{dirflags}:{path}:{oflags}:{rights_base}:{rights_inheriting}:{fdflags}")
}

const fn path_open_creates_file(request: &PathOpenRequest) -> bool {
    request.oflags & OFLAGS_CREATE_TRUNCATE != 0
}

const fn path_open_directory_requested(request: &PathOpenRequest) -> bool {
    request.oflags & OFLAGS_DIRECTORY != 0
}

fn validate_path_open_flags(
    request: &PathOpenRequest,
) -> std::result::Result<(), QuickJsWasiErrno> {
    if request.oflags & !(OFLAGS_CREATE_TRUNCATE | OFLAGS_DIRECTORY) == 0
        && request.fdflags & !FDFLAGS_NONBLOCK == 0
    {
        Ok(())
    } else {
        Err(QuickJsWasiErrno::Notcapable)
    }
}

fn validate_path_open_kind(
    request: &PathOpenRequest,
    kind: OpenKind,
) -> std::result::Result<(), QuickJsWasiErrno> {
    if path_open_directory_requested(request) && kind != OpenKind::Directory {
        return Err(QuickJsWasiErrno::Notdir);
    }
    if kind == OpenKind::File && request.fdflags != 0 {
        return Err(QuickJsWasiErrno::Notcapable);
    }
    Ok(())
}

fn seek_base_offset(
    current: usize,
    len: usize,
    whence: QuickJsWasiWhence,
) -> std::result::Result<i64, QuickJsWasiErrno> {
    match whence {
        QuickJsWasiWhence::Set => Ok(0),
        QuickJsWasiWhence::Cur => i64::try_from(current).map_err(|_| QuickJsWasiErrno::Inval),
        QuickJsWasiWhence::End => i64::try_from(len).map_err(|_| QuickJsWasiErrno::Inval),
    }
}

fn checked_seek_offset(base: i64, offset: i64) -> std::result::Result<usize, QuickJsWasiErrno> {
    let next = base.checked_add(offset).ok_or(QuickJsWasiErrno::Inval)?;
    if next < 0 {
        return Err(QuickJsWasiErrno::Inval);
    }
    usize::try_from(next).map_err(|_| QuickJsWasiErrno::Inval)
}

fn normalize_test_path(path: &str) -> String {
    let path = path.trim_end_matches('/');
    if path.is_empty() || path == "." || path == "/" {
        return ".".to_owned();
    }
    path.strip_prefix("./").unwrap_or(path).to_owned()
}

fn path_exists(
    files: &BTreeMap<String, Vec<u8>>,
    symlinks: &BTreeMap<String, Vec<u8>>,
    path: &str,
) -> bool {
    files.contains_key(path)
        || symlinks.contains_key(path)
        || directory_exists(files, symlinks, path)
}

fn open_path_kind(
    files: &BTreeMap<String, Vec<u8>>,
    symlinks: &BTreeMap<String, Vec<u8>>,
    path: &str,
) -> Option<OpenKind> {
    if files.contains_key(path) {
        Some(OpenKind::File)
    } else if directory_exists(files, symlinks, path) {
        Some(OpenKind::Directory)
    } else if let Some(target) = symlinks.get(path) {
        let target = normalize_test_path(&String::from_utf8_lossy(target));
        if files.contains_key(&target) {
            Some(OpenKind::File)
        } else if directory_exists(files, symlinks, &target) {
            Some(OpenKind::Directory)
        } else {
            None
        }
    } else {
        None
    }
}

fn path_stat(
    files: &BTreeMap<String, Vec<u8>>,
    symlinks: &BTreeMap<String, Vec<u8>>,
    path: &str,
    follow_symlink: bool,
) -> std::result::Result<QuickJsWasiFileStat, QuickJsWasiErrno> {
    if let Some(bytes) = files.get(path) {
        return file_stat(bytes);
    }
    if directory_exists(files, symlinks, path) {
        return directory_stat();
    }
    if let Some(target) = symlinks.get(path) {
        if !follow_symlink {
            return symlink_stat(target);
        }
        let target = normalize_test_path(&String::from_utf8_lossy(target));
        return path_stat(files, symlinks, &target, false);
    }
    Err(QuickJsWasiErrno::Noent)
}

fn directory_exists(
    files: &BTreeMap<String, Vec<u8>>,
    symlinks: &BTreeMap<String, Vec<u8>>,
    path: &str,
) -> bool {
    if path == "." {
        return true;
    }
    let prefix = format!("{path}/");
    files.keys().any(|file| file.starts_with(&prefix))
        || symlinks.keys().any(|link| link.starts_with(&prefix))
}

fn parent_path(path: &str) -> Option<&str> {
    path.rsplit_once('/')
        .map(|(parent, _)| if parent.is_empty() { "." } else { parent })
}

fn directory_entries(
    files: &BTreeMap<String, Vec<u8>>,
    symlinks: &BTreeMap<String, Vec<u8>>,
    path: &str,
) -> Vec<QuickJsWasiDirEntry> {
    let prefix = if path == "." {
        String::new()
    } else {
        format!("{path}/")
    };
    let mut entries = BTreeMap::new();
    for file in files.keys() {
        insert_directory_entry(
            &mut entries,
            &prefix,
            file,
            QuickJsWasiFileType::RegularFile,
        );
    }
    for link in symlinks.keys() {
        insert_directory_entry(
            &mut entries,
            &prefix,
            link,
            QuickJsWasiFileType::SymbolicLink,
        );
    }
    entries
        .into_iter()
        .map(|(name, file_type)| QuickJsWasiDirEntry::new(name, file_type))
        .collect()
}

fn insert_directory_entry(
    entries: &mut BTreeMap<String, QuickJsWasiFileType>,
    prefix: &str,
    path: &str,
    direct_type: QuickJsWasiFileType,
) {
    let Some(rest) = path.strip_prefix(prefix) else {
        return;
    };
    if rest.is_empty() {
        return;
    }
    let (name, file_type) = match rest.split_once('/') {
        Some((name, _)) => (name, QuickJsWasiFileType::Directory),
        None => (rest, direct_type),
    };
    entries.entry(name.to_owned()).or_insert(file_type);
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
fn quickjs_std_load_file_passes_service_paths_to_live_wasi_host() -> Result<()> {
    let (_engine, module) = quickjs_fixture()?;
    let host = LiveFileHost::with_file("#task/self/id", b"1\n".to_vec());
    let calls = host.calls();
    let options = QuickJsCreateOptions::new().with_wasi_host(host);
    let mut vm = module.create_runtime_with_options(options)?;

    vm.eval_module_discard(
        r##"
        import * as std from "qjs:std";
        globalThis.loadedTaskId = std.loadFile("#task/self/id");
        "##,
        "stdlib-load-service-file.mjs",
    )?;

    assert_eq!(vm.eval_string("loadedTaskId")?, "1\n");
    let calls = calls.lock().expect("test call lock");
    assert!(calls.iter().any(|call| {
        call == &format!(
            "open:3:1:#task/self/id:0:{LIBC_REGULAR_FILE_READ_RIGHTS}:{LIBC_REGULAR_FILE_INHERITING_RIGHTS}:0"
        )
    }));
    assert!(calls.iter().any(|call| call.starts_with("read:4:")));
    assert!(calls.iter().any(|call| call == "close:4"));
    Ok(())
}

#[test]
fn quickjs_os_readdir_uses_live_wasi_host() -> Result<()> {
    let (_engine, module) = quickjs_fixture()?;
    let host = LiveFileHost::with_file("input.txt", b"from live host".to_vec());
    host.files()
        .lock()
        .expect("test files lock")
        .insert("dir/nested.txt".to_owned(), b"nested".to_vec());
    let calls = host.calls();
    let options = QuickJsCreateOptions::new().with_wasi_host(host);
    let mut vm = module.create_runtime_with_options(options)?;

    vm.eval_module_discard(
        r#"
        import * as os from "qjs:os";

        function visible(entries) {
          return entries.filter((name) => name !== "." && name !== "..").sort().join(",");
        }

        const [rootEntries, rootErr] = os.readdir(".");
        const [dirEntries, dirErr] = os.readdir("dir");
        globalThis.rootList = rootErr + ":" + visible(rootEntries);
        globalThis.dirList = dirErr + ":" + visible(dirEntries);
        "#,
        "stdlib-readdir.mjs",
    )?;

    let calls = calls.lock().expect("test call lock");
    assert_eq!(
        vm.eval_string("rootList")?,
        "0:dir,input.txt",
        "calls: {calls:?}"
    );
    assert_eq!(
        vm.eval_string("dirList")?,
        "0:nested.txt",
        "calls: {calls:?}"
    );
    assert!(calls.iter().any(|call| {
        call
            == &format!(
                "open:3:1:.:{OFLAGS_DIRECTORY}:{LIBC_DIRECTORY_RIGHTS_BASE}:{DIRECTORY_RIGHTS_INHERITING}:{FDFLAGS_NONBLOCK}"
            )
    }));
    assert!(calls.iter().any(|call| {
        call
            == &format!(
                "open:3:1:dir:{OFLAGS_DIRECTORY}:{LIBC_DIRECTORY_RIGHTS_BASE}:{DIRECTORY_RIGHTS_INHERITING}:{FDFLAGS_NONBLOCK}"
            )
    }));
    assert!(calls.iter().any(|call| call == "readdir:4"));
    assert!(calls.iter().any(|call| call == "readdir:5"));
    Ok(())
}

#[test]
fn quickjs_os_symlink_readlink_and_lstat_use_live_wasi_host() -> Result<()> {
    let (_engine, module) = quickjs_fixture()?;
    let host = LiveFileHost::with_file("target.txt", b"from live host".to_vec());
    let calls = host.calls();
    let symlinks = host.symlinks();
    let options = QuickJsCreateOptions::new().with_wasi_host(host);
    let mut vm = module.create_runtime_with_options(options)?;

    vm.eval_module_discard(
        r#"
        import * as os from "qjs:os";

        const symlinkErr = os.symlink("target.txt", "link.txt");
        const [target, readlinkErr] = os.readlink("link.txt");
        const [linkStat, lstatErr] = os.lstat("link.txt");
        const [targetStat, statErr] = os.stat("link.txt");

        globalThis.symlinkSummary = [
          symlinkErr,
          readlinkErr,
          target,
          lstatErr,
          (linkStat.mode & os.S_IFMT) === os.S_IFLNK,
          linkStat.size,
          statErr,
          (targetStat.mode & os.S_IFMT) === os.S_IFREG,
          targetStat.size,
        ].join("|");
        "#,
        "stdlib-symlink.mjs",
    )?;

    assert_eq!(
        vm.eval_string("symlinkSummary")?,
        "0|0|target.txt|0|true|10|0|true|14"
    );
    assert_eq!(
        symlinks.lock().expect("test symlink lock").get("link.txt"),
        Some(&b"target.txt".to_vec())
    );
    let calls = calls.lock().expect("test call lock");
    assert!(
        calls
            .iter()
            .any(|call| call == "symlink:target.txt:3:link.txt")
    );
    assert!(calls.iter().any(|call| call == "readlink:3:link.txt"));
    assert!(calls.iter().any(|call| call == "pathstat:3:0:link.txt"));
    assert!(calls.iter().any(|call| call == "pathstat:3:1:link.txt"));
    Ok(())
}

#[test]
fn quickjs_os_truncate_and_ftruncate_use_live_wasi_host() -> Result<()> {
    let (_engine, module) = quickjs_fixture()?;
    let host = LiveFileHost::with_file("resize.txt", b"abcdef".to_vec());
    let calls = host.calls();
    let files = host.files();
    let options = QuickJsCreateOptions::new().with_wasi_host(host);
    let mut vm = module.create_runtime_with_options(options)?;

    vm.eval_module_discard(
        r#"
        import * as os from "qjs:os";

        const fd = os.open("resize.txt", os.O_RDWR);
        const shrinkErr = os.ftruncate(fd, 3);
        const [, shrinkStatErr] = os.stat("resize.txt");
        const shrinkSize = os.stat("resize.txt")[0].size;
        os.close(fd);

        const growErr = os.truncate("resize.txt", 5);
        const [growStat, growStatErr] = os.stat("resize.txt");

        globalThis.truncateSummary = [
          fd >= 0,
          shrinkErr,
          shrinkStatErr,
          shrinkSize,
          growErr,
          growStatErr,
          growStat.size,
        ].join("|");
        "#,
        "stdlib-truncate.mjs",
    )?;

    assert_eq!(vm.eval_string("truncateSummary")?, "true|0|0|3|0|0|5");
    assert_eq!(
        files.lock().expect("test files lock").get("resize.txt"),
        Some(&b"abc\0\0".to_vec())
    );
    let calls = calls.lock().expect("test call lock");
    assert!(calls.iter().any(|call| call == "set_size:4:3"));
    assert!(calls.iter().any(|call| call == "set_size:5:5"));
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
