use std::ffi::OsString;
use std::io::Read;
use std::sync::Arc;

use wanix_fs::{FileSystem, MemFs, NormalizedPath, OpenOptions};
use wanix_qjs::QuickJsTaskDriver;
use wanix_task::{Fd, Task, TaskTable};
use wanix_term::TermDevice;
use wanix_vfs::BindOptions;

use super::{
    CliError, CliOutput, QJS_GUEST_SCRIPT, QjsCommand, apply_qjs_task_runtime_limits,
    bind_host_mounts, configure_qjs_task, copy_script_directory, eval_qjs_source,
    guest_path_in_cwd, os_arg_to_string, parse_exit, parse_qjs_command_for, quickjs_runner,
    read_file, read_qjs_stdin, read_utf8_script,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct QjsTermCommand {
    qjs: QjsCommand,
    feed_after_eval: Vec<Vec<u8>>,
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
            feed_after_eval
                .push(os_arg_to_string(value, "qjs-term --feed-after-eval")?.into_bytes());
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

pub(super) fn run_qjs_term(
    command: QjsTermCommand,
    process_stdin: &mut dyn Read,
) -> Result<CliOutput, CliError> {
    let qjs_command = command.qjs;
    let feed_after_eval = command.feed_after_eval;
    let script_path = qjs_command.script_path.as_path();
    let script = read_utf8_script(script_path)?;
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
    copy_script_directory(script_path, &root, &qjs_command.cwd)?;
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
        eval_qjs_source(
            &mut runtime,
            &script,
            &guest_script,
            qjs_command.event_loop_wait_budget,
            eval_ready_io_turns,
        )?;
        if !feed_after_eval.is_empty() {
            feed_terminal_after_eval(&terminal, &terminal_id, &feed_after_eval)?;
            runtime.run_ready_io_turns(qjs_command.ready_io_turns)?;
        }
        runtime.finish()?;
        Ok(())
    })();

    finish_terminal_task_output(start_result, &task, &terminal, &terminal_id)
}

fn feed_terminal_after_eval(
    terminal: &TermDevice,
    terminal_id: &str,
    chunks: &[Vec<u8>],
) -> Result<(), CliError> {
    if chunks.is_empty() {
        return Ok(());
    }
    let mut data = terminal.open(
        &NormalizedPath::new(format!("{terminal_id}/data"))?,
        OpenOptions {
            write: true,
            ..OpenOptions::default()
        },
    )?;
    for chunk in chunks {
        data.write(chunk)?;
    }
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
) -> Result<CliOutput, CliError> {
    let stdout = read_file(terminal, &format!("{terminal_id}/data"))?;
    match result {
        Ok(()) => Ok(CliOutput::new(stdout, Vec::new(), parse_exit(&task.exit()))),
        Err(error) => Ok(CliOutput::new(
            stdout,
            format!("wanix-rust qjs-term: {error}\n").into_bytes(),
            1,
        )),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::parse_qjs_term_command;

    #[test]
    fn parse_qjs_term_collects_pre_script_post_eval_feeds() {
        let command = parse_qjs_term_command(&[
            "--ready-io-turns".into(),
            "2".into(),
            "--feed-after-eval".into(),
            "first".into(),
            "--feed-after-eval".into(),
            "second".into(),
            "demo.js".into(),
            "--".into(),
            "arg".into(),
        ])
        .unwrap();

        assert_eq!(
            command.feed_after_eval,
            [b"first".to_vec(), b"second".to_vec()]
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
}
