use wanix_fs::{File, FsError, FsResult, Metadata, OpenOptions};

use super::{FileAccess, TASK_FILE_READ_WRITE_MODE, file_metadata};
use crate::cmd::parse_cmd_argv;
use crate::{Fd, Task, TaskTable};

#[derive(Debug)]
pub(crate) struct ControlFile {
    table: TaskTable,
    task: Task,
    access: FileAccess,
    data: Vec<u8>,
}

impl ControlFile {
    pub(crate) fn new(table: TaskTable, task: Task, access: FileAccess) -> Self {
        Self {
            table,
            task,
            access,
            data: Vec::new(),
        }
    }
}

impl File for ControlFile {
    fn read(&mut self, _buf: &mut [u8]) -> FsResult<usize> {
        self.access.can_read()?;
        Ok(0)
    }

    fn write(&mut self, buf: &[u8]) -> FsResult<usize> {
        self.access.can_write()?;
        self.data.extend_from_slice(buf);
        let command = String::from_utf8_lossy(&self.data).trim().to_owned();
        match parse_control_command(&self.task, &command)? {
            ControlCommand::Pending => {}
            ControlCommand::Start => {
                self.table.start(self.task.id())?;
                self.data.clear();
            }
            ControlCommand::StartDetached => {
                self.table.start_detached(self.task.id())?;
                self.data.clear();
            }
            ControlCommand::Bind {
                source,
                fd,
                options,
            } => {
                match options {
                    Some(options) => self.task.bind_fd_from_namespace_with(source, fd, options)?,
                    None => self.task.bind_fd_from_namespace(source, fd)?,
                }
                self.data.clear();
            }
        }
        Ok(buf.len())
    }

    fn metadata(&self) -> FsResult<Metadata> {
        Ok(file_metadata(0, TASK_FILE_READ_WRITE_MODE))
    }
}

enum ControlCommand {
    Pending,
    Start,
    /// `start &`: start the task on its own host thread without waiting.
    ///
    /// Must arrive in one write — a write of exactly `start` fires the
    /// synchronous start immediately (the incremental-write contract).
    StartDetached,
    Bind {
        source: String,
        fd: Fd,
        options: Option<OpenOptions>,
    },
}

fn parse_control_command(task: &Task, command: &str) -> FsResult<ControlCommand> {
    if let Some(command) = parse_start_command(command) {
        return Ok(command);
    }

    let Some(parts) = parse_control_parts(command)? else {
        return Ok(ControlCommand::Pending);
    };
    parse_complete_control_command(task, &parts)
}

fn parse_start_command(command: &str) -> Option<ControlCommand> {
    if command.is_empty() {
        return Some(ControlCommand::Pending);
    }
    if !"start".starts_with(command) {
        return None;
    }
    Some(if command == "start" {
        ControlCommand::Start
    } else {
        ControlCommand::Pending
    })
}

fn parse_control_parts(command: &str) -> FsResult<Option<Vec<String>>> {
    let parts = match parse_cmd_argv(command) {
        Ok(Some(parts)) => parts,
        Ok(None) => return Ok(None),
        Err(FsError::Other(message))
            if message.starts_with("unterminated ") && command_may_be_bind(command) =>
        {
            return Ok(None);
        }
        Err(err) => return Err(err),
    };
    Ok(Some(parts))
}

fn parse_complete_control_command(task: &Task, parts: &[String]) -> FsResult<ControlCommand> {
    if control_parts_are_pending(parts) {
        return Ok(ControlCommand::Pending);
    }
    if let [command, ampersand] = parts
        && command == "start"
        && ampersand == "&"
    {
        return Ok(ControlCommand::StartDetached);
    }
    if let Some((source, destination, mode)) = bind_command_parts(parts) {
        let options = mode.map(parse_bind_options).transpose()?;
        return Ok(ControlCommand::Bind {
            source: source.to_owned(),
            fd: control_fd_destination(task, destination)?,
            options,
        });
    }
    Err(FsError::NotSupported)
}

/// Parses an optional `bind` open-mode token: `r`, `w`, or `rw`.
fn parse_bind_options(mode: &str) -> FsResult<OpenOptions> {
    match mode {
        "r" => Ok(OpenOptions::read()),
        "w" => Ok(OpenOptions {
            write: true,
            ..OpenOptions::default()
        }),
        "rw" => Ok(OpenOptions::read_write()),
        _ => Err(FsError::NotSupported),
    }
}

fn control_parts_are_pending(parts: &[String]) -> bool {
    parts.is_empty() || ("bind".starts_with(parts[0].as_str()) && parts.len() < 3)
}

fn bind_command_parts(parts: &[String]) -> Option<(&str, &str, Option<&str>)> {
    match parts {
        [command, source, destination] if command == "bind" => Some((source, destination, None)),
        [command, source, destination, mode] if command == "bind" => {
            Some((source, destination, Some(mode)))
        }
        _ => None,
    }
}

fn command_may_be_bind(command: &str) -> bool {
    command
        .split_whitespace()
        .next()
        .is_some_and(|word| "bind".starts_with(word))
}

fn control_fd_destination(task: &Task, destination: &str) -> FsResult<Fd> {
    let parts = destination.split('/').collect::<Vec<_>>();
    match parts.as_slice() {
        ["fd", fd] => parse_control_fd(fd),
        ["#task", "self", "fd", fd] => parse_control_fd(fd),
        ["#task", task_id, "fd", fd] => {
            let task_id = task_id.parse::<u64>().map_err(|_| FsError::NotSupported)?;
            if task_id == task.id().get() {
                parse_control_fd(fd)
            } else {
                Err(FsError::NotSupported)
            }
        }
        _ => Err(FsError::NotSupported),
    }
}

fn parse_control_fd(fd: &str) -> FsResult<Fd> {
    Ok(Fd::new(fd.parse().map_err(|_| FsError::InvalidFd)?))
}
