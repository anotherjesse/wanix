use wanix_fs::{DirEntry, File, FileSeekFrom, FileType, FsError, FsResult, Metadata};

use crate::cmd::parse_cmd_argv;
use crate::{Fd, Task, TaskId, TaskTable};

#[derive(Debug, Clone, Copy)]
pub(crate) struct FileAccess {
    read: bool,
    write: bool,
}

impl FileAccess {
    pub(crate) fn new(read: bool, write: bool) -> Self {
        Self { read, write }
    }

    pub(crate) fn read_only() -> Self {
        Self::new(true, false)
    }

    pub(crate) fn can_read(self) -> FsResult<()> {
        if self.read {
            Ok(())
        } else {
            Err(FsError::PermissionDenied)
        }
    }

    pub(crate) fn can_write(self) -> FsResult<()> {
        if self.write {
            Ok(())
        } else {
            Err(FsError::PermissionDenied)
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum Field {
    Id,
    Kind,
    Cmd,
    Env,
    Dir,
    Exit,
}

#[derive(Debug)]
pub(crate) struct FieldFile {
    task: Task,
    field: Field,
    access: FileAccess,
    read_data: Vec<u8>,
    write_data: Vec<u8>,
    offset: usize,
}

impl FieldFile {
    pub(crate) fn new(task: Task, field: Field, access: FileAccess) -> Self {
        let read_data = field_text(&task, field).into_bytes();
        Self {
            task,
            field,
            access,
            read_data,
            write_data: Vec::new(),
            offset: 0,
        }
    }
}

impl File for FieldFile {
    fn read(&mut self, buf: &mut [u8]) -> FsResult<usize> {
        self.access.can_read()?;
        read_from_slice(&self.read_data, &mut self.offset, buf)
    }

    fn write(&mut self, buf: &[u8]) -> FsResult<usize> {
        self.access.can_write()?;
        self.write_data.extend_from_slice(buf);
        let text = String::from_utf8_lossy(&self.write_data).trim().to_owned();
        match self.field {
            Field::Id | Field::Kind => Err(FsError::PermissionDenied),
            Field::Cmd => {
                self.task.set_cmd(text)?;
                Ok(buf.len())
            }
            Field::Env => {
                self.task.set_env_lines(text)?;
                Ok(buf.len())
            }
            Field::Dir => {
                self.task.set_dir(text)?;
                Ok(buf.len())
            }
            Field::Exit => {
                self.task.set_exit(text)?;
                Ok(buf.len())
            }
        }
    }

    fn metadata(&self) -> FsResult<Metadata> {
        Ok(file_metadata(
            self.read_data.len() as u64,
            field_mode(self.field),
        ))
    }

    fn seek(&mut self, from: FileSeekFrom) -> FsResult<u64> {
        self.access.can_read()?;
        self.offset = seek_offset(self.offset, self.read_data.len(), from)?;
        Ok(self.offset as u64)
    }

    fn tell(&self) -> FsResult<u64> {
        self.access.can_read()?;
        Ok(self.offset as u64)
    }

    fn is_seekable(&self) -> bool {
        self.access.read
    }
}

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
        Ok(file_metadata(0, 0o755))
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

#[derive(Debug)]
pub(crate) struct NewTaskFile {
    table: TaskTable,
    parent: Option<TaskId>,
    kind: String,
    data: Option<Vec<u8>>,
    offset: usize,
}

impl NewTaskFile {
    pub(crate) fn new(table: TaskTable, parent: Option<TaskId>, kind: impl Into<String>) -> Self {
        Self {
            table,
            parent,
            kind: kind.into(),
            data: None,
            offset: 0,
        }
    }
}

impl File for NewTaskFile {
    fn read(&mut self, buf: &mut [u8]) -> FsResult<usize> {
        if self.data.is_none() {
            let task = match self.parent {
                Some(parent) => self.table.allocate_child(&self.kind, parent)?,
                None => self.table.allocate_root(&self.kind)?,
            };
            self.data = Some(format!("{}\n", task.id().get()).into_bytes());
        }
        read_from_slice(
            self.data.as_ref().expect("allocation populated data"),
            &mut self.offset,
            buf,
        )
    }

    fn write(&mut self, _buf: &[u8]) -> FsResult<usize> {
        Err(FsError::PermissionDenied)
    }

    fn metadata(&self) -> FsResult<Metadata> {
        Ok(file_metadata(0, 0o555))
    }

    fn seek(&mut self, from: FileSeekFrom) -> FsResult<u64> {
        let len = self.data.as_ref().map_or(0, Vec::len);
        self.offset = seek_offset(self.offset, len, from)?;
        Ok(self.offset as u64)
    }

    fn tell(&self) -> FsResult<u64> {
        Ok(self.offset as u64)
    }

    fn is_seekable(&self) -> bool {
        true
    }
}

#[derive(Debug)]
pub(crate) struct FdProxyFile {
    task: Task,
    fd: Fd,
    access: FileAccess,
}

impl FdProxyFile {
    pub(crate) fn new(task: Task, fd: Fd, access: FileAccess) -> Self {
        Self { task, fd, access }
    }
}

impl File for FdProxyFile {
    fn read(&mut self, buf: &mut [u8]) -> FsResult<usize> {
        self.access.can_read()?;
        self.task.read_fd(self.fd, buf)
    }

    fn write(&mut self, buf: &[u8]) -> FsResult<usize> {
        self.access.can_write()?;
        self.task.write_fd(self.fd, buf)
    }

    fn metadata(&self) -> FsResult<Metadata> {
        self.task.fd_metadata(self.fd)
    }
}

pub(crate) fn field_text(task: &Task, field: Field) -> String {
    let value = match field {
        Field::Id => task.id().get().to_string(),
        Field::Kind => task.kind(),
        Field::Cmd => task.cmd(),
        Field::Env => task.env().join("\n"),
        Field::Dir => task.dir().to_string(),
        Field::Exit => task.exit(),
    };
    if value.ends_with('\n') {
        value
    } else {
        format!("{value}\n")
    }
}

pub(crate) fn field_metadata(task: &Task, field: &str) -> FsResult<Metadata> {
    let field = match field {
        "ctl" => return Ok(file_metadata(0, 0o755)),
        "id" => Field::Id,
        "kind" => Field::Kind,
        "cmd" => Field::Cmd,
        "env" => Field::Env,
        "dir" => Field::Dir,
        "exit" => Field::Exit,
        _ => return Err(FsError::NotFound),
    };
    Ok(file_metadata(
        field_text(task, field).len() as u64,
        field_mode(field),
    ))
}

pub(crate) fn task_entries() -> Vec<DirEntry> {
    [
        ("cmd", file_metadata(0, 0o755)),
        ("ctl", file_metadata(0, 0o755)),
        ("dir", file_metadata(2, 0o755)),
        ("env", file_metadata(1, 0o755)),
        ("exit", file_metadata(1, 0o755)),
        ("fd", directory_metadata()),
        ("id", file_metadata(2, 0o555)),
        ("kind", file_metadata(0, 0o555)),
    ]
    .into_iter()
    .map(|(name, metadata)| DirEntry::new(name, metadata))
    .collect()
}

pub(crate) fn directory_metadata() -> Metadata {
    Metadata::new(FileType::Directory, 2, 0o755)
}

pub(crate) fn file_metadata(len: u64, mode: u32) -> Metadata {
    Metadata::new(FileType::File, len, mode)
}

fn field_mode(field: Field) -> u32 {
    match field {
        Field::Id | Field::Kind => 0o555,
        Field::Cmd | Field::Env | Field::Dir | Field::Exit => 0o755,
    }
}

fn read_from_slice(data: &[u8], offset: &mut usize, buf: &mut [u8]) -> FsResult<usize> {
    let available = data.len().saturating_sub(*offset);
    let count = available.min(buf.len());
    buf[..count].copy_from_slice(&data[*offset..*offset + count]);
    *offset += count;
    Ok(count)
}

fn seek_offset(current: usize, len: usize, from: FileSeekFrom) -> FsResult<usize> {
    let base = match from {
        FileSeekFrom::Start(offset) => {
            return usize::try_from(offset).map_err(|_| FsError::InvalidOffset);
        }
        FileSeekFrom::Current(_) => current as i128,
        FileSeekFrom::End(_) => len as i128,
    };
    let delta = match from {
        FileSeekFrom::Start(_) => 0,
        FileSeekFrom::Current(offset) | FileSeekFrom::End(offset) => offset as i128,
    };
    let next = base + delta;
    if next < 0 || next > usize::MAX as i128 {
        return Err(FsError::InvalidOffset);
    }
    Ok(next as usize)
}
