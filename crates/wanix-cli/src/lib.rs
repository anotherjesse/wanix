//! Native CLI plumbing for Rust Wanix demos.

mod qjs_term;

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fmt;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
#[cfg(test)]
use std::sync::OnceLock;
use std::time::Duration;

use wanix_fs::{FileSystem, FsError, LocalFs, MemFs, NormalizedPath, OpenOptions};
use wanix_qjs::{QuickJsRunner, QuickJsTaskDriver, QuickJsTaskRuntime};
use wanix_task::{Fd, Task, TaskSpec, TaskTable, quote_cmd_argv};
use wanix_vfs::BindOptions;

const USAGE: &str = concat!(
    "usage: wanix-rust qjs [--env KEY=VALUE ...] [--cwd DIR] ",
    "[--stdin TEXT | --stdin-file PATH|-] [--event-loop-ms N] [--ready-io-turns N] ",
    "[--interrupt-after N] [--memory-limit-bytes N] ",
    "[--mount HOST=GUEST ...] <script.js> [-- arg ...]\n",
    "       wanix-rust qjs-term [--env KEY=VALUE ...] [--cwd DIR] ",
    "[--stdin TEXT | --stdin-file PATH|-] [--event-loop-ms N] [--ready-io-turns N] ",
    "[--feed-after-eval TEXT ...] [--feed-after-eval-file PATH|- ...] ",
    "[--feed-after-eval-lines PATH|- ...] ",
    "[--interrupt-after N] [--memory-limit-bytes N] ",
    "[--mount HOST=GUEST ...] <script.js> [-- arg ...]\n",
    "       wanix-rust qjs-snapshot [--env KEY=VALUE ...] [--cwd DIR] ",
    "[--stdin TEXT | --stdin-file PATH|-] [--interrupt-after N] ",
    "[--memory-limit-bytes N] [--event-loop-ms N] [--ready-io-turns N] ",
    "[--mount HOST=GUEST ...] ",
    "--snapshot FILE <script.js> [-- arg ...]\n",
    "       wanix-rust qjs-resume [--env KEY=VALUE ...] [--cwd DIR] ",
    "[--stdin TEXT | --stdin-file PATH|-] [--interrupt-after N] ",
    "[--memory-limit-bytes N] [--event-loop-ms N] [--ready-io-turns N] ",
    "[--mount HOST=GUEST ...] ",
    "--snapshot FILE <script.js> [-- arg ...]\n",
    "       wanix-rust qjs-restore [--cwd DIR] [--before-env KEY=VALUE ...] ",
    "[--after-env KEY=VALUE ...] [--before-arg VALUE ...] [--after-arg VALUE ...] ",
    "[--mount HOST=GUEST ...] <before.js> <after.js>\n",
    "       wanix-rust --help",
);
const QJS_GUEST_SCRIPT: &str = "main.js";
const QJS_RESTORE_BEFORE_SCRIPT: &str = "__wanix_restore/before/main.js";
const QJS_RESTORE_AFTER_SCRIPT: &str = "__wanix_restore/after/main.js";
const QJS_RESTORE_BEFORE_DIR: &str = "__wanix_restore/before";
const QJS_RESTORE_AFTER_DIR: &str = "__wanix_restore/after";

/// Captured native CLI output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliOutput {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    exit_code: i32,
}

impl CliOutput {
    fn new(stdout: Vec<u8>, stderr: Vec<u8>, exit_code: i32) -> Self {
        Self {
            stdout,
            stderr,
            exit_code,
        }
    }

    /// Returns stdout bytes that should be written to the native process.
    #[must_use]
    pub fn stdout(&self) -> &[u8] {
        &self.stdout
    }

    /// Returns stderr bytes that should be written to the native process.
    #[must_use]
    pub fn stderr(&self) -> &[u8] {
        &self.stderr
    }

    /// Returns the native process exit code.
    #[must_use]
    pub fn exit_code(&self) -> i32 {
        self.exit_code
    }
}

/// CLI execution error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliError {
    message: String,
    exit_code: i32,
}

impl CliError {
    fn new(message: impl Into<String>, exit_code: i32) -> Self {
        Self {
            message: message.into(),
            exit_code,
        }
    }

    fn usage(message: impl AsRef<str>) -> Self {
        Self::new(format!("{}\n\n{USAGE}", message.as_ref()), 2)
    }

    /// Returns the native process exit code for this error.
    #[must_use]
    pub fn exit_code(&self) -> i32 {
        self.exit_code
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for CliError {}

impl From<FsError> for CliError {
    fn from(error: FsError) -> Self {
        Self::new(error.to_string(), 1)
    }
}

/// Runs the native CLI command and returns captured process output.
///
/// # Errors
///
/// Returns a CLI error when arguments are invalid, files cannot be read, or the
/// selected Wanix runtime cannot be initialized.
pub fn run<I, S>(args: I) -> Result<CliOutput, CliError>
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
{
    run_with_process_stdin(args, io::empty())
}

/// Runs the native CLI command with a supplied native-process stdin reader.
///
/// # Errors
///
/// Returns a CLI error when arguments are invalid, files cannot be read, stdin
/// cannot be read, or the selected Wanix runtime cannot be initialized.
pub fn run_with_process_stdin<I, S, R>(args: I, mut process_stdin: R) -> Result<CliOutput, CliError>
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
    R: Read,
{
    let args = args.into_iter().map(Into::into).collect::<Vec<OsString>>();
    run_collected(args, &mut process_stdin)
}

/// Runs the native CLI command against supplied process IO streams.
///
/// Unlike [`run_with_process_stdin`], this lets commands that support live
/// output write to the supplied stdout/stderr streams during execution. Commands
/// without a streaming path still write their captured output before returning.
///
/// # Errors
///
/// Returns a CLI error when command execution fails before command-managed
/// output is available, or when the supplied output streams cannot be written.
pub fn run_with_process_io<I, S, R, W, E>(
    args: I,
    mut process_stdin: R,
    mut process_stdout: W,
    mut process_stderr: E,
) -> Result<i32, CliError>
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
    R: Read,
    W: Write,
    E: Write,
{
    let args = args.into_iter().map(Into::into).collect::<Vec<OsString>>();
    match args.as_slice() {
        [command, rest @ ..] if command == "qjs-term" => qjs_term::run_qjs_term_streaming(
            qjs_term::parse_qjs_term_command(rest)?,
            &mut process_stdin,
            &mut process_stdout,
            &mut process_stderr,
        ),
        _ => {
            let output = run_collected(args, &mut process_stdin)?;
            write_process_output(&mut process_stdout, "stdout", output.stdout())?;
            write_process_output(&mut process_stderr, "stderr", output.stderr())?;
            Ok(output.exit_code())
        }
    }
}

fn run_collected(args: Vec<OsString>, process_stdin: &mut dyn Read) -> Result<CliOutput, CliError> {
    match args.as_slice() {
        [] => Ok(help_output()),
        [help] if help == "--help" || help == "-h" => Ok(help_output()),
        [command, rest @ ..] if command == "qjs" => {
            run_qjs(parse_qjs_command(rest)?, process_stdin)
        }
        [command, rest @ ..] if command == "qjs-term" => {
            qjs_term::run_qjs_term(qjs_term::parse_qjs_term_command(rest)?, process_stdin)
        }
        [command, rest @ ..] if command == "qjs-snapshot" => run_qjs_snapshot(
            parse_qjs_snapshot_file_command(rest, "qjs-snapshot")?,
            process_stdin,
        ),
        [command, rest @ ..] if command == "qjs-resume" => run_qjs_resume(
            parse_qjs_snapshot_file_command(rest, "qjs-resume")?,
            process_stdin,
        ),
        [command, rest @ ..] if command == "qjs-restore" => {
            run_qjs_restore(parse_qjs_restore_command(rest)?)
        }
        [command, ..] => Err(CliError::usage(format!(
            "unknown wanix-rust command: {}",
            command.to_string_lossy()
        ))),
    }
}

fn write_process_output(output: &mut dyn Write, label: &str, bytes: &[u8]) -> Result<(), CliError> {
    output
        .write_all(bytes)
        .map_err(|error| CliError::new(format!("failed to write process {label}: {error}"), 1))
}

fn help_output() -> CliOutput {
    CliOutput::new(
        format!("wanix-rust: {}\n{USAGE}\n", wanix_qjs::FIRST_DEMO_TARGET).into_bytes(),
        Vec::new(),
        0,
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct QjsCommand {
    script_path: PathBuf,
    args: Vec<String>,
    env: Vec<String>,
    cwd: NormalizedPath,
    stdin: Option<QjsStdin>,
    event_loop_wait_budget: Duration,
    ready_io_turns: usize,
    interrupt_poll_budget: Option<usize>,
    memory_limit_bytes: Option<u32>,
    mounts: Vec<HostMount>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum QjsStdin {
    Bytes(Vec<u8>),
    File(PathBuf),
    Process,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct HostMount {
    host_path: PathBuf,
    guest_path: NormalizedPath,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct QjsRestoreCommand {
    before_script_path: PathBuf,
    after_script_path: PathBuf,
    before_args: Vec<String>,
    after_args: Vec<String>,
    before_env: Vec<String>,
    after_env: Vec<String>,
    cwd: NormalizedPath,
    mounts: Vec<HostMount>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct QjsSnapshotFileCommand {
    script_path: PathBuf,
    snapshot_path: PathBuf,
    args: Vec<String>,
    env: Vec<String>,
    cwd: NormalizedPath,
    stdin: Option<QjsStdin>,
    event_loop_wait_budget: Duration,
    ready_io_turns: usize,
    interrupt_poll_budget: Option<usize>,
    memory_limit_bytes: Option<u32>,
    mounts: Vec<HostMount>,
}

fn run_qjs(command: QjsCommand, process_stdin: &mut dyn Read) -> Result<CliOutput, CliError> {
    let script_path = command.script_path.as_path();
    let script = read_utf8_script(script_path)?;
    let stdin_bytes = read_qjs_stdin(command.stdin, process_stdin)?;

    let table = TaskTable::new();
    let runner = quickjs_runner()?;
    let mut driver = QuickJsTaskDriver::new(runner)
        .with_event_loop_wait_budget(command.event_loop_wait_budget)
        .with_ready_io_turns(command.ready_io_turns);
    if let Some(budget) = command.interrupt_poll_budget {
        driver = driver.with_interrupt_poll_budget(budget);
    }
    if let Some(bytes) = command.memory_limit_bytes {
        driver = driver.with_memory_limit_bytes(bytes);
    }
    table.register_driver("qjs", Arc::new(driver))?;
    let task = table.allocate_root("qjs")?;
    let task_spec = qjs_task_spec(QJS_GUEST_SCRIPT, &command.args, &command.env, &command.cwd)?;
    let task_cmd = task_cmd(QJS_GUEST_SCRIPT, &command.args);
    let task_env = command.env.join("\n");
    let task_dir = command.cwd.to_string();

    let root = Arc::new(MemFs::new());
    copy_script_directory(script_path, &root, &command.cwd)?;
    let guest_script = guest_path_in_cwd(&command.cwd, QJS_GUEST_SCRIPT)?;
    root.write_file(guest_script.as_str(), script.as_bytes())?;
    task.bind(root, ".", ".", BindOptions::default())?;
    bind_host_mounts(&task, &command.mounts)?;
    let (stdout, stderr) = attach_task_stdio(&task, stdin_bytes)?;
    task.set_spec(task_spec)?;
    task.set_cmd(task_cmd)?;
    task.set_env_lines(task_env)?;
    task.set_dir(task_dir)?;

    let start_result = table.start(task.id());
    let stdout = read_file(&*stdout, "stdout")?;
    let mut stderr = read_file(&*stderr, "stderr")?;
    match start_result {
        Ok(()) => Ok(CliOutput::new(stdout, stderr, parse_exit(&task.exit()))),
        Err(error) => {
            if !stderr.is_empty() && !stderr.ends_with(b"\n") {
                stderr.push(b'\n');
            }
            stderr.extend_from_slice(format!("wanix-rust qjs: {error}\n").as_bytes());
            Ok(CliOutput::new(stdout, stderr, 1))
        }
    }
}

fn run_qjs_snapshot(
    command: QjsSnapshotFileCommand,
    process_stdin: &mut dyn Read,
) -> Result<CliOutput, CliError> {
    let script_path = command.script_path.as_path();
    let script = read_utf8_script(script_path)?;
    let stdin_bytes = read_qjs_stdin(command.stdin, process_stdin)?;

    let runner = quickjs_runner()?;
    let table = TaskTable::new();
    table.register_driver("qjs", Arc::new(QuickJsTaskDriver::new(Arc::clone(&runner))))?;
    let task = table.allocate_root("qjs")?;
    let root = Arc::new(MemFs::new());
    copy_script_directory(script_path, &root, &command.cwd)?;
    let guest_script = guest_path_in_cwd(&command.cwd, QJS_GUEST_SCRIPT)?;
    root.write_file(guest_script.as_str(), script.as_bytes())?;
    task.bind(root, ".", ".", BindOptions::default())?;
    bind_host_mounts(&task, &command.mounts)?;
    let (stdout, stderr) = attach_task_stdio(&task, stdin_bytes)?;
    configure_qjs_task(
        &task,
        QJS_GUEST_SCRIPT,
        &command.args,
        &command.env,
        &command.cwd,
    )?;

    let snapshot_result = (|| -> Result<(), CliError> {
        let mut runtime = runner.create_task_runtime(&task)?;
        apply_qjs_task_runtime_limits(
            &mut runtime,
            command.interrupt_poll_budget,
            command.memory_limit_bytes,
        )?;
        eval_qjs_source(
            &mut runtime,
            &script,
            &guest_script,
            command.event_loop_wait_budget,
            command.ready_io_turns,
        )?;
        ensure_snapshot_task_fds_closed(&task)?;
        let snapshot = runtime.snapshot_bytes()?;
        std::fs::write(&command.snapshot_path, snapshot).map_err(|error| {
            CliError::new(
                format!(
                    "failed to write snapshot {}: {error}",
                    command.snapshot_path.display()
                ),
                1,
            )
        })?;
        runtime.finish()?;
        Ok(())
    })();

    finish_cli_task_output("qjs-snapshot", snapshot_result, &task, &stdout, &stderr)
}

fn run_qjs_resume(
    command: QjsSnapshotFileCommand,
    process_stdin: &mut dyn Read,
) -> Result<CliOutput, CliError> {
    let script_path = command.script_path.as_path();
    let script = read_utf8_script(script_path)?;
    let snapshot = std::fs::read(&command.snapshot_path).map_err(|error| {
        CliError::new(
            format!(
                "failed to read snapshot {}: {error}",
                command.snapshot_path.display()
            ),
            1,
        )
    })?;
    let stdin_bytes = read_qjs_stdin(command.stdin, process_stdin)?;

    let runner = quickjs_runner()?;
    let table = TaskTable::new();
    table.register_driver("qjs", Arc::new(QuickJsTaskDriver::new(Arc::clone(&runner))))?;
    let task = table.allocate_root("qjs")?;
    let root = Arc::new(MemFs::new());
    copy_script_directory(script_path, &root, &command.cwd)?;
    let guest_script = guest_path_in_cwd(&command.cwd, QJS_GUEST_SCRIPT)?;
    root.write_file(guest_script.as_str(), script.as_bytes())?;
    task.bind(root, ".", ".", BindOptions::default())?;
    bind_host_mounts(&task, &command.mounts)?;
    let (stdout, stderr) = attach_task_stdio(&task, stdin_bytes)?;
    configure_qjs_task(
        &task,
        QJS_GUEST_SCRIPT,
        &command.args,
        &command.env,
        &command.cwd,
    )?;

    let resume_result = (|| -> Result<(), CliError> {
        let mut runtime = runner.restore_task_runtime_from_bytes(&task, &snapshot)?;
        apply_qjs_task_runtime_limits(
            &mut runtime,
            command.interrupt_poll_budget,
            command.memory_limit_bytes,
        )?;
        if let Err(error) = eval_qjs_source(
            &mut runtime,
            &script,
            &guest_script,
            command.event_loop_wait_budget,
            command.ready_io_turns,
        ) {
            let _ = task.set_exit("1");
            return Err(error);
        }
        runtime.finish()?;
        Ok(())
    })();

    finish_cli_task_output("qjs-resume", resume_result, &task, &stdout, &stderr)
}

fn run_qjs_restore(command: QjsRestoreCommand) -> Result<CliOutput, CliError> {
    let before_script_path = command.before_script_path.as_path();
    let after_script_path = command.after_script_path.as_path();
    let before_script = read_utf8_script(before_script_path)?;
    let after_script = read_utf8_script(after_script_path)?;

    let runner = quickjs_runner()?;
    let table = TaskTable::new();
    table.register_driver("qjs", Arc::new(QuickJsTaskDriver::new(Arc::clone(&runner))))?;
    let before_task = table.allocate_root("qjs")?;

    let root = Arc::new(MemFs::new());
    copy_script_directory_into(
        before_script_path,
        &root,
        &command.cwd,
        QJS_RESTORE_BEFORE_DIR,
    )?;
    copy_script_directory_into(
        after_script_path,
        &root,
        &command.cwd,
        QJS_RESTORE_AFTER_DIR,
    )?;
    let before_guest_script = guest_path_in_cwd(&command.cwd, QJS_RESTORE_BEFORE_SCRIPT)?;
    let after_guest_script = guest_path_in_cwd(&command.cwd, QJS_RESTORE_AFTER_SCRIPT)?;
    root.write_file(before_guest_script.as_str(), before_script.as_bytes())?;
    root.write_file(after_guest_script.as_str(), after_script.as_bytes())?;
    before_task.bind(root, ".", ".", BindOptions::default())?;
    bind_host_mounts(&before_task, &command.mounts)?;

    let stdout = Arc::new(MemFs::new());
    stdout.write_file("stdout", b"")?;
    before_task.insert_fd(
        Fd::STDOUT,
        stdout.open(&NormalizedPath::new("stdout")?, OpenOptions::read_write())?,
        NormalizedPath::new("stdout")?,
    )?;
    let stderr = Arc::new(MemFs::new());
    stderr.write_file("stderr", b"")?;
    before_task.insert_fd(
        Fd::STDERR,
        stderr.open(&NormalizedPath::new("stderr")?, OpenOptions::read_write())?,
        NormalizedPath::new("stderr")?,
    )?;
    configure_qjs_task(
        &before_task,
        QJS_RESTORE_BEFORE_SCRIPT,
        &command.before_args,
        &command.before_env,
        &command.cwd,
    )?;

    let mut after_exit = 0;
    let restore_result = (|| -> Result<(), CliError> {
        let mut runtime = runner.create_task_runtime(&before_task)?;
        eval_qjs_source(
            &mut runtime,
            &before_script,
            &before_guest_script,
            Duration::ZERO,
            1,
        )?;
        ensure_snapshot_task_fds_closed(&before_task)?;
        let snapshot = runtime.snapshot_bytes()?;
        drop(runtime);

        let after_task = table.allocate_child_of("qjs", &before_task)?;
        configure_qjs_task(
            &after_task,
            QJS_RESTORE_AFTER_SCRIPT,
            &command.after_args,
            &command.after_env,
            &command.cwd,
        )?;
        bind_child_output_to_parent(&after_task, &before_task)?;

        let mut restored = runner.restore_task_runtime_from_bytes(&after_task, &snapshot)?;
        if let Err(error) = eval_qjs_source(
            &mut restored,
            &after_script,
            &after_guest_script,
            Duration::ZERO,
            1,
        ) {
            let _ = after_task.set_exit("1");
            return Err(error);
        }
        restored.finish()?;
        after_exit = parse_exit(&after_task.exit());
        Ok(())
    })();

    let stdout = read_file(&*stdout, "stdout")?;
    let mut stderr = read_file(&*stderr, "stderr")?;
    match restore_result {
        Ok(()) => Ok(CliOutput::new(stdout, stderr, after_exit)),
        Err(error) => {
            if !stderr.is_empty() && !stderr.ends_with(b"\n") {
                stderr.push(b'\n');
            }
            stderr.extend_from_slice(format!("wanix-rust qjs-restore: {error}\n").as_bytes());
            Ok(CliOutput::new(stdout, stderr, 1))
        }
    }
}

fn parse_qjs_command(args: &[OsString]) -> Result<QjsCommand, CliError> {
    parse_qjs_command_for(args, "qjs")
}

fn parse_qjs_command_for(args: &[OsString], command: &str) -> Result<QjsCommand, CliError> {
    let mut env = Vec::new();
    let mut cwd = NormalizedPath::new(".")?;
    let mut stdin = None;
    let mut event_loop_wait_budget = Duration::ZERO;
    let mut ready_io_turns = 1usize;
    let mut interrupt_poll_budget = None;
    let mut memory_limit_bytes = None;
    let mut mounts = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--env" {
            i += 1;
            let value = args
                .get(i)
                .ok_or_else(|| CliError::usage(format!("{command} --env expects KEY=VALUE")))?;
            let value = os_arg_to_string(value, &format!("{command} --env"))?;
            validate_env_line(&value, &format!("{command} --env"))?;
            env.push(value);
            i += 1;
        } else if args[i] == "--cwd" {
            i += 1;
            let value = args
                .get(i)
                .ok_or_else(|| CliError::usage(format!("{command} --cwd expects a Wanix path")))?;
            cwd = NormalizedPath::new(os_arg_to_string(value, &format!("{command} --cwd"))?)?;
            i += 1;
        } else if args[i] == "--stdin" {
            i += 1;
            let value = args
                .get(i)
                .ok_or_else(|| CliError::usage(format!("{command} --stdin expects text")))?;
            set_qjs_stdin(
                &mut stdin,
                QjsStdin::Bytes(
                    os_arg_to_string(value, &format!("{command} --stdin"))?.into_bytes(),
                ),
                command,
            )?;
            i += 1;
        } else if args[i] == "--stdin-file" {
            i += 1;
            let value = args.get(i).ok_or_else(|| {
                CliError::usage(format!("{command} --stdin-file expects PATH or -"))
            })?;
            let source = if value == "-" {
                QjsStdin::Process
            } else {
                QjsStdin::File(PathBuf::from(value))
            };
            set_qjs_stdin(&mut stdin, source, command)?;
            i += 1;
        } else if args[i] == "--event-loop-ms" {
            i += 1;
            let value = args.get(i).ok_or_else(|| {
                CliError::usage(format!("{command} --event-loop-ms expects milliseconds"))
            })?;
            event_loop_wait_budget =
                parse_duration_millis(value, &format!("{command} --event-loop-ms"))?;
            i += 1;
        } else if args[i] == "--ready-io-turns" {
            i += 1;
            let value = args.get(i).ok_or_else(|| {
                CliError::usage(format!("{command} --ready-io-turns expects a count"))
            })?;
            ready_io_turns = parse_usize(value, &format!("{command} --ready-io-turns"))?;
            i += 1;
        } else if args[i] == "--interrupt-after" {
            i += 1;
            let value = args.get(i).ok_or_else(|| {
                CliError::usage(format!("{command} --interrupt-after expects a count"))
            })?;
            interrupt_poll_budget =
                Some(parse_usize(value, &format!("{command} --interrupt-after"))?);
            i += 1;
        } else if args[i] == "--memory-limit-bytes" {
            i += 1;
            let value = args.get(i).ok_or_else(|| {
                CliError::usage(format!(
                    "{command} --memory-limit-bytes expects a byte count"
                ))
            })?;
            memory_limit_bytes = Some(parse_u32(
                value,
                &format!("{command} --memory-limit-bytes"),
            )?);
            i += 1;
        } else if args[i] == "--mount" {
            i += 1;
            let value = args
                .get(i)
                .ok_or_else(|| CliError::usage(format!("{command} --mount expects HOST=GUEST")))?;
            mounts.push(parse_host_mount(
                &os_arg_to_string(value, &format!("{command} --mount"))?,
                &format!("{command} --mount"),
            )?);
            i += 1;
        } else if args[i] == "--" {
            i += 1;
            break;
        } else {
            break;
        }
    }

    let script = args
        .get(i)
        .ok_or_else(|| CliError::usage(format!("{command} expects a script path")))?;
    let script_path = PathBuf::from(script);
    i += 1;

    if args.get(i).is_some_and(|arg| arg == "--") {
        i += 1;
    }

    let js_args = args[i..]
        .iter()
        .map(|arg| os_arg_to_string(arg, &format!("{command} script arg")))
        .collect::<Result<Vec<_>, CliError>>()?;

    Ok(QjsCommand {
        script_path,
        args: js_args,
        env,
        cwd,
        stdin,
        event_loop_wait_budget,
        ready_io_turns,
        interrupt_poll_budget,
        memory_limit_bytes,
        mounts,
    })
}

fn set_qjs_stdin(
    stdin: &mut Option<QjsStdin>,
    source: QjsStdin,
    command: &str,
) -> Result<(), CliError> {
    if stdin.is_some() {
        return Err(CliError::usage(format!(
            "{command} accepts only one of --stdin or --stdin-file"
        )));
    }
    *stdin = Some(source);
    Ok(())
}

fn read_qjs_stdin(
    source: Option<QjsStdin>,
    process_stdin: &mut dyn Read,
) -> Result<Option<Vec<u8>>, CliError> {
    match source {
        None => Ok(None),
        Some(QjsStdin::Bytes(bytes)) => Ok(Some(bytes)),
        Some(QjsStdin::File(path)) => std::fs::read(&path).map(Some).map_err(|error| {
            CliError::new(
                format!("failed to read stdin file {}: {error}", path.display()),
                1,
            )
        }),
        Some(QjsStdin::Process) => {
            let mut bytes = Vec::new();
            process_stdin.read_to_end(&mut bytes).map_err(|error| {
                CliError::new(format!("failed to read process stdin: {error}"), 1)
            })?;
            Ok(Some(bytes))
        }
    }
}

fn parse_host_mount(value: &str, label: &str) -> Result<HostMount, CliError> {
    let Some((host, guest)) = value.split_once('=') else {
        return Err(CliError::usage(format!("{label} expects HOST=GUEST")));
    };
    if host.is_empty() || guest.is_empty() {
        return Err(CliError::usage(format!("{label} expects HOST=GUEST")));
    }
    let guest_path = NormalizedPath::new(guest)?;
    if guest_path.as_str() == "." {
        return Err(CliError::usage(format!(
            "{label} guest path must not be . in this demo"
        )));
    }
    Ok(HostMount {
        host_path: PathBuf::from(host),
        guest_path,
    })
}

fn parse_qjs_snapshot_file_command(
    args: &[OsString],
    command: &str,
) -> Result<QjsSnapshotFileCommand, CliError> {
    let mut env = Vec::new();
    let mut cwd = NormalizedPath::new(".")?;
    let mut mounts = Vec::new();
    let mut stdin = None;
    let mut event_loop_wait_budget = Duration::ZERO;
    let mut ready_io_turns = 1usize;
    let mut interrupt_poll_budget = None;
    let mut memory_limit_bytes = None;
    let mut snapshot_path = None;
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--env" {
            i += 1;
            let value = args
                .get(i)
                .ok_or_else(|| CliError::usage(format!("{command} --env expects KEY=VALUE")))?;
            let value = os_arg_to_string(value, &format!("{command} --env"))?;
            validate_env_line(&value, &format!("{command} --env"))?;
            env.push(value);
            i += 1;
        } else if args[i] == "--cwd" {
            i += 1;
            let value = args
                .get(i)
                .ok_or_else(|| CliError::usage(format!("{command} --cwd expects a Wanix path")))?;
            cwd = NormalizedPath::new(os_arg_to_string(value, &format!("{command} --cwd"))?)?;
            i += 1;
        } else if args[i] == "--stdin" {
            i += 1;
            let value = args
                .get(i)
                .ok_or_else(|| CliError::usage(format!("{command} --stdin expects text")))?;
            set_qjs_stdin(
                &mut stdin,
                QjsStdin::Bytes(
                    os_arg_to_string(value, &format!("{command} --stdin"))?.into_bytes(),
                ),
                command,
            )?;
            i += 1;
        } else if args[i] == "--stdin-file" {
            i += 1;
            let value = args.get(i).ok_or_else(|| {
                CliError::usage(format!("{command} --stdin-file expects PATH or -"))
            })?;
            let source = if value == "-" {
                QjsStdin::Process
            } else {
                QjsStdin::File(PathBuf::from(value))
            };
            set_qjs_stdin(&mut stdin, source, command)?;
            i += 1;
        } else if args[i] == "--interrupt-after" {
            i += 1;
            let value = args.get(i).ok_or_else(|| {
                CliError::usage(format!("{command} --interrupt-after expects a count"))
            })?;
            interrupt_poll_budget =
                Some(parse_usize(value, &format!("{command} --interrupt-after"))?);
            i += 1;
        } else if args[i] == "--event-loop-ms" {
            i += 1;
            let value = args.get(i).ok_or_else(|| {
                CliError::usage(format!("{command} --event-loop-ms expects milliseconds"))
            })?;
            event_loop_wait_budget =
                parse_duration_millis(value, &format!("{command} --event-loop-ms"))?;
            i += 1;
        } else if args[i] == "--ready-io-turns" {
            i += 1;
            let value = args.get(i).ok_or_else(|| {
                CliError::usage(format!("{command} --ready-io-turns expects a count"))
            })?;
            ready_io_turns = parse_usize(value, &format!("{command} --ready-io-turns"))?;
            i += 1;
        } else if args[i] == "--memory-limit-bytes" {
            i += 1;
            let value = args.get(i).ok_or_else(|| {
                CliError::usage(format!(
                    "{command} --memory-limit-bytes expects a byte count"
                ))
            })?;
            memory_limit_bytes = Some(parse_u32(
                value,
                &format!("{command} --memory-limit-bytes"),
            )?);
            i += 1;
        } else if args[i] == "--mount" {
            i += 1;
            let value = args
                .get(i)
                .ok_or_else(|| CliError::usage(format!("{command} --mount expects HOST=GUEST")))?;
            mounts.push(parse_host_mount(
                &os_arg_to_string(value, &format!("{command} --mount"))?,
                &format!("{command} --mount"),
            )?);
            i += 1;
        } else if args[i] == "--snapshot" {
            i += 1;
            let value = args
                .get(i)
                .ok_or_else(|| CliError::usage(format!("{command} --snapshot expects FILE")))?;
            if snapshot_path.is_some() {
                return Err(CliError::usage(format!(
                    "{command} accepts only one --snapshot"
                )));
            }
            snapshot_path = Some(PathBuf::from(value));
            i += 1;
        } else if args[i] == "--" {
            i += 1;
            break;
        } else {
            break;
        }
    }

    let snapshot_path = snapshot_path
        .ok_or_else(|| CliError::usage(format!("{command} requires --snapshot FILE")))?;
    let script = args
        .get(i)
        .ok_or_else(|| CliError::usage(format!("{command} expects a script path")))?;
    let script_path = PathBuf::from(script);
    i += 1;

    if args.get(i).is_some_and(|arg| arg == "--") {
        i += 1;
    }

    let js_args = args[i..]
        .iter()
        .map(|arg| os_arg_to_string(arg, &format!("{command} script arg")))
        .collect::<Result<Vec<_>, CliError>>()?;

    Ok(QjsSnapshotFileCommand {
        script_path,
        snapshot_path,
        args: js_args,
        env,
        cwd,
        stdin,
        event_loop_wait_budget,
        ready_io_turns,
        interrupt_poll_budget,
        memory_limit_bytes,
        mounts,
    })
}

fn parse_qjs_restore_command(args: &[OsString]) -> Result<QjsRestoreCommand, CliError> {
    let mut cwd = NormalizedPath::new(".")?;
    let mut mounts = Vec::new();
    let mut before_args = Vec::new();
    let mut after_args = Vec::new();
    let mut before_env = Vec::new();
    let mut after_env = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--cwd" {
            i += 1;
            let value = args
                .get(i)
                .ok_or_else(|| CliError::usage("qjs-restore --cwd expects a Wanix path"))?;
            cwd = NormalizedPath::new(os_arg_to_string(value, "qjs-restore --cwd")?)?;
            i += 1;
        } else if args[i] == "--before-env" {
            i += 1;
            let value = args
                .get(i)
                .ok_or_else(|| CliError::usage("qjs-restore --before-env expects KEY=VALUE"))?;
            let value = os_arg_to_string(value, "qjs-restore --before-env")?;
            validate_env_line(&value, "qjs-restore --before-env")?;
            before_env.push(value);
            i += 1;
        } else if args[i] == "--after-env" {
            i += 1;
            let value = args
                .get(i)
                .ok_or_else(|| CliError::usage("qjs-restore --after-env expects KEY=VALUE"))?;
            let value = os_arg_to_string(value, "qjs-restore --after-env")?;
            validate_env_line(&value, "qjs-restore --after-env")?;
            after_env.push(value);
            i += 1;
        } else if args[i] == "--before-arg" {
            i += 1;
            let value = args
                .get(i)
                .ok_or_else(|| CliError::usage("qjs-restore --before-arg expects VALUE"))?;
            before_args.push(os_arg_to_string(value, "qjs-restore --before-arg")?);
            i += 1;
        } else if args[i] == "--after-arg" {
            i += 1;
            let value = args
                .get(i)
                .ok_or_else(|| CliError::usage("qjs-restore --after-arg expects VALUE"))?;
            after_args.push(os_arg_to_string(value, "qjs-restore --after-arg")?);
            i += 1;
        } else if args[i] == "--mount" {
            i += 1;
            let value = args
                .get(i)
                .ok_or_else(|| CliError::usage("qjs-restore --mount expects HOST=GUEST"))?;
            mounts.push(parse_host_mount(
                &os_arg_to_string(value, "qjs-restore --mount")?,
                "qjs-restore --mount",
            )?);
            i += 1;
        } else if args[i] == "--" {
            i += 1;
            break;
        } else {
            break;
        }
    }

    let before_script = args
        .get(i)
        .ok_or_else(|| CliError::usage("qjs-restore expects before.js and after.js"))?;
    let before_script_path = PathBuf::from(before_script);
    i += 1;

    let after_script = args
        .get(i)
        .ok_or_else(|| CliError::usage("qjs-restore expects before.js and after.js"))?;
    let after_script_path = PathBuf::from(after_script);
    i += 1;

    if let Some(extra) = args.get(i) {
        return Err(CliError::usage(format!(
            "unexpected qjs-restore argument: {}",
            extra.to_string_lossy()
        )));
    }

    Ok(QjsRestoreCommand {
        before_script_path,
        after_script_path,
        before_args,
        after_args,
        before_env,
        after_env,
        cwd,
        mounts,
    })
}

fn os_arg_to_string(arg: &OsString, label: &str) -> Result<String, CliError> {
    arg.clone()
        .into_string()
        .map_err(|_| CliError::usage(format!("{label} must be valid UTF-8")))
}

fn parse_duration_millis(arg: &OsString, label: &str) -> Result<Duration, CliError> {
    let value = os_arg_to_string(arg, label)?;
    let millis = value
        .parse::<u64>()
        .map_err(|_| CliError::usage(format!("{label} expects a non-negative integer")))?;
    Ok(Duration::from_millis(millis))
}

fn parse_usize(arg: &OsString, label: &str) -> Result<usize, CliError> {
    let value = os_arg_to_string(arg, label)?;
    value
        .parse::<usize>()
        .map_err(|_| CliError::usage(format!("{label} expects a non-negative integer")))
}

fn parse_u32(arg: &OsString, label: &str) -> Result<u32, CliError> {
    let value = os_arg_to_string(arg, label)?;
    value
        .parse::<u32>()
        .map_err(|_| CliError::usage(format!("{label} expects a 32-bit non-negative integer")))
}

fn validate_env_line(line: &str, label: &str) -> Result<(), CliError> {
    let Some((key, _value)) = line.split_once('=') else {
        return Err(CliError::usage(format!("{label} expects KEY=VALUE")));
    };
    if key.is_empty() || key.chars().any(char::is_whitespace) || line.contains('\n') {
        return Err(CliError::usage(format!("{label} expects KEY=VALUE")));
    }
    Ok(())
}

fn task_cmd(program: &str, args: &[String]) -> String {
    quote_cmd_argv(std::iter::once(program).chain(args.iter().map(String::as_str)))
}

fn qjs_task_spec(
    program: &str,
    args: &[String],
    env: &[String],
    cwd: &NormalizedPath,
) -> Result<TaskSpec, CliError> {
    let mut spec = TaskSpec::new(program)?;
    spec.args = args.to_vec();
    spec.env = env_map(env);
    spec.cwd = cwd.clone();
    Ok(spec)
}

fn configure_qjs_task(
    task: &Task,
    program: &str,
    args: &[String],
    env: &[String],
    cwd: &NormalizedPath,
) -> Result<(), CliError> {
    task.set_spec(qjs_task_spec(program, args, env, cwd)?)?;
    task.set_cmd(task_cmd(program, args))?;
    task.set_env_lines(env.join("\n"))?;
    task.set_dir(cwd.to_string())?;
    Ok(())
}

fn bind_child_output_to_parent(child: &Task, parent: &Task) -> Result<(), CliError> {
    let parent_id = parent.id().get();
    child.bind_fd_from_namespace(format!("#task/{parent_id}/fd/1"), Fd::STDOUT)?;
    child.bind_fd_from_namespace(format!("#task/{parent_id}/fd/2"), Fd::STDERR)?;
    Ok(())
}

fn attach_task_stdio(
    task: &Task,
    stdin_bytes: Option<Vec<u8>>,
) -> Result<(Arc<MemFs>, Arc<MemFs>), CliError> {
    attach_task_stdin(task, stdin_bytes)?;
    let stdout = Arc::new(MemFs::new());
    stdout.write_file("stdout", b"")?;
    task.insert_fd(
        Fd::STDOUT,
        stdout.open(&NormalizedPath::new("stdout")?, OpenOptions::read_write())?,
        NormalizedPath::new("stdout")?,
    )?;
    let stderr = Arc::new(MemFs::new());
    stderr.write_file("stderr", b"")?;
    task.insert_fd(
        Fd::STDERR,
        stderr.open(&NormalizedPath::new("stderr")?, OpenOptions::read_write())?,
        NormalizedPath::new("stderr")?,
    )?;
    Ok((stdout, stderr))
}

fn attach_task_stdin(task: &Task, stdin_bytes: Option<Vec<u8>>) -> Result<(), CliError> {
    if let Some(stdin_bytes) = stdin_bytes {
        let stdin = Arc::new(MemFs::new());
        stdin.write_file("stdin", stdin_bytes)?;
        task.insert_fd(
            Fd::STDIN,
            stdin.open(&NormalizedPath::new("stdin")?, OpenOptions::read())?,
            NormalizedPath::new("stdin")?,
        )?;
    }
    Ok(())
}

fn finish_cli_task_output(
    command: &str,
    result: Result<(), CliError>,
    task: &Task,
    stdout: &Arc<MemFs>,
    stderr: &Arc<MemFs>,
) -> Result<CliOutput, CliError> {
    let stdout = read_file(stdout.as_ref(), "stdout")?;
    let mut stderr = read_file(stderr.as_ref(), "stderr")?;
    match result {
        Ok(()) => Ok(CliOutput::new(stdout, stderr, parse_exit(&task.exit()))),
        Err(error) => {
            if !stderr.is_empty() && !stderr.ends_with(b"\n") {
                stderr.push(b'\n');
            }
            stderr.extend_from_slice(format!("wanix-rust {command}: {error}\n").as_bytes());
            Ok(CliOutput::new(stdout, stderr, 1))
        }
    }
}

fn bind_host_mounts(task: &Task, mounts: &[HostMount]) -> Result<(), CliError> {
    for mount in mounts {
        let local = Arc::new(LocalFs::new(&mount.host_path).map_err(|error| {
            CliError::new(
                format!(
                    "failed to mount {} at {}: {error}",
                    mount.host_path.display(),
                    mount.guest_path
                ),
                1,
            )
        })?);
        task.bind(
            local,
            ".",
            mount.guest_path.as_str(),
            BindOptions::default(),
        )?;
    }
    Ok(())
}

fn ensure_snapshot_task_fds_closed(task: &Task) -> Result<(), CliError> {
    let dynamic_fds = task
        .fd_numbers()
        .into_iter()
        .filter(|fd| fd.get() > Fd::STDERR.get())
        .map(|fd| fd.get().to_string())
        .collect::<Vec<_>>();
    if dynamic_fds.is_empty() {
        return Ok(());
    }
    Err(CliError::new(
        format!(
            "cannot snapshot qjs task with open Wanix task fds: {}",
            dynamic_fds.join(", ")
        ),
        1,
    ))
}

fn apply_qjs_task_runtime_limits(
    runtime: &mut QuickJsTaskRuntime,
    interrupt_poll_budget: Option<usize>,
    bytes: Option<u32>,
) -> Result<(), CliError> {
    if let Some(polls) = interrupt_poll_budget {
        runtime.set_interrupt_poll_budget(polls)?;
    }
    if let Some(bytes) = bytes {
        runtime.set_memory_limit(bytes)?;
    }
    Ok(())
}

fn eval_qjs_source(
    runtime: &mut QuickJsTaskRuntime,
    source: &str,
    filename: &str,
    event_loop_wait_budget: Duration,
    ready_io_turns: usize,
) -> Result<(), CliError> {
    if uses_module_syntax(source) {
        runtime.eval_module_discard_with_event_loop_limits(
            source,
            filename,
            event_loop_wait_budget,
            ready_io_turns,
        )?;
    } else {
        runtime.eval_discard_with_event_loop_limits(
            source,
            event_loop_wait_budget,
            ready_io_turns,
        )?;
    }
    Ok(())
}

fn uses_module_syntax(source: &str) -> bool {
    source.lines().any(|line| {
        let line = line.trim_start();
        line.starts_with("import ") || line.starts_with("export ")
    })
}

fn env_map(lines: &[String]) -> BTreeMap<String, String> {
    lines
        .iter()
        .filter_map(|line| {
            let (key, value) = line.split_once('=')?;
            Some((key.to_owned(), value.to_owned()))
        })
        .collect()
}

fn quickjs_runner() -> Result<Arc<QuickJsRunner>, CliError> {
    match std::env::var_os("WANIX_QJS_WASM") {
        Some(path) => QuickJsRunner::from_wasm_file(PathBuf::from(path)).map(Arc::new),
        None => bundled_quickjs_runner(),
    }
    .map_err(CliError::from)
}

#[cfg(not(test))]
fn bundled_quickjs_runner() -> Result<Arc<QuickJsRunner>, FsError> {
    QuickJsRunner::from_bundled_wasm().map(Arc::new)
}

#[cfg(test)]
fn bundled_quickjs_runner() -> Result<Arc<QuickJsRunner>, FsError> {
    static RUNNER: OnceLock<Result<Arc<QuickJsRunner>, String>> = OnceLock::new();
    match RUNNER.get_or_init(|| {
        QuickJsRunner::from_bundled_wasm()
            .map(Arc::new)
            .map_err(|err| err.to_string())
    }) {
        Ok(runner) => Ok(Arc::clone(runner)),
        Err(error) => Err(FsError::Other(error.clone())),
    }
}

fn copy_script_directory(
    script_path: &Path,
    root: &MemFs,
    cwd: &NormalizedPath,
) -> Result<(), CliError> {
    copy_script_directory_into(script_path, root, cwd, ".")
}

fn copy_script_directory_into(
    script_path: &Path,
    root: &MemFs,
    cwd: &NormalizedPath,
    guest_dir: &str,
) -> Result<(), CliError> {
    let base = script_path.parent().unwrap_or_else(|| Path::new("."));
    let guest_dir = NormalizedPath::new(guest_dir)?;
    copy_directory_tree(base, base, root, cwd, &guest_dir)
}

fn copy_directory_tree(
    base: &Path,
    dir: &Path,
    root: &MemFs,
    cwd: &NormalizedPath,
    guest_dir: &NormalizedPath,
) -> Result<(), CliError> {
    for entry in std::fs::read_dir(dir).map_err(|error| {
        CliError::new(
            format!("failed to read directory {}: {error}", dir.display()),
            1,
        )
    })? {
        let entry = entry.map_err(|error| {
            CliError::new(
                format!(
                    "failed to read directory entry in {}: {error}",
                    dir.display()
                ),
                1,
            )
        })?;
        let path = entry.path();
        let file_type = entry.file_type().map_err(|error| {
            CliError::new(format!("failed to stat {}: {error}", path.display()), 1)
        })?;
        if file_type.is_dir() {
            copy_directory_tree(base, &path, root, cwd, guest_dir)?;
            continue;
        }
        if !file_type.is_file() {
            continue;
        }
        let Some(guest_path) = guest_path_for_host_file(base, &path)? else {
            continue;
        };
        let bytes = std::fs::read(&path).map_err(|error| {
            CliError::new(format!("failed to read {}: {error}", path.display()), 1)
        })?;
        let guest_path = guest_path_under_dir(guest_dir, &guest_path)?;
        let guest_path = guest_path_in_cwd(cwd, &guest_path)?;
        root.write_file(guest_path, bytes)?;
    }
    Ok(())
}

fn guest_path_under_dir(guest_dir: &NormalizedPath, path: &str) -> Result<String, CliError> {
    if guest_dir.as_str() == "." {
        return Ok(path.to_owned());
    }
    Ok(NormalizedPath::new(format!("{guest_dir}/{path}"))?.to_string())
}

fn guest_path_in_cwd(cwd: &NormalizedPath, path: &str) -> Result<String, CliError> {
    if cwd.as_str() == "." {
        return Ok(path.to_owned());
    }
    Ok(NormalizedPath::new(format!("{cwd}/{path}"))?.to_string())
}

fn guest_path_for_host_file(base: &Path, path: &Path) -> Result<Option<String>, CliError> {
    let relative = path.strip_prefix(base).map_err(|error| {
        CliError::new(
            format!(
                "failed to map {} under {}: {error}",
                path.display(),
                base.display()
            ),
            1,
        )
    })?;
    let Some(path) = relative.to_str() else {
        return Ok(None);
    };
    let path = path.replace(std::path::MAIN_SEPARATOR, "/");
    if path.is_empty() || NormalizedPath::new(&path).is_err() {
        return Ok(None);
    }
    Ok(Some(path))
}

fn read_utf8_script(path: &Path) -> Result<String, CliError> {
    let script = std::fs::read(path)
        .map_err(|error| CliError::new(format!("failed to read {}: {error}", path.display()), 1))?;
    String::from_utf8(script).map_err(|error| {
        CliError::new(
            format!("script {} is not valid UTF-8: {error}", path.display()),
            1,
        )
    })
}

fn read_file(fs: &dyn FileSystem, path: &str) -> Result<Vec<u8>, CliError> {
    let mut file = fs.open(&NormalizedPath::new(path)?, OpenOptions::read())?;
    let mut out = Vec::new();
    let mut buf = [0; 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            return Ok(out);
        }
        out.extend_from_slice(&buf[..n]);
    }
}

fn parse_exit(exit: &str) -> i32 {
    exit.trim().parse().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::{self, Read, Write};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::{run, run_with_process_io, run_with_process_stdin};

    static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

    struct MarkerCheckedStdin {
        marker: PathBuf,
        bytes: Vec<u8>,
        offset: usize,
    }

    impl MarkerCheckedStdin {
        fn new(marker: PathBuf, bytes: impl Into<Vec<u8>>) -> Self {
            Self {
                marker,
                bytes: bytes.into(),
                offset: 0,
            }
        }
    }

    impl Read for MarkerCheckedStdin {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if !self.marker.exists() {
                return Err(io::Error::other(
                    "process stdin was read before qjs eval marker",
                ));
            }
            let remaining = self.bytes.len().saturating_sub(self.offset);
            let len = remaining.min(buf.len());
            if len == 0 {
                return Ok(0);
            }
            buf[..len].copy_from_slice(&self.bytes[self.offset..self.offset + len]);
            self.offset += len;
            Ok(len)
        }
    }

    struct MarkerStdout {
        marker: PathBuf,
        marker_bytes: Vec<u8>,
        needle: Vec<u8>,
        bytes: Vec<u8>,
    }

    impl MarkerStdout {
        fn new(marker: PathBuf, needle: impl Into<Vec<u8>>) -> Self {
            Self {
                marker,
                marker_bytes: b"streamed".to_vec(),
                needle: needle.into(),
                bytes: Vec::new(),
            }
        }

        fn bytes(&self) -> &[u8] {
            &self.bytes
        }
    }

    impl Write for MarkerStdout {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.bytes.extend_from_slice(buf);
            if self
                .bytes
                .windows(self.needle.len())
                .any(|window| window == self.needle)
            {
                fs::write(&self.marker, &self.marker_bytes)?;
            }
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    struct EofForbiddenStdin {
        bytes: Vec<u8>,
        offset: usize,
    }

    impl EofForbiddenStdin {
        fn new(bytes: impl Into<Vec<u8>>) -> Self {
            Self {
                bytes: bytes.into(),
                offset: 0,
            }
        }
    }

    impl Read for EofForbiddenStdin {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if self.offset == self.bytes.len() {
                return Err(io::Error::other(
                    "process stdin was read after the scripted terminal exit",
                ));
            }
            let remaining = self.bytes.len() - self.offset;
            let len = remaining.min(buf.len());
            buf[..len].copy_from_slice(&self.bytes[self.offset..self.offset + len]);
            self.offset += len;
            Ok(len)
        }
    }

    #[test]
    fn help_mentions_qjs_demo_target() {
        let output = run(["--help"]).unwrap();

        assert_eq!(output.exit_code(), 0);
        assert!(String::from_utf8_lossy(output.stdout()).contains("wanix-rust qjs"));
        assert!(String::from_utf8_lossy(output.stdout()).contains("wanix-rust qjs-term"));
        assert!(String::from_utf8_lossy(output.stdout()).contains("--feed-after-eval TEXT"));
        assert!(String::from_utf8_lossy(output.stdout()).contains("--feed-after-eval-file PATH|-"));
        assert!(
            String::from_utf8_lossy(output.stdout()).contains("--feed-after-eval-lines PATH|-")
        );
        assert!(String::from_utf8_lossy(output.stdout()).contains("--mount HOST=GUEST"));
        assert!(String::from_utf8_lossy(output.stdout()).contains("--stdin-file PATH|-"));
        assert!(String::from_utf8_lossy(output.stdout()).contains("wanix-rust qjs-snapshot"));
        assert!(String::from_utf8_lossy(output.stdout()).contains("wanix-rust qjs-resume"));
        assert!(String::from_utf8_lossy(output.stdout()).contains("wanix-rust qjs-restore"));
        assert!(output.stderr().is_empty());
    }

    #[test]
    fn qjs_command_runs_script_outside_chrome() {
        let script = write_temp_script(
            "demo script.js",
            r##"
import * as std from "qjs:std";
import { runtime } from "./lib.js";

const text = std.loadFile("main.js");
std.writeFile("created.txt", "made inside Wanix");
std.out.puts("task " + std.loadFile("#task/self/id").trim() + "\n");
std.out.puts(runtime + "\n");
std.out.puts(text.includes("made inside Wanix") + " " + std.loadFile("created.txt") + "\n");
std.out.flush();
"##,
        );
        fs::write(
            script.parent().unwrap().join("lib.js"),
            "export const runtime = 'Wanix ES module loader';",
        )
        .unwrap();

        let output = run(["qjs".into(), script.into_os_string()]).unwrap();

        assert_eq!(output.exit_code(), 0);
        assert_eq!(
            output.stdout(),
            b"task 1\nWanix ES module loader\ntrue made inside Wanix\n"
        );
        assert!(output.stderr().is_empty());
    }

    #[test]
    fn qjs_command_runs_quickjs_std_stdout_outside_chrome() {
        let script = write_temp_script(
            "std-demo.js",
            r#"
import * as std from "qjs:std";

std.out.puts("hello std\n");
std.out.flush();
"#,
        );

        let output = run(["qjs".into(), script.into_os_string()]).unwrap();

        assert_eq!(output.exit_code(), 0);
        assert_eq!(output.stdout(), b"hello std\n");
        assert!(output.stderr().is_empty());
    }

    #[test]
    fn qjs_term_command_runs_script_through_terminal_fds() {
        let script = write_temp_script(
            "term-demo.js",
            r##"
import * as std from "qjs:std";
import * as os from "qjs:os";

const bytes = new Uint8Array(64);
const count = os.read(0, bytes.buffer, 0, bytes.length);
const input = Array.from(bytes.slice(0, count)).map((byte) => String.fromCharCode(byte)).join("");

std.out.puts("task " + std.loadFile("#task/self/id").trim() + "\n");
std.out.puts("term " + std.loadFile("#term/1/id").trim() + "\n");
std.out.puts("input " + input.trimEnd() + "\n");
std.out.flush();
std.err.puts("stderr on terminal\n");
std.err.flush();
"##,
        );

        let output = run([
            "qjs-term".into(),
            "--stdin".into(),
            "typed input\n".into(),
            script.into_os_string(),
        ])
        .unwrap();

        assert_eq!(output.exit_code(), 0);
        assert_eq!(
            output.stdout(),
            b"task 1\r\nterm 1\r\ninput typed input\r\nstderr on terminal\r\n"
        );
        assert!(output.stderr().is_empty());
    }

    #[test]
    fn qjs_term_example_runs_terminal_transcript_demo() {
        let output = run([
            "qjs-term".into(),
            "--stdin".into(),
            "from native stdin\n".into(),
            example_script("qjs-term-demo.js").into_os_string(),
        ])
        .unwrap();

        assert_eq!(output.exit_code(), 0);
        assert_eq!(
            output.stdout(),
            b"terminal task: 1\r\nterminal id: 1\r\nterminal input: from native stdin\r\nterminal stderr: same screen\r\n"
        );
        assert!(output.stderr().is_empty());
    }

    #[test]
    fn qjs_term_example_ready_io_handler_reads_terminal_stdin_in_turns() {
        let output = run([
            "qjs-term".into(),
            "--stdin".into(),
            "abcdef".into(),
            "--ready-io-turns".into(),
            "2".into(),
            example_script("qjs-term-ready-io-demo.js").into_os_string(),
        ])
        .unwrap();

        assert_eq!(output.exit_code(), 0);
        assert_eq!(
            output.stdout(),
            b"terminal task: 1\r\nterminal id: 1\r\nsync\r\nterminal chunk 1: abcd\r\nterminal chunk 2: ef\r\nterminal handler done\r\n"
        );
        assert!(output.stderr().is_empty());
    }

    #[test]
    fn qjs_term_ready_io_does_not_fire_empty_terminal_stdin() {
        let script = write_temp_script(
            "term-idle-stdin.js",
            r##"
import * as std from "qjs:std";
import * as os from "qjs:os";

std.out.puts("armed\n");
os.setReadHandler(0, () => {
  const bytes = new Uint8Array(8);
  const count = os.read(0, bytes.buffer, 0, bytes.length);
  std.out.puts("unexpected read " + count + "\n");
  os.setReadHandler(0, null);
  std.out.flush();
});
std.out.flush();
"##,
        );

        let output = run([
            "qjs-term".into(),
            "--ready-io-turns".into(),
            "1".into(),
            script.into_os_string(),
        ])
        .unwrap();

        assert_eq!(output.exit_code(), 0);
        assert_eq!(output.stdout(), b"armed\r\n");
        assert!(output.stderr().is_empty());
    }

    #[test]
    fn qjs_term_example_feeds_terminal_input_after_eval() {
        let output = run([
            "qjs-term".into(),
            "--ready-io-turns".into(),
            "2".into(),
            "--feed-after-eval".into(),
            "abcd".into(),
            "--feed-after-eval".into(),
            "ef".into(),
            example_script("qjs-term-post-eval-feed-demo.js").into_os_string(),
        ])
        .unwrap();

        assert_eq!(output.exit_code(), 0);
        assert_eq!(
            output.stdout(),
            b"terminal task: 1\r\nterminal id: 1\r\nwaiting for terminal input\r\npost-eval chunk 1: abcd\r\npost-eval chunk 2: ef\r\npost-eval handler done\r\n"
        );
        assert!(output.stderr().is_empty());
    }

    #[test]
    fn qjs_term_shell_example_feeds_native_stdin_after_eval() {
        let output = run_with_process_stdin(
            [
                "qjs-term".into(),
                "--ready-io-turns".into(),
                "1".into(),
                "--feed-after-eval-file".into(),
                "-".into(),
                example_script("qjs-term-shell-demo.js").into_os_string(),
            ],
            b"echo hello terminal\nid\nexit\n".as_slice(),
        )
        .unwrap();

        assert_eq!(output.exit_code(), 0);
        assert_eq!(
            output.stdout(),
            b"shell task: 1\r\n$ hello terminal\r\n$ 1\r\n$ bye\r\n"
        );
        assert!(output.stderr().is_empty());
    }

    #[test]
    fn qjs_term_line_session_feeds_native_stdin_as_terminal_events() {
        let script = write_temp_script(
            "term-line-session.js",
            r##"
import * as std from "qjs:std";
import * as os from "qjs:os";

function stringFromBytes(bytes, count) {
  return Array.from(bytes.slice(0, count)).map((byte) => String.fromCharCode(byte)).join("");
}

function escaped(text) {
  return text.replace(/\n/g, "\\n");
}

let events = 0;
const bytes = new Uint8Array(64);

std.out.puts("session task: " + std.loadFile("#task/self/id").trim() + "\n");

os.setReadHandler(0, () => {
  const count = os.read(0, bytes.buffer, 0, bytes.length);
  if (count < 0) {
    throw new Error("line session terminal read failed: " + count);
  }
  events += 1;
  const text = stringFromBytes(bytes, count);
  std.out.puts("event " + events + ": " + escaped(text) + "\n");
  if (text === "exit\n") {
    os.setReadHandler(0, null);
  }
  std.out.flush();
});

std.out.flush();
"##,
        );

        let output = run_with_process_stdin(
            [
                "qjs-term".into(),
                "--ready-io-turns".into(),
                "1".into(),
                "--feed-after-eval-lines".into(),
                "-".into(),
                script.into_os_string(),
            ],
            b"first\nsecond\nexit\n".as_slice(),
        )
        .unwrap();

        assert_eq!(output.exit_code(), 0);
        assert_eq!(
            output.stdout(),
            b"session task: 1\r\nevent 1: first\\n\r\nevent 2: second\\n\r\nevent 3: exit\\n\r\n"
        );
        assert!(output.stderr().is_empty());
    }

    #[test]
    fn qjs_term_process_line_feed_reads_native_stdin_after_eval() {
        let host = temp_dir("wanix-cli-term-stream-order");
        let marker = host.join("armed.txt");
        let script = write_temp_script(
            "term-stream-order.js",
            r##"
import * as std from "qjs:std";
import * as os from "qjs:os";

function stringFromBytes(bytes, count) {
  return Array.from(bytes.slice(0, count)).map((byte) => String.fromCharCode(byte)).join("");
}

std.writeFile("host/armed.txt", "ready");
std.out.puts("armed\n");

os.setReadHandler(0, () => {
  const bytes = new Uint8Array(64);
  const count = os.read(0, bytes.buffer, 0, bytes.length);
  std.out.puts("stream " + stringFromBytes(bytes, count));
  os.setReadHandler(0, null);
  std.out.flush();
});

std.out.flush();
"##,
        );

        let output = run_with_process_stdin(
            [
                "qjs-term".into(),
                "--mount".into(),
                format!("{}=host", host.display()).into(),
                "--ready-io-turns".into(),
                "1".into(),
                "--feed-after-eval-lines".into(),
                "-".into(),
                script.into_os_string(),
            ],
            MarkerCheckedStdin::new(marker.clone(), b"streamed line\n"),
        )
        .unwrap();

        assert_eq!(output.exit_code(), 0);
        assert_eq!(output.stdout(), b"armed\r\nstream streamed line\r\n");
        assert!(output.stderr().is_empty());
        assert_eq!(fs::read(marker).unwrap(), b"ready");
        fs::remove_dir_all(host).unwrap();
    }

    #[test]
    fn qjs_term_streams_prompt_before_reading_native_stdin_line() {
        let host = temp_dir("wanix-cli-term-streamed-output-order");
        let marker = host.join("prompt-streamed.txt");
        let mut stdout = MarkerStdout::new(marker.clone(), b"$ ");
        let mut stderr = Vec::new();

        let exit_code = run_with_process_io(
            [
                "qjs-term".into(),
                "--ready-io-turns".into(),
                "1".into(),
                "--feed-after-eval-lines".into(),
                "-".into(),
                example_script("qjs-term-shell-demo.js").into_os_string(),
            ],
            MarkerCheckedStdin::new(marker.clone(), b"exit\n"),
            &mut stdout,
            &mut stderr,
        )
        .unwrap();

        assert_eq!(exit_code, 0);
        assert_eq!(stdout.bytes(), b"shell task: 1\r\n$ bye\r\n");
        assert!(stderr.is_empty());
        assert_eq!(fs::read(marker).unwrap(), b"streamed");
        fs::remove_dir_all(host).unwrap();
    }

    #[test]
    fn qjs_term_line_feed_stops_after_guest_exit_without_process_eof() {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        let exit_code = run_with_process_io(
            [
                "qjs-term".into(),
                "--ready-io-turns".into(),
                "1".into(),
                "--feed-after-eval-lines".into(),
                "-".into(),
                example_script("qjs-term-shell-demo.js").into_os_string(),
            ],
            EofForbiddenStdin::new(b"exit\n"),
            &mut stdout,
            &mut stderr,
        )
        .unwrap();

        assert_eq!(exit_code, 0);
        assert_eq!(stdout, b"shell task: 1\r\n$ bye\r\n");
        assert!(stderr.is_empty());
    }

    #[test]
    fn qjs_command_reads_script_sibling_with_quickjs_std_load_file() {
        let script = write_temp_script(
            "std-read-demo.js",
            r#"
import * as std from "qjs:std";

print(std.loadFile("input.txt"));
"#,
        );
        fs::write(script.parent().unwrap().join("input.txt"), "hello std file").unwrap();

        let output = run(["qjs".into(), script.into_os_string()]).unwrap();

        assert_eq!(output.exit_code(), 0);
        assert_eq!(output.stdout(), b"hello std file\n");
        assert!(output.stderr().is_empty());
    }

    #[test]
    fn qjs_command_reads_task_id_with_quickjs_std_load_file() {
        let script = write_temp_script(
            "std-task-id-demo.js",
            r##"
import * as std from "qjs:std";

print("source", std.loadFile("main.js").includes("qjs:std"));
print("id", std.loadFile("#task/self/id").trim());
"##,
        );

        let output = run([
            "qjs".into(),
            "--cwd".into(),
            "app".into(),
            script.into_os_string(),
        ])
        .unwrap();

        assert_eq!(output.exit_code(), 0);
        assert_eq!(output.stdout(), b"source true\nid 1\n");
        assert!(output.stderr().is_empty());
    }

    #[test]
    fn qjs_command_writes_script_sibling_with_quickjs_std_write_file() {
        let script = write_temp_script(
            "std-write-demo.js",
            r#"
import * as std from "qjs:std";

std.writeFile("created.txt", "hello from std write");
print(std.loadFile("created.txt"));
"#,
        );

        let output = run(["qjs".into(), script.clone().into_os_string()]).unwrap();

        assert_eq!(output.exit_code(), 0);
        assert_eq!(output.stdout(), b"hello from std write\n");
        assert!(output.stderr().is_empty());
    }

    #[test]
    fn qjs_command_mounts_host_directory_into_wanix_namespace() {
        let host = temp_dir("wanix-cli-mount");
        fs::write(host.join("input.txt"), "from host").unwrap();
        let script = write_temp_script(
            "mount-demo.js",
            r#"
import * as std from "qjs:std";

std.out.puts(std.loadFile("host/input.txt") + "\n");
std.writeFile("host/output.txt", "from qjs std");
std.out.puts(std.loadFile("host/output.txt") + "\n");
std.out.flush();
"#,
        );

        let output = run([
            "qjs".into(),
            "--mount".into(),
            format!("{}=host", host.display()).into(),
            script.into_os_string(),
        ])
        .unwrap();

        assert_eq!(output.exit_code(), 0);
        assert_eq!(output.stdout(), b"from host\nfrom qjs std\n");
        assert!(output.stderr().is_empty());
        assert_eq!(fs::read(host.join("output.txt")).unwrap(), b"from qjs std");
        fs::remove_dir_all(host).unwrap();
    }

    #[test]
    fn qjs_host_mount_example_writes_host_visible_file() {
        let host = temp_dir("wanix-cli-mount-example");
        fs::write(host.join("input.txt"), "native mount").unwrap();

        let output = run([
            "qjs".into(),
            "--mount".into(),
            format!("{}=host", host.display()).into(),
            example_script("qjs-host-mount.js").into_os_string(),
        ])
        .unwrap();

        assert_eq!(output.exit_code(), 0);
        assert_eq!(
            output.stdout(),
            b"host input: native mount\nhost output: mounted output for native mount\n"
        );
        assert!(output.stderr().is_empty());
        assert_eq!(
            fs::read(host.join("output.txt")).unwrap(),
            b"mounted output for native mount"
        );
        fs::remove_dir_all(host).unwrap();
    }

    #[test]
    fn qjs_command_rejects_invalid_host_mounts() {
        let script = write_temp_script("mount-error.js", "print('unused');");

        let missing_value = run(["qjs", "--mount"]).unwrap_err();
        assert_eq!(missing_value.exit_code(), 2);
        assert!(
            missing_value
                .to_string()
                .contains("qjs --mount expects HOST=GUEST")
        );

        let root_guest = run([
            "qjs".into(),
            "--mount".into(),
            "/tmp=.".into(),
            script.clone().into_os_string(),
        ])
        .unwrap_err();
        assert_eq!(root_guest.exit_code(), 2);
        assert!(
            root_guest
                .to_string()
                .contains("qjs --mount guest path must not be .")
        );

        let missing_host = run([
            "qjs".into(),
            "--mount".into(),
            "/definitely/not/a/wanix/test/path=host".into(),
            script.into_os_string(),
        ])
        .unwrap_err();
        assert_eq!(missing_host.exit_code(), 1);
        assert!(missing_host.to_string().contains("failed to mount"));
    }

    #[test]
    fn qjs_command_uses_quickjs_std_and_os_for_process_context() {
        let script = write_temp_script(
            "std-process-demo.js",
            r#"
import * as std from "qjs:std";
import * as os from "qjs:os";

const readStdin = () => {
  const bytes = new Uint8Array(64);
  const n = os.read(0, bytes.buffer, 0, bytes.length);
  return Array.from(bytes.slice(0, n)).map((byte) => String.fromCharCode(byte)).join("");
};

const env = std.getenviron();
std.out.puts("argv " + scriptArgs.join("/") + "\n");
std.out.puts("mode " + std.getenv("MODE") + "\n");
std.out.puts("env " + env.MODE + " " + (env.EMPTY === "") + " " + String(env.MISSING) + "\n");
std.out.puts("stdin " + readStdin() + "\n");
std.out.puts("source " + std.loadFile("main.js").includes("std.getenv") + "\n");
std.writeFile("created.txt", "made via std cwd");
std.out.puts("created " + std.loadFile("created.txt") + "\n");
std.out.flush();
"#,
        );

        let output = run([
            "qjs".into(),
            "--env".into(),
            "MODE=test".into(),
            "--env".into(),
            "EMPTY=".into(),
            "--cwd".into(),
            "app".into(),
            "--stdin".into(),
            "hello from fd0".into(),
            script.into_os_string(),
            "--".into(),
            "alpha".into(),
            "two words".into(),
            "beta".into(),
        ])
        .unwrap();

        assert_eq!(output.exit_code(), 0);
        assert_eq!(
            output.stdout(),
            b"argv main.js/alpha/two words/beta\nmode test\nenv test true undefined\nstdin hello from fd0\nsource true\ncreated made via std cwd\n"
        );
        assert!(output.stderr().is_empty());
    }

    #[test]
    fn qjs_command_starts_child_qjs_task_through_task_service() {
        let script = write_temp_script(
            "spawn-parent.js",
            r##"
import * as std from "qjs:std";
import * as os from "qjs:os";

function stringFromBytes(bytes, count) {
  return Array.from(bytes.slice(0, count)).map((byte) => String.fromCharCode(byte)).join("");
}

function bytesFromString(text) {
  return new Uint8Array(Array.from(text).map((char) => char.charCodeAt(0)));
}

function readServiceText(path) {
  const fd = os.open(path, os.O_RDONLY);
  if (fd < 0) {
    throw new Error("open " + path + ": " + fd);
  }
  const bytes = new Uint8Array(64);
  const count = os.read(fd, bytes.buffer, 0, bytes.length);
  os.close(fd);
  if (count < 0) {
    throw new Error("read " + path + ": " + count);
  }
  return stringFromBytes(bytes, count);
}

function writeServiceText(path, text) {
  const fd = os.open(path, os.O_WRONLY);
  if (fd < 0) {
    throw new Error("open " + path + ": " + fd);
  }
  const bytes = bytesFromString(text);
  const count = os.write(fd, bytes.buffer, 0, bytes.length);
  os.close(fd);
  if (count !== bytes.length) {
    throw new Error("short write " + path + ": " + count + "/" + bytes.length);
  }
}

const parent = readServiceText("#task/self/id").trim();
const child = readServiceText("#task/new/qjs").trim();
std.writeFile("child stdin.txt", "stdin from parent\n");
writeServiceText("#task/" + child + "/cmd", "spawn-child.js alpha 'two words' '' beta\n");
writeServiceText("#task/" + child + "/env", "MODE=child\n");
writeServiceText("#task/" + child + "/dir", ".\n");
writeServiceText("#task/" + child + "/ctl", "bind 'child stdin.txt' fd/0\n");
writeServiceText("#task/" + child + "/ctl", "bind #task/" + parent + "/fd/1 fd/1\n");
writeServiceText("#task/" + child + "/ctl", "bind #task/" + parent + "/fd/2 fd/2\n");
print("parent " + parent);
print("child " + child);
writeServiceText("#task/" + child + "/ctl", "start\n");
print("child exit " + readServiceText("#task/" + child + "/exit").trim());
"##,
        );
        fs::write(
            script.parent().unwrap().join("spawn-child.js"),
            r##"
import * as std from "qjs:std";
import * as os from "qjs:os";

function readStdin() {
  const bytes = new Uint8Array(64);
  const count = os.read(0, bytes.buffer, 0, bytes.length);
  if (count < 0) {
    throw new Error("stdin read failed: " + count);
  }
  return Array.from(bytes.slice(0, count)).map((byte) => String.fromCharCode(byte)).join("");
}

std.out.puts(
  "id " + std.loadFile("#task/self/id").trim()
    + " args " + scriptArgs.join("|")
    + " mode " + std.getenv("MODE")
    + " stdin " + readStdin().trimEnd()
    + "\n"
);
std.out.flush();
std.err.puts("stderr mode " + std.getenv("MODE") + "\n");
std.err.flush();
std.exit(5);
"##,
        )
        .unwrap();

        let output = run(["qjs".into(), script.into_os_string()]).unwrap();

        assert_eq!(output.exit_code(), 0);
        assert_eq!(
            output.stdout(),
            b"parent 1\nchild 2\nid 2 args spawn-child.js|alpha|two words||beta mode child stdin stdin from parent\nchild exit 5\n"
        );
        assert_eq!(output.stderr(), b"stderr mode child\n");
    }

    #[test]
    fn qjs_example_task_spawn_runs_through_quickjs_os_service_files() {
        let output = run([
            "qjs".into(),
            example_script("qjs-task-spawn.js").into_os_string(),
        ])
        .unwrap();

        assert_eq!(output.exit_code(), 0);
        assert_eq!(
            output.stdout(),
            b"parent task: 1\nchild task: 2\nchild task 2 args qjs-task-spawn-child.js|alpha|two words||beta mode spawned stdin stdin from parent\nchild exit: 5\n"
        );
        assert_eq!(output.stderr(), b"child stderr mode spawned\n");
    }

    #[test]
    fn qjs_command_exposes_os_open_fds_through_task_service() {
        let script = write_temp_script(
            "mirrored-fd-parent.js",
            r##"
import * as std from "qjs:std";
import * as os from "qjs:os";

function stringFromBytes(bytes, count) {
  return Array.from(bytes.slice(0, count)).map((byte) => String.fromCharCode(byte)).join("");
}

function bytesFromString(text) {
  return new Uint8Array(Array.from(text).map((char) => char.charCodeAt(0)));
}

function readServiceText(path) {
  const fd = os.open(path, os.O_RDONLY);
  if (fd < 0) {
    throw new Error("open " + path + ": " + fd);
  }
  const bytes = new Uint8Array(128);
  const count = os.read(fd, bytes.buffer, 0, bytes.length);
  os.close(fd);
  if (count < 0) {
    throw new Error("read " + path + ": " + count);
  }
  return stringFromBytes(bytes, count);
}

function writeServiceText(path, text) {
  const fd = os.open(path, os.O_WRONLY);
  if (fd < 0) {
    throw new Error("open " + path + ": " + fd);
  }
  const bytes = bytesFromString(text);
  const count = os.write(fd, bytes.buffer, 0, bytes.length);
  os.close(fd);
  if (count !== bytes.length) {
    throw new Error("short write " + path + ": " + count + "/" + bytes.length);
  }
}

const parent = readServiceText("#task/self/id").trim();
std.writeFile("service-visible.txt", "service fd visible");
const serviceFd = os.open("service-visible.txt", os.O_RDONLY);
std.out.puts("service read " + std.loadFile("#task/self/fd/" + serviceFd) + "\n");
os.close(serviceFd);

std.writeFile("child-input.txt", "stdin via mirrored fd\n");
const childInputFd = os.open("child-input.txt", os.O_RDONLY);
const child = readServiceText("#task/new/qjs").trim();
writeServiceText("#task/" + child + "/cmd", "mirrored-fd-child.js\n");
writeServiceText("#task/" + child + "/ctl", "bind #task/" + parent + "/fd/" + childInputFd + " fd/0\n");
writeServiceText("#task/" + child + "/ctl", "bind #task/" + parent + "/fd/1 fd/1\n");
os.close(childInputFd);
writeServiceText("#task/" + child + "/ctl", "start\n");
std.out.puts("child exit " + readServiceText("#task/" + child + "/exit").trim() + "\n");
std.out.flush();
"##,
        );
        fs::write(
            script.parent().unwrap().join("mirrored-fd-child.js"),
            r##"
import * as std from "qjs:std";
import * as os from "qjs:os";

const bytes = new Uint8Array(64);
const count = os.read(0, bytes.buffer, 0, bytes.length);
if (count < 0) {
  throw new Error("stdin read failed: " + count);
}
const input = Array.from(bytes.slice(0, count)).map((byte) => String.fromCharCode(byte)).join("");
std.out.puts("child task " + std.loadFile("#task/self/id").trim() + " stdin " + input);
std.out.flush();
std.exit(7);
"##,
        )
        .unwrap();

        let output = run(["qjs".into(), script.into_os_string()]).unwrap();

        assert_eq!(output.exit_code(), 0);
        assert_eq!(
            output.stdout(),
            b"service read service fd visible\nchild task 2 stdin stdin via mirrored fd\nchild exit 7\n"
        );
        assert!(output.stderr().is_empty());
    }

    #[test]
    fn qjs_host_mount_example_starts_child_task_from_mounted_script() {
        let host = temp_dir("wanix-cli-host-spawn");

        let output = run([
            "qjs".into(),
            "--mount".into(),
            format!("{}=host", host.display()).into(),
            example_script("qjs-host-spawn.js").into_os_string(),
        ])
        .unwrap();

        let child_output =
            b"child task 2 args host/qjs-host-spawn-child.js|mounted|two words mode host-child";
        assert_eq!(output.exit_code(), 0);
        assert_eq!(
            output.stdout(),
            [
                b"parent task: 1\nchild task: 2\n".as_slice(),
                child_output,
                b"\nchild exit: 4\nhost child output: ".as_slice(),
                child_output,
                b"\n".as_slice()
            ]
            .concat()
        );
        assert!(output.stderr().is_empty());
        assert_eq!(
            fs::read(host.join("parent-output.txt")).unwrap(),
            b"parent task 1"
        );
        assert_eq!(
            fs::read(host.join("child-output.txt")).unwrap(),
            child_output
        );
        assert!(host.join("qjs-host-spawn-child.js").exists());
        fs::remove_dir_all(host).unwrap();
    }

    #[test]
    fn qjs_restore_command_restores_vm_image_into_child_task() {
        let before_script = write_temp_script(
            "restore-before.js",
            r##"
import * as std from "qjs:std";

const task = std.loadFile("#task/self/id").trim();
globalThis.snapshotValue = "saved by task " + task;
std.writeFile("note.txt", "namespace note from " + task);
std.out.puts("before " + task + "\n");
std.out.flush();
"##,
        );
        let after_script = write_temp_script(
            "restore-after.js",
            r##"
import * as std from "qjs:std";

const task = std.loadFile("#task/self/id").trim();
std.out.puts("after " + task + "\n");
std.out.puts("vm " + globalThis.snapshotValue + "\n");
std.out.puts("file " + std.loadFile("note.txt") + "\n");
std.out.flush();
std.exit(7);
"##,
        );

        let output = run([
            "qjs-restore".into(),
            before_script.into_os_string(),
            after_script.into_os_string(),
        ])
        .unwrap();

        assert_eq!(output.exit_code(), 7);
        assert_eq!(
            output.stdout(),
            b"before 1\nafter 2\nvm saved by task 1\nfile namespace note from 1\n"
        );
        assert!(output.stderr().is_empty());
    }

    #[test]
    fn qjs_restore_keeps_before_and_after_script_dependencies_isolated() {
        let before_script = write_temp_script(
            "restore-before.js",
            r#"
import * as std from "qjs:std";
import { label } from "./dep.js";

globalThis.beforeDep = label;
std.out.puts("before " + label + "\n");
std.out.flush();
"#,
        );
        fs::write(
            before_script.parent().unwrap().join("dep.js"),
            "export const label = 'before dep';",
        )
        .unwrap();

        let after_script = write_temp_script(
            "restore-after.js",
            r#"
import * as std from "qjs:std";
import { label } from "./dep.js";

std.out.puts("after " + label + "\n");
std.out.puts("snapshot " + globalThis.beforeDep + "\n");
std.out.flush();
"#,
        );
        fs::write(
            after_script.parent().unwrap().join("dep.js"),
            "export const label = 'after dep';",
        )
        .unwrap();

        let output = run([
            "qjs-restore".into(),
            before_script.into_os_string(),
            after_script.into_os_string(),
        ])
        .unwrap();

        assert_eq!(output.exit_code(), 0);
        assert_eq!(
            output.stdout(),
            b"before before dep\nafter after dep\nsnapshot before dep\n"
        );
        assert!(output.stderr().is_empty());
    }

    #[test]
    fn qjs_restore_rejects_open_wasi_fds_mirrored_into_task_table_before_snapshot() {
        let before_script = write_temp_script(
            "restore-before-fd.js",
            r##"
import * as os from "qjs:os";
import * as std from "qjs:std";

const fd = os.open("__wanix_restore/before/main.js", os.O_RDONLY);
std.out.puts("opened wasi fd " + fd + "\n");
std.out.flush();
"##,
        );
        let after_script = write_temp_script("restore-after-fd.js", "print('after');");

        let output = run([
            "qjs-restore".into(),
            before_script.into_os_string(),
            after_script.into_os_string(),
        ])
        .unwrap();

        assert_eq!(output.exit_code(), 1);
        assert_eq!(output.stdout(), b"opened wasi fd 4\n");
        assert!(
            String::from_utf8_lossy(output.stderr())
                .contains("cannot snapshot qjs task with open Wanix task fds: 4")
        );
    }

    #[test]
    fn qjs_restore_example_runs_checked_in_snapshot_demo() {
        let output = run([
            "qjs-restore".into(),
            example_script("qjs-snapshot-before.js").into_os_string(),
            example_script("qjs-snapshot-after.js").into_os_string(),
        ])
        .unwrap();

        assert_eq!(output.exit_code(), 7);
        assert_eq!(
            output.stdout(),
            b"before task: 1\nafter task: 2\nvm state: preserved from task 1\nnamespace: namespace from task 1\n"
        );
        assert!(output.stderr().is_empty());
    }

    #[test]
    fn qjs_restore_example_starts_child_task_after_restore() {
        let output = run([
            "qjs-restore".into(),
            example_script("qjs-restore-spawn-before.js").into_os_string(),
            example_script("qjs-restore-spawn-after.js").into_os_string(),
        ])
        .unwrap();

        assert_eq!(output.exit_code(), 9);
        assert_eq!(
            output.stdout(),
            b"before task: 1\nafter task: 2\nvm state: preserved from task 1\nnamespace: namespace from task 1\nchild task: 3\nrestore child task: 3\nrestore child argv: __wanix_restore/after/qjs-restore-spawn-child.js|from-restore|two words\nrestore child env: restored-child\nrestore child note: child namespace from task 2 / preserved from task 1\nrestore child snapshot global: undefined\nchild exit: 5\n"
        );
        assert_eq!(output.stderr(), b"restore child stderr: restored-child\n");
    }

    #[test]
    fn qjs_restore_example_reattaches_after_process_context() {
        let output = run([
            "qjs-restore".into(),
            "--before-env".into(),
            "MODE=before".into(),
            "--after-env".into(),
            "MODE=after".into(),
            "--before-arg".into(),
            "prep".into(),
            "--before-arg".into(),
            "two words".into(),
            "--after-arg".into(),
            "resume".into(),
            "--after-arg".into(),
            "done value".into(),
            example_script("qjs-restore-context-before.js").into_os_string(),
            example_script("qjs-restore-context-after.js").into_os_string(),
        ])
        .unwrap();

        assert_eq!(output.exit_code(), 6);
        assert_eq!(
            output.stdout(),
            b"before task: 1\nbefore cmd: __wanix_restore/before/main.js prep 'two words'\nbefore argv: __wanix_restore/before/main.js|prep|two words\nbefore wasi env: before\nbefore task env: before\nafter task: 2\nafter cmd: __wanix_restore/after/main.js resume 'done value'\nafter argv: __wanix_restore/after/main.js|resume|done value\nafter wasi env: before\nafter task env: after\nsnapshot argv: __wanix_restore/before/main.js|prep|two words\nsnapshot env: before\n"
        );
        assert!(output.stderr().is_empty());
    }

    #[test]
    fn qjs_restore_mounts_host_directory_into_reattached_task_namespace() {
        let host = temp_dir("wanix-cli-restore-mount");
        let before_script = write_temp_script(
            "restore-before-host.js",
            r##"
import * as std from "qjs:std";

const task = std.loadFile("#task/self/id").trim();
globalThis.snapshotValue = "vm from " + task;
std.writeFile("host/before.txt", "before host task " + task);
std.out.puts("before " + task + "\n");
std.out.flush();
"##,
        );
        let after_script = write_temp_script(
            "restore-after-host.js",
            r##"
import * as std from "qjs:std";

const task = std.loadFile("#task/self/id").trim();
std.writeFile("host/after.txt", "after host task " + task + " with " + globalThis.snapshotValue);
std.out.puts("after " + task + "\n");
std.out.puts(std.loadFile("host/before.txt") + "\n");
std.out.puts(std.loadFile("host/after.txt") + "\n");
std.out.flush();
std.exit(6);
"##,
        );

        let output = run([
            "qjs-restore".into(),
            "--mount".into(),
            format!("{}=host", host.display()).into(),
            before_script.into_os_string(),
            after_script.into_os_string(),
        ])
        .unwrap();

        assert_eq!(output.exit_code(), 6);
        assert_eq!(
            output.stdout(),
            b"before 1\nafter 2\nbefore host task 1\nafter host task 2 with vm from 1\n"
        );
        assert!(output.stderr().is_empty());
        assert_eq!(
            fs::read(host.join("before.txt")).unwrap(),
            b"before host task 1"
        );
        assert_eq!(
            fs::read(host.join("after.txt")).unwrap(),
            b"after host task 2 with vm from 1"
        );
        fs::remove_dir_all(host).unwrap();
    }

    #[test]
    fn qjs_restore_host_mount_example_writes_host_visible_file() {
        let host = temp_dir("wanix-cli-restore-mount-example");

        let output = run([
            "qjs-restore".into(),
            "--mount".into(),
            format!("{}=host", host.display()).into(),
            example_script("qjs-snapshot-before.js").into_os_string(),
            example_script("qjs-snapshot-host-after.js").into_os_string(),
        ])
        .unwrap();

        assert_eq!(output.exit_code(), 8);
        assert_eq!(
            output.stdout(),
            b"before task: 1\nafter task: 2\nhost output: restored task 2 saw preserved from task 1\n"
        );
        assert!(output.stderr().is_empty());
        assert_eq!(
            fs::read(host.join("restored-output.txt")).unwrap(),
            b"restored task 2 saw preserved from task 1"
        );
        fs::remove_dir_all(host).unwrap();
    }

    #[test]
    fn qjs_snapshot_and_resume_persist_vm_image_between_cli_invocations() {
        let host = temp_dir("wanix-cli-persist-snapshot");
        let snapshot = host.join("quickjs.snapshot");

        let before = run([
            "qjs-snapshot".into(),
            "--env".into(),
            "MODE=before".into(),
            "--mount".into(),
            format!("{}=host", host.display()).into(),
            "--snapshot".into(),
            snapshot.clone().into_os_string(),
            example_script("qjs-persist-before.js").into_os_string(),
        ])
        .unwrap();

        assert_eq!(before.exit_code(), 0);
        assert_eq!(before.stdout(), b"snapshot task: 1\n");
        assert!(before.stderr().is_empty());
        assert!(fs::read(&snapshot).unwrap().len() > 1024);
        assert_eq!(
            fs::read(host.join("persist-before.txt")).unwrap(),
            b"host before task 1"
        );

        let after = run([
            "qjs-resume".into(),
            "--env".into(),
            "MODE=after".into(),
            "--mount".into(),
            format!("{}=host", host.display()).into(),
            "--snapshot".into(),
            snapshot.into_os_string(),
            example_script("qjs-persist-after.js").into_os_string(),
        ])
        .unwrap();

        assert_eq!(after.exit_code(), 6);
        assert_eq!(
            after.stdout(),
            b"resume task: 1\nvm: vm from task 1 mode before\nreattached mode: after\nhost: host before task 1\n"
        );
        assert!(after.stderr().is_empty());
        assert_eq!(
            fs::read(host.join("persist-after.txt")).unwrap(),
            b"host after task 1"
        );
        fs::remove_dir_all(host).unwrap();
    }

    #[test]
    fn qjs_snapshot_memory_limit_stops_allocation_before_snapshot_file() {
        let dir = temp_dir("wanix-cli-snapshot-memory-limit");
        let snapshot = dir.join("quickjs.snapshot");

        let output = run([
            "qjs-snapshot".into(),
            "--memory-limit-bytes".into(),
            (1024 * 1024).to_string().into(),
            "--snapshot".into(),
            snapshot.clone().into_os_string(),
            example_script("qjs-memory-limit-demo.js").into_os_string(),
        ])
        .unwrap();

        assert_eq!(output.exit_code(), 1);
        assert_eq!(output.stdout(), b"before allocation\n");
        let stderr = std::str::from_utf8(output.stderr()).unwrap();
        assert!(stderr.contains("wanix-rust qjs-snapshot:"), "{stderr}");
        assert!(stderr.contains("QuickJS exception"), "{stderr}");
        assert!(
            !snapshot.exists(),
            "snapshot should not be written after memory-limit failure"
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn qjs_resume_memory_limit_is_reattached_after_restore() {
        let dir = temp_dir("wanix-cli-resume-memory-limit");
        let snapshot = dir.join("quickjs.snapshot");
        let before_script = write_temp_script(
            "qjs-memory-limit-before.js",
            r#"
globalThis.snapshotReady = true;
print("snapshotted");
"#,
        );

        let before = run([
            "qjs-snapshot".into(),
            "--snapshot".into(),
            snapshot.clone().into_os_string(),
            before_script.into_os_string(),
        ])
        .unwrap();

        assert_eq!(before.exit_code(), 0);
        assert_eq!(before.stdout(), b"snapshotted\n");
        assert!(before.stderr().is_empty());
        assert!(fs::read(&snapshot).unwrap().len() > 1024);

        let after = run([
            "qjs-resume".into(),
            "--memory-limit-bytes".into(),
            (1024 * 1024).to_string().into(),
            "--snapshot".into(),
            snapshot.into_os_string(),
            example_script("qjs-memory-limit-demo.js").into_os_string(),
        ])
        .unwrap();

        assert_eq!(after.exit_code(), 1);
        assert_eq!(after.stdout(), b"before allocation\n");
        let stderr = std::str::from_utf8(after.stderr()).unwrap();
        assert!(stderr.contains("wanix-rust qjs-resume:"), "{stderr}");
        assert!(stderr.contains("QuickJS exception"), "{stderr}");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn qjs_snapshot_interrupt_budget_stops_cpu_loop_before_snapshot_file() {
        let dir = temp_dir("wanix-cli-snapshot-interrupt");
        let snapshot = dir.join("quickjs.snapshot");

        let output = run([
            "qjs-snapshot".into(),
            "--interrupt-after".into(),
            "1".into(),
            "--snapshot".into(),
            snapshot.clone().into_os_string(),
            example_script("qjs-interrupt-demo.js").into_os_string(),
        ])
        .unwrap();

        assert_eq!(output.exit_code(), 1);
        assert_eq!(output.stdout(), b"starting cpu loop\n");
        let stderr = std::str::from_utf8(output.stderr()).unwrap();
        assert!(stderr.contains("wanix-rust qjs-snapshot:"), "{stderr}");
        assert!(stderr.contains("interrupted"), "{stderr}");
        assert!(
            !snapshot.exists(),
            "snapshot should not be written after interrupt-budget failure"
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn qjs_resume_interrupt_budget_is_reattached_after_restore() {
        let dir = temp_dir("wanix-cli-resume-interrupt");
        let snapshot = dir.join("quickjs.snapshot");
        let before_script = write_temp_script(
            "qjs-interrupt-before.js",
            r#"
globalThis.snapshotReady = true;
print("snapshotted");
"#,
        );

        let before = run([
            "qjs-snapshot".into(),
            "--snapshot".into(),
            snapshot.clone().into_os_string(),
            before_script.into_os_string(),
        ])
        .unwrap();

        assert_eq!(before.exit_code(), 0);
        assert_eq!(before.stdout(), b"snapshotted\n");
        assert!(before.stderr().is_empty());
        assert!(fs::read(&snapshot).unwrap().len() > 1024);

        let after = run([
            "qjs-resume".into(),
            "--interrupt-after".into(),
            "1".into(),
            "--snapshot".into(),
            snapshot.into_os_string(),
            example_script("qjs-interrupt-demo.js").into_os_string(),
        ])
        .unwrap();

        assert_eq!(after.exit_code(), 1);
        assert_eq!(after.stdout(), b"starting cpu loop\n");
        let stderr = std::str::from_utf8(after.stderr()).unwrap();
        assert!(stderr.contains("wanix-rust qjs-resume:"), "{stderr}");
        assert!(stderr.contains("interrupted"), "{stderr}");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn qjs_snapshot_future_timer_runs_with_wait_budget_before_snapshot_file() {
        let dir = temp_dir("wanix-cli-snapshot-future-timer");
        let snapshot = dir.join("quickjs.snapshot");

        let output = run([
            "qjs-snapshot".into(),
            "--event-loop-ms".into(),
            "10".into(),
            "--snapshot".into(),
            snapshot.clone().into_os_string(),
            example_script("qjs-future-timer-demo.js").into_os_string(),
        ])
        .unwrap();

        assert_eq!(output.exit_code(), 0);
        assert_eq!(output.stdout(), b"sync\ntimeout\n");
        assert!(output.stderr().is_empty());
        assert!(fs::read(&snapshot).unwrap().len() > 1024);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn qjs_resume_future_timer_wait_budget_is_reattached_after_restore() {
        let dir = temp_dir("wanix-cli-resume-future-timer");
        let snapshot = dir.join("quickjs.snapshot");
        let before_script = write_temp_script(
            "qjs-future-timer-before.js",
            r#"
globalThis.snapshotReady = true;
print("snapshotted");
"#,
        );

        let before = run([
            "qjs-snapshot".into(),
            "--snapshot".into(),
            snapshot.clone().into_os_string(),
            before_script.into_os_string(),
        ])
        .unwrap();

        assert_eq!(before.exit_code(), 0);
        assert_eq!(before.stdout(), b"snapshotted\n");
        assert!(before.stderr().is_empty());
        assert!(fs::read(&snapshot).unwrap().len() > 1024);

        let after = run([
            "qjs-resume".into(),
            "--event-loop-ms".into(),
            "10".into(),
            "--snapshot".into(),
            snapshot.into_os_string(),
            example_script("qjs-future-timer-demo.js").into_os_string(),
        ])
        .unwrap();

        assert_eq!(after.exit_code(), 0);
        assert_eq!(after.stdout(), b"sync\ntimeout\n");
        assert!(after.stderr().is_empty());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn qjs_resume_ready_io_turns_are_reattached_after_restore() {
        let dir = temp_dir("wanix-cli-resume-ready-io");
        let snapshot = dir.join("quickjs.snapshot");
        let before_script = write_temp_script(
            "qjs-ready-io-before.js",
            r#"
globalThis.snapshotReady = true;
print("snapshotted");
"#,
        );

        let before = run([
            "qjs-snapshot".into(),
            "--snapshot".into(),
            snapshot.clone().into_os_string(),
            before_script.into_os_string(),
        ])
        .unwrap();

        assert_eq!(before.exit_code(), 0);
        assert_eq!(before.stdout(), b"snapshotted\n");
        assert!(before.stderr().is_empty());
        assert!(fs::read(&snapshot).unwrap().len() > 1024);

        let after = run([
            "qjs-resume".into(),
            "--stdin".into(),
            "abcdef".into(),
            "--ready-io-turns".into(),
            "2".into(),
            "--snapshot".into(),
            snapshot.into_os_string(),
            example_script("qjs-ready-io-turns-demo.js").into_os_string(),
        ])
        .unwrap();

        assert_eq!(after.exit_code(), 0);
        assert_eq!(after.stdout(), b"sync\nchunk 1: abc\nchunk 2: def\n");
        assert!(after.stderr().is_empty());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn qjs_snapshot_rejects_open_directory_wasi_fd() {
        let dir = temp_dir("wanix-cli-open-directory-snapshot");
        let snapshot = dir.join("quickjs.snapshot");
        let script = write_temp_script(
            "snapshot-open-directory.js",
            r#"
import * as os from "qjs:os";
import * as std from "qjs:std";

const fd = os.open(".", os.O_RDONLY);
std.out.puts("opened directory fd " + fd + "\n");
std.out.flush();
"#,
        );

        let output = run([
            "qjs-snapshot".into(),
            "--snapshot".into(),
            snapshot.into_os_string(),
            script.into_os_string(),
        ])
        .unwrap();

        assert_eq!(output.exit_code(), 1);
        assert_eq!(output.stdout(), b"opened directory fd 4\n");
        assert!(String::from_utf8_lossy(output.stderr()).contains("open dynamic WASI fd(s)"));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn qjs_snapshot_and_resume_reattach_stdin_between_cli_invocations() {
        let snapshot_dir = temp_dir("wanix-cli-persist-stdin");
        let snapshot = snapshot_dir.join("quickjs.snapshot");
        let before_script = write_temp_script(
            "qjs-persist-stdin-before.js",
            r#"
import * as std from "qjs:std";
import * as os from "qjs:os";

function readStdin() {
  const bytes = new Uint8Array(64);
  const n = os.read(0, bytes.buffer, 0, bytes.length);
  return Array.from(bytes.slice(0, n)).map((byte) => String.fromCharCode(byte)).join("");
}

globalThis.beforeStdin = readStdin().trimEnd();
std.out.puts("before stdin: " + globalThis.beforeStdin + "\n");
std.out.flush();
"#,
        );
        let after_script = write_temp_script(
            "qjs-persist-stdin-after.js",
            r#"
import * as std from "qjs:std";
import * as os from "qjs:os";

function readStdin() {
  const bytes = new Uint8Array(64);
  const n = os.read(0, bytes.buffer, 0, bytes.length);
  return Array.from(bytes.slice(0, n)).map((byte) => String.fromCharCode(byte)).join("");
}

std.out.puts("snapshot stdin: " + globalThis.beforeStdin + "\n");
std.out.puts("resume stdin: " + readStdin().trimEnd() + "\n");
std.out.flush();
"#,
        );

        let before = run([
            "qjs-snapshot".into(),
            "--stdin".into(),
            "before fd0\n".into(),
            "--snapshot".into(),
            snapshot.clone().into_os_string(),
            before_script.into_os_string(),
        ])
        .unwrap();

        assert_eq!(before.exit_code(), 0);
        assert_eq!(before.stdout(), b"before stdin: before fd0\n");
        assert!(before.stderr().is_empty());

        let after = run_with_process_stdin(
            [
                "qjs-resume".into(),
                "--stdin-file".into(),
                "-".into(),
                "--snapshot".into(),
                snapshot.into_os_string(),
                after_script.into_os_string(),
            ],
            b"after fd0\n".as_slice(),
        )
        .unwrap();

        assert_eq!(after.exit_code(), 0);
        assert_eq!(
            after.stdout(),
            b"snapshot stdin: before fd0\nresume stdin: after fd0\n"
        );
        assert!(after.stderr().is_empty());
        fs::remove_dir_all(snapshot_dir).unwrap();
    }

    #[test]
    fn qjs_snapshot_and_resume_reject_invalid_stdin_options() {
        let missing_snapshot_stdin = run(["qjs-snapshot", "--stdin"]).unwrap_err();
        assert_eq!(missing_snapshot_stdin.exit_code(), 2);
        assert!(
            missing_snapshot_stdin
                .to_string()
                .contains("qjs-snapshot --stdin expects text")
        );

        let missing_resume_stdin_file = run(["qjs-resume", "--stdin-file"]).unwrap_err();
        assert_eq!(missing_resume_stdin_file.exit_code(), 2);
        assert!(
            missing_resume_stdin_file
                .to_string()
                .contains("qjs-resume --stdin-file expects PATH or -")
        );

        let duplicate = run(["qjs-resume", "--stdin", "text", "--stdin-file", "-"]).unwrap_err();
        assert_eq!(duplicate.exit_code(), 2);
        assert!(
            duplicate
                .to_string()
                .contains("qjs-resume accepts only one of --stdin or --stdin-file")
        );
    }

    #[test]
    fn qjs_resume_reports_missing_snapshot_before_reading_process_stdin() {
        struct PanicOnRead;

        impl std::io::Read for PanicOnRead {
            fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
                panic!("resume should report the missing snapshot before reading stdin")
            }
        }

        let dir = temp_dir("wanix-cli-missing-resume-snapshot");
        let script = write_temp_script("resume-missing-snapshot.js", "print('unused');");

        let error = run_with_process_stdin(
            [
                "qjs-resume".into(),
                "--stdin-file".into(),
                "-".into(),
                "--snapshot".into(),
                dir.join("missing.snapshot").into_os_string(),
                script.into_os_string(),
            ],
            PanicOnRead,
        )
        .unwrap_err();

        assert_eq!(error.exit_code(), 1);
        assert!(error.to_string().contains("failed to read snapshot"));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn qjs_restore_rejects_invalid_host_mounts() {
        let before_script = write_temp_script("restore-mount-error-before.js", "print('unused');");
        let after_script = write_temp_script("restore-mount-error-after.js", "print('unused');");

        let missing_value = run(["qjs-restore", "--mount"]).unwrap_err();
        assert_eq!(missing_value.exit_code(), 2);
        assert!(
            missing_value
                .to_string()
                .contains("qjs-restore --mount expects HOST=GUEST")
        );

        let root_guest = run([
            "qjs-restore".into(),
            "--mount".into(),
            "/tmp=.".into(),
            before_script.clone().into_os_string(),
            after_script.clone().into_os_string(),
        ])
        .unwrap_err();
        assert_eq!(root_guest.exit_code(), 2);
        assert!(
            root_guest
                .to_string()
                .contains("qjs-restore --mount guest path must not be .")
        );

        let missing_host = run([
            "qjs-restore".into(),
            "--mount".into(),
            "/definitely/not/a/wanix/test/path=host".into(),
            before_script.into_os_string(),
            after_script.into_os_string(),
        ])
        .unwrap_err();
        assert_eq!(missing_host.exit_code(), 1);
        assert!(missing_host.to_string().contains("failed to mount"));
    }

    #[test]
    fn qjs_command_reports_failure_and_preserves_stdout() {
        let script = write_temp_script(
            "boom.js",
            r#"print("before failure"); throw new Error("boom");"#,
        );

        let output = run(["qjs".into(), script.into_os_string()]).unwrap();

        assert_eq!(output.exit_code(), 1);
        assert_eq!(output.stdout(), b"before failure\n");
        assert!(String::from_utf8_lossy(output.stderr()).contains("QuickJS error"));
    }

    #[test]
    fn qjs_command_uses_exit_status_requested_by_quickjs_std() {
        let script = write_temp_script(
            "std-exit.js",
            r#"
import * as std from "qjs:std";

std.out.puts("before std exit\n");
std.out.flush();
std.err.puts("stderr before std exit\n");
std.err.flush();
std.exit(9);
std.out.puts("after std exit\n");
std.err.puts("stderr after std exit\n");
"#,
        );

        let output = run(["qjs".into(), script.into_os_string()]).unwrap();

        assert_eq!(output.exit_code(), 9);
        assert_eq!(output.stdout(), b"before std exit\n");
        assert_eq!(output.stderr(), b"stderr before std exit\n");
    }

    #[test]
    fn qjs_command_runs_fd_demo_through_quickjs_os_fds() {
        let script = write_temp_script(
            "fd-demo.js",
            r#"
import * as std from "qjs:std";
import * as os from "qjs:os";

function stringFromBytes(bytes, count) {
  return Array.from(bytes.slice(0, count)).map((byte) => String.fromCharCode(byte)).join("");
}

function bytesFromString(text) {
  return new Uint8Array(Array.from(text).map((char) => char.charCodeAt(0)));
}

const input = os.open("main.js", os.O_RDONLY);
const inputBytes = new Uint8Array(1024);
const inputCount = os.read(input, inputBytes.buffer, 0, inputBytes.length);
os.close(input);
std.out.puts("read fd " + input + "\n");
std.out.puts("saw api " + stringFromBytes(inputBytes, inputCount).includes("os.open") + "\n");

const output = os.open("fd-output.txt", os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o666);
const outputBytes = bytesFromString("via cli fd");
const outputCount = os.write(output, outputBytes.buffer, 0, outputBytes.length);
os.close(output);
std.out.puts("write fd " + output + "\n");
std.out.puts("bytes " + outputCount + "\n");
std.out.puts(std.loadFile("fd-output.txt") + "\n");
std.out.flush();
"#,
        );

        let output = run(["qjs".into(), script.into_os_string()]).unwrap();

        assert_eq!(output.exit_code(), 0);
        assert_eq!(
            output.stdout(),
            b"read fd 4\nsaw api true\nwrite fd 5\nbytes 10\nvia cli fd\n"
        );
        assert!(output.stderr().is_empty());
    }

    #[test]
    fn qjs_example_fd_demo_runs_through_quickjs_os_fds() {
        let output = run([
            "qjs".into(),
            example_script("qjs-fd-demo.js").into_os_string(),
        ])
        .unwrap();

        assert_eq!(output.exit_code(), 0);
        assert_eq!(
            output.stdout(),
            b"read fd: 4\nsaw fd API: true\nwrite fd: 5\nbytes: 21\nhello from a Wanix fd\n"
        );
        assert!(output.stderr().is_empty());
    }

    #[test]
    fn qjs_command_appends_through_quickjs_os_and_std() {
        let script = write_temp_script(
            "append-demo.js",
            r#"
import * as std from "qjs:std";
import * as os from "qjs:os";

std.writeFile("log.txt", "start");
const fd = os.open("log.txt", os.O_WRONLY | os.O_APPEND);
os.seek(fd, 0, std.SEEK_SET);
const bytes = new Uint8Array([45, 111, 115]);
std.out.puts("os " + os.write(fd, bytes.buffer, 0, bytes.length) + "\n");
os.close(fd);

const file = std.open("log.txt", "a");
file.puts("-std");
file.close();

const fd2 = os.open("log.txt", os.O_WRONLY);
const file2 = std.fdopen(fd2, "a");
file2.puts("-fdopen");
file2.close();
std.out.puts("log " + std.loadFile("log.txt") + "\n");
std.out.flush();
"#,
        );

        let output = run(["qjs".into(), script.into_os_string()]).unwrap();

        assert_eq!(output.exit_code(), 0);
        assert_eq!(output.stdout(), b"os 3\nlog start-os-std-fdopen\n");
        assert!(output.stderr().is_empty());
    }

    #[test]
    fn qjs_example_append_demo_appends_through_quickjs_os_and_std() {
        let output = run([
            "qjs".into(),
            example_script("qjs-append-demo.js").into_os_string(),
        ])
        .unwrap();

        assert_eq!(output.exit_code(), 0);
        assert_eq!(output.stdout(), b"os bytes: 3\nlog: start-os-std-fdopen\n");
        assert!(output.stderr().is_empty());
    }

    #[test]
    fn qjs_example_sleep_demo_runs_timer_poll_oneoff() {
        let output = run([
            "qjs".into(),
            example_script("qjs-sleep-demo.js").into_os_string(),
        ])
        .unwrap();

        assert_eq!(output.exit_code(), 0);
        assert_eq!(output.stdout(), b"before sleep\nafter sleep\n");
        assert!(output.stderr().is_empty());
    }

    #[test]
    fn qjs_example_async_timer_demo_runs_immediate_event_loop() {
        let output = run([
            "qjs".into(),
            example_script("qjs-async-timer-demo.js").into_os_string(),
        ])
        .unwrap();

        assert_eq!(output.exit_code(), 0);
        let stdout = std::str::from_utf8(output.stdout()).unwrap();
        assert!(stdout.starts_with("sync\n"), "{stdout}");
        assert!(stdout.contains("sleepAsync\n"), "{stdout}");
        assert!(output.stderr().is_empty());
    }

    #[test]
    fn qjs_example_future_timer_demo_runs_with_wait_budget() {
        let output = run([
            "qjs".into(),
            "--event-loop-ms".into(),
            "10".into(),
            example_script("qjs-future-timer-demo.js").into_os_string(),
        ])
        .unwrap();

        assert_eq!(output.exit_code(), 0);
        assert_eq!(output.stdout(), b"sync\ntimeout\n");
        assert!(output.stderr().is_empty());
    }

    #[test]
    fn qjs_example_interval_demo_runs_with_wait_budget() {
        let output = run([
            "qjs".into(),
            "--event-loop-ms".into(),
            "10".into(),
            example_script("qjs-interval-demo.js").into_os_string(),
        ])
        .unwrap();

        assert_eq!(output.exit_code(), 0);
        assert_eq!(output.stdout(), b"sync\ntick 1\ntick 2\ntick 3\n");
        assert!(output.stderr().is_empty());
    }

    #[test]
    fn qjs_command_interrupt_budget_stops_cpu_bound_script() {
        let output = run([
            "qjs".into(),
            "--interrupt-after".into(),
            "1".into(),
            example_script("qjs-interrupt-demo.js").into_os_string(),
        ])
        .unwrap();

        assert_eq!(output.exit_code(), 1);
        assert_eq!(output.stdout(), b"starting cpu loop\n");
        let stderr = std::str::from_utf8(output.stderr()).unwrap();
        assert!(stderr.contains("wanix-rust qjs:"), "{stderr}");
        assert!(stderr.contains("interrupted"), "{stderr}");
    }

    #[test]
    fn qjs_command_memory_limit_stops_allocation_heavy_script() {
        let output = run([
            "qjs".into(),
            "--memory-limit-bytes".into(),
            (1024 * 1024).to_string().into(),
            example_script("qjs-memory-limit-demo.js").into_os_string(),
        ])
        .unwrap();

        assert_eq!(output.exit_code(), 1);
        assert_eq!(output.stdout(), b"before allocation\n");
        let stderr = std::str::from_utf8(output.stderr()).unwrap();
        assert!(stderr.contains("wanix-rust qjs:"), "{stderr}");
        assert!(stderr.contains("QuickJS exception"), "{stderr}");
    }

    #[test]
    fn qjs_example_fd_handler_demo_reads_ready_stdin() {
        let output = run([
            "qjs".into(),
            "--stdin".into(),
            "ready stdin".into(),
            example_script("qjs-fd-handler-demo.js").into_os_string(),
        ])
        .unwrap();

        assert_eq!(output.exit_code(), 0);
        assert_eq!(output.stdout(), b"sync\nhandler: ready stdin\n");
        assert!(output.stderr().is_empty());
    }

    #[test]
    fn qjs_example_ready_io_turns_demo_reads_stdin_twice() {
        let output = run([
            "qjs".into(),
            "--stdin".into(),
            "abcdef".into(),
            "--ready-io-turns".into(),
            "2".into(),
            example_script("qjs-ready-io-turns-demo.js").into_os_string(),
        ])
        .unwrap();

        assert_eq!(output.exit_code(), 0);
        assert_eq!(output.stdout(), b"sync\nchunk 1: abc\nchunk 2: def\n");
        assert!(output.stderr().is_empty());
    }

    #[test]
    fn qjs_command_updates_times_through_quickjs_os_utimes() {
        let script = write_temp_script(
            "utimes-demo.js",
            r#"
import * as std from "qjs:std";
import * as os from "qjs:os";

std.writeFile("stamp.txt", "timestamped");
std.out.puts("utimes " + os.utimes("stamp.txt", new Date(1000), new Date(2000)) + "\n");
const stat = os.stat("stamp.txt")[0];
std.out.puts("atime " + stat.atime + "\n");
std.out.puts("mtime " + stat.mtime + "\n");
std.out.flush();
"#,
        );

        let output = run(["qjs".into(), script.into_os_string()]).unwrap();

        assert_eq!(output.exit_code(), 0);
        assert_eq!(output.stdout(), b"utimes 0\natime 1000\nmtime 2000\n");
        assert!(output.stderr().is_empty());
    }

    #[test]
    fn qjs_example_utimes_demo_updates_times_through_quickjs_os() {
        let output = run([
            "qjs".into(),
            example_script("qjs-utimes-demo.js").into_os_string(),
        ])
        .unwrap();

        assert_eq!(output.exit_code(), 0);
        assert_eq!(
            output.stdout(),
            b"utimes: 0\natime: 1000\nmtime: 2000\nmessage: timestamped\n"
        );
        assert!(output.stderr().is_empty());
    }

    #[test]
    fn qjs_example_unlink_demo_removes_files_through_quickjs_os() {
        let output = run([
            "qjs".into(),
            example_script("qjs-unlink-demo.js").into_os_string(),
        ])
        .unwrap();

        assert_eq!(output.exit_code(), 0);
        assert_eq!(output.stdout(), b"deleted: true\nkept: keep me\n");
        assert!(output.stderr().is_empty());
    }

    #[test]
    fn qjs_command_renames_paths_through_quickjs_os() {
        let script = write_temp_script(
            "rename-demo.js",
            r#"
import * as std from "qjs:std";
import * as os from "qjs:os";

std.writeFile("old.txt", "from rename");
std.out.puts("rename " + os.rename("old.txt", "renamed.txt") + "\n");
const probe = os.open("old.txt", os.O_RDONLY);
if (probe >= 0) {
  os.close(probe);
}
std.out.puts("old " + (probe < 0) + "\n");
std.out.puts("new " + std.loadFile("renamed.txt") + "\n");
std.out.flush();
"#,
        );

        let output = run(["qjs".into(), script.into_os_string()]).unwrap();

        assert_eq!(output.exit_code(), 0);
        assert_eq!(output.stdout(), b"rename 0\nold true\nnew from rename\n");
        assert!(output.stderr().is_empty());
    }

    #[test]
    fn qjs_example_rename_demo_renames_paths_through_quickjs_os() {
        let output = run([
            "qjs".into(),
            example_script("qjs-rename-demo.js").into_os_string(),
        ])
        .unwrap();

        assert_eq!(output.exit_code(), 0);
        assert_eq!(
            output.stdout(),
            b"rename: 0\nold missing: true\nmessage: hello from rename\n"
        );
        assert!(output.stderr().is_empty());
    }

    #[test]
    fn qjs_command_creates_directories_through_quickjs_os() {
        let script = write_temp_script(
            "mkdir-demo.js",
            r#"
import * as std from "qjs:std";
import * as os from "qjs:os";

std.out.puts("mkdir " + os.mkdir("made", 0o777) + "\n");
std.writeFile("made/file.txt", "from mkdir");
std.out.puts(std.loadFile("made/file.txt") + "\n");
std.out.flush();
"#,
        );

        let output = run(["qjs".into(), script.into_os_string()]).unwrap();

        assert_eq!(output.exit_code(), 0);
        assert_eq!(output.stdout(), b"mkdir 0\nfrom mkdir\n");
        assert!(output.stderr().is_empty());
    }

    #[test]
    fn qjs_example_mkdir_demo_creates_directories_through_quickjs_os() {
        let output = run([
            "qjs".into(),
            example_script("qjs-mkdir-demo.js").into_os_string(),
        ])
        .unwrap();

        assert_eq!(output.exit_code(), 0);
        assert_eq!(
            output.stdout(),
            b"mkdir: 0\nmessage: hello from a created directory\n"
        );
        assert!(output.stderr().is_empty());
    }

    #[test]
    fn qjs_command_removes_directories_through_quickjs_os() {
        let script = write_temp_script(
            "rmdir-demo.js",
            r#"
import * as std from "qjs:std";
import * as os from "qjs:os";

os.mkdir("gone", 0o777);
std.out.puts("remove dir " + os.remove("gone") + "\n");
const probe = os.open("gone", os.O_RDONLY);
if (probe >= 0) {
  os.close(probe);
}
std.out.puts("gone " + (probe < 0) + "\n");
std.out.flush();
"#,
        );

        let output = run(["qjs".into(), script.into_os_string()]).unwrap();

        assert_eq!(output.exit_code(), 0);
        assert_eq!(output.stdout(), b"remove dir 0\ngone true\n");
        assert!(output.stderr().is_empty());
    }

    #[test]
    fn qjs_example_rmdir_demo_removes_directories_through_quickjs_os() {
        let output = run([
            "qjs".into(),
            example_script("qjs-rmdir-demo.js").into_os_string(),
        ])
        .unwrap();

        assert_eq!(output.exit_code(), 0);
        assert_eq!(
            output.stdout(),
            b"remove dir: 0\ngone: true\nkept: still here\n"
        );
        assert!(output.stderr().is_empty());
    }

    #[test]
    fn qjs_example_readdir_demo_lists_directories_through_quickjs_os() {
        let output = run([
            "qjs".into(),
            example_script("qjs-readdir-demo.js").into_os_string(),
        ])
        .unwrap();

        assert_eq!(output.exit_code(), 0);
        assert_eq!(output.stdout(), b"listing: a.txt,b.txt,nested\nnested: \n");
        assert!(output.stderr().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn qjs_example_symlink_demo_uses_live_wasi_host_mount() {
        let host = temp_dir("wanix-cli-symlink-demo");
        let output = run([
            "qjs".into(),
            "--mount".into(),
            format!("{}=host", host.display()).into(),
            example_script("qjs-symlink-demo.js").into_os_string(),
        ])
        .unwrap();

        assert_eq!(output.exit_code(), 0);
        assert_eq!(
            output.stdout(),
            b"symlink: 0\nreadlink: 0 target.txt\nlstat link: 0 true\nstat target: 0 true\nload: linked data\n"
        );
        assert!(output.stderr().is_empty());
        assert_eq!(
            fs::read_link(host.join("link.txt")).unwrap(),
            std::path::Path::new("target.txt")
        );
        assert_eq!(fs::read(host.join("target.txt")).unwrap(), b"linked data");
        fs::remove_dir_all(host).unwrap();
    }

    #[test]
    fn qjs_example_truncate_demo_resizes_wanix_namespace_file() {
        let output = run([
            "qjs".into(),
            example_script("qjs-truncate-demo.js").into_os_string(),
        ])
        .unwrap();

        assert_eq!(output.exit_code(), 0);
        assert_eq!(
            output.stdout(),
            b"ftruncate: 0\nsmall: abc\ntruncate: 0\nlen: 5\ncodes: 97,98,99,0,0\n"
        );
        assert!(output.stderr().is_empty());
    }

    #[test]
    fn qjs_command_attaches_stdin_as_wanix_task_fd_zero() {
        let script = write_temp_script(
            "stdin-demo.js",
            r##"
import * as std from "qjs:std";
import * as os from "qjs:os";

const readStdin = () => {
  const bytes = new Uint8Array(1024);
  const n = os.read(0, bytes.buffer, 0, bytes.length);
  return Array.from(bytes.slice(0, n)).map((byte) => String.fromCharCode(byte)).join("");
};

std.out.puts("stdin " + readStdin() + "\n");
std.out.puts("again " + JSON.stringify(readStdin()) + "\n");
std.out.puts("task " + std.loadFile("#task/self/id").trim() + "\n");
std.out.flush();
"##,
        );

        let output = run([
            "qjs".into(),
            "--stdin".into(),
            "hello from fd0".into(),
            script.into_os_string(),
        ])
        .unwrap();

        assert_eq!(output.exit_code(), 0);
        assert_eq!(
            output.stdout(),
            b"stdin hello from fd0\nagain \"\"\ntask 1\n"
        );
        assert!(output.stderr().is_empty());
    }

    #[test]
    fn qjs_command_reads_task_stdin_from_file() {
        let stdin_path = temp_dir("wanix-cli-stdin-file").join("input.txt");
        fs::write(&stdin_path, b"from stdin file\n").unwrap();

        let output = run([
            "qjs".into(),
            "--stdin-file".into(),
            stdin_path.into_os_string(),
            example_script("qjs-stdin-demo.js").into_os_string(),
        ])
        .unwrap();

        assert_eq!(output.exit_code(), 0);
        assert_eq!(output.stdout(), b"stdin: from stdin file\ntask id: 1\n");
        assert!(output.stderr().is_empty());
    }

    #[test]
    fn qjs_command_reads_task_stdin_from_native_stdin_dash() {
        let output = run_with_process_stdin(
            [
                "qjs".into(),
                "--stdin-file".into(),
                "-".into(),
                example_script("qjs-stdin-demo.js").into_os_string(),
            ],
            b"from host pipe\n".as_slice(),
        )
        .unwrap();

        assert_eq!(output.exit_code(), 0);
        assert_eq!(output.stdout(), b"stdin: from host pipe\ntask id: 1\n");
        assert!(output.stderr().is_empty());
    }

    #[test]
    fn qjs_command_flows_env_cwd_and_args_from_wanix_task_state() {
        let script = write_temp_script(
            "context.js",
            r##"
import * as std from "qjs:std";

const env = std.getenviron();
std.out.puts("cmd " + std.loadFile("#task/self/cmd").trim() + "\n");
std.out.puts("cwd " + std.loadFile("#task/self/dir").trim() + "\n");
std.out.puts("args " + scriptArgs.slice(1).join("/") + "\n");
std.out.puts("mode " + std.getenv("MODE") + "\n");
std.out.puts("all " + env.MODE + "\n");
std.out.puts("source " + std.loadFile("main.js").includes("std.loadFile") + "\n");
std.writeFile("created.txt", "made in cwd");
std.out.puts("created " + std.loadFile("created.txt") + "\n");
std.out.puts("id " + std.loadFile("#task/self/id").trim() + "\n");
std.out.flush();
"##,
        );

        let output = run([
            "qjs".into(),
            "--env".into(),
            "MODE=test".into(),
            "--cwd".into(),
            "app".into(),
            script.into_os_string(),
            "--".into(),
            "alpha".into(),
            "two words".into(),
            "beta".into(),
        ])
        .unwrap();

        assert_eq!(output.exit_code(), 0);
        assert_eq!(
            output.stdout(),
            b"cmd main.js alpha 'two words' beta\ncwd app\nargs alpha/two words/beta\nmode test\nall test\nsource true\ncreated made in cwd\nid 1\n"
        );
        assert!(output.stderr().is_empty());
    }

    #[test]
    fn qjs_command_rejects_invalid_env_keys_and_missing_stdin_text() {
        let script = write_temp_script("context.js", "print('unused');");

        let env_error = run([
            "qjs".into(),
            "--env".into(),
            "BAD KEY=value".into(),
            script.clone().into_os_string(),
        ])
        .unwrap_err();
        assert_eq!(env_error.exit_code(), 2);
        assert!(env_error.to_string().contains("KEY=VALUE"));

        let stdin_error = run(["qjs", "--stdin"]).unwrap_err();
        assert_eq!(stdin_error.exit_code(), 2);
        assert!(stdin_error.to_string().contains("--stdin expects text"));

        let stdin_file_error = run(["qjs", "--stdin-file"]).unwrap_err();
        assert_eq!(stdin_file_error.exit_code(), 2);
        assert!(
            stdin_file_error
                .to_string()
                .contains("--stdin-file expects PATH or -")
        );

        let duplicate_stdin = run([
            "qjs".into(),
            "--stdin".into(),
            "text".into(),
            "--stdin-file".into(),
            "-".into(),
            script.into_os_string(),
        ])
        .unwrap_err();
        assert_eq!(duplicate_stdin.exit_code(), 2);
        assert!(
            duplicate_stdin
                .to_string()
                .contains("only one of --stdin or --stdin-file")
        );
    }

    #[test]
    fn qjs_command_reports_missing_stdin_file() {
        let script = write_temp_script("context.js", "print('unused');");

        let error = run([
            "qjs".into(),
            "--stdin-file".into(),
            "/definitely/not/a/wanix/stdin/file".into(),
            script.into_os_string(),
        ])
        .unwrap_err();

        assert_eq!(error.exit_code(), 1);
        assert!(error.to_string().contains("failed to read stdin file"));
    }

    fn write_temp_script(name: &str, source: &str) -> PathBuf {
        let mut path = temp_dir("wanix-cli-test");
        path.push(name);
        fs::write(&path, source).unwrap();
        path
    }

    fn temp_dir(prefix: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        let nonce = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
        path.push(format!("{prefix}-{}-{nonce}", std::process::id()));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn example_script(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples")
            .join(name)
    }
}
