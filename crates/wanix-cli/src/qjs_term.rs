use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

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

#[derive(Debug, Clone, PartialEq, Eq)]
struct TermResize {
    columns: u16,
    rows: u16,
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

fn qjs_shell_command(mut command: QjsCommand, raw: bool) -> QjsCommand {
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
    let guest_script = guest_path_in_cwd(&qjs_command.cwd, QJS_GUEST_SCRIPT)?;
    root.write_file(guest_script.as_str(), script.as_bytes())?;
    task.bind(root, ".", ".", BindOptions::default())?;
    bind_host_mounts(&task, &qjs_command.mounts)?;

    let (terminal, terminal_id) = attach_task_terminal(&task, stdin_bytes)?;
    configure_qjs_task(
        &task,
        QJS_GUEST_SCRIPT,
        &qjs_command.args,
        &qjs_command.env,
        &qjs_command.cwd,
    )?;

    let start_result = (|| -> Result<(), CliError> {
        let mut runtime = runner.create_task_runtime(&task)?;
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
                qjs_command.ready_io_turns,
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
    ready_io_turns: usize,
    process_stdout: &mut dyn Write,
) -> Result<(), CliError> {
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
                    ready_io_turns,
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
                        ready_io_turns,
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
                    ready_io_turns,
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
                    ready_io_turns,
                    process_stdout,
                )?;
            }
            PostEvalFeed::RawBytesProcess => {
                flush_terminal_feed_batch(
                    terminal,
                    terminal_id,
                    runtime,
                    &mut current_batch,
                    ready_io_turns,
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
                    ready_io_turns,
                    process_stdout,
                )?;
            }
            PostEvalFeed::Resize(resize) => {
                flush_terminal_feed_batch(
                    terminal,
                    terminal_id,
                    runtime,
                    &mut current_batch,
                    ready_io_turns,
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
                    ready_io_turns,
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
        ready_io_turns,
        process_stdout,
    )
}

fn run_process_raw_byte_feed_session_after_eval(
    process_stdin: &mut dyn Read,
    terminal: &TermDevice,
    terminal_id: &str,
    runtime: &mut QuickJsTaskRuntime,
    ready_io_turns: usize,
    process_stdout: &mut dyn Write,
) -> Result<(), CliError> {
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
            ready_io_turns,
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
    ready_io_turns: usize,
    process_stdout: &mut dyn Write,
) -> Result<(), CliError> {
    let mut line = Vec::new();
    while read_process_line_after_eval(process_stdin, &mut line)? {
        feed_terminal_batch_and_pump(
            terminal,
            terminal_id,
            runtime,
            &[line.clone()],
            ready_io_turns,
            process_stdout,
        )?;
        if task_exited(runtime)? {
            break;
        }
    }
    Ok(())
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
        process_stdout,
    )
}

fn feed_terminal_batch_and_pump(
    terminal: &TermDevice,
    terminal_id: &str,
    runtime: &mut QuickJsTaskRuntime,
    batch: &[Vec<u8>],
    ready_io_turns: usize,
    process_stdout: &mut dyn Write,
) -> Result<(), CliError> {
    let result = (|| -> Result<(), CliError> {
        for chunk in batch {
            feed_terminal_after_eval(terminal, terminal_id, chunk)?;
        }
        runtime.run_ready_io_turns(ready_io_turns)?;
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
    process_stdout: &mut dyn Write,
) -> Result<(), CliError> {
    let result = (|| -> Result<(), CliError> {
        feed_terminal_after_eval(terminal, terminal_id, chunk)?;
        runtime.run_ready_io_turns(ready_io_turns)?;
        Ok(())
    })();
    drain_terminal_output(terminal, terminal_id, process_stdout)?;
    result
}

fn feed_terminal_resize_and_pump(
    terminal: &TermDevice,
    terminal_id: &str,
    runtime: &mut QuickJsTaskRuntime,
    resize: &TermResize,
    ready_io_turns: usize,
    process_stdout: &mut dyn Write,
) -> Result<(), CliError> {
    let result = (|| -> Result<(), CliError> {
        feed_terminal_resize_after_eval(terminal, terminal_id, resize)?;
        runtime.run_ready_io_turns(ready_io_turns)?;
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
    let mut data = terminal.open(
        &NormalizedPath::new(format!("{terminal_id}/data"))?,
        OpenOptions {
            read: true,
            ..OpenOptions::default()
        },
    )?;
    let mut buf = [0; 1024];
    loop {
        let count = data.read(&mut buf)?;
        if count == 0 {
            return Ok(());
        }
        write_process_output(process_stdout, "stdout", &buf[..count])?;
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{
        PostEvalFeed, QJS_SHELL_SCRIPT_SENTINEL, TermResize, parse_qjs_shell_command,
        parse_qjs_term_command,
    };

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
}
