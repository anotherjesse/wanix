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
    guest_path_in_cwd, parse_exit, quickjs_runner, read_file, read_qjs_stdin, read_utf8_script,
};

pub(super) fn run_qjs_term(
    command: QjsCommand,
    process_stdin: &mut dyn Read,
) -> Result<CliOutput, CliError> {
    let script_path = command.script_path.as_path();
    let script = read_utf8_script(script_path)?;
    let stdin_bytes = read_qjs_stdin(command.stdin, process_stdin)?;

    let runner = quickjs_runner()?;
    let table = TaskTable::new();
    let mut driver = QuickJsTaskDriver::new(Arc::clone(&runner))
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

    let root = Arc::new(MemFs::new());
    copy_script_directory(script_path, &root, &command.cwd)?;
    let guest_script = guest_path_in_cwd(&command.cwd, QJS_GUEST_SCRIPT)?;
    root.write_file(guest_script.as_str(), script.as_bytes())?;
    task.bind(root, ".", ".", BindOptions::default())?;
    bind_host_mounts(&task, &command.mounts)?;

    let (terminal, terminal_id) = attach_task_terminal(&task, stdin_bytes)?;
    configure_qjs_task(
        &task,
        QJS_GUEST_SCRIPT,
        &command.args,
        &command.env,
        &command.cwd,
    )?;

    let start_result = (|| -> Result<(), CliError> {
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
        runtime.finish()?;
        Ok(())
    })();

    finish_terminal_task_output(start_result, &task, &terminal, &terminal_id)
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
