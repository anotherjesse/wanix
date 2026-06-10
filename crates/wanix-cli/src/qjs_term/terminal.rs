use std::io::Write;
use std::sync::Arc;

use wanix_fs::{FileSystem, NormalizedPath, OpenOptions};
use wanix_task::{Fd, Task};
use wanix_term::TermDevice;
use wanix_vfs::BindOptions;

use super::pump::drain_terminal_output;
use crate::{CliError, parse_exit, write_process_output};

pub(crate) struct AttachedTerminal {
    pub(crate) device: Arc<TermDevice>,
    pub(crate) id: String,
}

pub(crate) fn attach_task_terminal(
    task: &Task,
    stdin_bytes: Option<Vec<u8>>,
) -> Result<AttachedTerminal, CliError> {
    let (terminal, id) = bind_terminal_device(task)?;
    bind_terminal_program_fds(task, &id)?;
    preload_terminal_stdin(&terminal, &id, stdin_bytes)?;
    Ok(AttachedTerminal {
        device: terminal,
        id,
    })
}

fn bind_terminal_device(task: &Task) -> Result<(Arc<TermDevice>, String), CliError> {
    let terminal = Arc::new(TermDevice::new());
    let id = terminal.alloc()?;
    task.bind(terminal.clone(), ".", "#term", BindOptions::default())?;
    Ok((terminal, id))
}

fn bind_terminal_program_fds(task: &Task, terminal_id: &str) -> Result<(), CliError> {
    let program = format!("#term/{terminal_id}/program");
    task.bind_fd_from_namespace(&program, Fd::STDIN)?;
    task.bind_fd_from_namespace(&program, Fd::STDOUT)?;
    task.bind_fd_from_namespace(&program, Fd::STDERR)?;
    Ok(())
}

fn preload_terminal_stdin(
    terminal: &TermDevice,
    terminal_id: &str,
    stdin_bytes: Option<Vec<u8>>,
) -> Result<(), CliError> {
    let Some(bytes) = stdin_bytes else {
        return Ok(());
    };
    let mut data = terminal.open(
        &NormalizedPath::new(format!("{terminal_id}/data"))?,
        OpenOptions {
            write: true,
            ..OpenOptions::default()
        },
    )?;
    data.write(&bytes)?;
    Ok(())
}

pub(super) fn finish_terminal_task_output(
    result: Result<(), CliError>,
    task: &Task,
    terminal: &AttachedTerminal,
    process_stdout: &mut dyn Write,
    process_stderr: &mut dyn Write,
) -> Result<i32, CliError> {
    drain_terminal_output(&terminal.device, &terminal.id, process_stdout)?;
    match result {
        Ok(()) => Ok(parse_exit(&task.exit())),
        Err(error) => {
            write_process_output(
                process_stderr,
                "stderr",
                format!("wanix qjs-term: {error}\n").as_bytes(),
            )?;
            Ok(1)
        }
    }
}
