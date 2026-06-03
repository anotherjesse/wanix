use rust_wasi_quickjs::{
    QuickJsWasiDirEntry, QuickJsWasiErrno, QuickJsWasiFdStat, QuickJsWasiFileStat,
    QuickJsWasiFileType, QuickJsWasiHost, QuickJsWasiPrestat, QuickJsWasiWhence,
};
use wanix_fs::FileType;
use wanix_wasi::{
    Errno, FileStat, WasiConfig, WasiCtx, WasiFd, WasiFdStat, WasiFileType, WasiRights, WasiWhence,
};

use crate::task_context::WanixExitState;

pub(crate) struct WanixQuickJsWasiHost {
    ctx: WasiCtx,
    exit_state: Option<WanixExitState>,
}

impl WanixQuickJsWasiHost {
    pub(crate) fn new(config: WasiConfig) -> Result<Self, Errno> {
        Ok(Self {
            ctx: WasiCtx::try_new(config)?,
            exit_state: None,
        })
    }

    pub(crate) fn new_with_exit_state(
        config: WasiConfig,
        exit_state: WanixExitState,
    ) -> Result<Self, Errno> {
        Ok(Self {
            ctx: WasiCtx::try_new(config)?,
            exit_state: Some(exit_state),
        })
    }
}

impl QuickJsWasiHost for WanixQuickJsWasiHost {
    fn snapshot_blockers(&mut self) -> Result<Vec<String>, QuickJsWasiErrno> {
        let open_fds = self.ctx.open_dynamic_fd_count();
        if open_fds == 0 {
            Ok(Vec::new())
        } else {
            Ok(vec![format!("{open_fds} open dynamic WASI fd(s)")])
        }
    }

    fn args(&mut self) -> Result<Vec<String>, QuickJsWasiErrno> {
        Ok(self.ctx.args().to_vec())
    }

    fn env(&mut self) -> Result<Vec<String>, QuickJsWasiErrno> {
        Ok(self.ctx.env().to_vec())
    }

    fn proc_exit(&mut self, code: u32) -> Result<(), QuickJsWasiErrno> {
        let code = i32::try_from(code).map_err(|_| QuickJsWasiErrno::Inval)?;
        if !(0..=255).contains(&code) {
            return Err(QuickJsWasiErrno::Inval);
        }
        let Some(exit_state) = &self.exit_state else {
            return Err(QuickJsWasiErrno::Nosys);
        };
        exit_state
            .request_exit(code)
            .map_err(|_| QuickJsWasiErrno::Io)
    }

    fn fd_prestat_get(&mut self, fd: u32) -> Result<QuickJsWasiPrestat, QuickJsWasiErrno> {
        self.ctx
            .fd_prestat_get(WasiFd::new(fd))
            .map(|prestat| QuickJsWasiPrestat::new(prestat.dir_name().to_owned()))
            .map_err(convert_errno)
    }

    fn path_open(
        &mut self,
        dirfd: u32,
        _dirflags: u32,
        path: &[u8],
        oflags: u16,
        rights_base: u64,
        rights_inheriting: u64,
        fdflags: u16,
    ) -> Result<u32, QuickJsWasiErrno> {
        let path = std::str::from_utf8(path).map_err(|_| QuickJsWasiErrno::Inval)?;
        self.ctx
            .path_open_preview1(
                WasiFd::new(dirfd),
                path,
                oflags,
                WasiRights::from_preview1_bits(rights_base),
                WasiRights::from_preview1_bits(rights_inheriting),
                fdflags,
            )
            .map(WasiFd::get)
            .map_err(convert_errno)
    }

    fn fd_read(&mut self, fd: u32, buf: &mut [u8]) -> Result<usize, QuickJsWasiErrno> {
        self.ctx
            .fd_read(WasiFd::new(fd), buf)
            .map_err(convert_errno)
    }

    fn fd_readdir(&mut self, fd: u32) -> Result<Vec<QuickJsWasiDirEntry>, QuickJsWasiErrno> {
        self.ctx
            .fd_read_dir(WasiFd::new(fd))
            .map(|entries| {
                entries
                    .into_iter()
                    .map(|entry| {
                        QuickJsWasiDirEntry::new(
                            entry.name().to_owned(),
                            convert_wanix_file_type(entry.metadata().file_type()),
                        )
                    })
                    .collect()
            })
            .map_err(convert_errno)
    }

    fn fd_write(&mut self, fd: u32, buf: &[u8]) -> Result<usize, QuickJsWasiErrno> {
        self.ctx
            .fd_write(WasiFd::new(fd), buf)
            .map_err(convert_errno)
    }

    fn fd_seek(
        &mut self,
        fd: u32,
        offset: i64,
        whence: QuickJsWasiWhence,
    ) -> Result<u64, QuickJsWasiErrno> {
        self.ctx
            .fd_seek(WasiFd::new(fd), offset, convert_whence(whence))
            .map_err(convert_errno)
    }

    fn fd_tell(&mut self, fd: u32) -> Result<u64, QuickJsWasiErrno> {
        self.ctx.fd_tell(WasiFd::new(fd)).map_err(convert_errno)
    }

    fn fd_close(&mut self, fd: u32) -> Result<(), QuickJsWasiErrno> {
        self.ctx.fd_close(WasiFd::new(fd)).map_err(convert_errno)
    }

    fn fd_fdstat_get(&mut self, fd: u32) -> Result<QuickJsWasiFdStat, QuickJsWasiErrno> {
        self.ctx
            .fd_fdstat_get(WasiFd::new(fd))
            .map(convert_fdstat)
            .map_err(convert_errno)
    }

    fn fd_filestat_get(&mut self, fd: u32) -> Result<QuickJsWasiFileStat, QuickJsWasiErrno> {
        self.ctx
            .fd_filestat_get(WasiFd::new(fd))
            .map(convert_filestat)
            .map_err(convert_errno)
    }

    fn path_filestat_get(
        &mut self,
        dirfd: u32,
        _flags: u32,
        path: &[u8],
    ) -> Result<QuickJsWasiFileStat, QuickJsWasiErrno> {
        let path = std::str::from_utf8(path).map_err(|_| QuickJsWasiErrno::Inval)?;
        self.ctx
            .path_filestat_get(WasiFd::new(dirfd), path)
            .map(convert_filestat)
            .map_err(convert_errno)
    }

    fn path_create_directory(&mut self, dirfd: u32, path: &[u8]) -> Result<(), QuickJsWasiErrno> {
        let path = std::str::from_utf8(path).map_err(|_| QuickJsWasiErrno::Inval)?;
        self.ctx
            .path_create_directory(WasiFd::new(dirfd), path)
            .map_err(convert_errno)
    }

    fn path_remove_directory(&mut self, dirfd: u32, path: &[u8]) -> Result<(), QuickJsWasiErrno> {
        let path = std::str::from_utf8(path).map_err(|_| QuickJsWasiErrno::Inval)?;
        self.ctx
            .path_remove_directory(WasiFd::new(dirfd), path)
            .map_err(convert_errno)
    }

    fn path_rename(
        &mut self,
        old_fd: u32,
        old_path: &[u8],
        new_fd: u32,
        new_path: &[u8],
    ) -> Result<(), QuickJsWasiErrno> {
        let old_path = std::str::from_utf8(old_path).map_err(|_| QuickJsWasiErrno::Inval)?;
        let new_path = std::str::from_utf8(new_path).map_err(|_| QuickJsWasiErrno::Inval)?;
        self.ctx
            .path_rename(WasiFd::new(old_fd), old_path, WasiFd::new(new_fd), new_path)
            .map_err(convert_errno)
    }

    fn path_unlink_file(&mut self, dirfd: u32, path: &[u8]) -> Result<(), QuickJsWasiErrno> {
        let path = std::str::from_utf8(path).map_err(|_| QuickJsWasiErrno::Inval)?;
        self.ctx
            .path_unlink_file(WasiFd::new(dirfd), path)
            .map_err(convert_errno)
    }
}

fn convert_errno(errno: Errno) -> QuickJsWasiErrno {
    match errno {
        Errno::Success => QuickJsWasiErrno::Io,
        Errno::Badf => QuickJsWasiErrno::Badf,
        Errno::Inval => QuickJsWasiErrno::Inval,
        Errno::Nametoolong => QuickJsWasiErrno::Nametoolong,
        Errno::Noent => QuickJsWasiErrno::Noent,
        Errno::Exist => QuickJsWasiErrno::Exist,
        Errno::Notdir => QuickJsWasiErrno::Notdir,
        Errno::Isdir => QuickJsWasiErrno::Isdir,
        Errno::Notempty => QuickJsWasiErrno::Notempty,
        Errno::Nosys => QuickJsWasiErrno::Nosys,
        Errno::Notcapable => QuickJsWasiErrno::Notcapable,
        Errno::Io => QuickJsWasiErrno::Io,
    }
}

fn convert_file_type(file_type: WasiFileType) -> QuickJsWasiFileType {
    match file_type {
        WasiFileType::Unknown => QuickJsWasiFileType::Unknown,
        WasiFileType::CharacterDevice => QuickJsWasiFileType::CharacterDevice,
        WasiFileType::Directory => QuickJsWasiFileType::Directory,
        WasiFileType::RegularFile => QuickJsWasiFileType::RegularFile,
        WasiFileType::SymbolicLink => QuickJsWasiFileType::SymbolicLink,
    }
}

fn convert_wanix_file_type(file_type: FileType) -> QuickJsWasiFileType {
    match file_type {
        FileType::File => QuickJsWasiFileType::RegularFile,
        FileType::Directory => QuickJsWasiFileType::Directory,
        FileType::Symlink => QuickJsWasiFileType::SymbolicLink,
    }
}

fn convert_fdstat(stat: WasiFdStat) -> QuickJsWasiFdStat {
    QuickJsWasiFdStat::new_with_fdflags(
        convert_file_type(stat.file_type()),
        stat.fdflags(),
        stat.rights_base().bits(),
        stat.rights_inheriting().bits(),
    )
}

fn convert_filestat(stat: FileStat) -> QuickJsWasiFileStat {
    QuickJsWasiFileStat::new(convert_file_type(stat.wasi_file_type()), stat.len())
}

fn convert_whence(whence: QuickJsWasiWhence) -> WasiWhence {
    match whence {
        QuickJsWasiWhence::Set => WasiWhence::Set,
        QuickJsWasiWhence::Cur => WasiWhence::Cur,
        QuickJsWasiWhence::End => WasiWhence::End,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use rust_wasi_quickjs::{
        QuickJsWasiDirEntry, QuickJsWasiErrno, QuickJsWasiFdStat, QuickJsWasiFileStat,
        QuickJsWasiFileType, QuickJsWasiHost, QuickJsWasiPrestat, QuickJsWasiWhence,
    };
    use wanix_fs::{FileSystem, MemFs, NormalizedPath, OpenOptions};
    use wanix_vfs::{BindOptions, Namespace};
    use wanix_wasi::{WasiConfig, WasiOpenOptions, WasiRights};

    use crate::task_context::WanixExitState;

    use super::WanixQuickJsWasiHost;

    #[test]
    fn adapter_fd_write_reaches_wanix_wasi_stdout() {
        let mut namespace = Namespace::new();
        let root = Arc::new(MemFs::new());
        root.write_file("stdout", b"").unwrap();
        namespace
            .bind(root.clone(), ".", ".", BindOptions::default())
            .unwrap();
        let stdout = root
            .open(
                &NormalizedPath::new("stdout").unwrap(),
                OpenOptions {
                    write: true,
                    ..OpenOptions::default()
                },
            )
            .unwrap();
        let config = WasiConfig::new(namespace).with_stdout(stdout, "stdout");
        let mut host = WanixQuickJsWasiHost::new(config).unwrap();

        assert_eq!(host.fd_write(1, b"via wasi").unwrap(), 8);

        let mut output = root
            .open(&NormalizedPath::new("stdout").unwrap(), OpenOptions::read())
            .unwrap();
        let mut bytes = [0; 16];
        let count = output.read(&mut bytes).unwrap();
        assert_eq!(&bytes[..count], b"via wasi");
    }

    #[test]
    fn adapter_path_open_and_fd_read_reach_wanix_namespace() {
        let mut namespace = Namespace::new();
        let root = Arc::new(MemFs::new());
        root.write_file("input.txt", b"from namespace").unwrap();
        namespace
            .bind(root, ".", ".", BindOptions::default())
            .unwrap();
        let mut host = WanixQuickJsWasiHost::new(WasiConfig::new(namespace)).unwrap();

        assert_eq!(
            host.fd_prestat_get(3).unwrap(),
            QuickJsWasiPrestat::new("/")
        );
        let stat = host.path_filestat_get(3, 0, b"input.txt").unwrap();
        assert_eq!(
            stat,
            QuickJsWasiFileStat::new(QuickJsWasiFileType::RegularFile, 14)
        );

        let fd = host
            .path_open(
                3,
                0,
                b"input.txt",
                0,
                (WasiRights::FD_READ | WasiRights::FD_TELL).bits(),
                0,
                0,
            )
            .unwrap();
        assert_eq!(
            host.snapshot_blockers().unwrap(),
            vec!["1 open dynamic WASI fd(s)".to_owned()]
        );
        assert_eq!(
            host.fd_fdstat_get(fd).unwrap(),
            QuickJsWasiFdStat::new(
                QuickJsWasiFileType::RegularFile,
                (WasiRights::FD_READ | WasiRights::FD_TELL).bits(),
                0
            )
        );
        let mut buf = [0; 32];
        let count = host.fd_read(fd, &mut buf).unwrap();
        assert_eq!(&buf[..count], b"from namespace");
        assert_eq!(host.fd_tell(fd).unwrap(), 14);
        host.fd_close(fd).unwrap();
        assert!(host.snapshot_blockers().unwrap().is_empty());
    }

    #[test]
    fn adapter_path_open_create_and_fd_write_reach_wanix_namespace() {
        let mut namespace = Namespace::new();
        let root = Arc::new(MemFs::new());
        namespace
            .bind(root.clone(), ".", ".", BindOptions::default())
            .unwrap();
        let mut host = WanixQuickJsWasiHost::new(WasiConfig::new(namespace)).unwrap();

        let fd = host
            .path_open(
                3,
                0,
                b"out.txt",
                WasiOpenOptions::OFLAGS_CREATE,
                WasiRights::FD_WRITE.bits(),
                0,
                0,
            )
            .unwrap();

        assert_eq!(host.fd_write(fd, b"created").unwrap(), 7);
        assert_eq!(
            host.fd_fdstat_get(fd).unwrap(),
            QuickJsWasiFdStat::new(
                QuickJsWasiFileType::RegularFile,
                WasiRights::FD_WRITE.bits(),
                0
            )
        );
        host.fd_close(fd).unwrap();
        assert_eq!(root.read_file("out.txt").unwrap(), b"created");
    }

    #[test]
    fn adapter_path_open_append_fdflag_reaches_wanix_namespace() {
        let mut namespace = Namespace::new();
        let root = Arc::new(MemFs::new());
        root.write_file("log.txt", b"start").unwrap();
        namespace
            .bind(root.clone(), ".", ".", BindOptions::default())
            .unwrap();
        let mut host = WanixQuickJsWasiHost::new(WasiConfig::new(namespace)).unwrap();

        let fd = host
            .path_open(
                3,
                0,
                b"log.txt",
                0,
                (WasiRights::FD_WRITE | WasiRights::FD_SEEK).bits(),
                0,
                WasiOpenOptions::FDFLAGS_APPEND,
            )
            .unwrap();
        host.fd_seek(fd, 0, QuickJsWasiWhence::Set).unwrap();
        assert_eq!(host.fd_write(fd, b"-appended").unwrap(), 9);
        host.fd_close(fd).unwrap();

        assert_eq!(root.read_file("log.txt").unwrap(), b"start-appended");
    }

    #[test]
    fn adapter_path_unlink_file_reaches_wanix_namespace() {
        let mut namespace = Namespace::new();
        let root = Arc::new(MemFs::new());
        root.write_file("remove.txt", b"remove me").unwrap();
        namespace
            .bind(root.clone(), ".", ".", BindOptions::default())
            .unwrap();
        let mut host = WanixQuickJsWasiHost::new(WasiConfig::new(namespace)).unwrap();

        host.path_unlink_file(3, b"remove.txt").unwrap();

        assert_eq!(
            root.metadata(&NormalizedPath::new("remove.txt").unwrap()),
            Err(wanix_fs::FsError::NotFound)
        );
        assert_eq!(
            host.path_unlink_file(3, b"remove.txt"),
            Err(QuickJsWasiErrno::Noent)
        );
    }

    #[test]
    fn adapter_path_create_directory_reaches_wanix_namespace() {
        let mut namespace = Namespace::new();
        let root = Arc::new(MemFs::new());
        root.create_dir_all("parent").unwrap();
        namespace
            .bind(root.clone(), ".", ".", BindOptions::default())
            .unwrap();
        let mut host = WanixQuickJsWasiHost::new(WasiConfig::new(namespace)).unwrap();

        host.path_create_directory(3, b"parent/newdir").unwrap();

        assert_eq!(
            root.metadata(&NormalizedPath::new("parent/newdir").unwrap())
                .unwrap()
                .file_type(),
            wanix_fs::FileType::Directory
        );
        assert_eq!(
            host.path_create_directory(3, b"parent/newdir"),
            Err(QuickJsWasiErrno::Exist)
        );
    }

    #[test]
    fn adapter_path_remove_directory_reaches_wanix_namespace() {
        let mut namespace = Namespace::new();
        let root = Arc::new(MemFs::new());
        root.create_dir_all("empty").unwrap();
        root.write_file("nonempty/file.txt", b"file").unwrap();
        namespace
            .bind(root.clone(), ".", ".", BindOptions::default())
            .unwrap();
        let mut host = WanixQuickJsWasiHost::new(WasiConfig::new(namespace)).unwrap();

        host.path_remove_directory(3, b"empty").unwrap();

        assert_eq!(
            root.metadata(&NormalizedPath::new("empty").unwrap()),
            Err(wanix_fs::FsError::NotFound)
        );
        assert_eq!(
            host.path_remove_directory(3, b"nonempty"),
            Err(QuickJsWasiErrno::Notempty)
        );
    }

    #[test]
    fn adapter_path_rename_reaches_wanix_namespace() {
        let mut namespace = Namespace::new();
        let root = Arc::new(MemFs::new());
        root.write_file("old.txt", b"old").unwrap();
        namespace
            .bind(root.clone(), ".", ".", BindOptions::default())
            .unwrap();
        let mut host = WanixQuickJsWasiHost::new(WasiConfig::new(namespace)).unwrap();

        host.path_rename(3, b"old.txt", 3, b"renamed.txt").unwrap();

        assert_eq!(
            root.metadata(&NormalizedPath::new("old.txt").unwrap()),
            Err(wanix_fs::FsError::NotFound)
        );
        assert_eq!(root.read_file("renamed.txt").unwrap(), b"old");
        assert_eq!(
            host.path_rename(3, b"missing.txt", 3, b"missing.txt"),
            Err(QuickJsWasiErrno::Noent)
        );
    }

    #[test]
    fn adapter_process_args_and_env_come_from_wanix_wasi_ctx() {
        let mut namespace = Namespace::new();
        let root = Arc::new(MemFs::new());
        namespace
            .bind(root, ".", ".", BindOptions::default())
            .unwrap();
        let config = WasiConfig::new(namespace)
            .with_args(["main.js", "--flag"])
            .with_env(["MODE=test", "EMPTY="]);
        let mut host = WanixQuickJsWasiHost::new(config).unwrap();

        assert_eq!(host.args().unwrap(), ["main.js", "--flag"]);
        assert_eq!(host.env().unwrap(), ["MODE=test", "EMPTY="]);
    }

    #[test]
    fn adapter_proc_exit_updates_wanix_exit_state() {
        let mut namespace = Namespace::new();
        let root = Arc::new(MemFs::new());
        namespace
            .bind(root, ".", ".", BindOptions::default())
            .unwrap();
        let exit_state = WanixExitState::default();
        let mut host = WanixQuickJsWasiHost::new_with_exit_state(
            WasiConfig::new(namespace),
            exit_state.clone(),
        )
        .unwrap();

        host.proc_exit(9).unwrap();
        host.proc_exit(10).unwrap();

        assert_eq!(exit_state.code().unwrap(), Some(9));
    }

    #[test]
    fn adapter_proc_exit_without_exit_state_is_unsupported() {
        let mut namespace = Namespace::new();
        let root = Arc::new(MemFs::new());
        namespace
            .bind(root, ".", ".", BindOptions::default())
            .unwrap();
        let mut host = WanixQuickJsWasiHost::new(WasiConfig::new(namespace)).unwrap();

        assert_eq!(host.proc_exit(1), Err(QuickJsWasiErrno::Nosys));
    }

    #[test]
    fn adapter_proc_exit_rejects_out_of_range_status() {
        let mut namespace = Namespace::new();
        let root = Arc::new(MemFs::new());
        namespace
            .bind(root, ".", ".", BindOptions::default())
            .unwrap();
        let exit_state = WanixExitState::default();
        let mut host =
            WanixQuickJsWasiHost::new_with_exit_state(WasiConfig::new(namespace), exit_state)
                .unwrap();

        assert_eq!(host.proc_exit(256), Err(QuickJsWasiErrno::Inval));
    }

    #[test]
    fn adapter_extra_preopen_metadata_and_reads_come_from_wanix_wasi_ctx() {
        let mut namespace = Namespace::new();
        let root = Arc::new(MemFs::new());
        root.write_file("mnt/extra.txt", b"from extra preopen")
            .unwrap();
        namespace
            .bind(root, ".", ".", BindOptions::default())
            .unwrap();
        let config = WasiConfig::new(namespace).with_preopen("mnt").unwrap();
        let mut host = WanixQuickJsWasiHost::new(config).unwrap();

        assert_eq!(
            host.fd_prestat_get(4).unwrap(),
            QuickJsWasiPrestat::new("/mnt")
        );
        assert_eq!(
            host.fd_readdir(4).unwrap(),
            vec![QuickJsWasiDirEntry::new(
                "extra.txt",
                QuickJsWasiFileType::RegularFile
            )]
        );

        let fd = host
            .path_open(4, 0, b"extra.txt", 0, WasiRights::FD_READ.bits(), 0, 0)
            .unwrap();
        let mut buf = [0; 32];
        let count = host.fd_read(fd, &mut buf).unwrap();
        assert_eq!(&buf[..count], b"from extra preopen");
        host.fd_close(fd).unwrap();
    }
}
