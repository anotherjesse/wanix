use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
    mpsc,
};
use std::time::Duration;

use wanix_fs::{
    File, FileSeekFrom, FileSystem, FileType, FsError, MemFs, Metadata, NormalizedPath, OpenOptions,
};
use wanix_vfs::BindOptions;

use crate::{
    CRATE_PURPOSE, Fd, NoopDriver, Task, TaskDriver, TaskId, TaskSpec, TaskTable, quote_cmd_argv,
};

fn read_file(fs: &dyn FileSystem, path: &str) -> String {
    let mut file = fs
        .open(&NormalizedPath::new(path).unwrap(), OpenOptions::read())
        .unwrap();
    let mut out = Vec::new();
    let mut buf = [0; 16];
    loop {
        let n = file.read(&mut buf).unwrap();
        if n == 0 {
            break;
        }
        out.extend_from_slice(&buf[..n]);
    }
    String::from_utf8(out).unwrap()
}

fn write_file(fs: &dyn FileSystem, path: &str, bytes: &[u8]) {
    let mut file = fs
        .open(
            &NormalizedPath::new(path).unwrap(),
            OpenOptions {
                read: false,
                write: true,
                create: false,
                truncate: false,
            },
        )
        .unwrap();
    file.write(bytes).unwrap();
}

fn open_write(fs: &dyn FileSystem, path: &str) -> Box<dyn File> {
    fs.open(
        &NormalizedPath::new(path).unwrap(),
        OpenOptions {
            read: false,
            write: true,
            create: false,
            truncate: false,
        },
    )
    .unwrap()
}

fn entry_names(fs: &dyn FileSystem, path: &str) -> Vec<String> {
    fs.read_dir(&NormalizedPath::new(path).unwrap())
        .unwrap()
        .into_iter()
        .map(|entry| entry.name().to_owned())
        .collect()
}

#[test]
fn purpose_is_declared() {
    assert!(!CRATE_PURPOSE.is_empty());
}

#[test]
fn task_spec_uses_explicit_program_args_env_and_cwd() {
    let mut spec = TaskSpec::new("bin/app").unwrap();
    spec.args.push("--help".to_owned());
    spec.env.insert("KEY".to_owned(), "value".to_owned());

    assert_eq!(spec.program.as_str(), "bin/app");
    assert_eq!(spec.args, ["--help"]);
    assert_eq!(spec.env["KEY"], "value");
    assert_eq!(spec.cwd.as_str(), ".");
}

#[test]
fn task_carries_id_spec_and_namespace() {
    let task = Task::new(
        TaskId::new(1),
        TaskSpec::new("bin/app").unwrap(),
        wanix_vfs::Namespace::new(),
    );

    assert_eq!(task.id().get(), 1);
    assert_eq!(task.spec().program.as_str(), "bin/app");
    assert!(task.namespace().bindings().is_empty());
    assert_eq!(Fd::STDOUT.get(), 1);
}

#[test]
fn task_debug_summarizes_state_without_environment_or_namespace() {
    let task = Task::new(
        TaskId::new(7),
        TaskSpec::new("bin/app").unwrap(),
        wanix_vfs::Namespace::new(),
    );
    task.set_cmd("bin/app --flag").unwrap();
    task.set_env_lines("SECRET=value").unwrap();
    task.set_dir("work").unwrap();
    task.set_exit("13").unwrap();
    task.bind(
        Arc::new(MemFs::new()),
        ".",
        "mounted-secret",
        BindOptions::default(),
    )
    .unwrap();

    let debug = format!("{task:?}");

    assert!(debug.contains("Task"));
    assert!(debug.contains("id: TaskId(7)"));
    assert!(debug.contains("kind: \"manual\""));
    assert!(debug.contains("cmd: \"bin/app --flag\""));
    assert!(debug.contains("dir: NormalizedPath(\"work\")"));
    assert!(debug.contains("exit: \"13\""));
    assert!(!debug.contains("env"));
    assert!(!debug.contains("namespace"));
    assert!(!debug.contains("spec"));
    assert!(!debug.contains("SECRET=value"));
    assert!(!debug.contains("mounted-secret"));
}

#[test]
fn task_table_debug_summarizes_task_and_driver_keys() {
    let table = TaskTable::new();
    table.register_noop_driver("qjs").unwrap();
    table.register_noop_driver("noop").unwrap();
    table.allocate_root("qjs").unwrap();
    table.allocate_root("noop").unwrap();

    let debug = format!("{table:?}");

    assert!(debug.contains("TaskTable"));
    assert!(debug.contains("next_id: 2"));
    assert!(debug.contains("tasks: [TaskId(1), TaskId(2)]"));
    assert!(debug.contains("drivers: [\"noop\", \"qjs\"]"));
    assert!(!debug.contains("Task {"));
}

#[test]
fn task_spec_can_be_replaced_without_mutating_task_fields() {
    let task = Task::new(
        TaskId::new(1),
        TaskSpec::new("bin/app").unwrap(),
        wanix_vfs::Namespace::new(),
    );
    task.set_cmd("raw command").unwrap();
    task.set_env_lines("RAW=1").unwrap();
    task.set_dir("raw-cwd").unwrap();
    let mut spec = TaskSpec::new("bin/other").unwrap();
    spec.args = vec!["two words".to_owned()];
    spec.env.insert("SPEC".to_owned(), "1".to_owned());
    spec.cwd = NormalizedPath::new("spec-cwd").unwrap();

    task.set_spec(spec.clone()).unwrap();

    assert_eq!(task.spec(), spec);
    assert_eq!(task.cmd(), "raw command");
    assert_eq!(task.env(), ["RAW=1"]);
    assert_eq!(task.dir().as_str(), "raw-cwd");
}

#[test]
fn task_table_allocates_root_and_self_taskfs_view() {
    let table = TaskTable::new();
    table.register_noop_driver("qjs").unwrap();

    let root = table.allocate_root("qjs").unwrap();
    let taskfs = table.filesystem_for(root.id());

    assert_eq!(root.id().get(), 1);
    assert_eq!(read_file(&taskfs, "1/id"), "1\n");
    assert_eq!(read_file(&taskfs, "self/id"), "1\n");
    assert_eq!(read_file(&root.namespace(), "#task/self/id"), "1\n");
    assert_eq!(entry_names(&taskfs, "new"), ["auto", "qjs"]);
    assert_eq!(entry_names(&taskfs, "."), ["1", "new", "self"]);
}

#[test]
fn task_field_files_support_seek_and_tell() {
    let table = TaskTable::new();
    table.register_noop_driver("qjs").unwrap();
    let root = table.allocate_root("qjs").unwrap();
    let taskfs = table.filesystem_for(root.id());
    let mut file = taskfs
        .open(
            &NormalizedPath::new("self/id").unwrap(),
            OpenOptions::read(),
        )
        .unwrap();
    let mut buf = [0; 8];

    assert!(file.is_seekable());
    assert_eq!(file.tell().unwrap(), 0);
    assert_eq!(
        file.seek(FileSeekFrom::Current(-1)),
        Err(FsError::InvalidOffset)
    );
    assert_eq!(file.seek(FileSeekFrom::End(-1)).unwrap(), 1);
    let count = file.read(&mut buf).unwrap();
    assert_eq!(&buf[..count], b"\n");
    assert_eq!(file.seek(FileSeekFrom::Start(0)).unwrap(), 0);
    let count = file.read(&mut buf).unwrap();
    assert_eq!(&buf[..count], b"1\n");

    let write_only = taskfs
        .open(
            &NormalizedPath::new("self/cmd").unwrap(),
            OpenOptions {
                write: true,
                ..OpenOptions::default()
            },
        )
        .unwrap();
    assert!(!write_only.is_seekable());
    assert_eq!(write_only.tell(), Err(FsError::PermissionDenied));
}

#[test]
fn task_new_allocates_child_and_clones_parent_namespace() {
    let table = TaskTable::new();
    table.register_noop_driver("qjs").unwrap();
    let root = table.allocate_root("qjs").unwrap();
    let shared = Arc::new(MemFs::new());
    shared.write_file("hello.txt", b"hello").unwrap();
    root.bind(shared, ".", "mnt", BindOptions::default())
        .unwrap();
    let taskfs = table.filesystem_for(root.id());

    assert_eq!(read_file(&taskfs, "new/qjs"), "2\n");
    let child = table.get(TaskId::new(2)).unwrap();

    assert_eq!(child.parent_id(), Some(root.id()));
    assert_eq!(read_file(&child.namespace(), "mnt/hello.txt"), "hello");
    assert_eq!(read_file(&root.namespace(), "#task/self/id"), "1\n");
    assert_eq!(read_file(&child.namespace(), "#task/self/id"), "2\n");
    assert!(matches!(
        taskfs.open(
            &NormalizedPath::new("new/unknown").unwrap(),
            OpenOptions::read()
        ),
        Err(FsError::NotFound)
    ));
}

#[test]
fn task_new_file_is_seekable_for_wasi_libc_reads() {
    let table = TaskTable::new();
    table.register_noop_driver("qjs").unwrap();
    let root = table.allocate_root("qjs").unwrap();
    let taskfs = table.filesystem_for(root.id());
    let mut new_qjs = taskfs
        .open(
            &NormalizedPath::new("new/qjs").unwrap(),
            OpenOptions::read(),
        )
        .unwrap();

    assert!(new_qjs.is_seekable());
    assert_eq!(new_qjs.tell().unwrap(), 0);
    let mut buf = [0; 8];
    assert_eq!(new_qjs.read(&mut buf).unwrap(), 2);
    assert_eq!(&buf[..2], b"2\n");
    assert_eq!(new_qjs.tell().unwrap(), 2);
    assert_eq!(new_qjs.seek(FileSeekFrom::Start(0)).unwrap(), 0);
    assert_eq!(new_qjs.read(&mut buf).unwrap(), 2);
    assert_eq!(&buf[..2], b"2\n");
}

#[test]
fn task_field_files_round_trip() {
    let table = TaskTable::new();
    table.register_noop_driver("qjs").unwrap();
    let task = table.allocate_root("qjs").unwrap();
    let taskfs = table.filesystem_for(task.id());

    write_file(&taskfs, "self/cmd", b"script.js --flag\n");
    write_file(&taskfs, "self/env", b"A=1\nB=2\n");
    write_file(&taskfs, "self/dir", b"src\n");
    write_file(&taskfs, "self/exit", b"0\n");

    assert_eq!(read_file(&taskfs, "self/cmd"), "script.js --flag\n");
    assert_eq!(read_file(&taskfs, "self/env"), "A=1\nB=2\n");
    assert_eq!(read_file(&taskfs, "self/dir"), "src\n");
    assert_eq!(read_file(&taskfs, "self/exit"), "0\n");
    assert!(matches!(
        taskfs
            .open(
                &NormalizedPath::new("self/dir").unwrap(),
                OpenOptions {
                    read: false,
                    write: true,
                    create: false,
                    truncate: false,
                }
            )
            .unwrap()
            .write(b"../bad"),
        Err(FsError::InvalidPath(_))
    ));
}

#[test]
fn task_field_files_accumulate_split_writes() {
    let table = TaskTable::new();
    table.register_noop_driver("qjs").unwrap();
    let task = table.allocate_root("qjs").unwrap();
    let taskfs = table.filesystem_for(task.id());

    let mut env = open_write(&taskfs, "self/env");
    env.write(b"A=").unwrap();
    env.write(b"1\nB=2\n").unwrap();

    let mut dir = open_write(&taskfs, "self/dir");
    dir.write(b"sr").unwrap();
    dir.write(b"c\n").unwrap();

    let mut exit = open_write(&taskfs, "self/exit");
    exit.write(b"  ").unwrap();
    exit.write(b"5\n").unwrap();

    assert_eq!(read_file(&taskfs, "self/env"), "A=1\nB=2\n");
    assert_eq!(read_file(&taskfs, "self/dir"), "src\n");
    assert_eq!(read_file(&taskfs, "self/exit"), "5\n");
}

#[test]
fn task_cmd_file_parses_shell_quoted_argv_without_replacing_raw_text() {
    let table = TaskTable::new();
    table.register_noop_driver("qjs").unwrap();
    let task = table.allocate_root("qjs").unwrap();
    let taskfs = table.filesystem_for(task.id());

    write_file(
        &taskfs,
        "self/cmd",
        br#"script.js 'two words' '' 'quote'"'"'test' plain\ arg
"#,
    );

    assert_eq!(
        read_file(&taskfs, "self/cmd"),
        "script.js 'two words' '' 'quote'\"'\"'test' plain\\ arg\n"
    );
    assert_eq!(
        task.cmd_argv().unwrap(),
        ["script.js", "two words", "", "quote'test", "plain arg"]
    );
}

#[test]
fn task_cmd_quote_helper_round_trips_through_parser() {
    let command = quote_cmd_argv(["script.js", "two words", "", "quote'test", "plain\\arg"]);

    assert_eq!(
        command,
        "script.js 'two words' '' 'quote'\"'\"'test' 'plain\\arg'"
    );
    assert_eq!(
        crate::cmd::parse_cmd_argv(&command).unwrap().unwrap(),
        ["script.js", "two words", "", "quote'test", "plain\\arg"]
    );
}

#[test]
fn task_cmd_file_rejects_unclosed_quotes() {
    let table = TaskTable::new();
    table.register_noop_driver("qjs").unwrap();
    let task = table.allocate_root("qjs").unwrap();
    let taskfs = table.filesystem_for(task.id());

    let err = taskfs
        .open(
            &NormalizedPath::new("self/cmd").unwrap(),
            OpenOptions {
                write: true,
                ..OpenOptions::default()
            },
        )
        .unwrap()
        .write(b"script.js 'unterminated")
        .unwrap_err();

    assert!(err.to_string().contains("unterminated quote"));
    assert_eq!(task.cmd(), "");
    assert_eq!(task.cmd_argv(), None);
}

#[test]
fn taskfs_open_flags_are_capabilities() {
    let table = TaskTable::new();
    table.register_noop_driver("qjs").unwrap();
    let task = table.allocate_root("qjs").unwrap();
    let taskfs = table.filesystem_for(task.id());
    let backing = MemFs::new();
    backing.write_file("stdout", b"").unwrap();
    task.insert_fd(
        Fd::STDOUT,
        backing
            .open(
                &NormalizedPath::new("stdout").unwrap(),
                OpenOptions::read_write(),
            )
            .unwrap(),
        NormalizedPath::new("stdout").unwrap(),
    )
    .unwrap();

    assert!(matches!(
        taskfs
            .open(
                &NormalizedPath::new("self/cmd").unwrap(),
                OpenOptions::read()
            )
            .unwrap()
            .write(b"nope"),
        Err(FsError::PermissionDenied)
    ));
    assert!(matches!(
        open_write(&taskfs, "self/cmd").read(&mut [0; 4]),
        Err(FsError::PermissionDenied)
    ));
    assert!(matches!(
        taskfs
            .open(
                &NormalizedPath::new("self/ctl").unwrap(),
                OpenOptions::read()
            )
            .unwrap()
            .write(b"start"),
        Err(FsError::PermissionDenied)
    ));
    assert!(matches!(
        taskfs.open(
            &NormalizedPath::new("new/qjs").unwrap(),
            OpenOptions {
                read: false,
                write: true,
                create: false,
                truncate: false,
            }
        ),
        Err(FsError::PermissionDenied)
    ));
    assert!(matches!(
        taskfs
            .open(
                &NormalizedPath::new("self/fd/1").unwrap(),
                OpenOptions::read()
            )
            .unwrap()
            .write(b"nope"),
        Err(FsError::PermissionDenied)
    ));
    assert!(matches!(
        open_write(&taskfs, "self/fd/1").read(&mut [0; 4]),
        Err(FsError::PermissionDenied)
    ));
}

#[derive(Debug)]
struct CountingDriver {
    starts: AtomicUsize,
}

impl CountingDriver {
    fn new() -> Self {
        Self {
            starts: AtomicUsize::new(0),
        }
    }

    fn starts(&self) -> usize {
        self.starts.load(Ordering::SeqCst)
    }
}

impl TaskDriver for CountingDriver {
    fn start(&self, task: &Task) -> wanix_fs::FsResult<()> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        task.set_exit("0")
    }
}

#[test]
fn ctl_start_invokes_registered_driver() {
    let table = TaskTable::new();
    let driver = Arc::new(CountingDriver::new());
    table.register_driver("qjs", driver.clone()).unwrap();
    let task = table.allocate_root("qjs").unwrap();
    let taskfs = table.filesystem_for(task.id());

    write_file(&taskfs, "self/ctl", b"start");

    assert_eq!(driver.starts(), 1);
    assert_eq!(read_file(&taskfs, "self/exit"), "0\n");
}

#[derive(Debug)]
struct SlowExitDriver;

impl TaskDriver for SlowExitDriver {
    fn start(&self, task: &Task) -> wanix_fs::FsResult<()> {
        std::thread::sleep(std::time::Duration::from_millis(50));
        task.set_exit("7")
    }
}

#[test]
fn ctl_start_detached_returns_before_exit_and_wait_blocks_until_done() {
    let table = TaskTable::new();
    table
        .register_driver("slow", Arc::new(SlowExitDriver))
        .unwrap();
    let task = table.allocate_root("slow").unwrap();
    let taskfs = table.filesystem_for(task.id());

    write_file(&taskfs, "self/ctl", b"start &");
    // The detached start returned while the driver is still sleeping on its
    // own thread: no exit yet, and the wait file reports not-ready.
    assert_eq!(task.exit(), "", "start & must not run the task inline");
    let wait = taskfs
        .open(
            &NormalizedPath::new("self/wait").unwrap(),
            OpenOptions::read(),
        )
        .unwrap();
    assert!(!wait.read_ready().unwrap(), "no exit recorded yet");
    drop(wait);

    // A wait read parks until the driver records the exit, then serves it.
    assert_eq!(read_file(&taskfs, "self/wait"), "7\n");
    assert_eq!(read_file(&taskfs, "self/exit"), "7\n");
}

#[test]
fn detached_start_records_synthetic_exit_when_driver_records_none() {
    // NoopDriver::start records no exit; waiters must still wake.
    let table = TaskTable::new();
    table.register_noop_driver("noop").unwrap();
    let task = table.allocate_root("noop").unwrap();
    table.start_detached(task.id()).unwrap();
    assert_eq!(task.wait_exit().unwrap(), "0");
}

#[test]
fn second_start_of_one_task_is_rejected_and_does_not_rerun_the_driver() {
    // Two starts of one task would run the driver twice against a shared fd
    // table and overwrite the recorded exit; the duplicate must be an error.
    let table = TaskTable::new();
    let driver = Arc::new(CountingDriver::new());
    table.register_driver("qjs", driver.clone()).unwrap();
    let task = table.allocate_root("qjs").unwrap();

    table.start(task.id()).unwrap();
    let err = table.start(task.id()).unwrap_err();

    assert!(
        err.to_string().contains("already started"),
        "unexpected error: {err}"
    );
    assert_eq!(driver.starts(), 1, "the driver must run exactly once");
    assert_eq!(task.exit(), "0");
}

#[test]
fn duplicate_detached_start_is_rejected_without_touching_the_live_run() {
    // `start &` returns immediately, so a duplicate is easy to race in. The
    // second one must fail synchronously — before any thread spawns — so it
    // can never close the live run's fds or overwrite its exit.
    let table = TaskTable::new();
    table
        .register_driver("slow", Arc::new(SlowExitDriver))
        .unwrap();
    let task = table.allocate_root("slow").unwrap();

    table.start_detached(task.id()).unwrap();
    let err = table.start_detached(task.id()).unwrap_err();

    assert!(
        err.to_string().contains("already started"),
        "unexpected error: {err}"
    );
    assert_eq!(task.wait_exit().unwrap(), "7", "first run's exit survives");
}

#[derive(Debug)]
struct PanickingDriver;

impl TaskDriver for PanickingDriver {
    fn start(&self, _task: &Task) -> wanix_fs::FsResult<()> {
        panic!("driver blew up");
    }
}

#[test]
fn detached_start_survives_a_panicking_driver() {
    // The no-hang contract: even when the driver panics, the detached thread
    // must release the task's fds (pipe peers see EOF/broken-pipe) and record
    // a synthesized 127 (waiters wake) instead of silently unwinding past both.
    let table = TaskTable::new();
    table
        .register_driver("boom", Arc::new(PanickingDriver))
        .unwrap();
    let task = table.allocate_root("boom").unwrap();
    let fs = Arc::new(MemFs::new());
    fs.write_file("out", b"").unwrap();
    task.insert_fd(
        Fd::STDOUT,
        fs.open(
            &NormalizedPath::new("out").unwrap(),
            OpenOptions::read_write(),
        )
        .unwrap(),
        NormalizedPath::new("out").unwrap(),
    )
    .unwrap();

    table.start_detached(task.id()).unwrap();

    assert_eq!(task.wait_exit().unwrap(), "127", "waiters must wake");
    assert!(
        task.fd_path(Fd::STDOUT).is_err(),
        "fds must be released after the panic"
    );
}

#[test]
fn ctl_bind_installs_fd_from_task_namespace() {
    let table = TaskTable::new();
    table.register_noop_driver("qjs").unwrap();
    let task = table.allocate_root("qjs").unwrap();
    let root = Arc::new(MemFs::new());
    root.write_file("stdout", b"").unwrap();
    task.bind(root.clone(), ".", ".", BindOptions::default())
        .unwrap();
    let taskfs = table.filesystem_for(task.id());

    write_file(&taskfs, "self/ctl", b"bind stdout fd/1\n");

    assert_eq!(entry_names(&taskfs, "self/fd"), ["1"]);
    write_file(&taskfs, "self/fd/1", b"from fd bind");
    assert_eq!(root.read_file("stdout").unwrap(), b"from fd bind");
    assert_eq!(task.fd_path(Fd::STDOUT).unwrap().as_str(), "stdout");
}

#[test]
fn ctl_bind_accepts_explicit_open_mode() {
    // The optional 4th token selects the open mode (r/w/rw). Write-only binds are
    // what let a #pipe write end (which rejects a read-write open) land on fd 1.
    let table = TaskTable::new();
    table.register_noop_driver("qjs").unwrap();
    let task = table.allocate_root("qjs").unwrap();
    let root = Arc::new(MemFs::new());
    root.write_file("stdout", b"").unwrap();
    task.bind(root.clone(), ".", ".", BindOptions::default())
        .unwrap();
    let taskfs = table.filesystem_for(task.id());

    write_file(&taskfs, "self/ctl", b"bind stdout fd/1 w\n");

    assert_eq!(entry_names(&taskfs, "self/fd"), ["1"]);
    write_file(&taskfs, "self/fd/1", b"written via w-mode bind");
    assert_eq!(
        root.read_file("stdout").unwrap(),
        b"written via w-mode bind"
    );
}

#[test]
fn ctl_bind_parses_shell_quoted_source_paths() {
    let table = TaskTable::new();
    table.register_noop_driver("qjs").unwrap();
    let task = table.allocate_root("qjs").unwrap();
    let root = Arc::new(MemFs::new());
    root.write_file("child stdin.txt", b"input with spaces")
        .unwrap();
    task.bind(root.clone(), ".", ".", BindOptions::default())
        .unwrap();
    let taskfs = table.filesystem_for(task.id());

    write_file(&taskfs, "self/ctl", b"bind 'child stdin.txt' fd/0\n");

    assert_eq!(entry_names(&taskfs, "self/fd"), ["0"]);
    assert_eq!(read_file(&taskfs, "self/fd/0"), "input with spaces");
    assert_eq!(task.fd_path(Fd::STDIN).unwrap().as_str(), "child stdin.txt");
}

#[test]
fn ctl_bind_accepts_self_addressed_fd_destinations() {
    let table = TaskTable::new();
    table.register_noop_driver("qjs").unwrap();
    let task = table.allocate_root("qjs").unwrap();
    let root = Arc::new(MemFs::new());
    root.write_file("stderr", b"").unwrap();
    task.bind(root.clone(), ".", ".", BindOptions::default())
        .unwrap();
    let taskfs = table.filesystem_for(task.id());

    write_file(
        &taskfs,
        "self/ctl",
        format!("bind stderr #task/{}/fd/2\n", task.id().get()).as_bytes(),
    );

    write_file(&taskfs, "self/fd/2", b"self addressed");
    assert_eq!(root.read_file("stderr").unwrap(), b"self addressed");
}

#[test]
fn ctl_bind_can_wire_child_fd_to_explicit_parent_fd() {
    let table = TaskTable::new();
    table.register_noop_driver("qjs").unwrap();
    let parent = table.allocate_root("qjs").unwrap();
    let backing = Arc::new(MemFs::new());
    backing.write_file("stdout", b"").unwrap();
    backing.write_file("replacement", b"").unwrap();
    parent
        .insert_fd(
            Fd::STDOUT,
            backing
                .open(
                    &NormalizedPath::new("stdout").unwrap(),
                    OpenOptions::read_write(),
                )
                .unwrap(),
            NormalizedPath::new("stdout").unwrap(),
        )
        .unwrap();
    let taskfs = table.filesystem_for(parent.id());

    assert_eq!(read_file(&taskfs, "new/qjs"), "2\n");
    let child = table.get(TaskId::new(2)).unwrap();
    write_file(
        &taskfs,
        "2/ctl",
        format!("bind #task/{}/fd/1 fd/1\n", parent.id().get()).as_bytes(),
    );
    parent.close_fd(Fd::STDOUT).unwrap();
    parent
        .insert_fd(
            Fd::STDOUT,
            backing
                .open(
                    &NormalizedPath::new("replacement").unwrap(),
                    OpenOptions::read_write(),
                )
                .unwrap(),
            NormalizedPath::new("replacement").unwrap(),
        )
        .unwrap();
    write_file(&taskfs, "2/fd/1", b"via parent fd");

    assert_eq!(parent.fd_numbers(), [Fd::STDOUT]);
    assert_eq!(child.fd_numbers(), [Fd::STDOUT]);
    assert_eq!(backing.read_file("stdout").unwrap(), b"via parent fd");
    assert!(backing.read_file("replacement").unwrap().is_empty());
}

#[test]
fn split_writes_accumulate_for_fields_and_ctl() {
    let table = TaskTable::new();
    let driver = Arc::new(CountingDriver::new());
    table.register_driver("qjs", driver.clone()).unwrap();
    let task = table.allocate_root("qjs").unwrap();
    let root = Arc::new(MemFs::new());
    root.write_file("stdout", b"").unwrap();
    task.bind(root.clone(), ".", ".", BindOptions::default())
        .unwrap();
    let taskfs = table.filesystem_for(task.id());

    let mut cmd = open_write(&taskfs, "self/cmd");
    cmd.write(b"script").unwrap();
    cmd.write(b".js --flag\n").unwrap();
    assert_eq!(read_file(&taskfs, "self/cmd"), "script.js --flag\n");

    let mut ctl = open_write(&taskfs, "self/ctl");
    ctl.write(b"sta").unwrap();
    assert_eq!(driver.starts(), 0);
    ctl.write(b"rt").unwrap();
    assert_eq!(driver.starts(), 1);

    let mut ctl = open_write(&taskfs, "self/ctl");
    ctl.write(b"bind stdout ").unwrap();
    assert!(task.fd_numbers().is_empty());
    ctl.write(b"fd/1\n").unwrap();
    assert_eq!(task.fd_numbers(), [Fd::STDOUT]);
    write_file(&taskfs, "self/fd/1", b"split bind");
    assert_eq!(root.read_file("stdout").unwrap(), b"split bind");

    root.write_file("child stdin.txt", b"split quoted bind")
        .unwrap();
    let mut ctl = open_write(&taskfs, "self/ctl");
    ctl.write(b"bind 'child ").unwrap();
    assert_eq!(task.fd_numbers(), [Fd::STDOUT]);
    ctl.write(b"stdin.txt' fd/0\n").unwrap();
    assert_eq!(task.fd_numbers(), [Fd::STDIN, Fd::STDOUT]);
    assert_eq!(read_file(&taskfs, "self/fd/0"), "split quoted bind");
}

#[derive(Debug)]
struct MatchingDriver {
    starts: AtomicUsize,
}

impl MatchingDriver {
    fn new() -> Self {
        Self {
            starts: AtomicUsize::new(0),
        }
    }
}

impl TaskDriver for MatchingDriver {
    fn check(&self, task: &Task) -> bool {
        task.cmd().contains("run-me")
    }

    fn start(&self, task: &Task) -> wanix_fs::FsResult<()> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        task.set_exit("auto-ok")
    }
}

#[test]
fn auto_task_selects_matching_driver_outside_table_lock() {
    let table = TaskTable::new();
    let driver = Arc::new(MatchingDriver::new());
    table.register_driver("qjs", driver).unwrap();
    let task = table.allocate_root("auto").unwrap();

    task.set_cmd("run-me").unwrap();
    table.start(task.id()).unwrap();

    assert_eq!(task.kind(), "qjs");
    assert_eq!(task.exit(), "auto-ok");
}

#[test]
fn fd_table_allocates_dynamic_fds_and_taskfs_proxies_io() {
    let table = TaskTable::new();
    table.register_noop_driver("qjs").unwrap();
    let task = table.allocate_root("qjs").unwrap();
    let backing = MemFs::new();
    backing.write_file("stdout", b"").unwrap();
    let stdout = backing
        .open(
            &NormalizedPath::new("stdout").unwrap(),
            OpenOptions::read_write(),
        )
        .unwrap();

    task.insert_fd(Fd::STDOUT, stdout, NormalizedPath::new("stdout").unwrap())
        .unwrap();
    let data = MemFs::new();
    data.write_file("data", b"abc").unwrap();
    let fd = task
        .open_fd(
            data.open(&NormalizedPath::new("data").unwrap(), OpenOptions::read())
                .unwrap(),
            NormalizedPath::new("data").unwrap(),
        )
        .unwrap();
    let taskfs = table.filesystem_for(task.id());

    assert_eq!(fd.get(), 3);
    assert_eq!(entry_names(&taskfs, "self/fd"), ["1", "3"]);
    write_file(&taskfs, "self/fd/1", b"hello");
    assert_eq!(backing.read_file("stdout").unwrap(), b"hello");
    assert_eq!(read_file(&taskfs, "self/fd/3"), "abc");

    let mut open_service_file = taskfs
        .open(
            &NormalizedPath::new("self/fd/3").unwrap(),
            OpenOptions::read(),
        )
        .unwrap();
    let mut eof_buf = [0; 1];
    assert_eq!(open_service_file.read(&mut eof_buf).unwrap(), 0);
    open_service_file.seek(FileSeekFrom::Start(0)).unwrap();
    task.close_fd(fd).unwrap();
    let mut buf = [0; 8];
    let count = open_service_file.read(&mut buf).unwrap();
    assert_eq!(&buf[..count], b"abc");
    assert!(matches!(
        taskfs.open(
            &NormalizedPath::new("self/fd/3").unwrap(),
            OpenOptions::read()
        ),
        Err(FsError::InvalidFd)
    ));
}

#[test]
fn fd_table_non_replacing_insert_preserves_existing_fd() {
    let table = TaskTable::new();
    table.register_noop_driver("qjs").unwrap();
    let task = table.allocate_root("qjs").unwrap();
    let backing = MemFs::new();
    backing.write_file("existing", b"first").unwrap();
    backing.write_file("candidate", b"second").unwrap();
    task.insert_fd(
        Fd::new(4),
        backing
            .open(
                &NormalizedPath::new("existing").unwrap(),
                OpenOptions::read(),
            )
            .unwrap(),
        NormalizedPath::new("existing").unwrap(),
    )
    .unwrap();

    let result = task.insert_fd_if_vacant(
        Fd::new(4),
        backing
            .open(
                &NormalizedPath::new("candidate").unwrap(),
                OpenOptions::read(),
            )
            .unwrap(),
        NormalizedPath::new("candidate").unwrap(),
    );

    assert_eq!(result, Err(FsError::AlreadyExists));
    assert_eq!(task.fd_path(Fd::new(4)).unwrap().as_str(), "existing");
    let mut buf = [0; 16];
    let count = task.read_fd(Fd::new(4), &mut buf).unwrap();
    assert_eq!(&buf[..count], b"first");
}

#[test]
fn close_all_fds_releases_open_fds() {
    // A finished task releases its fds (Unix/Plan 9 semantics); this is what
    // drops a pipeline producer's #pipe writer so the consumer observes EOF.
    let table = TaskTable::new();
    table.register_noop_driver("noop").unwrap();
    let task = table.allocate_root("noop").unwrap();

    let backing = MemFs::new();
    backing.write_file("f", b"x").unwrap();
    task.insert_fd(
        Fd::STDOUT,
        backing
            .open(
                &NormalizedPath::new("f").unwrap(),
                OpenOptions::read_write(),
            )
            .unwrap(),
        NormalizedPath::new("f").unwrap(),
    )
    .unwrap();
    assert_eq!(task.fd_numbers(), vec![Fd::STDOUT]);

    task.close_all_fds();
    assert!(
        task.fd_numbers().is_empty(),
        "a finished task should hold no fds"
    );
}

#[derive(Debug)]
struct ReentrantFile {
    task: Task,
}

impl File for ReentrantFile {
    fn read_ready(&self) -> wanix_fs::FsResult<bool> {
        self.task.set_exit("read-ready-reentered")?;
        Ok(false)
    }

    fn read(&mut self, buf: &mut [u8]) -> wanix_fs::FsResult<usize> {
        self.task.set_exit("reentered")?;
        let bytes = b"ok";
        buf[..bytes.len()].copy_from_slice(bytes);
        Ok(bytes.len())
    }

    fn write_ready(&self) -> wanix_fs::FsResult<bool> {
        self.task.set_exit("write-ready-reentered")?;
        Ok(true)
    }

    fn metadata(&self) -> wanix_fs::FsResult<Metadata> {
        Ok(Metadata::new(FileType::File, 2, 0o644))
    }
}

#[test]
fn fd_io_does_not_hold_task_lock_while_calling_file() {
    let table = TaskTable::new();
    table.register_noop_driver("qjs").unwrap();
    let task = table.allocate_root("qjs").unwrap();
    let fd = task
        .open_fd(
            Box::new(ReentrantFile { task: task.clone() }),
            NormalizedPath::new("reentrant").unwrap(),
        )
        .unwrap();
    let (tx, rx) = mpsc::channel();
    let reader = task.clone();

    std::thread::spawn(move || {
        let mut buf = [0; 2];
        let result = reader.read_fd(fd, &mut buf).map(|n| (n, buf));
        tx.send(result).unwrap();
    });

    let (count, buf) = rx.recv_timeout(Duration::from_secs(2)).unwrap().unwrap();
    assert_eq!(count, 2);
    assert_eq!(&buf, b"ok");
    assert_eq!(task.exit(), "reentered");

    assert!(!task.fd_read_ready(fd).unwrap());
    assert_eq!(task.exit(), "read-ready-reentered");
    assert!(task.fd_write_ready(fd).unwrap());
    assert_eq!(task.exit(), "write-ready-reentered");
}

#[test]
fn noop_driver_is_available_for_early_cycles() {
    let table = TaskTable::new();
    table.register_driver("noop", Arc::new(NoopDriver)).unwrap();
    let task = table.allocate_root("noop").unwrap();

    table.start(task.id()).unwrap();
    assert_eq!(task.exit(), "");
}
