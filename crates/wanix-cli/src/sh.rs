//! `wanix sh` — the Wanix-native shell as a first-class CLI entry.
//!
//! `sh` is sugar over the compiled-wasm task path: it runs the bundled
//! `wanix-sh` guest ([`wanix_wasm::SHELL_WASM`]) through [`WasmTaskDriver`]
//! against a namespace shaped for a shell session — the host `--cwd` preopened
//! at the root, `#pipe` for pipelines, the installable command `bin` (plus the
//! shell itself as `bin/sh.wasm`, so subshells resolve), and any `--mount-mesh`
//! imports dialed over the native mesh wire.
//!
//! `sh -c LINE` is the captured one-line run (this module); interactive `sh`
//! is a live terminal session over `#term` (see [`session`], Unix only): the
//! shell task runs detached and the CLI only pumps bytes, because the guest
//! REPL owns echo and line editing per ADR 0003.

use std::io::Read;
use std::sync::Arc;

use wanix_fs::{LocalFs, MemFs, NormalizedPath};
use wanix_pipe::PipeDevice;
use wanix_qjs::QuickJsTaskDriver;
use wanix_task::{Task, TaskTable};
use wanix_vfs::{BindOptions, Namespace};
use wanix_wasm::WasmTaskDriver;

mod args;
#[cfg(test)]
mod both_kinds_tests;
#[cfg(unix)]
mod session;

pub(crate) use args::{ShCommand, parse_sh_command};
#[cfg(unix)]
pub(crate) use session::run_sh_session;

use crate::mesh::IrohMount;
use crate::{
    CliError, CliOutput, attach_task_stdio, bind_mesh_mounts_into, configure_qjs_task,
    finish_cli_task_output, quickjs_runner,
};

/// Where the bundled shell lives in the session namespace: inside the command
/// `bin`, so the shell resolves itself like any other command.
const SH_PROGRAM: &str = "bin/sh.wasm";

/// Runs `sh -c LINE` as a captured one-shot (the collected CLI path).
///
/// # Errors
///
/// Returns a usage error without `-c` (the interactive session needs a live
/// terminal), and a CLI error when the namespace or task cannot be built.
pub(crate) fn run_sh(
    command: ShCommand,
    _process_stdin: &mut dyn Read,
) -> Result<CliOutput, CliError> {
    let Some(line) = command.line.clone() else {
        return Err(CliError::usage(
            "sh without -c is an interactive terminal session and needs a pollable host \
             terminal; run the wanix binary on a tty or pass -c LINE",
        ));
    };
    // Declared first so the mesh keepalives drop LAST — after the task and its
    // namespace bindings (each keepalive owns the runtime its mount runs on).
    let (namespace, _mesh_mounts) = sh_namespace(&command)?;
    let (table, task) = allocate_sh_task(namespace)?;
    configure_sh_task(&task, Some(&line), &command.env)?;
    // Empty stdin (immediate EOF), installed explicitly so a child command's
    // `bind #task/<shell>/fd/0 fd/0` inherit has a real fd to proxy.
    let (stdout, stderr) = attach_task_stdio(&task, Some(Vec::new()))?;
    let result = table.start(task.id()).map_err(CliError::from);
    finish_cli_task_output("sh", result, &task, &stdout, &stderr)
}

/// Builds the shell-session namespace: host cwd at the root, `#pipe`, the
/// command `bin`, and the dialed `--mount-mesh` imports.
///
/// The returned [`IrohMount`] keepalives must outlive every operation on the
/// namespace (and on any task allocated from it).
fn sh_namespace(command: &ShCommand) -> Result<(Namespace, Vec<IrohMount>), CliError> {
    let mut namespace = Namespace::new();
    let dir = command.cwd.as_str();
    let local = Arc::new(
        LocalFs::new(dir)
            .map_err(|error| CliError::new(format!("sh: cannot open {dir}: {error}"), 1))?,
    );
    namespace.bind(local, ".", ".", BindOptions::default())?;
    namespace.bind(
        Arc::new(PipeDevice::new()),
        ".",
        "#pipe",
        BindOptions::default(),
    )?;
    namespace.bind(sh_bin()?, ".", "bin", BindOptions::default())?;
    let mesh_mounts = bind_mesh_mounts_into(&mut namespace, &command.mesh_mounts)?;
    Ok((namespace, mesh_mounts))
}

/// The command `bin` for a shell session: the installable command set plus the
/// bundled shell itself, so `sh` inside the shell launches a subshell.
fn sh_bin() -> Result<Arc<MemFs>, CliError> {
    let bin = wanix_wasm::command_bin();
    bin.write_file("sh.wasm", wanix_wasm::SHELL_WASM)?;
    Ok(bin)
}

/// Allocates the shell task on its own single-task table with both real task
/// drivers registered (`#task` is bound by the table, so the shell can launch
/// `.wasm` and `.js` child commands alike — ADR 0002's both-first-class
/// contract, dispatched by extension through the table's `auto` kind).
fn allocate_sh_task(namespace: Namespace) -> Result<(TaskTable, Task), CliError> {
    let table = TaskTable::new();
    table.register_driver("wasm", Arc::new(WasmTaskDriver::new()))?;
    table.register_driver("qjs", Arc::new(QuickJsTaskDriver::new(quickjs_runner()?)))?;
    let task = table.allocate_root_with_namespace("auto", namespace)?;
    Ok((table, task))
}

/// Configures the shell task spec: `bin/sh.wasm [-c LINE]` at the namespace
/// root (the shell implements cwd in guest code, mirroring the qjs-shell rule).
fn configure_sh_task(task: &Task, line: Option<&str>, env: &[String]) -> Result<(), CliError> {
    let args = match line {
        Some(line) => vec!["-c".to_owned(), line.to_owned()],
        None => Vec::new(),
    };
    configure_qjs_task(task, SH_PROGRAM, &args, env, &NormalizedPath::new(".")?)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};

    use wanix_fs::{FileSystem, MemFs, NormalizedPath};

    use super::{ShCommand, run_sh};
    use crate::mesh::mounts::test_support::serve_native;
    use crate::qjs_args::MeshMountSpec;

    fn temp_dir(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = PathBuf::from(format!(
            "target/wanix-cli-{label}-{}-{nanos}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sh_command(root: &std::path::Path, line: &str) -> ShCommand {
        ShCommand {
            line: Some(line.to_owned()),
            env: Vec::new(),
            cwd: NormalizedPath::new(root.to_str().unwrap()).unwrap(),
            mesh_mounts: Vec::new(),
        }
    }

    #[test]
    fn sh_dash_c_runs_a_line_against_the_host_cwd() {
        let root = temp_dir("sh-line");

        let command = sh_command(&root, "echo hello > out.txt; cat < out.txt");
        let output = run_sh(command, &mut std::io::empty()).unwrap();

        assert_eq!(
            output.exit_code(),
            0,
            "stderr: {}",
            String::from_utf8_lossy(output.stderr())
        );
        assert_eq!(output.stdout(), b"hello\n");
        // The redirect landed on the real host directory.
        assert_eq!(
            std::fs::read_to_string(root.join("out.txt")).unwrap(),
            "hello\n"
        );
        std::fs::remove_dir_all(root).ok();
    }

    /// The e2e mounts proof: a served volume is dialed via `--mount-mesh`, the
    /// shell writes through the mounted path and reads it back, and the bytes
    /// land on the served root across the native mesh wire.
    #[test]
    fn sh_dash_c_writes_and_reads_through_a_mesh_mount() {
        let host = Arc::new(MemFs::new());
        let (server, url) = serve_native(host.clone() as Arc<dyn FileSystem>, 9);
        let root = temp_dir("sh-mesh");

        let mut command = sh_command(&root, "echo hi > /vol/x; cat < /vol/x");
        command.mesh_mounts = vec![MeshMountSpec {
            addr: url,
            guest_path: NormalizedPath::new("vol").unwrap(),
        }];
        let output = run_sh(command, &mut std::io::empty()).unwrap();

        assert_eq!(
            output.exit_code(),
            0,
            "stderr: {}",
            String::from_utf8_lossy(output.stderr())
        );
        assert_eq!(output.stdout(), b"hi\n");
        // The shell's redirect crossed the wire into the served root.
        assert_eq!(host.read_file("x").unwrap(), b"hi\n");
        drop(server);
        std::fs::remove_dir_all(root).ok();
    }

    /// Interactive entry through the fd-aware terminal path: the REPL prompts,
    /// echoes the typed line, reads a file through a mesh mount, and host stdin
    /// EOF lands as `\r` + Ctrl-D so the session exits cleanly after the queued
    /// input is processed in order.
    #[cfg(unix)]
    #[test]
    fn sh_interactive_session_reads_a_mesh_mount_and_exits_on_eof() {
        use std::io::Write;
        use std::os::fd::AsRawFd;
        use std::os::unix::net::UnixStream;

        let host = Arc::new(MemFs::new());
        host.write_file("seed.txt", b"served\n").unwrap();
        let (server, url) = serve_native(host.clone() as Arc<dyn FileSystem>, 10);

        let root = temp_dir("sh-interactive");
        let (input_reader, mut input_writer) = UnixStream::pair().unwrap();
        let input_fd = input_reader.as_raw_fd();
        // Typed input is queued by the terminal device, so it can be written
        // before the prompt appears; EOF after it maps to `\r` + Ctrl-D.
        input_writer.write_all(b"cat < /vol/seed.txt\r").unwrap();
        drop(input_writer);

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let exit_code = crate::run_with_process_io_and_stdin_fd(
            [
                "sh".to_owned(),
                "--cwd".to_owned(),
                root.display().to_string(),
                "--mount-mesh".to_owned(),
                format!("{url}=/vol"),
            ],
            input_reader,
            input_fd,
            &mut stdout,
            &mut stderr,
        )
        .unwrap();

        assert_eq!(
            exit_code,
            0,
            "stderr: {} stdout: {}",
            String::from_utf8_lossy(&stderr),
            String::from_utf8_lossy(&stdout)
        );
        let transcript = String::from_utf8_lossy(&stdout).into_owned();
        // Guest-owned echo (ADR 0003): the typed line comes back, then the
        // file content crosses the mesh mount, then the next prompt.
        assert!(
            transcript.contains("cat < /vol/seed.txt\r\nserved\r\n"),
            "echo + mesh-mounted file content expected: {transcript:?}"
        );
        assert!(transcript.contains("$ "), "prompt expected: {transcript:?}");
        drop(server);
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn sh_without_dash_c_refuses_the_captured_path() {
        let command = ShCommand {
            line: None,
            env: Vec::new(),
            cwd: NormalizedPath::new(".").unwrap(),
            mesh_mounts: Vec::new(),
        };
        let error = run_sh(command, &mut std::io::empty()).unwrap_err();
        assert_eq!(error.exit_code(), 2);
        assert!(error.to_string().contains("interactive terminal session"));
    }
}
