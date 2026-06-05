use wanix_fs::{File, FsError, FsResult, Metadata};

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
            ControlCommand::Bind { source, fd } => {
                self.task.bind_fd_from_namespace(source, fd)?;
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
    Bind { source: String, fd: Fd },
}

fn parse_control_command(task: &Task, command: &str) -> FsResult<ControlCommand> {
    if command.is_empty() {
        return Ok(ControlCommand::Pending);
    }
    if "start".starts_with(command) {
        return Ok(if command == "start" {
            ControlCommand::Start
        } else {
            ControlCommand::Pending
        });
    }

    let parts = match parse_cmd_argv(command) {
        Ok(Some(parts)) => parts,
        Ok(None) => return Ok(ControlCommand::Pending),
        Err(FsError::Other(message))
            if message.starts_with("unterminated ") && command_may_be_bind(command) =>
        {
            return Ok(ControlCommand::Pending);
        }
        Err(err) => return Err(err),
    };
    if parts.is_empty() || ("bind".starts_with(parts[0].as_str()) && parts.len() < 3) {
        return Ok(ControlCommand::Pending);
    }
    if let [command, source, destination] = parts.as_slice()
        && command == "bind"
    {
        return Ok(ControlCommand::Bind {
            source: source.clone(),
            fd: control_fd_destination(task, destination)?,
        });
    }
    Err(FsError::NotSupported)
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
