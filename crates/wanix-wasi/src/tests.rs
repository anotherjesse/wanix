use std::sync::Arc;

use super::{
    CRATE_PURPOSE, Errno, Preopen, WasiConfig, WasiCtx, WasiFd, WasiFileType, WasiOpenOptions,
    WasiRights, WasiWhence,
};
use wanix_fs::{FileSystem, FileType, FsError, MemFs, NormalizedPath, OpenOptions};
use wanix_task::TaskTable;
use wanix_vfs::{BindOptions, Namespace};

fn fixture(entries: &[(&str, &[u8])]) -> Arc<MemFs> {
    let fs = Arc::new(MemFs::new());
    for (path, data) in entries {
        fs.write_file(path, data).unwrap();
    }
    fs
}

fn namespace_with_root(root: Arc<dyn FileSystem>) -> Namespace {
    let mut namespace = Namespace::new();
    namespace
        .bind(root, ".", ".", BindOptions::default())
        .unwrap();
    namespace
}

#[test]
fn purpose_is_declared() {
    assert!(!CRATE_PURPOSE.is_empty());
}

#[test]
fn config_exposes_namespace_and_root_preopen() {
    let config = WasiConfig::new(Namespace::new());

    assert_eq!(config.preopens()[0].guest_path().as_str(), ".");
    assert!(config.namespace().bindings().is_empty());
    assert!(config.stdio_fds().is_empty());
    assert_eq!(Errno::Success, Errno::Success);
}

#[test]
fn preopen_validates_guest_paths() {
    assert!(Preopen::new("root").is_ok());
    assert!(Preopen::new("/host").is_err());
}

#[test]
fn root_preopen_stats_and_lists_namespace() {
    let root = fixture(&[("hello.txt", b"hello"), ("dir/nested.txt", b"nested")]);
    let ctx = WasiCtx::new(WasiConfig::new(namespace_with_root(root)));

    let prestat = ctx.fd_prestat_get(WasiFd::ROOT).unwrap();
    assert_eq!(prestat.dir_name(), "/");
    assert_eq!(prestat.dir_name_len(), 1);
    assert_eq!(prestat.to_preview1_bytes(), [0, 0, 0, 0, 1, 0, 0, 0]);
    let mut preopen_name = [0; 1];
    assert_eq!(
        ctx.fd_prestat_dir_name(WasiFd::ROOT, &mut preopen_name)
            .unwrap(),
        1
    );
    assert_eq!(&preopen_name, b"/");
    let stat = ctx.fd_filestat_get(WasiFd::ROOT).unwrap();
    assert_eq!(stat.file_type(), FileType::Directory);
    assert_eq!(stat.wasi_file_type(), WasiFileType::Directory);
    assert_eq!(
        ctx.fd_read_dir(WasiFd::ROOT)
            .unwrap()
            .into_iter()
            .map(|entry| entry.name().to_owned())
            .collect::<Vec<_>>(),
        ["dir", "hello.txt"]
    );
    assert_eq!(
        ctx.path_read_dir(WasiFd::ROOT, "dir")
            .unwrap()
            .into_iter()
            .map(|entry| entry.name().to_owned())
            .collect::<Vec<_>>(),
        ["nested.txt"]
    );
}

#[test]
fn multiple_preopens_are_validated_and_shift_dynamic_fds() {
    let root = fixture(&[("mnt/inner.txt", b"inner")]);
    let config = WasiConfig::new(namespace_with_root(root))
        .with_preopen("mnt")
        .unwrap();
    let mut ctx = WasiCtx::try_new(config).unwrap();

    let fd = ctx
        .path_open(WasiFd::new(4), "inner.txt", WasiOpenOptions::read())
        .unwrap();
    let mut buf = [0; 8];
    let count = ctx.fd_read(fd, &mut buf).unwrap();

    assert_eq!(fd.get(), 5);
    assert_eq!(&buf[..count], b"inner");
    assert_eq!(
        ctx.fd_prestat_get(WasiFd::new(4)).unwrap().dir_name(),
        "/mnt"
    );
    let mut name = [0; 4];
    assert_eq!(
        ctx.fd_prestat_dir_name(WasiFd::new(4), &mut name).unwrap(),
        4
    );
    assert_eq!(&name, b"/mnt");
    assert_eq!(
        ctx.fd_prestat_dir_name(WasiFd::new(4), &mut [0; 3]),
        Err(Errno::Inval)
    );
    assert_eq!(ctx.fd_close(WasiFd::new(4)), Err(Errno::Badf));
}

#[test]
fn preopens_must_exist_and_be_directories() {
    let missing = WasiConfig::new(namespace_with_root(fixture(&[])))
        .with_preopen("missing")
        .unwrap();
    assert_eq!(WasiCtx::try_new(missing).unwrap_err(), Errno::Noent);

    let file = WasiConfig::new(namespace_with_root(fixture(&[("file.txt", b"file")])))
        .with_preopen("file.txt")
        .unwrap();
    assert_eq!(WasiCtx::try_new(file).unwrap_err(), Errno::Notdir);
}

#[test]
fn path_open_and_fd_read_use_wanix_namespace() {
    let root = fixture(&[("hello.txt", b"hello")]);
    let mut ctx = WasiCtx::new(WasiConfig::new(namespace_with_root(root)));

    let fd = ctx
        .path_open(WasiFd::ROOT, "hello.txt", WasiOpenOptions::read())
        .unwrap();
    let mut buf = [0; 8];
    let count = ctx.fd_read(fd, &mut buf).unwrap();

    assert_eq!(fd.get(), 4);
    assert_eq!(&buf[..count], b"hello");
    assert_eq!(ctx.fd_filestat_get(fd).unwrap().file_type(), FileType::File);
}

#[test]
fn unconfigured_standard_fds_are_closed() {
    let root = fixture(&[]);
    let mut ctx = WasiCtx::new(WasiConfig::new(namespace_with_root(root)));

    assert_eq!(ctx.fd_read(WasiFd::STDIN, &mut [0; 1]), Err(Errno::Badf));
    assert_eq!(ctx.fd_write(WasiFd::STDOUT, b"out"), Err(Errno::Badf));
    assert_eq!(ctx.fd_write(WasiFd::STDERR, b"err"), Err(Errno::Badf));
    assert_eq!(ctx.fd_filestat_get(WasiFd::STDOUT), Err(Errno::Badf));
}

#[test]
fn configured_standard_fds_read_write_and_remain_non_closeable() {
    let root = fixture(&[("hello.txt", b"hello")]);
    let stdin = fixture(&[("stdin", b"input")]);
    let stdout = fixture(&[("stdout", b"")]);
    let stderr = fixture(&[("stderr", b"")]);
    let config = WasiConfig::new(namespace_with_root(root))
        .with_stdin(
            stdin
                .open(&NormalizedPath::new("stdin").unwrap(), OpenOptions::read())
                .unwrap(),
            "stdin",
        )
        .with_stdout(
            stdout
                .open(
                    &NormalizedPath::new("stdout").unwrap(),
                    OpenOptions::read_write(),
                )
                .unwrap(),
            "stdout",
        )
        .with_stderr(
            stderr
                .open(
                    &NormalizedPath::new("stderr").unwrap(),
                    OpenOptions::read_write(),
                )
                .unwrap(),
            "stderr",
        );
    assert_eq!(
        config.stdio_fds(),
        [WasiFd::STDIN, WasiFd::STDOUT, WasiFd::STDERR]
    );
    let mut ctx = WasiCtx::new(config);

    let mut buf = [0; 8];
    let count = ctx.fd_read(WasiFd::STDIN, &mut buf).unwrap();
    assert_eq!(&buf[..count], b"input");
    assert_eq!(ctx.fd_write(WasiFd::STDOUT, b"out").unwrap(), 3);
    assert_eq!(ctx.fd_write(WasiFd::STDERR, b"err").unwrap(), 3);
    assert_eq!(
        ctx.fd_filestat_get(WasiFd::STDOUT).unwrap().file_type(),
        FileType::File
    );
    let stdin_stat = ctx.fd_fdstat_get(WasiFd::STDIN).unwrap();
    assert_eq!(stdin_stat.file_type(), WasiFileType::CharacterDevice);
    assert!(stdin_stat.rights_base().contains(WasiRights::FD_READ));
    assert!(!stdin_stat.rights_base().contains(WasiRights::FD_WRITE));
    assert_eq!(stdin_stat.rights_inheriting(), WasiRights::NONE);
    let stdout_stat = ctx.fd_fdstat_get(WasiFd::STDOUT).unwrap();
    assert_eq!(stdout_stat.file_type(), WasiFileType::CharacterDevice);
    assert!(stdout_stat.rights_base().contains(WasiRights::FD_WRITE));
    assert!(!stdout_stat.rights_base().contains(WasiRights::FD_READ));
    let file_fd = ctx
        .path_open(WasiFd::ROOT, "hello.txt", WasiOpenOptions::read())
        .unwrap();

    assert_eq!(file_fd.get(), 4);
    assert_eq!(stdout.read_file("stdout").unwrap(), b"out");
    assert_eq!(stderr.read_file("stderr").unwrap(), b"err");
    assert_eq!(ctx.fd_write(WasiFd::STDIN, b"nope"), Err(Errno::Notcapable));
    assert_eq!(
        ctx.fd_read(WasiFd::STDOUT, &mut buf),
        Err(Errno::Notcapable)
    );
    assert_eq!(ctx.fd_close(WasiFd::STDOUT), Err(Errno::Badf));
    assert_eq!(ctx.fd_read_dir(WasiFd::STDIN), Err(Errno::Notdir));
}

#[test]
fn task_service_paths_are_reachable_through_wasi_namespace() {
    let table = TaskTable::new();
    table.register_noop_driver("qjs").unwrap();
    let task = table.allocate_root("qjs").unwrap();
    let mut ctx = WasiCtx::new(WasiConfig::new(task.namespace()));

    let fd = ctx
        .path_open(WasiFd::ROOT, "#task/self/id", WasiOpenOptions::read())
        .unwrap();
    let mut buf = [0; 8];
    let count = ctx.fd_read(fd, &mut buf).unwrap();

    assert_eq!(&buf[..count], b"1\n");
}

#[test]
fn writes_and_creates_flow_back_to_namespace() {
    let root = Arc::new(MemFs::new());
    let mut ctx = WasiCtx::new(WasiConfig::new(namespace_with_root(root.clone())));

    let fd = ctx
        .path_open(WasiFd::ROOT, "out.txt", WasiOpenOptions::create_write())
        .unwrap();
    assert_eq!(ctx.fd_write(fd, b"created").unwrap(), 7);

    assert_eq!(root.read_file("out.txt").unwrap(), b"created");
    assert_eq!(
        ctx.path_filestat_get(WasiFd::ROOT, "out.txt")
            .unwrap()
            .len(),
        7
    );
    let fdstat = ctx.fd_fdstat_get(fd).unwrap();
    assert_eq!(fdstat.file_type(), WasiFileType::RegularFile);
    assert!(fdstat.rights_base().contains(WasiRights::FD_WRITE));
    assert!(!fdstat.rights_base().contains(WasiRights::FD_READ));
    assert_eq!(fdstat.rights_inheriting(), WasiRights::NONE);
}

#[test]
fn rights_are_enforced_on_open_fds() {
    let root = fixture(&[("read.txt", b"read"), ("write.txt", b"")]);
    let mut ctx = WasiCtx::new(WasiConfig::new(namespace_with_root(root)));

    let read_fd = ctx
        .path_open(WasiFd::ROOT, "read.txt", WasiOpenOptions::read())
        .unwrap();
    assert_eq!(ctx.fd_write(read_fd, b"nope"), Err(Errno::Notcapable));

    let write_fd = ctx
        .path_open(
            WasiFd::ROOT,
            "write.txt",
            WasiOpenOptions {
                read: false,
                write: true,
                create: false,
                truncate: false,
            },
        )
        .unwrap();
    assert_eq!(ctx.fd_read(write_fd, &mut [0; 4]), Err(Errno::Notcapable));
}

#[test]
fn directories_can_be_opened_and_read_but_not_written() {
    let root = fixture(&[("dir/file.txt", b"file")]);
    let mut ctx = WasiCtx::new(WasiConfig::new(namespace_with_root(root)));

    let dir_fd = ctx
        .path_open(WasiFd::ROOT, "dir", WasiOpenOptions::read())
        .unwrap();

    assert_eq!(
        ctx.fd_read_dir(dir_fd)
            .unwrap()
            .into_iter()
            .map(|entry| entry.name().to_owned())
            .collect::<Vec<_>>(),
        ["file.txt"]
    );
    let fdstat = ctx.fd_fdstat_get(dir_fd).unwrap();
    assert_eq!(fdstat.file_type(), WasiFileType::Directory);
    assert!(fdstat.rights_base().contains(WasiRights::PATH_OPEN));
    assert!(fdstat.rights_base().contains(WasiRights::FD_READDIR));
    assert!(
        fdstat
            .rights_inheriting()
            .contains(WasiRights::FD_FILESTAT_GET)
    );
    assert_eq!(ctx.fd_write(dir_fd, b"nope"), Err(Errno::Isdir));
    assert_eq!(
        ctx.path_open(WasiFd::ROOT, "dir", WasiOpenOptions::create_write()),
        Err(Errno::Isdir)
    );
}

#[test]
fn readdir_inherits_namespace_union_synthesis_and_hidden_filtering() {
    let lower = fixture(&[("visible.txt", b"visible"), ("#hidden", b"hidden")]);
    let upper = fixture(&[("other.txt", b"other")]);
    let mut namespace = namespace_with_root(lower.clone());
    namespace
        .bind(upper, ".", ".", BindOptions::default())
        .unwrap();
    namespace
        .bind(
            lower,
            "visible.txt",
            "deep/path/visible.txt",
            BindOptions::default(),
        )
        .unwrap();
    let mut ctx = WasiCtx::new(WasiConfig::new(namespace));

    assert_eq!(
        ctx.fd_read_dir(WasiFd::ROOT)
            .unwrap()
            .into_iter()
            .map(|entry| entry.name().to_owned())
            .collect::<Vec<_>>(),
        ["deep", "other.txt", "visible.txt"]
    );
    assert_eq!(
        ctx.path_read_dir(WasiFd::ROOT, "deep/path")
            .unwrap()
            .into_iter()
            .map(|entry| entry.name().to_owned())
            .collect::<Vec<_>>(),
        ["visible.txt"]
    );
    let fd = ctx
        .path_open(WasiFd::ROOT, "#hidden", WasiOpenOptions::read())
        .unwrap();
    let mut buf = [0; 8];
    let count = ctx.fd_read(fd, &mut buf).unwrap();
    assert_eq!(&buf[..count], b"hidden");
}

#[test]
fn close_only_accepts_dynamic_fds() {
    let root = fixture(&[("hello.txt", b"hello")]);
    let mut ctx = WasiCtx::new(WasiConfig::new(namespace_with_root(root)));
    let fd = ctx
        .path_open(WasiFd::ROOT, "hello.txt", WasiOpenOptions::read())
        .unwrap();

    assert_eq!(ctx.fd_close(WasiFd::ROOT), Err(Errno::Badf));
    ctx.fd_close(fd).unwrap();
    assert_eq!(ctx.fd_read(fd, &mut [0; 1]), Err(Errno::Badf));
}

#[test]
fn fdstat_reports_preopen_and_regular_file_rights() {
    let root = fixture(&[("read.txt", b"read"), ("write.txt", b"")]);
    let mut ctx = WasiCtx::new(WasiConfig::new(namespace_with_root(root)));

    let root_stat = ctx.fd_fdstat_get(WasiFd::ROOT).unwrap();
    assert_eq!(root_stat.file_type(), WasiFileType::Directory);
    assert!(root_stat.rights_base().contains(WasiRights::PATH_OPEN));
    assert!(
        root_stat
            .rights_base()
            .contains(WasiRights::PATH_FILESTAT_GET)
    );
    assert!(
        root_stat
            .rights_base()
            .contains(WasiRights::FD_FILESTAT_GET)
    );
    assert!(
        root_stat
            .rights_base()
            .contains(WasiRights::PATH_CREATE_FILE)
    );
    assert!(
        root_stat
            .rights_base()
            .contains(WasiRights::PATH_FILESTAT_SET_SIZE)
    );
    assert!(root_stat.rights_base().contains(WasiRights::FD_READDIR));
    assert!(root_stat.rights_inheriting().contains(WasiRights::FD_READ));
    assert!(root_stat.rights_inheriting().contains(WasiRights::FD_WRITE));
    assert!(
        root_stat
            .rights_inheriting()
            .contains(WasiRights::PATH_OPEN)
    );
    assert!(
        root_stat
            .rights_inheriting()
            .contains(WasiRights::FD_READDIR)
    );
    assert!(!root_stat.rights_inheriting().contains(WasiRights::FD_SEEK));
    assert!(!root_stat.rights_inheriting().contains(WasiRights::FD_TELL));

    let read_fd = ctx
        .path_open(WasiFd::ROOT, "read.txt", WasiOpenOptions::read())
        .unwrap();
    let read_stat = ctx.fd_fdstat_get(read_fd).unwrap();
    assert_eq!(read_stat.file_type(), WasiFileType::RegularFile);
    assert!(read_stat.rights_base().contains(WasiRights::FD_READ));
    assert!(!read_stat.rights_base().contains(WasiRights::FD_WRITE));
    let read_stat_bytes = read_stat.to_preview1_bytes();
    assert_eq!(
        read_stat_bytes[0],
        WasiFileType::RegularFile.preview1_code()
    );
    assert_eq!(
        u64::from_le_bytes(read_stat_bytes[8..16].try_into().unwrap()),
        read_stat.rights_base().bits()
    );
    assert_eq!(
        u64::from_le_bytes(read_stat_bytes[16..24].try_into().unwrap()),
        0
    );

    let write_fd = ctx
        .path_open(
            WasiFd::ROOT,
            "write.txt",
            WasiOpenOptions {
                read: false,
                write: true,
                create: false,
                truncate: false,
            },
        )
        .unwrap();
    let write_stat = ctx.fd_fdstat_get(write_fd).unwrap();
    assert!(write_stat.rights_base().contains(WasiRights::FD_WRITE));
    assert!(!write_stat.rights_base().contains(WasiRights::FD_READ));

    assert_eq!(ctx.fd_fdstat_get(WasiFd::new(99)), Err(Errno::Badf));
    assert_eq!(ctx.fd_prestat_get(read_fd), Err(Errno::Badf));
}

#[test]
fn fd_seek_is_explicitly_unsupported_until_files_are_seekable() {
    let root = fixture(&[("hello.txt", b"hello")]);
    let mut ctx = WasiCtx::new(WasiConfig::new(namespace_with_root(root)));
    let fd = ctx
        .path_open(WasiFd::ROOT, "hello.txt", WasiOpenOptions::read())
        .unwrap();

    assert_eq!(WasiWhence::from_preview1(0).unwrap(), WasiWhence::Set);
    assert_eq!(WasiWhence::from_preview1(1).unwrap(), WasiWhence::Cur);
    assert_eq!(WasiWhence::from_preview1(2).unwrap(), WasiWhence::End);
    assert_eq!(WasiWhence::from_preview1(3), Err(Errno::Inval));
    assert_eq!(ctx.fd_seek(fd, 0, WasiWhence::Set), Err(Errno::Nosys));
    assert_eq!(
        ctx.fd_seek(WasiFd::new(99), 0, WasiWhence::Set),
        Err(Errno::Badf)
    );
}

#[test]
fn path_errors_map_to_wasi_errno() {
    let root = fixture(&[]);
    let mut ctx = WasiCtx::new(WasiConfig::new(namespace_with_root(root)));

    assert_eq!(
        ctx.path_open(WasiFd::ROOT, "../bad", WasiOpenOptions::read()),
        Err(Errno::Notcapable)
    );
    assert_eq!(
        ctx.path_open(WasiFd::ROOT, "/abs", WasiOpenOptions::read()),
        Err(Errno::Notcapable)
    );
    assert_eq!(
        ctx.path_open(WasiFd::ROOT, "bad//path", WasiOpenOptions::read()),
        Err(Errno::Notcapable)
    );
    assert_eq!(
        ctx.path_open(WasiFd::ROOT, "bad\\path", WasiOpenOptions::read()),
        Err(Errno::Notcapable)
    );
    assert_eq!(
        ctx.path_open(
            WasiFd::ROOT,
            format!("{}x", "a".repeat(4096)),
            WasiOpenOptions::read()
        ),
        Err(Errno::Nametoolong)
    );
    assert_eq!(
        ctx.path_open(WasiFd::ROOT, "missing", WasiOpenOptions::read()),
        Err(Errno::Noent)
    );
    assert_eq!(
        ctx.path_filestat_get(WasiFd::new(99), "."),
        Err(Errno::Badf)
    );
}

#[test]
fn errno_mapping_is_pinned_for_filesystem_errors() {
    assert_eq!(Errno::from(FsError::InvalidPath("x".into())), Errno::Inval);
    assert_eq!(Errno::from(FsError::NotFound), Errno::Noent);
    assert_eq!(Errno::from(FsError::NotSupported), Errno::Nosys);
    assert_eq!(Errno::from(FsError::PermissionDenied), Errno::Notcapable);
    assert_eq!(Errno::from(FsError::AlreadyExists), Errno::Exist);
    assert_eq!(Errno::from(FsError::NotDirectory), Errno::Notdir);
    assert_eq!(Errno::from(FsError::IsDirectory), Errno::Isdir);
    assert_eq!(Errno::from(FsError::InvalidFd), Errno::Badf);
    assert_eq!(Errno::from(FsError::NotEmpty), Errno::Io);
    assert_eq!(Errno::from(FsError::Other("opaque".into())), Errno::Io);
}

#[test]
fn preview1_numeric_codes_are_pinned_for_import_wrappers() {
    assert_eq!(Errno::Success.preview1_code(), 0);
    assert_eq!(Errno::Success.preview1_result(), 0);
    assert_eq!(Errno::Badf.preview1_code(), 8);
    assert_eq!(Errno::Exist.preview1_code(), 20);
    assert_eq!(Errno::Inval.preview1_code(), 28);
    assert_eq!(Errno::Io.preview1_code(), 29);
    assert_eq!(Errno::Isdir.preview1_code(), 31);
    assert_eq!(Errno::Nametoolong.preview1_code(), 37);
    assert_eq!(Errno::Noent.preview1_code(), 44);
    assert_eq!(Errno::Nosys.preview1_code(), 52);
    assert_eq!(Errno::Notdir.preview1_code(), 54);
    assert_eq!(Errno::Notcapable.preview1_code(), 76);

    assert_eq!(WasiFileType::Unknown.preview1_code(), 0);
    assert_eq!(WasiFileType::CharacterDevice.preview1_code(), 2);
    assert_eq!(WasiFileType::Directory.preview1_code(), 3);
    assert_eq!(WasiFileType::RegularFile.preview1_code(), 4);
    assert_eq!(WasiFileType::SymbolicLink.preview1_code(), 7);
    assert_eq!(WasiRights::PATH_CREATE_FILE.bits(), 1 << 10);
    assert_eq!(WasiRights::PATH_OPEN.bits(), 1 << 13);
    assert_eq!(WasiRights::FD_READDIR.bits(), 1 << 14);
    assert_eq!(WasiRights::PATH_FILESTAT_SET_SIZE.bits(), 1 << 19);
    assert_eq!(WasiRights::FD_FILESTAT_GET.bits(), 1 << 21);
}
