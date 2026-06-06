use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use super::{
    CRATE_PURPOSE, DEFAULT_CLOCK_TIME_NS, Errno, FileStat, Preopen, WasiConfig, WasiCtx, WasiFd,
    WasiFdObserver, WasiFile, WasiFileType, WasiFilestatSetTimes, WasiLookupFlags, WasiOpenOptions,
    WasiPathOpen, WasiRights, WasiWhence,
};
use wanix_fs::{
    File, FileSystem, FileType, FsError, FsResult, LocalFs, MemFs, Metadata, NormalizedPath,
    OpenOptions,
};
use wanix_task::TaskTable;
use wanix_vfs::{BindOptions, Namespace};

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

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

fn path(value: &str) -> NormalizedPath {
    NormalizedPath::new(value).unwrap()
}

fn wasi_ctx(config: WasiConfig) -> WasiCtx {
    WasiCtx::try_new(config).unwrap()
}

fn temp_host_dir(label: &str) -> std::path::PathBuf {
    let mut path = std::env::temp_dir();
    let nonce = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
    path.push(format!("wanix-wasi-{label}-{}-{nonce}", std::process::id()));
    std::fs::create_dir_all(&path).unwrap();
    path
}

#[derive(Debug)]
struct ReadinessFile {
    read_ready: bool,
    write_ready: bool,
}

impl File for ReadinessFile {
    fn read(&mut self, _buf: &mut [u8]) -> FsResult<usize> {
        Ok(0)
    }

    fn write(&mut self, buf: &[u8]) -> FsResult<usize> {
        Ok(buf.len())
    }

    fn read_ready(&self) -> FsResult<bool> {
        Ok(self.read_ready)
    }

    fn write_ready(&self) -> FsResult<bool> {
        Ok(self.write_ready)
    }

    fn metadata(&self) -> FsResult<Metadata> {
        Ok(Metadata::new(FileType::File, 0, 0o666))
    }
}

#[test]
fn purpose_is_declared() {
    assert!(!CRATE_PURPOSE.is_empty());
}

#[test]
fn config_exposes_namespace_and_root_preopen() {
    let config = WasiConfig::new(Namespace::new());

    assert_eq!(config.preopens()[0].source_path().as_str(), ".");
    assert_eq!(config.preopens()[0].guest_path().as_str(), ".");
    assert!(config.namespace().bindings().is_empty());
    assert!(config.stdio_fds().is_empty());
    assert!(config.args().is_empty());
    assert!(config.env().is_empty());
    assert_eq!(config.clock_time_ns(), DEFAULT_CLOCK_TIME_NS);
    assert_eq!(Errno::Success, Errno::Success);
}

#[test]
fn config_and_ctx_expose_process_args_and_env() {
    let config = WasiConfig::new(Namespace::new())
        .with_args(["main.js", "--flag"])
        .with_env(["MODE=test", "EMPTY="])
        .with_clock_time_ns(9_000_000_000);

    assert_eq!(config.args(), ["main.js", "--flag"]);
    assert_eq!(config.env(), ["MODE=test", "EMPTY="]);
    assert_eq!(config.clock_time_ns(), 9_000_000_000);
    let debug = format!("{config:?}");
    assert!(debug.contains("arg_count"));
    assert!(debug.contains("env_count"));
    assert!(debug.contains("clock_time_ns"));
    assert!(!debug.contains("main.js"));
    assert!(!debug.contains("MODE=test"));

    let ctx = wasi_ctx(config);

    assert_eq!(ctx.args(), ["main.js", "--flag"]);
    assert_eq!(ctx.env(), ["MODE=test", "EMPTY="]);
}

#[test]
fn ctx_debug_summarizes_handle_kinds_without_dynamic_file_internals() {
    let root = fixture(&[("input.txt", b"hello"), ("dir/nested.txt", b"nested")]);
    let config = WasiConfig::new(namespace_with_root(root))
        .with_stdin(
            Box::new(ReadinessFile {
                read_ready: true,
                write_ready: false,
            }),
            "stdin-debug-label",
        )
        .with_args(["main.js", "--secret-arg"])
        .with_env(["MODE=secret-env"])
        .with_preopen("dir")
        .unwrap();
    let mut ctx = wasi_ctx(config);

    let dir_fd = ctx
        .path_open(WasiFd::ROOT, "dir", WasiOpenOptions::read())
        .unwrap();
    let file_fd = ctx
        .path_open(WasiFd::ROOT, "input.txt", WasiOpenOptions::read_write())
        .unwrap();

    assert!(dir_fd.get() > WasiFd::ROOT.get());
    assert!(file_fd.get() > dir_fd.get());
    let debug = format!("{ctx:?}");
    assert!(debug.contains("Stdio"));
    assert!(debug.contains("stdin-debug-label"));
    assert!(debug.contains("Preopen"));
    assert!(debug.contains("Directory"));
    assert!(debug.contains("path: NormalizedPath(\"dir\")"));
    assert!(debug.contains("File"));
    assert!(debug.contains("path: NormalizedPath(\"input.txt\")"));
    assert!(debug.contains("read: true"));
    assert!(debug.contains("write: true"));
    assert!(debug.contains("arg_count"));
    assert!(debug.contains("env_count"));
    assert!(!debug.contains("main.js"));
    assert!(!debug.contains("--secret-arg"));
    assert!(!debug.contains("MODE=secret-env"));
    assert_eq!(debug.matches("file: WasiFile").count(), 1, "{debug}");
}

#[test]
fn preopen_validates_guest_paths() {
    assert!(Preopen::new("root").is_ok());
    assert_eq!(
        Preopen::mapped("app", ".").unwrap().source_path().as_str(),
        "app"
    );
    assert!(Preopen::new("/host").is_err());
    assert!(Preopen::mapped("/host", ".").is_err());
    assert!(Preopen::mapped("app", "/guest").is_err());
}

#[test]
fn root_preopen_stats_and_lists_namespace() {
    let root = fixture(&[("hello.txt", b"hello"), ("dir/nested.txt", b"nested")]);
    let ctx = wasi_ctx(WasiConfig::new(namespace_with_root(root)));

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
fn root_preopen_can_map_guest_root_to_namespace_subdirectory() {
    let root = fixture(&[
        ("root.txt", b"from root"),
        ("app/main.js", b"from app"),
        ("app/nested.txt", b"nested"),
    ]);
    let mut ctx = wasi_ctx(
        WasiConfig::new(namespace_with_root(root))
            .with_root_preopen_source(NormalizedPath::new("app").unwrap()),
    );

    assert_eq!(ctx.fd_prestat_get(WasiFd::ROOT).unwrap().dir_name(), "/");
    assert_eq!(
        ctx.fd_read_dir(WasiFd::ROOT)
            .unwrap()
            .into_iter()
            .map(|entry| entry.name().to_owned())
            .collect::<Vec<_>>(),
        ["main.js", "nested.txt"]
    );
    let fd = ctx
        .path_open(WasiFd::ROOT, "main.js", WasiOpenOptions::read())
        .unwrap();
    let mut buf = [0; 16];
    let count = ctx.fd_read(fd, &mut buf).unwrap();

    assert_eq!(&buf[..count], b"from app");
    assert_eq!(
        ctx.path_open(WasiFd::ROOT, "root.txt", WasiOpenOptions::read()),
        Err(Errno::Noent)
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
    let mut ctx = wasi_ctx(WasiConfig::new(namespace_with_root(root)));

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
    let mut ctx = wasi_ctx(WasiConfig::new(namespace_with_root(root)));

    assert_eq!(ctx.fd_read(WasiFd::STDIN, &mut [0; 1]), Err(Errno::Badf));
    assert_eq!(ctx.fd_write(WasiFd::STDOUT, b"out"), Err(Errno::Badf));
    assert_eq!(ctx.fd_read_ready(WasiFd::STDIN), Err(Errno::Badf));
    assert_eq!(ctx.fd_write_ready(WasiFd::STDOUT), Err(Errno::Badf));
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
    let mut ctx = wasi_ctx(config);

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
    assert!(stdin_stat.rights_base().contains(WasiRights::FD_SEEK));
    assert!(stdin_stat.rights_base().contains(WasiRights::FD_TELL));
    assert_eq!(stdin_stat.rights_inheriting(), WasiRights::NONE);
    let stdout_stat = ctx.fd_fdstat_get(WasiFd::STDOUT).unwrap();
    assert_eq!(stdout_stat.file_type(), WasiFileType::CharacterDevice);
    assert!(stdout_stat.rights_base().contains(WasiRights::FD_WRITE));
    assert!(!stdout_stat.rights_base().contains(WasiRights::FD_READ));
    assert!(stdout_stat.rights_base().contains(WasiRights::FD_SEEK));
    assert!(stdout_stat.rights_base().contains(WasiRights::FD_TELL));
    let file_fd = ctx
        .path_open(WasiFd::ROOT, "hello.txt", WasiOpenOptions::read())
        .unwrap();

    assert_eq!(file_fd.get(), 4);
    assert_eq!(stdout.read_file("stdout").unwrap(), b"out");
    assert_eq!(stderr.read_file("stderr").unwrap(), b"err");
    assert_eq!(ctx.fd_write(WasiFd::STDIN, b"nope"), Err(Errno::Notcapable));
    assert!(ctx.fd_read_ready(WasiFd::STDIN).unwrap());
    assert_eq!(ctx.fd_write_ready(WasiFd::STDIN), Err(Errno::Notcapable));
    assert_eq!(
        ctx.fd_read(WasiFd::STDOUT, &mut buf),
        Err(Errno::Notcapable)
    );
    assert!(ctx.fd_write_ready(WasiFd::STDOUT).unwrap());
    assert_eq!(ctx.fd_read_ready(WasiFd::STDOUT), Err(Errno::Notcapable));
    assert_eq!(ctx.fd_close(WasiFd::STDOUT), Err(Errno::Badf));
    assert_eq!(ctx.fd_read_dir(WasiFd::STDIN), Err(Errno::Notdir));
}

#[test]
fn standard_fd_readiness_comes_from_attached_files() {
    let root = fixture(&[]);
    let ctx = wasi_ctx(
        WasiConfig::new(namespace_with_root(root))
            .with_stdin(
                Box::new(ReadinessFile {
                    read_ready: false,
                    write_ready: true,
                }),
                "pending stdin",
            )
            .with_stdout(
                Box::new(ReadinessFile {
                    read_ready: true,
                    write_ready: false,
                }),
                "blocked stdout",
            ),
    );

    assert!(!ctx.fd_read_ready(WasiFd::STDIN).unwrap());
    assert!(!ctx.fd_write_ready(WasiFd::STDOUT).unwrap());
}

#[test]
fn regular_fd_readiness_respects_open_capabilities() {
    let root = fixture(&[("read.txt", b"hello")]);
    let mut ctx = wasi_ctx(WasiConfig::new(namespace_with_root(root)));

    let read_fd = ctx
        .path_open(WasiFd::ROOT, "read.txt", WasiOpenOptions::read())
        .unwrap();
    let write_fd = ctx
        .path_open(WasiFd::ROOT, "write.txt", WasiOpenOptions::create_write())
        .unwrap();
    let read_write_fd = ctx
        .path_open(WasiFd::ROOT, "read.txt", WasiOpenOptions::read_write())
        .unwrap();

    assert!(ctx.fd_read_ready(read_fd).unwrap());
    assert_eq!(ctx.fd_write_ready(read_fd), Err(Errno::Notcapable));
    assert_eq!(ctx.fd_read_ready(write_fd), Err(Errno::Notcapable));
    assert!(ctx.fd_write_ready(write_fd).unwrap());
    assert!(ctx.fd_read_ready(read_write_fd).unwrap());
    assert!(ctx.fd_write_ready(read_write_fd).unwrap());
}

#[test]
fn task_service_paths_are_reachable_through_wasi_namespace() {
    let table = TaskTable::new();
    table.register_noop_driver("qjs").unwrap();
    let task = table.allocate_root("qjs").unwrap();
    let mut ctx = wasi_ctx(WasiConfig::new(task.namespace()));

    let fd = ctx
        .path_open(WasiFd::ROOT, "#task/self/id", WasiOpenOptions::read())
        .unwrap();
    let mut buf = [0; 8];
    let count = ctx.fd_read(fd, &mut buf).unwrap();

    assert_eq!(&buf[..count], b"1\n");
}

#[test]
fn service_paths_remain_rooted_when_guest_root_maps_to_cwd() {
    let table = TaskTable::new();
    table.register_noop_driver("qjs").unwrap();
    let task = table.allocate_root("qjs").unwrap();
    let root = fixture(&[
        ("app/main.js", b"from app"),
        ("app/#svc/id", b"from cwd hash path"),
        ("app/#term/1/winch", b"wrong cwd terminal"),
        ("#task/self/id", b"not the task service"),
        ("#term/1/winch", b"rooted terminal service"),
    ]);
    task.bind(root, ".", ".", BindOptions::default()).unwrap();
    let mut ctx = wasi_ctx(
        WasiConfig::new(task.namespace())
            .with_root_preopen_source(NormalizedPath::new("app").unwrap()),
    );

    let app_fd = ctx
        .path_open(WasiFd::ROOT, "main.js", WasiOpenOptions::read())
        .unwrap();
    let service_fd = ctx
        .path_open(WasiFd::ROOT, "#task/self/id", WasiOpenOptions::read())
        .unwrap();
    let term_fd = ctx
        .path_open(WasiFd::ROOT, "#term/1/winch", WasiOpenOptions::read())
        .unwrap();
    let hash_fd = ctx
        .path_open(WasiFd::ROOT, "#svc/id", WasiOpenOptions::read())
        .unwrap();
    let mut buf = [0; 32];
    let app_count = ctx.fd_read(app_fd, &mut buf).unwrap();
    assert_eq!(&buf[..app_count], b"from app");
    let service_count = ctx.fd_read(service_fd, &mut buf).unwrap();
    assert_eq!(&buf[..service_count], b"1\n");
    let term_count = ctx.fd_read(term_fd, &mut buf).unwrap();
    assert_eq!(&buf[..term_count], b"rooted terminal service");
    let hash_count = ctx.fd_read(hash_fd, &mut buf).unwrap();

    assert_eq!(&buf[..hash_count], b"from cwd hash path");
}

#[test]
fn writes_and_creates_flow_back_to_namespace() {
    let root = Arc::new(MemFs::new());
    let mut ctx = wasi_ctx(WasiConfig::new(namespace_with_root(root.clone())));

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

#[cfg(unix)]
#[test]
fn path_filestat_get_lookup_flags_control_final_symlink_following_on_host_mounts() {
    use std::os::unix::fs::symlink;

    let root = temp_host_dir("lookup-root");
    let outside = temp_host_dir("lookup-outside");
    std::fs::write(root.join("target.txt"), "inside").unwrap();
    std::fs::write(outside.join("secret.txt"), "outside").unwrap();
    symlink("target.txt", root.join("inside-link")).unwrap();
    symlink(outside.join("secret.txt"), root.join("outside-link")).unwrap();
    symlink(&outside, root.join("dir-link")).unwrap();

    let local = Arc::new(LocalFs::new(&root).unwrap());
    let ctx = wasi_ctx(WasiConfig::new(namespace_with_root(local)));

    let link = ctx
        .path_filestat_get_with_flags(WasiFd::ROOT, 0, "inside-link")
        .unwrap();
    assert_eq!(link.file_type(), FileType::Symlink);

    let followed = ctx
        .path_filestat_get_with_flags(WasiFd::ROOT, WasiLookupFlags::SYMLINK_FOLLOW, "inside-link")
        .unwrap();
    assert_eq!(followed.file_type(), FileType::File);
    assert_eq!(followed.len(), 6);

    assert_eq!(
        ctx.path_filestat_get_with_flags(WasiFd::ROOT, 0, "outside-link")
            .unwrap()
            .file_type(),
        FileType::Symlink
    );
    assert_eq!(
        ctx.path_filestat_get_with_flags(
            WasiFd::ROOT,
            WasiLookupFlags::SYMLINK_FOLLOW,
            "outside-link"
        ),
        Err(Errno::Notcapable)
    );
    assert_eq!(
        ctx.path_filestat_get_with_flags(WasiFd::ROOT, 0, "dir-link/secret.txt"),
        Err(Errno::Notcapable)
    );
    assert_eq!(
        ctx.path_filestat_get_with_flags(WasiFd::ROOT, 1 << 1, "inside-link"),
        Err(Errno::Notcapable)
    );

    std::fs::remove_dir_all(root).unwrap();
    std::fs::remove_dir_all(outside).unwrap();
}

#[cfg(unix)]
#[test]
fn path_readlink_and_symlink_reach_host_mounts_without_following_escapes() {
    let root = temp_host_dir("link-root");
    std::fs::write(root.join("target.txt"), "inside").unwrap();
    let local = Arc::new(LocalFs::new(&root).unwrap());
    let mut ctx = wasi_ctx(WasiConfig::new(namespace_with_root(local)));

    ctx.path_symlink(b"target.txt", WasiFd::ROOT, "created-link")
        .unwrap();
    assert_eq!(
        ctx.path_readlink(WasiFd::ROOT, "created-link").unwrap(),
        b"target.txt"
    );
    assert_eq!(
        ctx.path_filestat_get_with_flags(WasiFd::ROOT, 0, "created-link")
            .unwrap()
            .file_type(),
        FileType::Symlink
    );

    let fd = ctx
        .path_open(WasiFd::ROOT, "created-link", WasiOpenOptions::read())
        .unwrap();
    let mut buf = [0; 16];
    let count = ctx.fd_read(fd, &mut buf).unwrap();
    assert_eq!(&buf[..count], b"inside");

    assert_eq!(
        ctx.path_readlink(WasiFd::ROOT, "target.txt"),
        Err(Errno::Inval)
    );
    assert_eq!(
        ctx.path_symlink([0], WasiFd::ROOT, "bad-link"),
        Err(Errno::Inval)
    );

    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn path_readlink_and_symlink_require_directory_rights() {
    let root = fixture(&[("dir/file.txt", b"file")]);
    let mut ctx = wasi_ctx(WasiConfig::new(namespace_with_root(root)));
    let dir_fd = ctx
        .path_open_preview1(
            WasiFd::ROOT,
            "dir",
            0,
            WasiRights::PATH_OPEN,
            WasiRights::NONE,
            0,
        )
        .unwrap();

    assert_eq!(ctx.path_readlink(dir_fd, "link"), Err(Errno::Notcapable));
    assert_eq!(
        ctx.path_symlink(b"file.txt", dir_fd, "link"),
        Err(Errno::Notcapable)
    );
}

#[test]
fn path_filestat_set_times_updates_namespace_metadata() {
    let root = fixture(&[("stamp.txt", b"stamp")]);
    let ctx = wasi_ctx(
        WasiConfig::new(namespace_with_root(root.clone())).with_clock_time_ns(9_000_000_000),
    );

    ctx.path_filestat_set_times(
        WasiFd::ROOT,
        0,
        "stamp.txt",
        1_000_000_000,
        2_000_000_000,
        WasiFilestatSetTimes::ATIM | WasiFilestatSetTimes::MTIM,
    )
    .unwrap();

    let stat = ctx.path_filestat_get(WasiFd::ROOT, "stamp.txt").unwrap();
    assert_eq!(stat.accessed_time_ns(), 1_000_000_000);
    assert_eq!(stat.modified_time_ns(), 2_000_000_000);
    assert_eq!(stat.changed_time_ns(), 0);
    let bytes = stat.to_preview1_bytes();
    assert_eq!(
        u64::from_le_bytes(bytes[40..48].try_into().unwrap()),
        1_000_000_000
    );
    assert_eq!(
        u64::from_le_bytes(bytes[48..56].try_into().unwrap()),
        2_000_000_000
    );

    ctx.path_filestat_set_times(
        WasiFd::ROOT,
        0,
        "stamp.txt",
        3_000_000_000,
        99,
        WasiFilestatSetTimes::ATIM,
    )
    .unwrap();
    let stat = ctx.path_filestat_get(WasiFd::ROOT, "stamp.txt").unwrap();
    assert_eq!(stat.accessed_time_ns(), 3_000_000_000);
    assert_eq!(stat.modified_time_ns(), 2_000_000_000);

    ctx.path_filestat_set_times(
        WasiFd::ROOT,
        0,
        "stamp.txt",
        123,
        4_000_000_000,
        WasiFilestatSetTimes::ATIM_NOW | WasiFilestatSetTimes::MTIM,
    )
    .unwrap();
    let stat = ctx.path_filestat_get(WasiFd::ROOT, "stamp.txt").unwrap();
    assert_eq!(stat.accessed_time_ns(), 9_000_000_000);
    assert_eq!(stat.modified_time_ns(), 4_000_000_000);

    ctx.path_filestat_set_times(
        WasiFd::ROOT,
        0,
        "stamp.txt",
        5_000_000_000,
        456,
        WasiFilestatSetTimes::ATIM | WasiFilestatSetTimes::MTIM_NOW,
    )
    .unwrap();
    let stat = ctx.path_filestat_get(WasiFd::ROOT, "stamp.txt").unwrap();
    assert_eq!(stat.accessed_time_ns(), 5_000_000_000);
    assert_eq!(stat.modified_time_ns(), 9_000_000_000);

    ctx.path_filestat_set_times(WasiFd::ROOT, 0, "stamp.txt", 4, 5, 0)
        .unwrap();
    let stat = ctx.path_filestat_get(WasiFd::ROOT, "stamp.txt").unwrap();
    assert_eq!(stat.accessed_time_ns(), 5_000_000_000);
    assert_eq!(stat.modified_time_ns(), 9_000_000_000);
    assert_eq!(
        root.metadata(&path("stamp.txt"))
            .unwrap()
            .modified_time_ns(),
        9_000_000_000
    );
}

#[test]
fn path_filestat_set_times_validates_rights_and_flags() {
    let root = fixture(&[("dir/file.txt", b"stamp")]);
    let mut ctx = wasi_ctx(WasiConfig::new(namespace_with_root(root)));
    let dir_fd = ctx
        .path_open_preview1(
            WasiFd::ROOT,
            "dir",
            0,
            WasiRights::PATH_OPEN | WasiRights::PATH_FILESTAT_GET,
            WasiRights::NONE,
            0,
        )
        .unwrap();

    assert_eq!(
        ctx.path_filestat_set_times(
            dir_fd,
            0,
            "file.txt",
            1,
            2,
            WasiFilestatSetTimes::ATIM | WasiFilestatSetTimes::MTIM,
        ),
        Err(Errno::Notcapable)
    );
    assert_eq!(
        ctx.path_filestat_set_times(dir_fd, 0, "file.txt", 1, 2, WasiFilestatSetTimes::MTIM_NOW),
        Err(Errno::Notcapable)
    );
    assert_eq!(
        ctx.path_filestat_set_times(
            WasiFd::ROOT,
            2,
            "dir/file.txt",
            1,
            2,
            WasiFilestatSetTimes::ATIM,
        ),
        Err(Errno::Notcapable)
    );
    assert_eq!(
        ctx.path_filestat_set_times(WasiFd::ROOT, 0, "dir/file.txt", 1, 2, 1 << 9),
        Err(Errno::Inval)
    );
    assert_eq!(
        ctx.path_filestat_set_times(
            WasiFd::ROOT,
            0,
            "dir/file.txt",
            1,
            2,
            WasiFilestatSetTimes::ATIM | WasiFilestatSetTimes::ATIM_NOW,
        ),
        Err(Errno::Inval)
    );
}

#[test]
fn fd_filestat_set_times_updates_namespace_metadata() {
    let root = fixture(&[("stamp.txt", b"stamp")]);
    let mut ctx = wasi_ctx(
        WasiConfig::new(namespace_with_root(root.clone())).with_clock_time_ns(9_000_000_000),
    );
    let fd = ctx
        .path_open_preview1(
            WasiFd::ROOT,
            "stamp.txt",
            0,
            WasiRights::FD_READ | WasiRights::FD_FILESTAT_GET | WasiRights::FD_FILESTAT_SET_TIMES,
            WasiRights::NONE,
            0,
        )
        .unwrap();

    ctx.fd_filestat_set_times(
        fd,
        1_000_000_000,
        2_000_000_000,
        WasiFilestatSetTimes::ATIM | WasiFilestatSetTimes::MTIM,
    )
    .unwrap();

    let stat = ctx.fd_filestat_get(fd).unwrap();
    assert_eq!(stat.accessed_time_ns(), 1_000_000_000);
    assert_eq!(stat.modified_time_ns(), 2_000_000_000);
    assert_eq!(stat.changed_time_ns(), 0);

    ctx.fd_filestat_set_times(fd, 3_000_000_000, 99, WasiFilestatSetTimes::ATIM)
        .unwrap();
    let stat = ctx.fd_filestat_get(fd).unwrap();
    assert_eq!(stat.accessed_time_ns(), 3_000_000_000);
    assert_eq!(stat.modified_time_ns(), 2_000_000_000);

    ctx.fd_filestat_set_times(
        fd,
        99,
        4_000_000_000,
        WasiFilestatSetTimes::ATIM_NOW | WasiFilestatSetTimes::MTIM,
    )
    .unwrap();
    let stat = ctx.fd_filestat_get(fd).unwrap();
    assert_eq!(stat.accessed_time_ns(), 9_000_000_000);
    assert_eq!(stat.modified_time_ns(), 4_000_000_000);

    ctx.fd_filestat_set_times(
        fd,
        5_000_000_000,
        99,
        WasiFilestatSetTimes::ATIM | WasiFilestatSetTimes::MTIM_NOW,
    )
    .unwrap();
    let stat = ctx.fd_filestat_get(fd).unwrap();
    assert_eq!(stat.accessed_time_ns(), 5_000_000_000);
    assert_eq!(stat.modified_time_ns(), 9_000_000_000);

    ctx.fd_filestat_set_times(fd, 4, 5, 0).unwrap();
    let stat = ctx.fd_filestat_get(fd).unwrap();
    assert_eq!(stat.accessed_time_ns(), 5_000_000_000);
    assert_eq!(stat.modified_time_ns(), 9_000_000_000);
    assert_eq!(
        root.metadata(&path("stamp.txt"))
            .unwrap()
            .modified_time_ns(),
        9_000_000_000
    );
}

#[test]
fn fd_filestat_set_times_validates_rights_and_flags() {
    let root = fixture(&[("dir/file.txt", b"stamp")]);
    let mut ctx = wasi_ctx(WasiConfig::new(namespace_with_root(root)));
    let fd_without_set_times = ctx
        .path_open_preview1(
            WasiFd::ROOT,
            "dir/file.txt",
            0,
            WasiRights::FD_READ | WasiRights::FD_FILESTAT_GET,
            WasiRights::NONE,
            0,
        )
        .unwrap();
    let fd_with_set_times = ctx
        .path_open_preview1(
            WasiFd::ROOT,
            "dir/file.txt",
            0,
            WasiRights::FD_READ | WasiRights::FD_FILESTAT_GET | WasiRights::FD_FILESTAT_SET_TIMES,
            WasiRights::NONE,
            0,
        )
        .unwrap();

    assert_eq!(
        ctx.fd_filestat_set_times(
            fd_without_set_times,
            1,
            2,
            WasiFilestatSetTimes::ATIM | WasiFilestatSetTimes::MTIM,
        ),
        Err(Errno::Notcapable)
    );
    assert_eq!(
        ctx.fd_filestat_set_times(fd_without_set_times, 1, 2, WasiFilestatSetTimes::ATIM_NOW),
        Err(Errno::Notcapable)
    );
    assert_eq!(
        ctx.fd_filestat_set_times(
            fd_with_set_times,
            1,
            2,
            WasiFilestatSetTimes::ATIM | WasiFilestatSetTimes::ATIM_NOW,
        ),
        Err(Errno::Inval)
    );
    assert_eq!(
        ctx.fd_filestat_set_times(WasiFd::new(99), 1, 2, WasiFilestatSetTimes::ATIM),
        Err(Errno::Badf)
    );
}

#[test]
fn path_unlink_file_removes_namespace_files() {
    let root = fixture(&[("remove.txt", b"remove me"), ("dir/keep.txt", b"keep")]);
    let ctx = wasi_ctx(WasiConfig::new(namespace_with_root(root.clone())));

    ctx.path_unlink_file(WasiFd::ROOT, "remove.txt").unwrap();

    assert_eq!(
        root.metadata(&NormalizedPath::new("remove.txt").unwrap()),
        Err(FsError::NotFound)
    );
    assert_eq!(ctx.path_unlink_file(WasiFd::ROOT, "dir"), Err(Errno::Isdir));
    assert_eq!(
        ctx.path_unlink_file(WasiFd::ROOT, "missing.txt"),
        Err(Errno::Noent)
    );
}

#[test]
fn path_create_directory_creates_namespace_directories() {
    let root = fixture(&[("parent/file.txt", b"file")]);
    let ctx = wasi_ctx(WasiConfig::new(namespace_with_root(root.clone())));

    ctx.path_create_directory(WasiFd::ROOT, "parent/newdir")
        .unwrap();

    assert_eq!(
        root.metadata(&NormalizedPath::new("parent/newdir").unwrap())
            .unwrap()
            .file_type(),
        FileType::Directory
    );
    assert_eq!(
        ctx.path_create_directory(WasiFd::ROOT, "parent/newdir"),
        Err(Errno::Exist)
    );
    assert_eq!(
        ctx.path_create_directory(WasiFd::ROOT, "missing/newdir"),
        Err(Errno::Noent)
    );
    assert_eq!(
        ctx.path_create_directory(WasiFd::ROOT, "parent/file.txt/child"),
        Err(Errno::Notdir)
    );
    assert_eq!(
        ctx.path_create_directory(WasiFd::ROOT, "."),
        Err(Errno::Exist)
    );
}

#[test]
fn path_create_directory_respects_root_preopen_source() {
    let root = fixture(&[("app/existing.txt", b"file")]);
    let ctx = wasi_ctx(
        WasiConfig::new(namespace_with_root(root.clone()))
            .with_root_preopen_source(NormalizedPath::new("app").unwrap()),
    );

    ctx.path_create_directory(WasiFd::ROOT, "newdir").unwrap();

    assert_eq!(
        root.metadata(&NormalizedPath::new("app/newdir").unwrap())
            .unwrap()
            .file_type(),
        FileType::Directory
    );
    assert_eq!(
        root.metadata(&NormalizedPath::new("newdir").unwrap()),
        Err(FsError::NotFound)
    );
}

#[test]
fn path_create_directory_requires_directory_right() {
    let root = fixture(&[("dir/file.txt", b"file")]);
    let mut ctx = wasi_ctx(WasiConfig::new(namespace_with_root(root)));
    let dir_fd = ctx
        .path_open_preview1(
            WasiFd::ROOT,
            "dir",
            0,
            WasiRights::PATH_OPEN,
            WasiRights::NONE,
            0,
        )
        .unwrap();

    assert_eq!(
        ctx.path_create_directory(dir_fd, "newdir"),
        Err(Errno::Notcapable)
    );
}

#[test]
fn path_remove_directory_removes_empty_namespace_directories() {
    let root = fixture(&[("nonempty/file.txt", b"file"), ("file.txt", b"file")]);
    root.create_dir_all("empty").unwrap();
    let ctx = wasi_ctx(WasiConfig::new(namespace_with_root(root.clone())));

    ctx.path_remove_directory(WasiFd::ROOT, "empty").unwrap();

    assert_eq!(
        root.metadata(&NormalizedPath::new("empty").unwrap()),
        Err(FsError::NotFound)
    );
    assert_eq!(
        ctx.path_remove_directory(WasiFd::ROOT, "nonempty"),
        Err(Errno::Notempty)
    );
    assert_eq!(
        ctx.path_remove_directory(WasiFd::ROOT, "file.txt"),
        Err(Errno::Notdir)
    );
    assert_eq!(
        ctx.path_remove_directory(WasiFd::ROOT, "missing"),
        Err(Errno::Noent)
    );
    assert_eq!(
        ctx.path_remove_directory(WasiFd::ROOT, "."),
        Err(Errno::Notcapable)
    );
}

#[test]
fn path_remove_directory_respects_root_preopen_source() {
    let root = fixture(&[("app/file.txt", b"file")]);
    root.create_dir_all("app/empty").unwrap();
    root.create_dir_all("empty").unwrap();
    let ctx = wasi_ctx(
        WasiConfig::new(namespace_with_root(root.clone()))
            .with_root_preopen_source(NormalizedPath::new("app").unwrap()),
    );

    ctx.path_remove_directory(WasiFd::ROOT, "empty").unwrap();

    assert_eq!(
        root.metadata(&NormalizedPath::new("app/empty").unwrap()),
        Err(FsError::NotFound)
    );
    assert_eq!(
        root.metadata(&NormalizedPath::new("empty").unwrap())
            .unwrap()
            .file_type(),
        FileType::Directory
    );
}

#[test]
fn path_remove_directory_requires_directory_right() {
    let root = fixture(&[("dir/empty/file.txt", b"file")]);
    let mut ctx = wasi_ctx(WasiConfig::new(namespace_with_root(root)));
    let dir_fd = ctx
        .path_open_preview1(
            WasiFd::ROOT,
            "dir",
            0,
            WasiRights::PATH_OPEN,
            WasiRights::NONE,
            0,
        )
        .unwrap();

    assert_eq!(
        ctx.path_remove_directory(dir_fd, "empty"),
        Err(Errno::Notcapable)
    );
}

#[test]
fn path_rename_renames_namespace_paths() {
    let root = fixture(&[
        ("old.txt", b"old"),
        ("target.txt", b"target"),
        ("dir/sub/file.txt", b"nested"),
    ]);
    root.create_dir_all("empty").unwrap();
    let ctx = wasi_ctx(WasiConfig::new(namespace_with_root(root.clone())));

    ctx.path_rename(WasiFd::ROOT, "old.txt", WasiFd::ROOT, "renamed.txt")
        .unwrap();
    assert_eq!(root.metadata(&path("old.txt")), Err(FsError::NotFound));
    assert_eq!(root.read_file("renamed.txt").unwrap(), b"old");

    ctx.path_rename(WasiFd::ROOT, "renamed.txt", WasiFd::ROOT, "target.txt")
        .unwrap();
    assert_eq!(root.read_file("target.txt").unwrap(), b"old");

    ctx.path_rename(WasiFd::ROOT, "dir", WasiFd::ROOT, "empty")
        .unwrap();
    assert_eq!(root.metadata(&path("dir")), Err(FsError::NotFound));
    assert_eq!(root.read_file("empty/sub/file.txt").unwrap(), b"nested");

    assert_eq!(
        ctx.path_rename(WasiFd::ROOT, "missing", WasiFd::ROOT, "missing"),
        Err(Errno::Noent)
    );
    assert_eq!(
        ctx.path_rename(WasiFd::ROOT, ".", WasiFd::ROOT, "root"),
        Err(Errno::Notcapable)
    );
}

#[test]
fn path_rename_respects_root_preopen_source() {
    let root = fixture(&[("app/old.txt", b"app"), ("old.txt", b"root")]);
    let ctx = wasi_ctx(
        WasiConfig::new(namespace_with_root(root.clone()))
            .with_root_preopen_source(NormalizedPath::new("app").unwrap()),
    );

    ctx.path_rename(WasiFd::ROOT, "old.txt", WasiFd::ROOT, "renamed.txt")
        .unwrap();

    assert_eq!(root.metadata(&path("app/old.txt")), Err(FsError::NotFound));
    assert_eq!(root.read_file("app/renamed.txt").unwrap(), b"app");
    assert_eq!(root.read_file("old.txt").unwrap(), b"root");
}

#[test]
fn path_rename_requires_source_and_target_directory_rights() {
    let root = fixture(&[("src/old.txt", b"old")]);
    root.create_dir_all("dst").unwrap();
    let mut ctx = wasi_ctx(WasiConfig::new(namespace_with_root(root.clone())));
    let source_fd = ctx
        .path_open_preview1(
            WasiFd::ROOT,
            "src",
            0,
            WasiRights::PATH_RENAME_SOURCE,
            WasiRights::NONE,
            0,
        )
        .unwrap();
    let target_fd = ctx
        .path_open_preview1(
            WasiFd::ROOT,
            "dst",
            0,
            WasiRights::PATH_RENAME_TARGET,
            WasiRights::NONE,
            0,
        )
        .unwrap();

    ctx.path_rename(source_fd, "old.txt", target_fd, "new.txt")
        .unwrap();
    assert_eq!(root.metadata(&path("src/old.txt")), Err(FsError::NotFound));
    assert_eq!(root.read_file("dst/new.txt").unwrap(), b"old");

    let root = fixture(&[("src/old.txt", b"old")]);
    root.create_dir_all("dst").unwrap();
    let mut ctx = wasi_ctx(WasiConfig::new(namespace_with_root(root)));
    let target_only_fd = ctx
        .path_open_preview1(
            WasiFd::ROOT,
            "src",
            0,
            WasiRights::PATH_RENAME_TARGET,
            WasiRights::NONE,
            0,
        )
        .unwrap();
    let source_only_fd = ctx
        .path_open_preview1(
            WasiFd::ROOT,
            "dst",
            0,
            WasiRights::PATH_RENAME_SOURCE,
            WasiRights::NONE,
            0,
        )
        .unwrap();

    assert_eq!(
        ctx.path_rename(target_only_fd, "old.txt", WasiFd::ROOT, "renamed.txt"),
        Err(Errno::Notcapable)
    );
    assert_eq!(
        ctx.path_rename(WasiFd::ROOT, "src/old.txt", source_only_fd, "renamed.txt"),
        Err(Errno::Notcapable)
    );
}

#[test]
fn rights_are_enforced_on_open_fds() {
    let root = fixture(&[("read.txt", b"read"), ("write.txt", b"")]);
    let mut ctx = wasi_ctx(WasiConfig::new(namespace_with_root(root)));

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
                append: false,
            },
        )
        .unwrap();
    assert_eq!(ctx.fd_read(write_fd, &mut [0; 4]), Err(Errno::Notcapable));
}

#[test]
fn fd_filestat_set_size_updates_regular_files() {
    let root = fixture(&[("file.txt", b"hello"), ("read.txt", b"read")]);
    let mut ctx = wasi_ctx(WasiConfig::new(namespace_with_root(root.clone())));

    let fd = ctx
        .path_open(
            WasiFd::ROOT,
            "file.txt",
            WasiOpenOptions {
                read: true,
                write: true,
                create: false,
                truncate: false,
                append: false,
            },
        )
        .unwrap();
    assert!(
        ctx.fd_fdstat_get(fd)
            .unwrap()
            .rights_base()
            .contains(WasiRights::FD_FILESTAT_SET_SIZE)
    );

    assert_eq!(ctx.fd_seek(fd, 4, WasiWhence::Set).unwrap(), 4);
    ctx.fd_filestat_set_size(fd, 8).unwrap();
    assert_eq!(ctx.fd_tell(fd).unwrap(), 4);
    assert_eq!(root.read_file("file.txt").unwrap(), b"hello\0\0\0");
    assert_eq!(ctx.fd_filestat_get(fd).unwrap().len(), 8);

    ctx.fd_filestat_set_size(fd, 2).unwrap();
    assert_eq!(ctx.fd_tell(fd).unwrap(), 4);
    assert_eq!(root.read_file("file.txt").unwrap(), b"he");
    assert_eq!(ctx.fd_filestat_get(fd).unwrap().len(), 2);

    let read_fd = ctx
        .path_open(WasiFd::ROOT, "read.txt", WasiOpenOptions::read())
        .unwrap();
    assert_eq!(ctx.fd_filestat_set_size(read_fd, 1), Err(Errno::Notcapable));
    let dir_fd = ctx
        .path_open(WasiFd::ROOT, ".", WasiOpenOptions::read())
        .unwrap();
    assert_eq!(ctx.fd_filestat_set_size(dir_fd, 1), Err(Errno::Notcapable));
}

#[test]
fn directories_can_be_opened_and_read_but_not_written() {
    let root = fixture(&[("dir/file.txt", b"file")]);
    let mut ctx = wasi_ctx(WasiConfig::new(namespace_with_root(root)));

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
    let mut ctx = wasi_ctx(WasiConfig::new(namespace));

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
    let mut ctx = wasi_ctx(WasiConfig::new(namespace_with_root(root)));
    assert_eq!(ctx.open_dynamic_fd_count(), 0);
    let fd = ctx
        .path_open(WasiFd::ROOT, "hello.txt", WasiOpenOptions::read())
        .unwrap();
    assert_eq!(ctx.open_dynamic_fd_count(), 1);

    assert_eq!(ctx.fd_close(WasiFd::ROOT), Err(Errno::Badf));
    ctx.fd_close(fd).unwrap();
    assert_eq!(ctx.open_dynamic_fd_count(), 0);
    assert_eq!(ctx.fd_read(fd, &mut [0; 1]), Err(Errno::Badf));
}

#[derive(Debug)]
struct FailingCloseObserver {
    closed: Arc<Mutex<Vec<WasiFd>>>,
}

impl WasiFdObserver for FailingCloseObserver {
    fn file_opened(
        &self,
        _fd: WasiFd,
        _file: WasiFile,
        _path: &NormalizedPath,
    ) -> Result<(), Errno> {
        Ok(())
    }

    fn fd_closed(&self, fd: WasiFd) -> Result<(), Errno> {
        self.closed.lock().unwrap().push(fd);
        Err(Errno::Io)
    }
}

#[test]
fn fd_close_keeps_wasi_fd_open_when_observer_close_fails() {
    let root = fixture(&[("hello.txt", b"hello")]);
    let closed = Arc::new(Mutex::new(Vec::new()));
    let mut ctx = wasi_ctx(WasiConfig::new(namespace_with_root(root)).with_fd_observer(
        FailingCloseObserver {
            closed: Arc::clone(&closed),
        },
    ));
    let fd = ctx
        .path_open(WasiFd::ROOT, "hello.txt", WasiOpenOptions::read())
        .unwrap();

    assert_eq!(ctx.fd_close(fd), Err(Errno::Io));
    assert_eq!(closed.lock().unwrap().as_slice(), [fd]);
    assert_eq!(ctx.open_dynamic_fd_count(), 1);

    let mut buf = [0; 8];
    let count = ctx.fd_read(fd, &mut buf).unwrap();
    assert_eq!(&buf[..count], b"hello");
}

#[test]
fn fdstat_reports_preopen_and_regular_file_rights() {
    let root = fixture(&[("read.txt", b"read"), ("write.txt", b"")]);
    let mut ctx = wasi_ctx(WasiConfig::new(namespace_with_root(root)));

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
            .contains(WasiRights::PATH_CREATE_DIRECTORY)
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
    assert!(
        root_stat
            .rights_base()
            .contains(WasiRights::PATH_FILESTAT_SET_TIMES)
    );
    assert!(root_stat.rights_base().contains(WasiRights::PATH_READLINK));
    assert!(root_stat.rights_base().contains(WasiRights::PATH_SYMLINK));
    assert!(
        root_stat
            .rights_base()
            .contains(WasiRights::PATH_REMOVE_DIRECTORY)
    );
    assert!(
        root_stat
            .rights_base()
            .contains(WasiRights::PATH_UNLINK_FILE)
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
    assert!(root_stat.rights_inheriting().contains(WasiRights::FD_SEEK));
    assert!(root_stat.rights_inheriting().contains(WasiRights::FD_TELL));

    let read_fd = ctx
        .path_open(WasiFd::ROOT, "read.txt", WasiOpenOptions::read())
        .unwrap();
    let read_stat = ctx.fd_fdstat_get(read_fd).unwrap();
    assert_eq!(read_stat.file_type(), WasiFileType::RegularFile);
    assert!(read_stat.rights_base().contains(WasiRights::FD_READ));
    assert!(!read_stat.rights_base().contains(WasiRights::FD_WRITE));
    assert!(read_stat.rights_base().contains(WasiRights::FD_SEEK));
    assert!(read_stat.rights_base().contains(WasiRights::FD_TELL));
    let read_file_stat = ctx.fd_filestat_get(read_fd).unwrap();
    let read_file_stat_bytes = read_file_stat.to_preview1_bytes();
    assert_eq!(
        read_file_stat_bytes[16],
        WasiFileType::RegularFile.preview1_code()
    );
    assert_eq!(
        u64::from_le_bytes(read_file_stat_bytes[32..40].try_into().unwrap()),
        read_file_stat.len()
    );
    assert_eq!(read_file_stat_bytes[..16], [0; 16]);
    assert_eq!(read_file_stat_bytes[17..32], [0; 15]);
    assert_eq!(read_file_stat_bytes[40..], [0; 24]);
    let root_file_stat = ctx.fd_filestat_get(WasiFd::ROOT).unwrap();
    let root_file_stat_bytes = root_file_stat.to_preview1_bytes();
    assert_eq!(
        root_file_stat_bytes[16],
        WasiFileType::Directory.preview1_code()
    );
    assert_eq!(
        u64::from_le_bytes(root_file_stat_bytes[32..40].try_into().unwrap()),
        root_file_stat.len()
    );
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
                append: false,
            },
        )
        .unwrap();
    let write_stat = ctx.fd_fdstat_get(write_fd).unwrap();
    assert!(write_stat.rights_base().contains(WasiRights::FD_WRITE));
    assert!(!write_stat.rights_base().contains(WasiRights::FD_READ));
    assert!(write_stat.rights_base().contains(WasiRights::FD_SEEK));
    assert!(write_stat.rights_base().contains(WasiRights::FD_TELL));

    assert_eq!(ctx.fd_fdstat_get(WasiFd::new(99)), Err(Errno::Badf));
    assert_eq!(ctx.fd_prestat_get(read_fd), Err(Errno::Badf));
}

#[test]
fn fd_seek_and_tell_use_seekable_file_handles() {
    let root = fixture(&[("hello.txt", b"hello")]);
    let mut ctx = wasi_ctx(WasiConfig::new(namespace_with_root(root)));
    let fd = ctx
        .path_open(WasiFd::ROOT, "hello.txt", WasiOpenOptions::read())
        .unwrap();

    assert_eq!(WasiWhence::from_preview1(0).unwrap(), WasiWhence::Set);
    assert_eq!(WasiWhence::from_preview1(1).unwrap(), WasiWhence::Cur);
    assert_eq!(WasiWhence::from_preview1(2).unwrap(), WasiWhence::End);
    assert_eq!(WasiWhence::from_preview1(3), Err(Errno::Inval));
    assert_eq!(ctx.fd_tell(fd).unwrap(), 0);
    assert_eq!(ctx.fd_seek(fd, 2, WasiWhence::Set).unwrap(), 2);
    assert_eq!(ctx.fd_tell(fd).unwrap(), 2);
    assert_eq!(ctx.fd_seek(fd, -1, WasiWhence::Cur).unwrap(), 1);
    assert_eq!(ctx.fd_seek(fd, -1, WasiWhence::End).unwrap(), 4);
    assert_eq!(ctx.fd_seek(fd, -1, WasiWhence::Set), Err(Errno::Inval));
    assert_eq!(ctx.fd_seek(fd, -10, WasiWhence::Cur), Err(Errno::Inval));
    assert_eq!(
        ctx.fd_seek(WasiFd::new(99), 0, WasiWhence::Set),
        Err(Errno::Badf)
    );
    assert_eq!(
        ctx.fd_seek(WasiFd::new(99), -1, WasiWhence::Set),
        Err(Errno::Badf)
    );
    assert_eq!(
        ctx.fd_seek(WasiFd::ROOT, 0, WasiWhence::Set),
        Err(Errno::Notcapable)
    );
    assert_eq!(
        ctx.fd_seek(WasiFd::ROOT, -1, WasiWhence::Set),
        Err(Errno::Notcapable)
    );
    assert_eq!(ctx.fd_tell(WasiFd::ROOT), Err(Errno::Notcapable));
}

#[test]
fn preview1_append_fdflag_writes_at_end_of_file() {
    let root = fixture(&[("log.txt", b"start")]);
    let mut ctx = wasi_ctx(WasiConfig::new(namespace_with_root(root.clone())));
    let fd = ctx
        .path_open_preview1(
            WasiFd::ROOT,
            "log.txt",
            0,
            WasiRights::FD_WRITE | WasiRights::FD_SEEK | WasiRights::FD_TELL,
            WasiRights::NONE,
            WasiOpenOptions::FDFLAGS_APPEND,
        )
        .unwrap();

    assert_eq!(ctx.fd_tell(fd).unwrap(), 0);
    assert_eq!(
        ctx.fd_fdstat_get(fd).unwrap().fdflags(),
        WasiOpenOptions::FDFLAGS_APPEND
    );
    assert_eq!(ctx.fd_write(fd, b"-one").unwrap(), 4);
    assert_eq!(root.read_file("log.txt").unwrap(), b"start-one");
    ctx.fd_seek(fd, 0, WasiWhence::Set).unwrap();
    assert_eq!(ctx.fd_write(fd, b"-two").unwrap(), 4);
    assert_eq!(root.read_file("log.txt").unwrap(), b"start-one-two");
}

#[test]
fn preview1_fd_fdstat_set_flags_updates_append_mode() {
    let root = fixture(&[("log.txt", b"start"), ("read.txt", b"read")]);
    let mut ctx = wasi_ctx(WasiConfig::new(namespace_with_root(root.clone())));
    let fd = ctx
        .path_open_preview1(
            WasiFd::ROOT,
            "log.txt",
            0,
            WasiRights::FD_WRITE | WasiRights::FD_SEEK | WasiRights::FD_TELL,
            WasiRights::NONE,
            0,
        )
        .unwrap();

    assert_eq!(ctx.fd_fdstat_get(fd).unwrap().fdflags(), 0);
    ctx.fd_fdstat_set_flags(fd, WasiOpenOptions::FDFLAGS_APPEND)
        .unwrap();
    assert_eq!(
        ctx.fd_fdstat_get(fd).unwrap().fdflags(),
        WasiOpenOptions::FDFLAGS_APPEND
    );
    ctx.fd_seek(fd, 0, WasiWhence::Set).unwrap();
    assert_eq!(ctx.fd_write(fd, b"-app").unwrap(), 4);
    assert_eq!(root.read_file("log.txt").unwrap(), b"start-app");

    ctx.fd_fdstat_set_flags(fd, 0).unwrap();
    assert_eq!(ctx.fd_fdstat_get(fd).unwrap().fdflags(), 0);
    ctx.fd_seek(fd, 0, WasiWhence::Set).unwrap();
    assert_eq!(ctx.fd_write(fd, b"HEAD").unwrap(), 4);
    assert_eq!(root.read_file("log.txt").unwrap(), b"HEADt-app");

    let read_fd = ctx
        .path_open_preview1(
            WasiFd::ROOT,
            "read.txt",
            0,
            WasiRights::FD_READ,
            WasiRights::NONE,
            0,
        )
        .unwrap();
    assert_eq!(
        ctx.fd_fdstat_set_flags(read_fd, WasiOpenOptions::FDFLAGS_APPEND),
        Err(Errno::Notcapable)
    );
    assert_eq!(
        ctx.fd_fdstat_set_flags(fd, WasiOpenOptions::FDFLAGS_DSYNC),
        Err(Errno::Notcapable)
    );
    assert_eq!(ctx.fd_fdstat_set_flags(WasiFd::ROOT, 0), Ok(()));
    assert_eq!(
        ctx.fd_fdstat_set_flags(WasiFd::ROOT, WasiOpenOptions::FDFLAGS_APPEND),
        Err(Errno::Notcapable)
    );
    assert_eq!(
        ctx.fd_fdstat_set_flags(WasiFd::new(99), 0),
        Err(Errno::Badf)
    );
}

#[test]
fn preview1_path_open_flags_convert_to_wanix_open_options() {
    let read = WasiOpenOptions::from_preview1(
        0,
        WasiRights::FD_READ | WasiRights::FD_SEEK | WasiRights::FD_TELL,
        0,
    )
    .unwrap();
    assert_eq!(
        read,
        WasiOpenOptions {
            read: true,
            write: false,
            create: false,
            truncate: false,
            append: false,
        }
    );

    let create_truncate = WasiOpenOptions::from_preview1(
        WasiOpenOptions::OFLAGS_CREATE | WasiOpenOptions::OFLAGS_TRUNCATE,
        WasiRights::FD_WRITE | WasiRights::FD_FILESTAT_GET,
        0,
    )
    .unwrap();
    assert_eq!(
        create_truncate,
        WasiOpenOptions {
            read: false,
            write: true,
            create: true,
            truncate: true,
            append: false,
        }
    );

    assert_eq!(
        WasiOpenOptions::from_preview1(WasiOpenOptions::OFLAGS_CREATE, WasiRights::FD_READ, 0),
        Err(Errno::Notcapable)
    );
    assert_eq!(
        WasiOpenOptions::from_preview1(WasiOpenOptions::OFLAGS_DIRECTORY, WasiRights::FD_READ, 0),
        Err(Errno::Notcapable)
    );
    assert_eq!(
        WasiOpenOptions::from_preview1(WasiOpenOptions::OFLAGS_EXCLUSIVE, WasiRights::FD_WRITE, 0),
        Err(Errno::Notcapable)
    );
    assert_eq!(
        WasiOpenOptions::from_preview1(1 << 15, WasiRights::FD_READ, 0),
        Err(Errno::Notcapable)
    );
    assert_eq!(
        WasiOpenOptions::from_preview1(0, WasiRights::FD_WRITE, WasiOpenOptions::FDFLAGS_APPEND)
            .unwrap(),
        WasiOpenOptions {
            read: false,
            write: true,
            create: false,
            truncate: false,
            append: true,
        }
    );
    assert_eq!(
        WasiOpenOptions::from_preview1(0, WasiRights::FD_READ, WasiOpenOptions::FDFLAGS_APPEND),
        Err(Errno::Notcapable)
    );
    assert_eq!(
        WasiOpenOptions::from_preview1(0, WasiRights::FD_WRITE, WasiOpenOptions::FDFLAGS_DSYNC),
        Err(Errno::Notcapable)
    );
    assert_eq!(
        WasiOpenOptions::from_preview1(0, WasiRights::PATH_OPEN, 0),
        Err(Errno::Notcapable)
    );
    assert_eq!(
        WasiOpenOptions::from_preview1(
            0,
            WasiRights::from_preview1_bits(WasiRights::FD_READ.bits() | (1 << 63)),
            0
        ),
        Err(Errno::Notcapable)
    );
    assert_eq!(
        WasiRights::from_preview1_bits(WasiRights::FD_READ.bits()),
        WasiRights::FD_READ
    );

    let directory = WasiPathOpen::from_preview1(
        WasiOpenOptions::OFLAGS_DIRECTORY,
        WasiRights::PATH_OPEN | WasiRights::FD_READDIR,
        WasiRights::FD_READ,
        WasiOpenOptions::FDFLAGS_NONBLOCK,
    )
    .unwrap();
    assert_eq!(
        directory.options(),
        WasiOpenOptions {
            read: false,
            write: false,
            create: false,
            truncate: false,
            append: false,
        }
    );
    assert_eq!(
        directory.rights_base(),
        WasiRights::PATH_OPEN | WasiRights::FD_READDIR
    );
    assert_eq!(directory.rights_inheriting(), WasiRights::FD_READ);
    assert!(directory.directory());
    assert_eq!(
        WasiPathOpen::from_preview1(
            WasiOpenOptions::OFLAGS_DIRECTORY | WasiOpenOptions::OFLAGS_CREATE,
            WasiRights::PATH_OPEN | WasiRights::FD_READDIR,
            WasiRights::NONE,
            0
        ),
        Err(Errno::Notcapable)
    );
    assert_eq!(
        WasiPathOpen::from_preview1(1 << 15, WasiRights::FD_READ, WasiRights::NONE, 0),
        Err(Errno::Notcapable)
    );
    assert_eq!(
        WasiPathOpen::from_preview1(
            0,
            WasiRights::FD_READ,
            WasiRights::NONE,
            WasiOpenOptions::FDFLAGS_DSYNC
        ),
        Err(Errno::Notcapable)
    );
    assert_eq!(
        WasiPathOpen::from_preview1(
            WasiOpenOptions::OFLAGS_TRUNCATE,
            WasiRights::FD_READ,
            WasiRights::NONE,
            0
        ),
        Err(Errno::Notcapable)
    );
    assert_eq!(
        WasiPathOpen::from_preview1(
            0,
            WasiRights::FD_READ,
            WasiRights::NONE,
            WasiOpenOptions::FDFLAGS_APPEND
        ),
        Err(Errno::Notcapable)
    );
    assert_eq!(
        WasiPathOpen::from_preview1(
            0,
            WasiRights::FD_READ,
            WasiRights::from_preview1_bits(1 << 63),
            0
        ),
        Err(Errno::Notcapable)
    );
}

#[test]
fn preview1_path_open_preserves_reduced_file_rights() {
    let root = fixture(&[("data.txt", b"data")]);
    let mut ctx = wasi_ctx(WasiConfig::new(namespace_with_root(root)));

    let fd = ctx
        .path_open_preview1(
            WasiFd::ROOT,
            "data.txt",
            0,
            WasiRights::FD_READ,
            WasiRights::NONE,
            0,
        )
        .unwrap();
    let fdstat = ctx.fd_fdstat_get(fd).unwrap();
    assert_eq!(fdstat.file_type(), WasiFileType::RegularFile);
    assert_eq!(fdstat.rights_base(), WasiRights::FD_READ);
    assert_eq!(fdstat.rights_inheriting(), WasiRights::NONE);

    let mut buf = [0; 8];
    let count = ctx.fd_read(fd, &mut buf).unwrap();
    assert_eq!(&buf[..count], b"data");
    assert_eq!(ctx.fd_write(fd, b"nope"), Err(Errno::Notcapable));
    assert_eq!(ctx.fd_filestat_get(fd), Err(Errno::Notcapable));
    assert_eq!(ctx.fd_seek(fd, 0, WasiWhence::Set), Err(Errno::Notcapable));
    assert_eq!(ctx.fd_tell(fd), Err(Errno::Notcapable));
}

#[test]
fn preview1_path_open_projects_libc_regular_file_rights() {
    let root = fixture(&[("data.txt", b"data")]);
    let mut ctx = wasi_ctx(WasiConfig::new(namespace_with_root(root)));
    let libc_read_rights = WasiRights::FD_READ
        | WasiRights::FD_SEEK
        | WasiRights::FD_TELL
        | WasiRights::PATH_CREATE_FILE
        | WasiRights::PATH_OPEN
        | WasiRights::FD_READDIR
        | WasiRights::PATH_FILESTAT_GET
        | WasiRights::PATH_FILESTAT_SET_SIZE
        | WasiRights::PATH_FILESTAT_SET_TIMES
        | WasiRights::FD_FILESTAT_GET;
    let libc_inheriting_rights = libc_read_rights | WasiRights::FD_WRITE;

    let fd = ctx
        .path_open_preview1(
            WasiFd::ROOT,
            "data.txt",
            0,
            libc_read_rights,
            libc_inheriting_rights,
            0,
        )
        .unwrap();
    let fdstat = ctx.fd_fdstat_get(fd).unwrap();

    assert_eq!(fdstat.file_type(), WasiFileType::RegularFile);
    assert_eq!(
        fdstat.rights_base(),
        WasiRights::FD_READ
            | WasiRights::FD_SEEK
            | WasiRights::FD_TELL
            | WasiRights::FD_FILESTAT_GET
    );
    assert_eq!(fdstat.rights_inheriting(), WasiRights::NONE);
    let mut buf = [0; 8];
    let count = ctx.fd_read(fd, &mut buf).unwrap();
    assert_eq!(&buf[..count], b"data");
    assert_eq!(ctx.fd_write(fd, b"nope"), Err(Errno::Notcapable));
}

#[test]
fn preview1_path_open_projects_libc_rights_for_rooted_service_paths() {
    let table = TaskTable::new();
    table.register_noop_driver("qjs").unwrap();
    let task = table.allocate_root("qjs").unwrap();
    let root = fixture(&[("app/main.js", b"from app")]);
    task.bind(root, ".", ".", BindOptions::default()).unwrap();
    let mut ctx = wasi_ctx(
        WasiConfig::new(task.namespace())
            .with_root_preopen_source(NormalizedPath::new("app").unwrap()),
    );
    let libc_read_rights = WasiRights::FD_READ
        | WasiRights::FD_SEEK
        | WasiRights::FD_TELL
        | WasiRights::PATH_CREATE_FILE
        | WasiRights::PATH_OPEN
        | WasiRights::FD_READDIR
        | WasiRights::PATH_FILESTAT_GET
        | WasiRights::PATH_FILESTAT_SET_SIZE
        | WasiRights::PATH_FILESTAT_SET_TIMES
        | WasiRights::FD_FILESTAT_GET;
    let libc_inheriting_rights = libc_read_rights | WasiRights::FD_WRITE;

    let fd = ctx
        .path_open_preview1(
            WasiFd::ROOT,
            "#task/self/id",
            0,
            libc_read_rights,
            libc_inheriting_rights,
            0,
        )
        .unwrap();
    let mut buf = [0; 8];
    let count = ctx.fd_read(fd, &mut buf).unwrap();

    assert_eq!(&buf[..count], b"1\n");
}

#[test]
fn preview1_path_open_projects_libc_write_rights_for_task_service_files() {
    let table = TaskTable::new();
    table.register_noop_driver("qjs").unwrap();
    let task = table.allocate_root("qjs").unwrap();
    let mut ctx = wasi_ctx(WasiConfig::new(task.namespace()));
    let libc_write_rights = WasiRights::FD_SEEK
        | WasiRights::FD_TELL
        | WasiRights::FD_WRITE
        | WasiRights::PATH_CREATE_FILE
        | WasiRights::PATH_OPEN
        | WasiRights::PATH_FILESTAT_GET
        | WasiRights::PATH_FILESTAT_SET_SIZE
        | WasiRights::PATH_FILESTAT_SET_TIMES
        | WasiRights::FD_FILESTAT_GET;
    let libc_inheriting_rights = libc_write_rights | WasiRights::FD_READ | WasiRights::FD_READDIR;

    let fd = ctx
        .path_open_preview1(
            WasiFd::ROOT,
            "#task/self/cmd",
            0,
            libc_write_rights,
            libc_inheriting_rights,
            0,
        )
        .unwrap();
    let fdstat = ctx.fd_fdstat_get(fd).unwrap();

    assert_eq!(fdstat.file_type(), WasiFileType::RegularFile);
    assert_eq!(
        fdstat.rights_base(),
        WasiRights::FD_WRITE | WasiRights::FD_FILESTAT_GET
    );
    assert_eq!(ctx.fd_write(fd, b"child.js\n").unwrap(), 9);
    assert_eq!(ctx.fd_seek(fd, 0, WasiWhence::Set), Err(Errno::Notcapable));
    assert_eq!(task.cmd(), "child.js");
}

#[test]
fn preview1_path_open_projects_libc_regular_file_write_rights() {
    let root = fixture(&[("created.txt", b"older longer content")]);
    let mut ctx = wasi_ctx(WasiConfig::new(namespace_with_root(root.clone())));
    let libc_write_rights = WasiRights::FD_SEEK
        | WasiRights::FD_TELL
        | WasiRights::FD_WRITE
        | WasiRights::PATH_CREATE_FILE
        | WasiRights::PATH_OPEN
        | WasiRights::PATH_FILESTAT_GET
        | WasiRights::PATH_FILESTAT_SET_SIZE
        | WasiRights::PATH_FILESTAT_SET_TIMES
        | WasiRights::FD_FILESTAT_GET;
    let libc_inheriting_rights = libc_write_rights | WasiRights::FD_READ | WasiRights::FD_READDIR;

    let fd = ctx
        .path_open_preview1(
            WasiFd::ROOT,
            "created.txt",
            WasiOpenOptions::OFLAGS_CREATE | WasiOpenOptions::OFLAGS_TRUNCATE,
            libc_write_rights,
            libc_inheriting_rights,
            0,
        )
        .unwrap();
    let fdstat = ctx.fd_fdstat_get(fd).unwrap();

    assert_eq!(fdstat.file_type(), WasiFileType::RegularFile);
    assert_eq!(
        fdstat.rights_base(),
        WasiRights::FD_WRITE
            | WasiRights::FD_SEEK
            | WasiRights::FD_TELL
            | WasiRights::FD_FILESTAT_GET
    );
    assert_eq!(fdstat.rights_inheriting(), WasiRights::NONE);
    assert_eq!(ctx.fd_write(fd, b"created by libc").unwrap(), 15);
    assert_eq!(root.read_file("created.txt").unwrap(), b"created by libc");
    assert_eq!(ctx.fd_read(fd, &mut [0; 1]), Err(Errno::Notcapable));
}

#[test]
fn preview1_path_open_preserves_reduced_directory_rights() {
    let root = fixture(&[("dir/file.txt", b"data")]);
    let mut ctx = wasi_ctx(WasiConfig::new(namespace_with_root(root)));

    let dir_fd = ctx
        .path_open_preview1(
            WasiFd::ROOT,
            "dir",
            WasiOpenOptions::OFLAGS_DIRECTORY,
            WasiRights::FD_READDIR,
            WasiRights::NONE,
            WasiOpenOptions::FDFLAGS_NONBLOCK,
        )
        .unwrap();
    let fdstat = ctx.fd_fdstat_get(dir_fd).unwrap();
    assert_eq!(fdstat.file_type(), WasiFileType::Directory);
    assert_eq!(fdstat.rights_base(), WasiRights::FD_READDIR);
    assert_eq!(fdstat.rights_inheriting(), WasiRights::NONE);
    assert_eq!(
        ctx.fd_read_dir(dir_fd)
            .unwrap()
            .into_iter()
            .map(|entry| entry.name().to_owned())
            .collect::<Vec<_>>(),
        ["file.txt"]
    );
    assert_eq!(
        ctx.path_open_preview1(
            WasiFd::ROOT,
            "dir/file.txt",
            WasiOpenOptions::OFLAGS_DIRECTORY,
            WasiRights::FD_READDIR,
            WasiRights::NONE,
            WasiOpenOptions::FDFLAGS_NONBLOCK,
        ),
        Err(Errno::Notdir)
    );
    assert_eq!(
        ctx.path_open_preview1(
            dir_fd,
            "file.txt",
            0,
            WasiRights::FD_READ,
            WasiRights::NONE,
            0
        ),
        Err(Errno::Notcapable)
    );
    assert_eq!(
        ctx.path_filestat_get(dir_fd, "file.txt"),
        Err(Errno::Notcapable)
    );
}

#[test]
fn preview1_path_open_accepts_libc_directory_rights() {
    let root = fixture(&[("dir/file.txt", b"data")]);
    let mut ctx = wasi_ctx(WasiConfig::new(namespace_with_root(root)));
    let libc_directory_base = WasiRights::from_preview1_bits(
        WasiRights::DIRECTORY_INHERITING.bits() & !WasiRights::FD_WRITE.bits(),
    );

    let dir_fd = ctx
        .path_open_preview1(
            WasiFd::ROOT,
            ".",
            WasiOpenOptions::OFLAGS_DIRECTORY,
            libc_directory_base,
            WasiRights::DIRECTORY_INHERITING,
            WasiOpenOptions::FDFLAGS_NONBLOCK,
        )
        .unwrap();
    let fdstat = ctx.fd_fdstat_get(dir_fd).unwrap();

    assert_eq!(fdstat.file_type(), WasiFileType::Directory);
    assert_eq!(fdstat.rights_base(), libc_directory_base);
    assert_eq!(fdstat.rights_inheriting(), WasiRights::DIRECTORY_INHERITING);
    assert_eq!(
        ctx.fd_read_dir(dir_fd)
            .unwrap()
            .into_iter()
            .map(|entry| entry.name().to_owned())
            .collect::<Vec<_>>(),
        ["dir"]
    );
}

#[test]
fn preview1_path_open_enforces_parent_inheriting_rights() {
    let root = fixture(&[("dir/file.txt", b"data")]);
    let mut ctx = wasi_ctx(WasiConfig::new(namespace_with_root(root)));

    let dir_fd = ctx
        .path_open_preview1(
            WasiFd::ROOT,
            "dir",
            0,
            WasiRights::PATH_OPEN | WasiRights::PATH_FILESTAT_GET,
            WasiRights::FD_READ,
            0,
        )
        .unwrap();
    assert_eq!(ctx.fd_read_dir(dir_fd), Err(Errno::Notcapable));
    assert_eq!(
        ctx.path_filestat_get(dir_fd, "file.txt")
            .unwrap()
            .file_type(),
        FileType::File
    );
    assert_eq!(
        ctx.path_open_preview1(
            dir_fd,
            "file.txt",
            0,
            WasiRights::FD_WRITE,
            WasiRights::NONE,
            0
        ),
        Err(Errno::Notcapable)
    );
    assert_eq!(
        ctx.path_open(
            dir_fd,
            "file.txt",
            WasiOpenOptions {
                read: false,
                write: true,
                create: false,
                truncate: false,
                append: false,
            },
        ),
        Err(Errno::Notcapable)
    );
    let logical_file_fd = ctx
        .path_open(dir_fd, "file.txt", WasiOpenOptions::read())
        .unwrap();
    assert_eq!(
        ctx.path_unlink_file(dir_fd, "file.txt"),
        Err(Errno::Notcapable)
    );
    assert_eq!(
        ctx.fd_fdstat_get(logical_file_fd).unwrap().rights_base(),
        WasiRights::FD_READ
    );
    assert_eq!(ctx.fd_filestat_get(logical_file_fd), Err(Errno::Notcapable));
    let file_fd = ctx
        .path_open_preview1(
            dir_fd,
            "file.txt",
            0,
            WasiRights::FD_READ,
            WasiRights::NONE,
            0,
        )
        .unwrap();
    assert_eq!(
        ctx.fd_fdstat_get(file_fd).unwrap().rights_base(),
        WasiRights::FD_READ
    );
}

#[test]
fn path_errors_map_to_wasi_errno() {
    let root = fixture(&[]);
    let mut ctx = wasi_ctx(WasiConfig::new(namespace_with_root(root)));

    assert_eq!(
        ctx.path_open(WasiFd::ROOT, "../bad", WasiOpenOptions::read()),
        Err(Errno::Notcapable)
    );
    assert_eq!(
        ctx.path_open(WasiFd::ROOT, "/abs", WasiOpenOptions::read()),
        Err(Errno::Notcapable)
    );
    assert_eq!(
        ctx.path_open(WasiFd::ROOT, "", WasiOpenOptions::read()),
        Err(Errno::Notcapable)
    );
    assert_eq!(
        ctx.path_open(WasiFd::ROOT, "bad/path/", WasiOpenOptions::read()),
        Err(Errno::Notcapable)
    );
    assert_eq!(
        ctx.path_open(WasiFd::ROOT, "bad//path", WasiOpenOptions::read()),
        Err(Errno::Notcapable)
    );
    assert_eq!(
        ctx.path_open(WasiFd::ROOT, "bad/./path", WasiOpenOptions::read()),
        Err(Errno::Notcapable)
    );
    assert_eq!(
        ctx.path_open(WasiFd::ROOT, "bad\\path", WasiOpenOptions::read()),
        Err(Errno::Notcapable)
    );
    assert_eq!(
        ctx.path_open(WasiFd::ROOT, "bad\0path", WasiOpenOptions::read()),
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
    assert_eq!(Errno::from(FsError::InvalidOffset), Errno::Inval);
    assert_eq!(Errno::from(FsError::InvalidTime), Errno::Inval);
    assert_eq!(Errno::from(FsError::NotEmpty), Errno::Notempty);
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
    assert_eq!(Errno::Notempty.preview1_code(), 55);
    assert_eq!(Errno::Notcapable.preview1_code(), 76);

    assert_eq!(WasiFileType::Unknown.preview1_code(), 0);
    assert_eq!(WasiFileType::CharacterDevice.preview1_code(), 2);
    assert_eq!(WasiFileType::Directory.preview1_code(), 3);
    assert_eq!(WasiFileType::RegularFile.preview1_code(), 4);
    assert_eq!(WasiFileType::SymbolicLink.preview1_code(), 7);
    assert_eq!(WasiRights::PATH_CREATE_DIRECTORY.bits(), 1 << 9);
    assert_eq!(WasiRights::PATH_CREATE_FILE.bits(), 1 << 10);
    assert_eq!(WasiRights::PATH_OPEN.bits(), 1 << 13);
    assert_eq!(WasiRights::FD_READDIR.bits(), 1 << 14);
    assert_eq!(WasiRights::PATH_READLINK.bits(), 1 << 15);
    assert_eq!(WasiRights::PATH_RENAME_SOURCE.bits(), 1 << 16);
    assert_eq!(WasiRights::PATH_RENAME_TARGET.bits(), 1 << 17);
    assert_eq!(WasiRights::PATH_FILESTAT_GET.bits(), 1 << 18);
    assert_eq!(WasiRights::PATH_FILESTAT_SET_SIZE.bits(), 1 << 19);
    assert_eq!(WasiRights::PATH_FILESTAT_SET_TIMES.bits(), 1 << 20);
    assert_eq!(WasiRights::FD_FILESTAT_GET.bits(), 1 << 21);
    assert_eq!(WasiRights::FD_FILESTAT_SET_SIZE.bits(), 1 << 22);
    assert_eq!(WasiRights::FD_FILESTAT_SET_TIMES.bits(), 1 << 23);
    assert_eq!(WasiRights::PATH_SYMLINK.bits(), 1 << 24);
    assert_eq!(WasiRights::PATH_REMOVE_DIRECTORY.bits(), 1 << 25);
    assert_eq!(WasiRights::PATH_UNLINK_FILE.bits(), 1 << 26);
    assert!(
        WasiRights::DIRECTORY_BASE.contains(WasiRights::PATH_RENAME_SOURCE)
            && WasiRights::DIRECTORY_BASE.contains(WasiRights::PATH_RENAME_TARGET)
    );
    assert_eq!(WasiOpenOptions::OFLAGS_CREATE, 1 << 0);
    assert_eq!(WasiOpenOptions::OFLAGS_DIRECTORY, 1 << 1);
    assert_eq!(WasiOpenOptions::OFLAGS_EXCLUSIVE, 1 << 2);
    assert_eq!(WasiOpenOptions::OFLAGS_TRUNCATE, 1 << 3);
    assert_eq!(WasiOpenOptions::FDFLAGS_APPEND, 1 << 0);
    assert_eq!(WasiOpenOptions::FDFLAGS_DSYNC, 1 << 1);
    assert_eq!(WasiOpenOptions::FDFLAGS_NONBLOCK, 1 << 2);
    assert_eq!(WasiOpenOptions::FDFLAGS_RSYNC, 1 << 3);
    assert_eq!(WasiOpenOptions::FDFLAGS_SYNC, 1 << 4);
    assert_eq!(WasiFilestatSetTimes::ATIM, 1 << 0);
    assert_eq!(WasiFilestatSetTimes::ATIM_NOW, 1 << 1);
    assert_eq!(WasiFilestatSetTimes::MTIM, 1 << 2);
    assert_eq!(WasiFilestatSetTimes::MTIM_NOW, 1 << 3);
    assert_eq!(FileStat::PREVIEW1_SIZE, 64);
}
