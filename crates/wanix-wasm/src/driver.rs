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
        let result = run_wasm_task(task);
        // A finished task releases its fds (Unix/Plan 9: an exited process holds
        // no descriptors), so a pipeline producer's `#pipe` writer drops and the
        // consumer observes EOF.
        task.close_all_fds();
        match result {
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
    use wanix_pipe::PipeDevice;
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

    const SHELL_GUEST: &[u8] = crate::SHELL_WASM;

    #[test]
    fn shell_guest_runs_echo_via_dash_c() {
        // End-to-end: the wanix-sh shell, compiled to wasm, runs as an ordinary
        // wasm task and `shell -c "echo hi"` prints `hi` to the task's stdout.
        let fs = Arc::new(MemFs::new());
        fs.write_file("shell.wasm", SHELL_GUEST)
            .expect("seed shell.wasm");

        let task = wasm_task(namespace_on(&fs), "shell.wasm -c \"echo hi\"");

        // Capture the guest's stdout (fd 1).
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

        let driver = WasmTaskDriver::new();
        driver.start(&task).expect("shell task ran");
        assert_eq!(task.exit(), "0", "echo should exit 0");

        let out = String::from_utf8(sink.read_file("out").expect("read sink")).expect("utf8");
        assert_eq!(out, "hi\n", "shell -c \"echo hi\" should print hi");
    }

    #[test]
    fn shell_guest_launches_child_wasm_command() {
        // End-to-end: the shell (a wasm task) launches another wasm command as a
        // child task via the #task device, and the child's stdout — inherited
        // from the shell — flows to the shell's captured fd 1.
        let fs = Arc::new(MemFs::new());
        fs.write_file("shell.wasm", SHELL_GUEST)
            .expect("seed shell.wasm");
        fs.write_file("guest.wasm", RUST_GUEST)
            .expect("seed guest.wasm");
        fs.create_dir_all("dir").expect("make /dir");
        fs.write_file("dir/alpha.txt", b"a").expect("seed file");

        // The real wasm driver runs both the shell and its child.
        let table = TaskTable::new();
        table
            .register_driver("wasm", Arc::new(WasmTaskDriver::new()))
            .expect("register wasm driver");
        let shell = table
            .allocate_root_with_namespace("auto", namespace_on(&fs))
            .expect("allocate shell");
        shell
            .set_cmd("shell.wasm -c \"guest.wasm --list /dir\"")
            .expect("set cmd");

        // Wire the shell's stdio (fd 0/1/2); the child inherits these.
        let cap = wire_shell_stdio(&shell);

        table.start(shell.id()).expect("run shell");
        let err = String::from_utf8_lossy(&cap.read_file("err").expect("read err")).into_owned();
        assert_eq!(shell.exit(), "0", "shell should exit 0; stderr={err:?}");

        let out = String::from_utf8(cap.read_file("out").expect("read out")).expect("utf8");
        assert!(
            out.contains("rust-wasm: /dir has 1 entries"),
            "child task output should appear on inherited stdout: {out:?}"
        );
        assert!(
            out.contains("alpha.txt file"),
            "child listing should appear: {out:?}"
        );
    }

    fn namespace_with_pipe(fs: &Arc<MemFs>) -> Namespace {
        let mut ns = namespace_on(fs);
        ns.bind(
            Arc::new(PipeDevice::new()),
            ".",
            "#pipe",
            BindOptions::default(),
        )
        .expect("bind #pipe");
        ns
    }

    /// Wires the shell's fd 0/1/2 to capture files and returns the capture fs.
    fn wire_shell_stdio(shell: &Task) -> Arc<MemFs> {
        let cap = Arc::new(MemFs::new());
        for (fd, name) in [(Fd::STDIN, "in"), (Fd::STDOUT, "out"), (Fd::STDERR, "err")] {
            cap.write_file(name, b"").expect("seed capture file");
            let file = cap
                .open(
                    &NormalizedPath::new(name).expect("path"),
                    OpenOptions::read_write(),
                )
                .expect("open capture");
            shell
                .insert_fd(fd, file, NormalizedPath::new(name).expect("path"))
                .expect("install fd");
        }
        cap
    }

    fn run_shell_pipeline(cmd: &str) -> (String, String) {
        let fs = Arc::new(MemFs::new());
        fs.write_file("shell.wasm", SHELL_GUEST)
            .expect("seed shell");
        // rust-guest provides `--cat` (stdin->stdout) and `--list`.
        fs.write_file("guest.wasm", RUST_GUEST).expect("seed guest");
        fs.create_dir_all("dir").expect("make /dir");
        fs.write_file("dir/alpha.txt", b"a").expect("seed file");

        let table = TaskTable::new();
        table
            .register_driver("wasm", Arc::new(WasmTaskDriver::new()))
            .expect("register wasm driver");
        let shell = table
            .allocate_root_with_namespace("auto", namespace_with_pipe(&fs))
            .expect("allocate shell");
        shell.set_cmd(cmd).expect("set cmd");
        let cap = wire_shell_stdio(&shell);

        table.start(shell.id()).expect("run shell");
        let out = String::from_utf8(cap.read_file("out").expect("read out")).expect("utf8");
        let err = String::from_utf8_lossy(&cap.read_file("err").expect("read err")).into_owned();
        assert_eq!(shell.exit(), "0", "shell should exit 0; stderr={err:?}");
        (out, err)
    }

    #[test]
    fn shell_pipes_builtin_producer_into_external_consumer() {
        // echo (builtin) fills a #pipe; `guest.wasm --cat` (external) drains it.
        // Exercises the shell opening the pipe writer and closing it for EOF.
        let (out, _err) = run_shell_pipeline("shell.wasm -c \"echo hi | guest.wasm --cat\"");
        assert_eq!(
            out, "hi\n",
            "piped bytes should reach the consumer's stdout"
        );
    }

    #[test]
    fn shell_pipes_external_producer_into_external_consumer() {
        // `guest.wasm --list` (external) writes a #pipe via a write-only bind;
        // its exit drops the writer; `guest.wasm --cat` (external) drains it.
        let (out, _err) =
            run_shell_pipeline("shell.wasm -c \"guest.wasm --list /dir | guest.wasm --cat\"");
        assert!(
            out.contains("rust-wasm: /dir has 1 entries"),
            "producer listing should flow through the pipe: {out:?}"
        );
        assert!(
            out.contains("alpha.txt file"),
            "listing entry missing: {out:?}"
        );
    }

    #[test]
    fn shell_pipeline_streams_four_pipe_capacities_through_concurrent_stages() {
        // The ADR 0010 tier-2 proof: the producer streams 256 KiB — four times
        // the bounded #pipe's 64 KiB capacity — so it MUST block mid-stream
        // and can only finish because the consumer drains concurrently on its
        // own thread. Under the old sequential model (producer runs to
        // completion before the consumer starts) this pipeline deadlocks.
        const TOTAL: usize = 4 * 64 * 1024;
        let (out, _err) = run_shell_pipeline(&format!(
            "shell.wasm -c \"guest.wasm --gen {TOTAL} | guest.wasm --cat\""
        ));
        assert_eq!(out.len(), TOTAL, "every byte crossed the bounded pipe");
        assert!(
            out.as_bytes().iter().all(|b| b.is_ascii_lowercase()),
            "payload pattern survived the pipe"
        );
    }

    /// Runs `cmd` as a detached shell task and returns (stdout, stderr, exit),
    /// panicking if the shell does not finish within a generous deadline —
    /// the regression mode here is an eternal hang, which must fail the test
    /// rather than wedge the suite.
    fn run_shell_to_completion(cmd: &str) -> (String, String, String) {
        let fs = Arc::new(MemFs::new());
        fs.write_file("shell.wasm", SHELL_GUEST)
            .expect("seed shell");
        fs.write_file("guest.wasm", RUST_GUEST).expect("seed guest");

        let table = TaskTable::new();
        table
            .register_driver("wasm", Arc::new(WasmTaskDriver::new()))
            .expect("register wasm driver");
        let shell = table
            .allocate_root_with_namespace("auto", namespace_with_pipe(&fs))
            .expect("allocate shell");
        shell.set_cmd(cmd).expect("set cmd");
        let cap = wire_shell_stdio(&shell);

        table.start_detached(shell.id()).expect("start shell");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
        while shell.exit().is_empty() {
            assert!(
                std::time::Instant::now() < deadline,
                "shell hung running {cmd:?}"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let out = String::from_utf8(cap.read_file("out").expect("read out")).expect("utf8");
        let err = String::from_utf8_lossy(&cap.read_file("err").expect("read err")).into_owned();
        (out, err, shell.exit())
    }

    #[test]
    fn pipeline_with_failed_consumer_launch_does_not_hang_the_shell() {
        // Regression: the consumer's spawn fails at the `>` pre-truncate (a
        // missing parent dir) BEFORE its pipe-reader fd is bound, so nothing
        // ever drains the pipe. The producer streams 4x the bounded #pipe
        // capacity: unless the shell breaks the dead consumer's pipe, the
        // producer parks forever mid-write and the shell hangs in spawn_wait.
        let (out, err, exit) = run_shell_to_completion(
            "shell.wasm -c \"guest.wasm --gen 262144 | guest.wasm --cat > missing/out; echo status=$?\"",
        );
        assert_eq!(exit, "0", "shell finishes; stderr={err:?}");
        assert!(
            out.contains("status=127"),
            "failed launch reports 127: {out:?} stderr={err:?}"
        );
    }

    #[test]
    fn builtin_abort_mid_pipeline_does_not_hang_the_shell() {
        // Regression: `cat < missing.txt` aborts the builtin pass, so the
        // trailing builtin never drains the external's output pipe. The
        // external streams 4x the pipe capacity: unless the shell breaks the
        // skipped consumer's pipe, it parks forever and spawn_wait hangs.
        let (_out, err, exit) = run_shell_to_completion(
            "shell.wasm -c \"cat < missing.txt | guest.wasm --gen 262144 | cat\"",
        );
        assert_eq!(exit, "1", "the aborted builtin's error decides the status");
        assert!(
            err.contains("missing.txt"),
            "original failure reported: {err:?}"
        );
    }

    #[test]
    fn shell_pipeline_reports_last_stage_exit_status() {
        // bash semantics end to end: a failing producer does not decide the
        // pipeline status; the last stage does — and stage exits are collected
        // from real concurrent tasks via #task/<id>/wait.
        let fs = Arc::new(MemFs::new());
        fs.write_file("shell.wasm", SHELL_GUEST)
            .expect("seed shell");
        fs.write_file("guest.wasm", RUST_GUEST).expect("seed guest");

        let table = TaskTable::new();
        table
            .register_driver("wasm", Arc::new(WasmTaskDriver::new()))
            .expect("register wasm driver");
        let shell = table
            .allocate_root_with_namespace("auto", namespace_with_pipe(&fs))
            .expect("allocate shell");
        // `false` (builtin producer) fails; the trailing external `--cat`
        // exits 0 through #task/<id>/wait.
        shell
            .set_cmd("shell.wasm -c \"false | guest.wasm --cat; echo status=$?\"")
            .expect("set cmd");
        let cap = wire_shell_stdio(&shell);

        table.start(shell.id()).expect("run shell");
        assert_eq!(shell.exit(), "0", "shell itself exits 0");
        let out = String::from_utf8(cap.read_file("out").expect("read out")).expect("utf8");
        assert!(
            out.contains("status=0"),
            "pipeline status should be the last stage's: {out:?}"
        );
    }

    #[test]
    fn shell_resolves_external_command_from_bin() {
        // `lister` (no slash, not a builtin) resolves to bin/lister.wasm and runs.
        let fs = Arc::new(MemFs::new());
        fs.write_file("shell.wasm", SHELL_GUEST)
            .expect("seed shell");
        fs.create_dir_all("bin").expect("make bin");
        fs.write_file("bin/lister.wasm", RUST_GUEST)
            .expect("seed bin command");
        fs.create_dir_all("dir").expect("make dir");
        fs.write_file("dir/alpha.txt", b"a").expect("seed file");

        let table = TaskTable::new();
        table
            .register_driver("wasm", Arc::new(WasmTaskDriver::new()))
            .expect("register wasm driver");
        let shell = table
            .allocate_root_with_namespace("auto", namespace_with_pipe(&fs))
            .expect("allocate shell");
        shell
            .set_cmd("shell.wasm -c \"lister --list /dir\"")
            .expect("set cmd");
        let cap = wire_shell_stdio(&shell);

        table.start(shell.id()).expect("run shell");
        let err = String::from_utf8_lossy(&cap.read_file("err").expect("read err")).into_owned();
        assert_eq!(shell.exit(), "0", "shell should exit 0; stderr={err:?}");
        let out = String::from_utf8(cap.read_file("out").expect("read out")).expect("utf8");
        assert!(
            out.contains("/dir has 1 entries"),
            "resolved bin command should run: {out:?}"
        );
    }

    #[test]
    fn shell_exports_env_to_child() {
        // `export` populates the shell env; a spawned child inherits it via the
        // child's #task env field.
        let fs = Arc::new(MemFs::new());
        fs.write_file("shell.wasm", SHELL_GUEST)
            .expect("seed shell");
        fs.create_dir_all("bin").expect("make bin");
        fs.write_file("bin/printenv.wasm", RUST_GUEST)
            .expect("seed bin command");

        let table = TaskTable::new();
        table
            .register_driver("wasm", Arc::new(WasmTaskDriver::new()))
            .expect("register wasm driver");
        let shell = table
            .allocate_root_with_namespace("auto", namespace_with_pipe(&fs))
            .expect("allocate shell");
        shell
            .set_cmd("shell.wasm -c \"export GREETING=hi; printenv --env\"")
            .expect("set cmd");
        let cap = wire_shell_stdio(&shell);

        table.start(shell.id()).expect("run shell");
        let err = String::from_utf8_lossy(&cap.read_file("err").expect("read err")).into_owned();
        assert_eq!(shell.exit(), "0", "shell should exit 0; stderr={err:?}");
        let out = String::from_utf8(cap.read_file("out").expect("read out")).expect("utf8");
        assert!(
            out.contains("GREETING=hi"),
            "child should inherit exported env: {out:?}"
        );
    }

    #[test]
    fn shell_redirects_to_and_from_a_real_file() {
        // `echo hello > out.txt` writes a real file; `cat < out.txt` reads it.
        let fs = Arc::new(MemFs::new());
        fs.write_file("shell.wasm", SHELL_GUEST)
            .expect("seed shell");

        let table = TaskTable::new();
        table
            .register_driver("wasm", Arc::new(WasmTaskDriver::new()))
            .expect("register wasm driver");
        let shell = table
            .allocate_root_with_namespace("auto", namespace_with_pipe(&fs))
            .expect("allocate shell");
        shell
            .set_cmd("shell.wasm -c \"echo hello > out.txt; cat < out.txt\"")
            .expect("set cmd");
        let cap = wire_shell_stdio(&shell);

        table.start(shell.id()).expect("run shell");
        let err = String::from_utf8_lossy(&cap.read_file("err").expect("read err")).into_owned();
        assert_eq!(shell.exit(), "0", "shell should exit 0; stderr={err:?}");

        assert_eq!(
            fs.read_file("out.txt").expect("out.txt written"),
            b"hello\n",
            "`>` should write the file"
        );
        let out = String::from_utf8(cap.read_file("out").expect("read out")).expect("utf8");
        assert_eq!(out, "hello\n", "`<` should feed the file to cat");
    }

    fn run_shell_with_bin(cmd: &str) -> (String, String) {
        let fs = Arc::new(MemFs::new());
        fs.write_file("shell.wasm", SHELL_GUEST)
            .expect("seed shell");

        let mut ns = namespace_with_pipe(&fs);
        // Mount the installable command set (jaq, …) at `bin`; children inherit it.
        ns.bind(crate::command_bin(), ".", "bin", BindOptions::default())
            .expect("bind command bin");

        let table = TaskTable::new();
        table
            .register_driver("wasm", Arc::new(WasmTaskDriver::new()))
            .expect("register wasm driver");
        let shell = table
            .allocate_root_with_namespace("auto", ns)
            .expect("allocate shell");
        shell.set_cmd(cmd).expect("set cmd");
        let cap = wire_shell_stdio(&shell);

        table.start(shell.id()).expect("run shell");
        let out = String::from_utf8(cap.read_file("out").expect("read out")).expect("utf8");
        let err = String::from_utf8_lossy(&cap.read_file("err").expect("read err")).into_owned();
        assert_eq!(shell.exit(), "0", "shell should exit 0; stderr={err:?}");
        (out, err)
    }

    #[test]
    fn shell_pipes_echo_into_jaq_map() {
        // The flagship: jaq is a plain external command resolved from bin, fed by
        // a builtin producer through a real #pipe. No shell special-casing of jq.
        let (out, _err) = run_shell_with_bin("shell.wasm -c \"echo '[1,2,3]' | jaq 'map(.+1)'\"");
        assert_eq!(out.trim(), "[2,3,4]", "echo array | jaq map: {out:?}");
    }

    #[test]
    fn shell_pipes_echo_into_jaq_stdlib_add() {
        // `add` exercises the jaq stdlib wiring through the resolved command.
        let (out, _err) = run_shell_with_bin("shell.wasm -c \"echo '[1,2,3]' | jaq add\"");
        assert_eq!(out.trim(), "6", "echo array | jaq add: {out:?}");
    }

    #[test]
    fn shell_tool_builtin_drives_a_mounted_toolfs_end_to_end() {
        // The real shell.wasm guest runs `cat | tool | cat` against a REAL
        // ToolFS bound into the task namespace at /n/upper: the builtin's
        // job-protocol dance (new → in → ctl run → out → result.json → close)
        // crosses the WASI boundary like any other namespace file access.
        use wanix_tool::runners::UpperRunner;
        use wanix_tool::{ToolPrincipal, ToolService, ToolSpec};

        let fs = Arc::new(MemFs::new());
        fs.write_file("shell.wasm", SHELL_GUEST)
            .expect("seed shell");
        fs.write_file("notes.txt", b"hello mesh")
            .expect("seed input");

        let service = ToolService::new(
            ToolSpec::v0("upper", "Uppercase UTF-8 text."),
            Box::new(UpperRunner),
            Box::new(|| 0),
        );
        let mut ns = namespace_with_pipe(&fs);
        ns.bind(
            Arc::new(service.open_view(ToolPrincipal::local("shell"))),
            ".",
            "n/upper",
            BindOptions::default(),
        )
        .expect("bind ToolFS at /n/upper");

        let table = TaskTable::new();
        table
            .register_driver("wasm", Arc::new(WasmTaskDriver::new()))
            .expect("register wasm driver");
        let shell = table
            .allocate_root_with_namespace("auto", ns)
            .expect("allocate shell");
        shell
            .set_cmd("shell.wasm -c \"cat < notes.txt | tool /n/upper | cat\"")
            .expect("set cmd");
        let cap = wire_shell_stdio(&shell);

        table.start(shell.id()).expect("run shell");
        let err = String::from_utf8_lossy(&cap.read_file("err").expect("read err")).into_owned();
        assert_eq!(shell.exit(), "0", "shell should exit 0; stderr={err:?}");
        let out = String::from_utf8(cap.read_file("out").expect("read out")).expect("utf8");
        assert_eq!(out, "HELLO MESH", "tool output flows through the pipeline");
    }

    // ---- interactive REPL over #term ------------------------------------

    use wanix_term::TermDevice;

    fn term_path(id: &str, name: &str) -> NormalizedPath {
        NormalizedPath::new(format!("{id}/{name}")).expect("term path")
    }

    /// Writes terminal input on the data side (what a terminal client types).
    fn type_into_terminal(term: &TermDevice, id: &str, bytes: &[u8]) {
        let mut data = term
            .open(
                &term_path(id, "data"),
                OpenOptions {
                    write: true,
                    ..OpenOptions::default()
                },
            )
            .expect("open data side for write");
        data.write(bytes).expect("feed terminal input");
    }

    /// Drains whatever the program has written to the terminal so far into
    /// `seen`, returning once `seen` contains `until` (or panicking after a
    /// generous deadline — the guest is compiling on first use).
    fn read_terminal_until(term: &TermDevice, id: &str, seen: &mut Vec<u8>, until: &str) {
        let mut data = term
            .open(&term_path(id, "data"), OpenOptions::read())
            .expect("open data side for read");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        let mut buf = [0u8; 1024];
        loop {
            let n = data.read(&mut buf).expect("read terminal output");
            seen.extend_from_slice(&buf[..n]);
            if String::from_utf8_lossy(seen).contains(until) {
                return;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "terminal output never contained {until:?}: {:?}",
                String::from_utf8_lossy(seen)
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    #[test]
    fn shell_repl_serves_a_terminal_session_over_blocking_reads() {
        // The river: an interactive wanix-sh session through #term. The shell
        // (a wasm task) blocks reading fd 0 = #term/<id>/program; this test
        // plays the terminal client on the data side — typing only after the
        // prompt appears, so every read genuinely parks and wakes.
        let fs = Arc::new(MemFs::new());
        fs.write_file("shell.wasm", SHELL_GUEST)
            .expect("seed shell.wasm");

        let term = TermDevice::new();
        let id = term.alloc().expect("alloc terminal");

        // No -c: the shell starts its REPL.
        let task = wasm_task(namespace_on(&fs), "shell.wasm");
        for fd in [Fd::STDIN, Fd::STDOUT, Fd::STDERR] {
            let file = term
                .open(&term_path(&id, "program"), OpenOptions::read_write())
                .expect("open program side");
            task.insert_fd(fd, file, term_path(&id, "program"))
                .expect("install fd");
        }

        let shell = task.clone();
        let session = std::thread::spawn(move || WasmTaskDriver::new().start(&shell));

        // Prompt appears before any input exists: the first stdin read parks.
        let mut seen = Vec::new();
        read_terminal_until(&term, &id, &mut seen, "$ ");

        // Type a command; the parked read wakes, the shell echoes (program
        // writes map \n to \r\n) and runs the line, then prompts again.
        type_into_terminal(&term, &id, b"echo hi\r");
        read_terminal_until(&term, &id, &mut seen, "hi\r\nhi\r\n");
        read_terminal_until(&term, &id, &mut seen, "hi\r\n/ $ ");

        // Ctrl-D on the empty line ends the session.
        type_into_terminal(&term, &id, b"\x04");
        session
            .join()
            .expect("session thread")
            .expect("shell task ran");
        assert_eq!(task.exit(), "0", "Ctrl-D exits the REPL cleanly");

        let transcript = String::from_utf8_lossy(&seen).into_owned();
        assert!(
            transcript.contains("echo hi\r\nhi\r\n"),
            "echoed line then output: {transcript:?}"
        );
    }
}
