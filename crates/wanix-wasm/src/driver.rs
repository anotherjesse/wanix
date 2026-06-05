//! Wanix task driver for compiled `wasm32-wasi` command tasks.

use wanix_fs::{FileSystem, FsError, FsResult, NormalizedPath, OpenOptions};
use wanix_task::{Task, TaskDriver, task_command, task_program_for_check};

use crate::WasiRunner;
use crate::task_stdio::task_wasi_config;

const NAMESPACE_READ_CHUNK_BYTES: usize = 64 * 1024;

/// Runs a `.wasm` program as a first-class Wanix task.
///
/// `check` matches a task whose program ends with `.wasm`; `start` reads the
/// module bytes from the task's own namespace, compiles them, builds a live
/// [`crate::WasiConfig`](wanix_wasi::WasiConfig) from the task (namespace, cwd
/// preopen, env, argv, and fds 0/1/2), runs `_start`, and records the guest
/// exit code through [`Task::set_exit`].
#[derive(Debug, Default, Clone)]
pub struct WasmTaskDriver;

impl WasmTaskDriver {
    /// Creates a wasm task driver.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl TaskDriver for WasmTaskDriver {
    fn check(&self, task: &Task) -> bool {
        task_program_for_check(task).is_some_and(|program| program.ends_with(".wasm"))
    }

    fn start(&self, task: &Task) -> FsResult<()> {
        match run_wasm_task(task) {
            Ok(code) => task.set_exit(code.to_string()),
            Err(err) => {
                let _ = task.set_exit("1");
                Err(err)
            }
        }
    }
}

fn run_wasm_task(task: &Task) -> FsResult<i32> {
    let command = task_command(task)?;
    let bytes = read_namespace_bytes(&task.namespace(), &command.program)?;
    let runner = WasiRunner::from_bytes_cached(&bytes, &crate::module_cache_dir())
        .map_err(|err| FsError::Other(format!("failed to compile wasm task: {err:?}")))?;
    let config = task_wasi_config(task);
    runner
        .run(config)
        .map_err(|err| FsError::Other(format!("wasm task failed: {err:?}")))
}

fn read_namespace_bytes(namespace: &impl FileSystem, path: &NormalizedPath) -> FsResult<Vec<u8>> {
    let mut file = namespace.open(path, OpenOptions::read())?;
    let mut bytes = Vec::new();
    let mut buf = [0u8; NAMESPACE_READ_CHUNK_BYTES];
    loop {
        let read = file.read(&mut buf)?;
        if read == 0 {
            break;
        }
        bytes.extend_from_slice(&buf[..read]);
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use wanix_fs::{FileSystem, MemFs, NormalizedPath, OpenOptions};
    use wanix_task::{Fd, Task, TaskDriver, TaskTable};
    use wanix_vfs::{BindOptions, Namespace};

    use super::WasmTaskDriver;

    const RUST_GUEST: &[u8] = include_bytes!("../fixtures/rust-guest.wasm");

    fn namespace_on(fs: &Arc<MemFs>) -> Namespace {
        let mut ns = Namespace::new();
        ns.bind(fs.clone(), ".", ".", BindOptions::default())
            .expect("bind shared fs at root");
        ns
    }

    fn wasm_task(namespace: Namespace, cmd: &str) -> Task {
        let table = TaskTable::new();
        table.register_noop_driver("wasm").expect("register driver");
        let task = table
            .allocate_root_with_namespace("wasm", namespace)
            .expect("allocate task");
        task.set_cmd(cmd).expect("set cmd");
        task
    }

    #[test]
    fn check_matches_wasm_programs_only() {
        let fs = Arc::new(MemFs::new());
        let driver = WasmTaskDriver::new();
        assert!(driver.check(&wasm_task(namespace_on(&fs), "guest.wasm")));
        assert!(!driver.check(&wasm_task(namespace_on(&fs), "main.js")));
        assert!(!driver.check(&wasm_task(namespace_on(&fs), "noop")));
    }

    #[test]
    fn start_runs_wasm_task_and_records_exit() {
        let fs = Arc::new(MemFs::new());
        fs.write_file("guest.wasm", RUST_GUEST).expect("seed wasm");
        fs.create_dir_all("dir").expect("make /dir");
        fs.write_file("dir/a.txt", b"a").expect("write a");

        let task = wasm_task(namespace_on(&fs), "guest.wasm --list /dir");
        let driver = WasmTaskDriver::new();
        assert!(driver.check(&task), "driver should claim the wasm task");

        // Wire fd 1 so the guest's stdout flows into the task fd table.
        let sink = Arc::new(MemFs::new());
        sink.write_file("out", b"").expect("seed sink");
        let stdout = sink
            .open(
                &NormalizedPath::new("out").expect("path"),
                OpenOptions::read_write(),
            )
            .expect("open sink");
        task.insert_fd(
            Fd::STDOUT,
            stdout,
            NormalizedPath::new("out").expect("path"),
        )
        .expect("install fd 1");

        driver.start(&task).expect("wasm task ran");
        assert_eq!(task.exit(), "0", "observable task exit should be 0");

        let text = String::from_utf8(sink.read_file("out").expect("read sink")).expect("utf8");
        assert!(
            text.contains("/dir has 1 entries"),
            "guest should list the shared dir: {text:?}"
        );
    }

    #[test]
    fn shared_namespace_is_visible_to_the_wasm_task() {
        // The task's namespace and `fs` are one MemFs: a file seeded through the
        // shared fs is visible to the wasm guest, and the guest's rename is in
        // turn observable when read back through `fs`.
        let fs = Arc::new(MemFs::new());
        fs.write_file("guest.wasm", RUST_GUEST).expect("seed wasm");
        fs.create_dir_all("shared").expect("make /shared");
        fs.write_file("shared/from-task.txt", b"task bytes")
            .expect("seed shared file");

        let namespace = namespace_on(&fs);
        let task = wasm_task(
            namespace,
            "guest.wasm --rename /shared/from-task.txt /shared/renamed.txt",
        );
        let driver = WasmTaskDriver::new();
        driver.start(&task).expect("wasm task ran");
        assert_eq!(task.exit(), "0");

        // Read the result back through fs: the guest's rename is visible.
        assert_eq!(
            fs.read_file("shared/renamed.txt").expect("renamed exists"),
            b"task bytes",
            "wasm task and fs share one MemFs"
        );
        assert!(
            fs.read_file("shared/from-task.txt").is_err(),
            "source should be gone after the guest rename"
        );
    }

    #[test]
    fn auto_task_starts_wasm_driver_and_writes_shared_namespace() {
        // Task-level proof (distinct from `driver.start` called directly): a task
        // allocated as `auto` from a shared `Namespace`, with the real
        // `WasmTaskDriver` registered, is claimed by `check()` and auto-started
        // through `TaskTable::start`. The exit is observable on the task and the
        // file the guest writes is visible back through the shared MemFs.
        let fs = Arc::new(MemFs::new());
        fs.write_file("guest.wasm", RUST_GUEST).expect("seed wasm");
        fs.create_dir_all("shared").expect("make /shared");
        fs.write_file("shared/in.txt", b"shared input")
            .expect("seed input");

        let table = TaskTable::new();
        table
            .register_driver("wasm", Arc::new(WasmTaskDriver::new()))
            .expect("register wasm driver");
        let task = table
            .allocate_root_with_namespace("auto", namespace_on(&fs))
            .expect("allocate auto task");
        task.set_cmd("guest.wasm /shared/in.txt /shared/out.txt")
            .expect("set cmd");

        // Auto-start: the table selects the wasm driver via `check()`.
        table.start(task.id()).expect("auto-start wasm task");

        assert_eq!(task.kind(), "wasm", "auto task resolved to the wasm driver");
        assert_eq!(task.exit(), "0", "observable task exit should be 0");

        let out = String::from_utf8(
            fs.read_file("shared/out.txt")
                .expect("guest output visible in shared namespace"),
        )
        .expect("utf8");
        assert_eq!(
            out, "rust-wasm saw: shared input",
            "the file the wasm task wrote is visible through the shared MemFs"
        );
    }
}
