use rust_wasi_quickjs::{
    QuickJsWasiErrno, QuickJsWasiFdStat, QuickJsWasiFileStat, QuickJsWasiFileType, QuickJsWasiHost,
    QuickJsWasiPrestat, QuickJsWasiWhence,
};
use wanix_wasi::{
    Errno, FileStat, WasiConfig, WasiCtx, WasiFd, WasiFdStat, WasiFileType, WasiRights, WasiWhence,
};

pub(crate) struct WanixQuickJsWasiHost {
    ctx: WasiCtx,
}

impl WanixQuickJsWasiHost {
    pub(crate) fn new(config: WasiConfig) -> Result<Self, Errno> {
        Ok(Self {
            ctx: WasiCtx::try_new(config)?,
        })
    }
}

impl QuickJsWasiHost for WanixQuickJsWasiHost {
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

fn convert_fdstat(stat: WasiFdStat) -> QuickJsWasiFdStat {
    QuickJsWasiFdStat::new(
        convert_file_type(stat.file_type()),
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
        QuickJsWasiFdStat, QuickJsWasiFileStat, QuickJsWasiFileType, QuickJsWasiHost,
        QuickJsWasiPrestat,
    };
    use wanix_fs::{FileSystem, MemFs, NormalizedPath, OpenOptions};
    use wanix_vfs::{BindOptions, Namespace};
    use wanix_wasi::{WasiConfig, WasiRights};

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
            .path_open(3, 0, b"input.txt", 0, WasiRights::FD_READ.bits(), 0, 0)
            .unwrap();
        assert_eq!(
            host.fd_fdstat_get(fd).unwrap(),
            QuickJsWasiFdStat::new(
                QuickJsWasiFileType::RegularFile,
                WasiRights::FD_READ.bits(),
                0
            )
        );
        let mut buf = [0; 32];
        let count = host.fd_read(fd, &mut buf).unwrap();
        assert_eq!(&buf[..count], b"from namespace");
        host.fd_close(fd).unwrap();
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

        let fd = host
            .path_open(4, 0, b"extra.txt", 0, WasiRights::FD_READ.bits(), 0, 0)
            .unwrap();
        let mut buf = [0; 32];
        let count = host.fd_read(fd, &mut buf).unwrap();
        assert_eq!(&buf[..count], b"from extra preopen");
        host.fd_close(fd).unwrap();
    }
}
