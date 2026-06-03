use std::sync::Arc;

use super::{CRATE_PURPOSE, Errno, Preopen, WasiConfig, WasiCtx, WasiFd, WasiOpenOptions};
use wanix_fs::{FileSystem, FileType, FsError, MemFs};
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

    let stat = ctx.fd_filestat_get(WasiFd::ROOT).unwrap();
    assert_eq!(stat.file_type(), FileType::Directory);
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
