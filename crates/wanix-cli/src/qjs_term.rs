#[cfg(all(test, unix))]
use std::collections::VecDeque;
use std::ffi::OsString;
#[cfg(unix)]
use std::io;
use std::io::{Read, Write};
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
#[cfg(all(test, unix))]
use std::sync::Mutex;
use std::time::Duration;

use wanix_fs::{FileSystem, MemFs, NormalizedPath, OpenOptions};
use wanix_qjs::{QuickJsTaskDriver, QuickJsTaskRuntime};
use wanix_task::{Fd, Task, TaskTable};
use wanix_term::TermDevice;
use wanix_vfs::BindOptions;

use super::{
    CliError, CliOutput, QJS_GUEST_SCRIPT, QjsCommand, apply_qjs_task_runtime_limits,
    bind_host_mounts, configure_qjs_task, copy_script_directory, eval_qjs_source,
    guest_path_in_cwd, os_arg_to_string, parse_exit, parse_qjs_command_for, quickjs_runner,
    read_qjs_stdin, read_utf8_script, write_process_output,
};

const QJS_SHELL_SOURCE: &str = include_str!("../../../examples/qjs-term-shell-demo.js");
const QJS_SHELL_SCRIPT_SENTINEL: &str = "__wanix_qjs_shell.js";
const QJS_SHELL_READY_IO_TURNS: usize = 2;
const QJS_SHELL_IDLE_EVENT_LOOP_BUDGET_MS: u64 = 20;

mod session;

pub(crate) use session::QjsShellSession;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct QjsTermCommand {
    qjs: QjsCommand,
    feed_after_eval: Vec<PostEvalFeed>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct QjsShellCommand {
    qjs: QjsCommand,
    raw: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TermResize {
    columns: u16,
    rows: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProcessInputMode {
    Blocking,
    #[cfg(unix)]
    PollFd(libc::c_int),
}

#[derive(Debug, Clone)]
enum ProcessResizeSource {
    None,
    #[cfg(unix)]
    TerminalSizeFd(TerminalSizeSource),
    #[cfg(all(test, unix))]
    Queue(Arc<Mutex<VecDeque<(u16, u16)>>>),
}

#[derive(Debug, Clone)]
struct ProcessEventSources {
    input_mode: ProcessInputMode,
    resize_source: ProcessResizeSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TerminalPumpPolicy {
    ready_io_turns: usize,
    event_loop_wait_budget: Duration,
    input_mode: ProcessInputMode,
}

#[derive(Debug, Clone)]
struct TerminalPumpState {
    policy: TerminalPumpPolicy,
    resize_source: ProcessResizeSource,
}

#[cfg(unix)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TerminalSizeSource {
    fd: libc::c_int,
    last: Option<TermResize>,
}

impl ProcessResizeSource {
    fn next_resize(&mut self) -> Result<Option<TermResize>, CliError> {
        match self {
            Self::None => Ok(None),
            #[cfg(unix)]
            Self::TerminalSizeFd(source) => source.next_resize(),
            #[cfg(all(test, unix))]
            Self::Queue(queue) => {
                let Some((columns, rows)) = queue
                    .lock()
                    .map_err(|_| CliError::new("test resize queue lock poisoned", 1))?
                    .pop_front()
                else {
                    return Ok(None);
                };
                Ok(Some(TermResize { columns, rows }))
            }
        }
    }
}

impl ProcessEventSources {
    fn blocking() -> Self {
        Self {
            input_mode: ProcessInputMode::Blocking,
            resize_source: ProcessResizeSource::None,
        }
    }

    #[cfg(unix)]
    fn input_fd(input_fd: libc::c_int) -> Self {
        Self {
            input_mode: ProcessInputMode::PollFd(input_fd),
            resize_source: ProcessResizeSource::None,
        }
    }

    #[cfg(unix)]
    fn terminal_fds(input_fd: libc::c_int, terminal_size_fd: libc::c_int) -> Self {
        Self {
            input_mode: ProcessInputMode::PollFd(input_fd),
            resize_source: ProcessResizeSource::TerminalSizeFd(TerminalSizeSource::new(
                terminal_size_fd,
            )),
        }
    }

    #[cfg(all(test, unix))]
    fn resize_queue(input_fd: libc::c_int, resize_queue: Arc<Mutex<VecDeque<(u16, u16)>>>) -> Self {
        Self {
            input_mode: ProcessInputMode::PollFd(input_fd),
            resize_source: ProcessResizeSource::Queue(resize_queue),
        }
    }
}

#[cfg(unix)]
impl TerminalSizeSource {
    fn new(fd: libc::c_int) -> Self {
        Self { fd, last: None }
    }

    fn next_resize(&mut self) -> Result<Option<TermResize>, CliError> {
        let Some(resize) = terminal_size_for_fd(self.fd)? else {
            return Ok(None);
        };
        if self.last == Some(resize) {
            return Ok(None);
        }
        self.last = Some(resize);
        Ok(Some(resize))
    }
}

impl TermResize {
    fn payload(&self) -> Vec<u8> {
        format!("{} {}\n", self.columns, self.rows).into_bytes()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PostEvalFeed {
    Bytes(Vec<u8>),
    File(PathBuf),
    Process,
    LinesFile(PathBuf),
    LinesProcess,
    RawBytesProcess,
    Resize(TermResize),
}

pub(super) fn parse_qjs_term_command(args: &[OsString]) -> Result<QjsTermCommand, CliError> {
    let mut qjs_args = Vec::new();
    let mut feed_after_eval = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--feed-after-eval" {
            i += 1;
            let value = args
                .get(i)
                .ok_or_else(|| CliError::usage("qjs-term --feed-after-eval expects text"))?;
            feed_after_eval.push(PostEvalFeed::Bytes(
                os_arg_to_string(value, "qjs-term --feed-after-eval")?.into_bytes(),
            ));
            i += 1;
        } else if args[i] == "--feed-after-eval-file" {
            i += 1;
            let value = args.get(i).ok_or_else(|| {
                CliError::usage("qjs-term --feed-after-eval-file expects PATH or -")
            })?;
            if value == "-" {
                feed_after_eval.push(PostEvalFeed::Process);
            } else {
                feed_after_eval.push(PostEvalFeed::File(PathBuf::from(value)));
            }
            i += 1;
        } else if args[i] == "--feed-after-eval-lines" {
            i += 1;
            let value = args.get(i).ok_or_else(|| {
                CliError::usage("qjs-term --feed-after-eval-lines expects PATH or -")
            })?;
            if value == "-" {
                feed_after_eval.push(PostEvalFeed::LinesProcess);
            } else {
                feed_after_eval.push(PostEvalFeed::LinesFile(PathBuf::from(value)));
            }
            i += 1;
        } else if args[i] == "--resize-after-eval" {
            i += 1;
            let value = args
                .get(i)
                .ok_or_else(|| CliError::usage("qjs-term --resize-after-eval expects COLSxROWS"))?;
            feed_after_eval.push(PostEvalFeed::Resize(parse_term_resize(
                value,
                "qjs-term --resize-after-eval",
            )?));
            i += 1;
        } else if args[i] == "--" {
            qjs_args.extend_from_slice(&args[i..]);
            break;
        } else if qjs_option_takes_value(&args[i]) {
            qjs_args.push(args[i].clone());
            i += 1;
            if let Some(value) = args.get(i) {
                qjs_args.push(value.clone());
                i += 1;
            }
        } else {
            qjs_args.extend_from_slice(&args[i..]);
            break;
        }
    }
    Ok(QjsTermCommand {
        qjs: parse_qjs_command_for(&qjs_args, "qjs-term")?,
        feed_after_eval,
    })
}

pub(super) fn parse_qjs_shell_command(args: &[OsString]) -> Result<QjsShellCommand, CliError> {
    let mut qjs_args = Vec::new();
    let mut raw = false;
    for arg in args {
        if arg == "--raw" {
            raw = true;
        } else {
            qjs_args.push(arg.clone());
        }
    }
    qjs_args.push(OsString::from(QJS_SHELL_SCRIPT_SENTINEL));
    let qjs = parse_qjs_command_for(&qjs_args, "qjs-shell")?;
    if qjs.script_path != Path::new(QJS_SHELL_SCRIPT_SENTINEL) || !qjs.args.is_empty() {
        return Err(CliError::usage(
            "qjs-shell does not accept a script path or script arguments",
        ));
    }
    if qjs.stdin.is_some() {
        return Err(CliError::usage(
            "qjs-shell reads native stdin as terminal input; use qjs-term for explicit stdin fixtures",
        ));
    }
    Ok(QjsShellCommand { qjs, raw })
}

fn qjs_option_takes_value(arg: &OsString) -> bool {
    matches!(
        arg.to_str(),
        Some(
            "--env"
                | "--cwd"
                | "--stdin"
                | "--stdin-file"
                | "--event-loop-ms"
                | "--ready-io-turns"
                | "--interrupt-after"
                | "--memory-limit-bytes"
                | "--mount"
        )
    )
}

fn parse_term_resize(arg: &OsString, label: &str) -> Result<TermResize, CliError> {
    let value = os_arg_to_string(arg, label)?;
    let Some((columns, rows)) = value.split_once('x').or_else(|| value.split_once('X')) else {
        return Err(CliError::usage(format!("{label} expects COLSxROWS")));
    };
    let columns = parse_positive_u16(columns, &format!("{label} columns"))?;
    let rows = parse_positive_u16(rows, &format!("{label} rows"))?;
    Ok(TermResize { columns, rows })
}

fn parse_positive_u16(value: &str, label: &str) -> Result<u16, CliError> {
    let number = value
        .parse::<u16>()
        .map_err(|_| CliError::usage(format!("{label} expects an integer from 1 to 65535")))?;
    if number == 0 {
        return Err(CliError::usage(format!(
            "{label} expects an integer from 1 to 65535"
        )));
    }
    Ok(number)
}

pub(super) fn run_qjs_term(
    command: QjsTermCommand,
    process_stdin: &mut dyn Read,
) -> Result<CliOutput, CliError> {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let exit_code = run_qjs_term_streaming(command, process_stdin, &mut stdout, &mut stderr)?;
    Ok(CliOutput::new(stdout, stderr, exit_code))
}

pub(super) fn run_qjs_term_streaming(
    command: QjsTermCommand,
    process_stdin: &mut dyn Read,
    process_stdout: &mut dyn Write,
    process_stderr: &mut dyn Write,
) -> Result<i32, CliError> {
    run_qjs_term_program_streaming(
        command.qjs,
        command.feed_after_eval,
        QjsTermProgram::HostScript,
        process_stdin,
        process_stdout,
        process_stderr,
    )
}

pub(super) fn run_qjs_shell(
    command: QjsShellCommand,
    process_stdin: &mut dyn Read,
) -> Result<CliOutput, CliError> {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let exit_code = run_qjs_shell_streaming(command, process_stdin, &mut stdout, &mut stderr)?;
    Ok(CliOutput::new(stdout, stderr, exit_code))
}

pub(super) fn run_qjs_shell_streaming(
    command: QjsShellCommand,
    process_stdin: &mut dyn Read,
    process_stdout: &mut dyn Write,
    process_stderr: &mut dyn Write,
) -> Result<i32, CliError> {
    run_qjs_term_program_streaming(
        qjs_shell_command(command.qjs, command.raw),
        vec![if command.raw {
            PostEvalFeed::RawBytesProcess
        } else {
            PostEvalFeed::LinesProcess
        }],
        QjsTermProgram::BundledShell,
        process_stdin,
        process_stdout,
        process_stderr,
    )
}

#[cfg(unix)]
pub(super) fn run_qjs_shell_streaming_with_input_fd(
    command: QjsShellCommand,
    process_stdin: &mut dyn Read,
    input_fd: libc::c_int,
    process_stdout: &mut dyn Write,
    process_stderr: &mut dyn Write,
) -> Result<i32, CliError> {
    run_qjs_term_program_streaming_with_input_mode(
        qjs_shell_command(command.qjs, command.raw),
        vec![if command.raw {
            PostEvalFeed::RawBytesProcess
        } else {
            PostEvalFeed::LinesProcess
        }],
        QjsTermProgram::BundledShell,
        ProcessEventSources::input_fd(input_fd),
        process_stdin,
        process_stdout,
        process_stderr,
    )
}

#[cfg(unix)]
pub(super) fn run_qjs_shell_streaming_with_terminal_fds(
    command: QjsShellCommand,
    process_stdin: &mut dyn Read,
    input_fd: libc::c_int,
    terminal_size_fd: libc::c_int,
    process_stdout: &mut dyn Write,
    process_stderr: &mut dyn Write,
) -> Result<i32, CliError> {
    run_qjs_term_program_streaming_with_input_mode(
        qjs_shell_command(command.qjs, command.raw),
        vec![if command.raw {
            PostEvalFeed::RawBytesProcess
        } else {
            PostEvalFeed::LinesProcess
        }],
        QjsTermProgram::BundledShell,
        ProcessEventSources::terminal_fds(input_fd, terminal_size_fd),
        process_stdin,
        process_stdout,
        process_stderr,
    )
}

#[cfg(all(test, unix))]
pub(super) fn run_qjs_shell_streaming_with_resize_queue(
    command: QjsShellCommand,
    process_stdin: &mut dyn Read,
    input_fd: libc::c_int,
    resize_queue: Arc<Mutex<VecDeque<(u16, u16)>>>,
    process_stdout: &mut dyn Write,
    process_stderr: &mut dyn Write,
) -> Result<i32, CliError> {
    run_qjs_term_program_streaming_with_input_mode(
        qjs_shell_command(command.qjs, command.raw),
        vec![if command.raw {
            PostEvalFeed::RawBytesProcess
        } else {
            PostEvalFeed::LinesProcess
        }],
        QjsTermProgram::BundledShell,
        ProcessEventSources::resize_queue(input_fd, resize_queue),
        process_stdin,
        process_stdout,
        process_stderr,
    )
}

fn qjs_shell_command(mut command: QjsCommand, raw: bool) -> QjsCommand {
    command.ready_io_turns = command.ready_io_turns.max(QJS_SHELL_READY_IO_TURNS);
    if raw {
        command.env.push("WANIX_QJS_SHELL_RAW=1".to_owned());
    }
    command
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QjsTermProgram {
    HostScript,
    BundledShell,
}

fn run_qjs_term_program_streaming(
    qjs_command: QjsCommand,
    feed_after_eval: Vec<PostEvalFeed>,
    program: QjsTermProgram,
    process_stdin: &mut dyn Read,
    process_stdout: &mut dyn Write,
    process_stderr: &mut dyn Write,
) -> Result<i32, CliError> {
    run_qjs_term_program_streaming_with_input_mode(
        qjs_command,
        feed_after_eval,
        program,
        ProcessEventSources::blocking(),
        process_stdin,
        process_stdout,
        process_stderr,
    )
}

fn run_qjs_term_program_streaming_with_input_mode(
    qjs_command: QjsCommand,
    feed_after_eval: Vec<PostEvalFeed>,
    program: QjsTermProgram,
    event_sources: ProcessEventSources,
    process_stdin: &mut dyn Read,
    process_stdout: &mut dyn Write,
    process_stderr: &mut dyn Write,
) -> Result<i32, CliError> {
    let script = read_qjs_term_program(&qjs_command.script_path, program)?;
    let stdin_bytes = read_qjs_stdin(qjs_command.stdin, process_stdin)?;

    let runner = quickjs_runner()?;
    let table = TaskTable::new();
    let mut driver = QuickJsTaskDriver::new(Arc::clone(&runner))
        .with_event_loop_wait_budget(qjs_command.event_loop_wait_budget)
        .with_ready_io_turns(qjs_command.ready_io_turns);
    if let Some(budget) = qjs_command.interrupt_poll_budget {
        driver = driver.with_interrupt_poll_budget(budget);
    }
    if let Some(bytes) = qjs_command.memory_limit_bytes {
        driver = driver.with_memory_limit_bytes(bytes);
    }
    table.register_driver("qjs", Arc::new(driver))?;
    let task = table.allocate_root("qjs")?;

    let root = Arc::new(MemFs::new());
    if program == QjsTermProgram::HostScript {
        copy_script_directory(&qjs_command.script_path, &root, &qjs_command.cwd)?;
    }
    let runtime_cwd = if program == QjsTermProgram::BundledShell {
        // The bundled shell implements cwd in guest code; keep the WASI root
        // at namespace root so shell navigation is not trapped below --cwd.
        NormalizedPath::new(".")?
    } else {
        qjs_command.cwd.clone()
    };
    let program_path = if program == QjsTermProgram::BundledShell {
        QJS_SHELL_SCRIPT_SENTINEL
    } else {
        QJS_GUEST_SCRIPT
    };
    let guest_script = if program == QjsTermProgram::BundledShell {
        QJS_SHELL_SCRIPT_SENTINEL.to_owned()
    } else {
        guest_path_in_cwd(&qjs_command.cwd, QJS_GUEST_SCRIPT)?
    };
    root.write_file(guest_script.as_str(), script.as_bytes())?;
    task.bind(root, ".", ".", BindOptions::default())?;
    bind_host_mounts(&task, &qjs_command.mounts)?;

    let (terminal, terminal_id) = attach_task_terminal(&task, stdin_bytes)?;
    let mut task_env = qjs_command.env.clone();
    if program == QjsTermProgram::BundledShell {
        task_env.push(format!("WANIX_TERM_ID={terminal_id}"));
    }
    configure_qjs_task(
        &task,
        program_path,
        &qjs_command.args,
        &task_env,
        &runtime_cwd,
    )?;

    let start_result = (|| -> Result<(), CliError> {
        let mut runtime = runner.create_task_runtime(&task)?;
        if program == QjsTermProgram::BundledShell {
            task.set_dir(qjs_command.cwd.as_str())?;
        }
        apply_qjs_task_runtime_limits(
            &mut runtime,
            qjs_command.interrupt_poll_budget,
            qjs_command.memory_limit_bytes,
        )?;
        let eval_ready_io_turns = if feed_after_eval.is_empty() {
            qjs_command.ready_io_turns
        } else {
            0
        };
        let eval_result = eval_qjs_source(
            &mut runtime,
            &script,
            &guest_script,
            qjs_command.event_loop_wait_budget,
            eval_ready_io_turns,
        );
        drain_terminal_output(&terminal, &terminal_id, process_stdout)?;
        eval_result?;
        if !feed_after_eval.is_empty() {
            run_post_eval_feeds(
                feed_after_eval,
                process_stdin,
                &terminal,
                &terminal_id,
                &mut runtime,
                TerminalPumpState {
                    policy: TerminalPumpPolicy {
                        ready_io_turns: qjs_command.ready_io_turns,
                        event_loop_wait_budget: qjs_command.event_loop_wait_budget,
                        input_mode: event_sources.input_mode,
                    },
                    resize_source: event_sources.resize_source,
                },
                process_stdout,
            )?;
        }
        let finish_result = runtime.finish();
        drain_terminal_output(&terminal, &terminal_id, process_stdout)?;
        finish_result?;
        Ok(())
    })();

    finish_terminal_task_output(
        start_result,
        &task,
        &terminal,
        &terminal_id,
        process_stdout,
        process_stderr,
    )
}

fn read_qjs_term_program(script_path: &Path, program: QjsTermProgram) -> Result<String, CliError> {
    match program {
        QjsTermProgram::HostScript => read_utf8_script(script_path),
        QjsTermProgram::BundledShell => Ok(QJS_SHELL_SOURCE.to_owned()),
    }
}

fn run_post_eval_feeds(
    feeds: Vec<PostEvalFeed>,
    process_stdin: &mut dyn Read,
    terminal: &TermDevice,
    terminal_id: &str,
    runtime: &mut QuickJsTaskRuntime,
    mut pump_state: TerminalPumpState,
    process_stdout: &mut dyn Write,
) -> Result<(), CliError> {
    let policy = pump_state.policy;
    let mut current_batch = Vec::new();
    for feed in feeds {
        match feed {
            PostEvalFeed::Bytes(bytes) => current_batch.push(bytes),
            PostEvalFeed::File(path) => {
                let bytes = std::fs::read(&path).map_err(|error| {
                    CliError::new(
                        format!(
                            "failed to read post-eval feed file {}: {error}",
                            path.display()
                        ),
                        1,
                    )
                })?;
                current_batch.push(bytes);
            }
            PostEvalFeed::Process => {
                let mut bytes = Vec::new();
                process_stdin.read_to_end(&mut bytes).map_err(|error| {
                    CliError::new(
                        format!("failed to read process stdin after eval: {error}"),
                        1,
                    )
                })?;
                current_batch.push(bytes);
            }
            PostEvalFeed::LinesFile(path) => {
                flush_terminal_feed_batch(
                    terminal,
                    terminal_id,
                    runtime,
                    &mut current_batch,
                    policy.ready_io_turns,
                    policy.event_loop_wait_budget,
                    process_stdout,
                )?;
                if task_exited(runtime)? {
                    return Ok(());
                }
                let bytes = std::fs::read(&path).map_err(|error| {
                    CliError::new(
                        format!(
                            "failed to read post-eval feed lines file {}: {error}",
                            path.display()
                        ),
                        1,
                    )
                })?;
                for line in split_feed_lines(bytes) {
                    feed_terminal_batch_and_pump(
                        terminal,
                        terminal_id,
                        runtime,
                        &[line],
                        policy.ready_io_turns,
                        policy.event_loop_wait_budget,
                        process_stdout,
                    )?;
                    if task_exited(runtime)? {
                        return Ok(());
                    }
                }
            }
            PostEvalFeed::LinesProcess => {
                flush_terminal_feed_batch(
                    terminal,
                    terminal_id,
                    runtime,
                    &mut current_batch,
                    policy.ready_io_turns,
                    policy.event_loop_wait_budget,
                    process_stdout,
                )?;
                if task_exited(runtime)? {
                    return Ok(());
                }
                run_process_line_feed_session_after_eval(
                    process_stdin,
                    terminal,
                    terminal_id,
                    runtime,
                    &mut pump_state,
                    process_stdout,
                )?;
            }
            PostEvalFeed::RawBytesProcess => {
                flush_terminal_feed_batch(
                    terminal,
                    terminal_id,
                    runtime,
                    &mut current_batch,
                    policy.ready_io_turns,
                    policy.event_loop_wait_budget,
                    process_stdout,
                )?;
                if task_exited(runtime)? {
                    return Ok(());
                }
                run_process_raw_byte_feed_session_after_eval(
                    process_stdin,
                    terminal,
                    terminal_id,
                    runtime,
                    &mut pump_state,
                    process_stdout,
                )?;
            }
            PostEvalFeed::Resize(resize) => {
                flush_terminal_feed_batch(
                    terminal,
                    terminal_id,
                    runtime,
                    &mut current_batch,
                    policy.ready_io_turns,
                    policy.event_loop_wait_budget,
                    process_stdout,
                )?;
                if task_exited(runtime)? {
                    return Ok(());
                }
                feed_terminal_resize_and_pump(
                    terminal,
                    terminal_id,
                    runtime,
                    &resize,
                    policy.ready_io_turns,
                    policy.event_loop_wait_budget,
                    process_stdout,
                )?;
                if task_exited(runtime)? {
                    return Ok(());
                }
            }
        }
    }
    flush_terminal_feed_batch(
        terminal,
        terminal_id,
        runtime,
        &mut current_batch,
        policy.ready_io_turns,
        policy.event_loop_wait_budget,
        process_stdout,
    )
}

fn run_process_raw_byte_feed_session_after_eval(
    process_stdin: &mut dyn Read,
    terminal: &TermDevice,
    terminal_id: &str,
    runtime: &mut QuickJsTaskRuntime,
    pump_state: &mut TerminalPumpState,
    process_stdout: &mut dyn Write,
) -> Result<(), CliError> {
    let policy = pump_state.policy;
    #[cfg(unix)]
    if let ProcessInputMode::PollFd(input_fd) = policy.input_mode {
        return run_process_polled_feed_session_after_eval(
            process_stdin,
            input_fd,
            terminal,
            terminal_id,
            runtime,
            pump_state,
            process_stdout,
        );
    }

    let mut byte = [0; 1];
    loop {
        let count = process_stdin.read(&mut byte).map_err(|error| {
            CliError::new(
                format!("failed to read process stdin raw bytes after eval: {error}"),
                1,
            )
        })?;
        if count == 0 {
            return Ok(());
        }
        feed_terminal_chunk_and_pump(
            terminal,
            terminal_id,
            runtime,
            &byte[..count],
            policy.ready_io_turns,
            policy.event_loop_wait_budget,
            process_stdout,
        )?;
        if task_exited(runtime)? {
            break;
        }
    }
    Ok(())
}

fn split_feed_lines(bytes: Vec<u8>) -> Vec<Vec<u8>> {
    let mut chunks = Vec::new();
    let mut start = 0;
    for (index, byte) in bytes.iter().enumerate() {
        if *byte == b'\n' {
            chunks.push(bytes[start..=index].to_vec());
            start = index + 1;
        }
    }
    if start < bytes.len() {
        chunks.push(bytes[start..].to_vec());
    }
    chunks
}

fn run_process_line_feed_session_after_eval(
    process_stdin: &mut dyn Read,
    terminal: &TermDevice,
    terminal_id: &str,
    runtime: &mut QuickJsTaskRuntime,
    pump_state: &mut TerminalPumpState,
    process_stdout: &mut dyn Write,
) -> Result<(), CliError> {
    let policy = pump_state.policy;
    #[cfg(unix)]
    if let ProcessInputMode::PollFd(input_fd) = policy.input_mode {
        return run_process_polled_feed_session_after_eval(
            process_stdin,
            input_fd,
            terminal,
            terminal_id,
            runtime,
            pump_state,
            process_stdout,
        );
    }

    let mut line = Vec::new();
    while read_process_line_after_eval(process_stdin, &mut line)? {
        feed_terminal_batch_and_pump(
            terminal,
            terminal_id,
            runtime,
            &[line.clone()],
            policy.ready_io_turns,
            policy.event_loop_wait_budget,
            process_stdout,
        )?;
        if task_exited(runtime)? {
            break;
        }
    }
    Ok(())
}

#[cfg(unix)]
fn run_process_polled_feed_session_after_eval(
    process_stdin: &mut dyn Read,
    input_fd: libc::c_int,
    terminal: &TermDevice,
    terminal_id: &str,
    runtime: &mut QuickJsTaskRuntime,
    pump_state: &mut TerminalPumpState,
    process_stdout: &mut dyn Write,
) -> Result<(), CliError> {
    let _nonblocking = NonBlockingFd::enter(input_fd)?;
    let mut bytes = [0; 1024];
    let poll_timeout = Duration::from_millis(QJS_SHELL_IDLE_EVENT_LOOP_BUDGET_MS);
    let policy = pump_state.policy;
    let idle_budget = qjs_shell_idle_event_loop_budget(policy.event_loop_wait_budget);
    loop {
        match poll_process_stdin(input_fd, poll_timeout)? {
            ProcessStdinPoll::Ready => {
                pump_terminal_resize_if_changed(
                    terminal,
                    terminal_id,
                    runtime,
                    pump_state,
                    process_stdout,
                )?;
                let count = match process_stdin.read(&mut bytes) {
                    Ok(count) => count,
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        pump_terminal_resize_if_changed(
                            terminal,
                            terminal_id,
                            runtime,
                            pump_state,
                            process_stdout,
                        )?;
                        pump_terminal_idle(
                            terminal,
                            terminal_id,
                            runtime,
                            policy.ready_io_turns,
                            idle_budget,
                            process_stdout,
                        )?;
                        continue;
                    }
                    Err(error) => {
                        return Err(CliError::new(
                            format!("failed to read process stdin after eval: {error}"),
                            1,
                        ));
                    }
                };
                if count == 0 {
                    return Ok(());
                }
                feed_terminal_chunk_and_pump(
                    terminal,
                    terminal_id,
                    runtime,
                    &bytes[..count],
                    policy.ready_io_turns,
                    policy.event_loop_wait_budget,
                    process_stdout,
                )?;
            }
            ProcessStdinPoll::Idle => {
                pump_terminal_resize_if_changed(
                    terminal,
                    terminal_id,
                    runtime,
                    pump_state,
                    process_stdout,
                )?;
                pump_terminal_idle(
                    terminal,
                    terminal_id,
                    runtime,
                    policy.ready_io_turns,
                    idle_budget,
                    process_stdout,
                )?;
            }
        }
        if task_exited(runtime)? {
            return Ok(());
        }
    }
}

#[cfg(unix)]
#[derive(Debug)]
struct NonBlockingFd {
    fd: libc::c_int,
    original_flags: libc::c_int,
}

#[cfg(unix)]
impl NonBlockingFd {
    fn enter(fd: libc::c_int) -> Result<Self, CliError> {
        let original_flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if original_flags < 0 {
            return Err(CliError::new(
                format!(
                    "failed to read process stdin flags: {}",
                    io::Error::last_os_error()
                ),
                1,
            ));
        }
        let nonblocking_flags = original_flags | libc::O_NONBLOCK;
        if unsafe { libc::fcntl(fd, libc::F_SETFL, nonblocking_flags) } < 0 {
            return Err(CliError::new(
                format!(
                    "failed to enter nonblocking process stdin mode: {}",
                    io::Error::last_os_error()
                ),
                1,
            ));
        }
        Ok(Self { fd, original_flags })
    }
}

#[cfg(unix)]
impl Drop for NonBlockingFd {
    fn drop(&mut self) {
        let _ = unsafe { libc::fcntl(self.fd, libc::F_SETFL, self.original_flags) };
    }
}

#[cfg(unix)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProcessStdinPoll {
    Ready,
    Idle,
}

#[cfg(unix)]
fn poll_process_stdin(
    input_fd: libc::c_int,
    timeout: Duration,
) -> Result<ProcessStdinPoll, CliError> {
    let mut poll_fd = libc::pollfd {
        fd: input_fd,
        events: libc::POLLIN,
        revents: 0,
    };
    loop {
        let result = unsafe { libc::poll(&mut poll_fd, 1, poll_timeout_millis(timeout)) };
        if result == 0 {
            return Ok(ProcessStdinPoll::Idle);
        }
        if result < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(CliError::new(
                format!("failed to poll process stdin: {error}"),
                1,
            ));
        }
        if poll_fd.revents & libc::POLLNVAL != 0 {
            return Err(CliError::new("failed to poll process stdin: invalid fd", 1));
        }
        if poll_fd.revents & (libc::POLLIN | libc::POLLHUP | libc::POLLERR) != 0 {
            return Ok(ProcessStdinPoll::Ready);
        }
        return Ok(ProcessStdinPoll::Idle);
    }
}

#[cfg(unix)]
fn poll_timeout_millis(timeout: Duration) -> libc::c_int {
    let millis = timeout.as_millis();
    millis.min(libc::c_int::MAX as u128) as libc::c_int
}

fn qjs_shell_idle_event_loop_budget(configured: Duration) -> Duration {
    if configured.is_zero() {
        Duration::from_millis(QJS_SHELL_IDLE_EVENT_LOOP_BUDGET_MS)
    } else {
        configured
    }
}

#[cfg(unix)]
fn terminal_size_for_fd(fd: libc::c_int) -> Result<Option<TermResize>, CliError> {
    // SAFETY: `isatty` only observes the supplied file descriptor.
    if unsafe { libc::isatty(fd) } == 0 {
        return Ok(None);
    }
    let mut size = std::mem::MaybeUninit::<libc::winsize>::zeroed();
    // SAFETY: `size` points to valid writable memory for TIOCGWINSZ.
    if unsafe { libc::ioctl(fd, libc::TIOCGWINSZ, size.as_mut_ptr()) } != 0 {
        return Err(CliError::new(
            format!(
                "failed to read native terminal size: {}",
                io::Error::last_os_error()
            ),
            1,
        ));
    }
    // SAFETY: ioctl succeeded and initialized the winsize value.
    let size = unsafe { size.assume_init() };
    if size.ws_col == 0 || size.ws_row == 0 {
        return Ok(None);
    }
    Ok(Some(TermResize {
        columns: size.ws_col,
        rows: size.ws_row,
    }))
}

fn read_process_line_after_eval(
    process_stdin: &mut dyn Read,
    line: &mut Vec<u8>,
) -> Result<bool, CliError> {
    line.clear();
    let mut byte = [0; 1];
    loop {
        let count = process_stdin.read(&mut byte).map_err(|error| {
            CliError::new(
                format!("failed to read process stdin lines after eval: {error}"),
                1,
            )
        })?;
        if count == 0 {
            return Ok(!line.is_empty());
        }
        line.push(byte[0]);
        if byte[0] == b'\n' {
            return Ok(true);
        }
    }
}

fn flush_terminal_feed_batch(
    terminal: &TermDevice,
    terminal_id: &str,
    runtime: &mut QuickJsTaskRuntime,
    batch: &mut Vec<Vec<u8>>,
    ready_io_turns: usize,
    event_loop_wait_budget: Duration,
    process_stdout: &mut dyn Write,
) -> Result<(), CliError> {
    if batch.is_empty() {
        return Ok(());
    }
    let flushed = std::mem::take(batch);
    feed_terminal_batch_and_pump(
        terminal,
        terminal_id,
        runtime,
        &flushed,
        ready_io_turns,
        event_loop_wait_budget,
        process_stdout,
    )
}

fn feed_terminal_batch_and_pump(
    terminal: &TermDevice,
    terminal_id: &str,
    runtime: &mut QuickJsTaskRuntime,
    batch: &[Vec<u8>],
    ready_io_turns: usize,
    event_loop_wait_budget: Duration,
    process_stdout: &mut dyn Write,
) -> Result<(), CliError> {
    let result = (|| -> Result<(), CliError> {
        for chunk in batch {
            feed_terminal_after_eval(terminal, terminal_id, chunk)?;
        }
        runtime.run_event_loop_turns(event_loop_wait_budget, ready_io_turns)?;
        Ok(())
    })();
    drain_terminal_output(terminal, terminal_id, process_stdout)?;
    result
}

fn feed_terminal_chunk_and_pump(
    terminal: &TermDevice,
    terminal_id: &str,
    runtime: &mut QuickJsTaskRuntime,
    chunk: &[u8],
    ready_io_turns: usize,
    event_loop_wait_budget: Duration,
    process_stdout: &mut dyn Write,
) -> Result<(), CliError> {
    let result = (|| -> Result<(), CliError> {
        feed_terminal_after_eval(terminal, terminal_id, chunk)?;
        runtime.run_event_loop_turns(event_loop_wait_budget, ready_io_turns)?;
        Ok(())
    })();
    drain_terminal_output(terminal, terminal_id, process_stdout)?;
    result
}

fn pump_terminal_idle(
    terminal: &TermDevice,
    terminal_id: &str,
    runtime: &mut QuickJsTaskRuntime,
    ready_io_turns: usize,
    event_loop_wait_budget: Duration,
    process_stdout: &mut dyn Write,
) -> Result<(), CliError> {
    let result = runtime
        .run_event_loop_turns(event_loop_wait_budget, ready_io_turns)
        .map_err(CliError::from);
    drain_terminal_output(terminal, terminal_id, process_stdout)?;
    result
}

fn pump_terminal_resize_if_changed(
    terminal: &TermDevice,
    terminal_id: &str,
    runtime: &mut QuickJsTaskRuntime,
    pump_state: &mut TerminalPumpState,
    process_stdout: &mut dyn Write,
) -> Result<(), CliError> {
    let Some(resize) = pump_state.resize_source.next_resize()? else {
        return Ok(());
    };
    feed_terminal_resize_and_pump(
        terminal,
        terminal_id,
        runtime,
        &resize,
        pump_state.policy.ready_io_turns,
        pump_state.policy.event_loop_wait_budget,
        process_stdout,
    )
}

fn feed_terminal_resize_and_pump(
    terminal: &TermDevice,
    terminal_id: &str,
    runtime: &mut QuickJsTaskRuntime,
    resize: &TermResize,
    ready_io_turns: usize,
    event_loop_wait_budget: Duration,
    process_stdout: &mut dyn Write,
) -> Result<(), CliError> {
    let result = (|| -> Result<(), CliError> {
        feed_terminal_resize_after_eval(terminal, terminal_id, resize)?;
        runtime.run_event_loop_turns(event_loop_wait_budget, ready_io_turns)?;
        Ok(())
    })();
    drain_terminal_output(terminal, terminal_id, process_stdout)?;
    result
}

fn task_exited(runtime: &QuickJsTaskRuntime) -> Result<bool, CliError> {
    Ok(runtime.exit_code()?.is_some())
}

fn feed_terminal_after_eval(
    terminal: &TermDevice,
    terminal_id: &str,
    chunk: &[u8],
) -> Result<(), CliError> {
    if chunk.is_empty() {
        return Ok(());
    }
    let mut data = terminal.open(
        &NormalizedPath::new(format!("{terminal_id}/data"))?,
        OpenOptions {
            write: true,
            ..OpenOptions::default()
        },
    )?;
    data.write(chunk)?;
    Ok(())
}

fn feed_terminal_resize_after_eval(
    terminal: &TermDevice,
    terminal_id: &str,
    resize: &TermResize,
) -> Result<(), CliError> {
    let mut winch = terminal.open(
        &NormalizedPath::new(format!("{terminal_id}/winch"))?,
        OpenOptions {
            write: true,
            ..OpenOptions::default()
        },
    )?;
    winch.write(&resize.payload())?;
    Ok(())
}

fn attach_task_terminal(
    task: &Task,
    stdin_bytes: Option<Vec<u8>>,
) -> Result<(Arc<TermDevice>, String), CliError> {
    let terminal = Arc::new(TermDevice::new());
    let id = terminal.alloc()?;
    task.bind(terminal.clone(), ".", "#term", BindOptions::default())?;

    let program = format!("#term/{id}/program");
    task.bind_fd_from_namespace(&program, Fd::STDIN)?;
    task.bind_fd_from_namespace(&program, Fd::STDOUT)?;
    task.bind_fd_from_namespace(&program, Fd::STDERR)?;

    if let Some(bytes) = stdin_bytes {
        let mut data = terminal.open(
            &NormalizedPath::new(format!("{id}/data"))?,
            OpenOptions {
                write: true,
                ..OpenOptions::default()
            },
        )?;
        data.write(&bytes)?;
    }

    Ok((terminal, id))
}

fn finish_terminal_task_output(
    result: Result<(), CliError>,
    task: &Task,
    terminal: &TermDevice,
    terminal_id: &str,
    process_stdout: &mut dyn Write,
    process_stderr: &mut dyn Write,
) -> Result<i32, CliError> {
    drain_terminal_output(terminal, terminal_id, process_stdout)?;
    match result {
        Ok(()) => Ok(parse_exit(&task.exit())),
        Err(error) => {
            write_process_output(
                process_stderr,
                "stderr",
                format!("wanix-rust qjs-term: {error}\n").as_bytes(),
            )?;
            Ok(1)
        }
    }
}

fn drain_terminal_output(
    terminal: &TermDevice,
    terminal_id: &str,
    process_stdout: &mut dyn Write,
) -> Result<(), CliError> {
    write_process_output(
        process_stdout,
        "stdout",
        &drain_terminal_output_bytes(terminal, terminal_id)?,
    )
}

fn drain_terminal_output_bytes(
    terminal: &TermDevice,
    terminal_id: &str,
) -> Result<Vec<u8>, CliError> {
    let mut data = terminal.open(
        &NormalizedPath::new(format!("{terminal_id}/data"))?,
        OpenOptions {
            read: true,
            ..OpenOptions::default()
        },
    )?;
    let mut output = Vec::new();
    let mut buf = [0; 1024];
    loop {
        let count = data.read(&mut buf)?;
        if count == 0 {
            return Ok(output);
        }
        output.extend_from_slice(&buf[..count]);
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{
        PostEvalFeed, QJS_SHELL_SCRIPT_SENTINEL, QjsShellSession, TermResize,
        parse_qjs_shell_command, parse_qjs_term_command,
    };
    use wanix_fs::{FileSystem, FsError, NormalizedPath, OpenOptions};

    #[test]
    fn parse_qjs_term_collects_pre_script_post_eval_feeds() {
        let command = parse_qjs_term_command(&[
            "--ready-io-turns".into(),
            "2".into(),
            "--feed-after-eval".into(),
            "first".into(),
            "--feed-after-eval-file".into(),
            "second.txt".into(),
            "demo.js".into(),
            "--".into(),
            "arg".into(),
        ])
        .unwrap();

        assert_eq!(
            command.feed_after_eval,
            [
                PostEvalFeed::Bytes(b"first".to_vec()),
                PostEvalFeed::File(PathBuf::from("second.txt"))
            ]
        );
        assert_eq!(command.qjs.script_path, PathBuf::from("demo.js"));
        assert_eq!(command.qjs.args, vec!["arg".to_owned()]);
        assert_eq!(command.qjs.ready_io_turns, 2);
    }

    #[test]
    fn parse_qjs_term_preserves_feed_option_after_script_as_argument() {
        let command = parse_qjs_term_command(&[
            "demo.js".into(),
            "--".into(),
            "--feed-after-eval".into(),
            "script-arg".into(),
        ])
        .unwrap();

        assert!(command.feed_after_eval.is_empty());
        assert_eq!(
            command.qjs.args,
            vec!["--feed-after-eval".to_owned(), "script-arg".to_owned()]
        );
    }

    #[test]
    fn parse_qjs_term_collects_process_post_eval_feed() {
        let command = parse_qjs_term_command(&[
            "--feed-after-eval-file".into(),
            "-".into(),
            "demo.js".into(),
        ])
        .unwrap();

        assert_eq!(command.feed_after_eval, [PostEvalFeed::Process]);
        assert_eq!(command.qjs.script_path, PathBuf::from("demo.js"));
    }

    #[test]
    fn parse_qjs_term_collects_line_segmented_post_eval_feed() {
        let command = parse_qjs_term_command(&[
            "--feed-after-eval-lines".into(),
            "session.txt".into(),
            "--feed-after-eval-lines".into(),
            "-".into(),
            "demo.js".into(),
        ])
        .unwrap();

        assert_eq!(
            command.feed_after_eval,
            [
                PostEvalFeed::LinesFile(PathBuf::from("session.txt")),
                PostEvalFeed::LinesProcess
            ]
        );
        assert_eq!(command.qjs.script_path, PathBuf::from("demo.js"));
    }

    #[test]
    fn parse_qjs_term_collects_post_eval_resize_feed() {
        let command = parse_qjs_term_command(&[
            "--resize-after-eval".into(),
            "100x40".into(),
            "demo.js".into(),
        ])
        .unwrap();

        assert_eq!(
            command.feed_after_eval,
            [PostEvalFeed::Resize(TermResize {
                columns: 100,
                rows: 40
            })]
        );
        assert_eq!(command.qjs.script_path, PathBuf::from("demo.js"));
    }

    #[test]
    fn parse_qjs_term_rejects_invalid_post_eval_resize_feed() {
        let error = parse_qjs_term_command(&[
            "--resize-after-eval".into(),
            "100by40".into(),
            "demo.js".into(),
        ])
        .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("qjs-term --resize-after-eval expects COLSxROWS")
        );
    }

    #[test]
    fn parse_qjs_shell_uses_bundled_script_sentinel_without_script_args() {
        let command = parse_qjs_shell_command(&[
            "--raw".into(),
            "--cwd".into(),
            "app".into(),
            "--ready-io-turns".into(),
            "2".into(),
        ])
        .unwrap();

        assert_eq!(
            command.qjs.script_path,
            PathBuf::from(QJS_SHELL_SCRIPT_SENTINEL)
        );
        assert_eq!(command.qjs.cwd.as_str(), "app");
        assert_eq!(command.qjs.ready_io_turns, 2);
        assert!(command.qjs.args.is_empty());
        assert!(command.qjs.stdin.is_none());
        assert!(command.raw);
    }

    #[test]
    fn parse_qjs_shell_rejects_script_path_and_preloaded_stdin() {
        let script_error = parse_qjs_shell_command(&["demo.js".into()]).unwrap_err();
        assert!(
            script_error
                .to_string()
                .contains("qjs-shell does not accept a script path")
        );

        let stdin_error =
            parse_qjs_shell_command(&["--stdin".into(), "preloaded".into()]).unwrap_err();
        assert!(
            stdin_error
                .to_string()
                .contains("qjs-shell reads native stdin as terminal input")
        );
    }

    #[test]
    fn qjs_shell_session_runs_bundled_shell_from_host_root() {
        let root = temp_dir("wanix-qjs-shell-session");
        fs::write(root.join("visible.txt"), "served root").unwrap();
        let (mut session, initial_output) = QjsShellSession::start(&root).unwrap();

        assert_eq!(initial_output, b"shell task: 1\r\n$ ");
        let output = session.input(b"echo hello ws\nexit\n").unwrap();

        assert_eq!(output, b"echo hello ws\r\nhello ws\r\n$ exit\r\nbye\r\n");
        assert!(session.is_finished());
    }

    #[test]
    fn qjs_shell_session_pumps_delayed_output_without_input() {
        let root = temp_dir("wanix-qjs-shell-session-pump");
        let (mut session, initial_output) = QjsShellSession::start(&root).unwrap();

        assert_eq!(initial_output, b"shell task: 1\r\n$ ");
        let output = session.input(b"later tick\n").unwrap();
        assert_eq!(output, b"later tick\r\nscheduled\r\n");

        let output = session.pump().unwrap();
        assert_eq!(output, b"later: tick\r\n$ ");
        assert!(!session.is_finished());

        let output = session.input(b"exit\n").unwrap();
        assert_eq!(output, b"exit\r\nbye\r\n");
        assert!(session.is_finished());
    }

    #[test]
    fn qjs_shell_session_closes_owned_terminal_on_drop() {
        let root = temp_dir("wanix-qjs-shell-session-drop");
        let (session, _initial_output) = QjsShellSession::start(&root).unwrap();
        let terminal = session.terminal_for_test();
        let terminal_id = session.terminal_id_for_test().to_owned();

        drop(session);

        let result = terminal.open(
            &NormalizedPath::new(format!("{terminal_id}/data")).unwrap(),
            OpenOptions {
                read: true,
                ..OpenOptions::default()
            },
        );
        assert!(matches!(result, Err(FsError::NotFound)));
    }

    #[test]
    fn qjs_shell_session_close_releases_terminal_resource() {
        let root = temp_dir("wanix-qjs-shell-session-close");
        let (mut session, _initial_output) = QjsShellSession::start(&root).unwrap();
        let terminal = session.terminal_for_test();
        let terminal_id = session.terminal_id_for_test().to_owned();

        assert!(
            terminal
                .metadata(&NormalizedPath::new(format!("{terminal_id}/id")).unwrap())
                .is_ok()
        );

        session.close_terminal_resource().unwrap();
        session.close_terminal_resource().unwrap();

        assert!(matches!(
            terminal.metadata(&NormalizedPath::new(format!("{terminal_id}/id")).unwrap()),
            Err(FsError::NotFound)
        ));
    }

    #[test]
    fn qjs_shell_session_close_tolerates_external_terminal_close() {
        let root = temp_dir("wanix-qjs-shell-session-external-close");
        let (mut session, _initial_output) = QjsShellSession::start(&root).unwrap();
        let terminal = session.terminal_for_test();
        let terminal_id = session.terminal_id_for_test().to_owned();

        terminal.close(&terminal_id).unwrap();

        session.close_terminal_resource().unwrap();
        session.close_terminal_resource().unwrap();
    }

    #[test]
    fn qjs_shell_session_can_start_in_served_cwd() {
        let root = temp_dir("wanix-qjs-shell-session-cwd");
        fs::create_dir(root.join("app")).unwrap();
        let cwd = NormalizedPath::new("app").unwrap();
        let (mut session, initial_output) = QjsShellSession::start_in_cwd(&root, &cwd).unwrap();

        assert_eq!(initial_output, b"shell task: 1\r\n$ ");
        assert!(session.resize(100, 40).unwrap().is_empty());
        let output = session.input(b"pwd\nsize\nexit\n").unwrap();

        assert_eq!(
            output,
            b"pwd\r\napp\r\n$ size\r\nsize 100 40\r\n$ exit\r\nbye\r\n"
        );
        assert!(session.is_finished());
    }

    #[test]
    fn qjs_shell_session_starts_child_qjs_task() {
        let root = temp_dir("wanix-qjs-shell-session-child");
        fs::write(root.join("input.txt"), "redirected stdin\n").unwrap();
        fs::write(
            root.join("foreground-child.js"),
            r##"import * as std from "qjs:std";
import * as os from "qjs:os";

const bytes = new Uint8Array(64);
const count = os.read(0, bytes.buffer, 0, bytes.length);
if (count < 0) {
  throw new Error("stdin read failed: " + count);
}
const text = Array.from(bytes.slice(0, count)).map((byte) => String.fromCharCode(byte)).join("");
std.out.puts("foreground child stdin " + text.trimEnd() + "\n");
std.exit(0);
"##,
        )
        .unwrap();
        fs::write(
            root.join("terminal-child.js"),
            r#"import * as std from "qjs:std";

std.out.puts("terminal child stdout\n");
std.err.puts("terminal child stderr\n");
std.exit(0);
"#,
        )
        .unwrap();
        fs::write(
            root.join("child.js"),
            r##"import * as std from "qjs:std";
import * as os from "qjs:os";

function readStdin() {
  const bytes = new Uint8Array(64);
  const count = os.read(0, bytes.buffer, 0, bytes.length);
  if (count < 0) {
    throw new Error("stdin read failed: " + count);
  }
  return Array.from(bytes.slice(0, count)).map((byte) => String.fromCharCode(byte)).join("");
}

std.out.puts("child task " + std.loadFile("#task/self/id").trim() + "\n");
std.out.puts("child cwd " + std.loadFile("#task/self/dir").trim() + "\n");
std.out.puts("child argv " + scriptArgs.join("|") + "\n");
std.out.puts("child stdin " + readStdin().trimEnd() + "\n");
std.out.puts("child raw " + std.getenv("WANIX_QJS_SHELL_RAW") + "\n");
std.out.puts("child mode " + std.getenv("MODE") + "\n");
std.out.flush();
std.err.puts("child stderr " + scriptArgs[1] + "\n");
std.err.flush();
std.exit(7);
"##,
        )
        .unwrap();
        let (mut session, initial_output) = QjsShellSession::start(&root).unwrap();

        assert_eq!(initial_output, b"shell task: 1\r\n$ ");
        let output = session
            .input(b"qjs foreground-child.js\nforeground stdin\n")
            .unwrap();
        assert_eq!(
            output,
            b"qjs foreground-child.js\r\nforeground child stdin foreground stdin\r\n$ "
        );
        let output = session
            .input(
                b"qjs terminal-child.js\nenv\nsetenv MODE throwaway\nenv MODE\nunsetenv MODE\nenv MODE\nsetenv MODE shell-mode\nenv MODE\nqjs child.js alpha 'two words' < input.txt > child-out.txt 2> child-err.txt\nstatus\nps\ncat child-out.txt\ncat child-err.txt\nexit\n",
            )
            .unwrap();

        assert_eq!(
            output,
            b"qjs terminal-child.js\r\nterminal child stdout\r\nterminal child stderr\r\n$ env\r\nWANIX_QJS_SHELL_RAW=1\r\n$ setenv MODE throwaway\r\n$ env MODE\r\nMODE=throwaway\r\n$ unsetenv MODE\r\n$ env MODE\r\n$ setenv MODE shell-mode\r\n$ env MODE\r\nMODE=shell-mode\r\n$ qjs child.js alpha 'two words' < input.txt > child-out.txt 2> child-err.txt\r\nqjs exit 7\r\n$ status\r\nstatus 7\r\n$ ps\r\nid kind exit dir cmd\r\n1 qjs - . __wanix_qjs_shell.js\r\n2 qjs 0 . foreground-child.js\r\n3 qjs 0 . terminal-child.js\r\n4 qjs 7 . child.js alpha 'two words'\r\n$ cat child-out.txt\r\nchild task 4\r\nchild cwd .\r\nchild argv child.js|alpha|two words\r\nchild stdin redirected stdin\r\nchild raw 1\r\nchild mode shell-mode\r\n$ cat child-err.txt\r\nchild stderr alpha\r\n$ exit\r\nbye\r\n"
        );
        assert!(session.is_finished());
    }

    #[test]
    fn qjs_shell_session_navigates_and_edits_served_files() {
        let root = temp_dir("wanix-qjs-shell-session-files");
        fs::write(root.join("visible.txt"), "served root\n").unwrap();
        fs::create_dir(root.join("app")).unwrap();
        fs::write(root.join("app").join("note.txt"), "from app\n").unwrap();
        let (mut session, initial_output) = QjsShellSession::start(&root).unwrap();

        assert_eq!(initial_output, b"shell task: 1\r\n$ ");
        let output = session
            .input(
                b"ls\ncat visible.txt\ncd app\npwd\nls\ncat note.txt\nwrite made.txt made by shell\ncat made.txt\nmkdir docs\nwrite docs/readme.txt copied note\ncp docs/readme.txt copy.txt\ncat copy.txt\nmv copy.txt moved.txt\ncat moved.txt\nrm moved.txt\ncat moved.txt\nrmdir docs\nrm docs/readme.txt\nrmdir docs\nls\ncat missing.txt\ncd ..\npwd\nexit\n",
            )
            .unwrap();

        assert_eq!(
            output,
            b"ls\r\napp visible.txt\r\n$ cat visible.txt\r\nserved root\r\n$ cd app\r\n$ pwd\r\napp\r\n$ ls\r\nnote.txt\r\n$ cat note.txt\r\nfrom app\r\n$ write made.txt made by shell\r\nwrote made.txt\r\n$ cat made.txt\r\nmade by shell\r\n$ mkdir docs\r\n$ write docs/readme.txt copied note\r\nwrote docs/readme.txt\r\n$ cp docs/readme.txt copy.txt\r\n$ cat copy.txt\r\ncopied note\r\n$ mv copy.txt moved.txt\r\n$ cat moved.txt\r\ncopied note\r\n$ rm moved.txt\r\n$ cat moved.txt\r\ncat: moved.txt: not found\r\n$ rmdir docs\r\nrmdir: docs: directory not empty\r\n$ rm docs/readme.txt\r\n$ rmdir docs\r\n$ ls\r\nmade.txt note.txt\r\n$ cat missing.txt\r\ncat: missing.txt: not found\r\n$ cd ..\r\n$ pwd\r\n.\r\n$ exit\r\nbye\r\n"
        );
        assert_eq!(
            fs::read_to_string(root.join("app").join("made.txt")).unwrap(),
            "made by shell\n"
        );
        assert!(!root.join("app").join("docs").exists());
        assert!(!root.join("app").join("moved.txt").exists());
        assert!(session.is_finished());
    }

    #[cfg(unix)]
    #[test]
    fn qjs_shell_session_reports_stat_metadata() {
        use std::os::unix::fs::symlink;

        let root = temp_dir("wanix-qjs-shell-session-stat");
        fs::write(root.join("visible.txt"), "served root\n").unwrap();
        fs::create_dir(root.join("app")).unwrap();
        symlink("visible.txt", root.join("link.txt")).unwrap();
        let (mut session, initial_output) = QjsShellSession::start(&root).unwrap();

        assert_eq!(initial_output, b"shell task: 1\r\n$ ");
        let output = session
            .input(
                b"stat visible.txt\nstat app\nlstat link.txt\nstat link.txt\nstat missing.txt\nexit\n",
            )
            .unwrap();
        let output = String::from_utf8(output).unwrap();

        assert!(
            output.contains("visible.txt type file mode 100"),
            "{output}"
        );
        assert!(
            output.contains("visible.txt type file") && output.contains("size 12"),
            "{output}"
        );
        assert!(output.contains("app type dir mode 40"), "{output}");
        assert!(
            output.contains("link.txt type symlink mode 120"),
            "{output}"
        );
        assert!(output.contains("link.txt type file mode 100"), "{output}");
        assert!(output.contains("stat: missing.txt: errno "), "{output}");
        assert!(output.ends_with("$ exit\r\nbye\r\n"), "{output}");
        assert!(session.is_finished());
    }

    #[test]
    fn qjs_shell_session_reports_served_resize() {
        let root = temp_dir("wanix-qjs-shell-session-resize");
        let (mut session, initial_output) = QjsShellSession::start(&root).unwrap();

        assert_eq!(initial_output, b"shell task: 1\r\n$ ");
        assert!(session.resize(100, 40).unwrap().is_empty());
        let output = session.input(b"size\nexit\n").unwrap();

        assert_eq!(output, b"size\r\nsize 100 40\r\n$ exit\r\nbye\r\n");
        assert!(session.is_finished());
    }

    fn temp_dir(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("{name}-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&path).unwrap();
        path
    }
}
