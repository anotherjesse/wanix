use wanix_fs::{File, FileSeekFrom, FsError, FsResult, Metadata};

use super::{
    FileAccess, TASK_FILE_READ_ONLY_MODE, TASK_FILE_READ_WRITE_MODE, file_metadata,
    read_from_slice, seek_offset,
};
use crate::Task;

#[derive(Debug, Clone, Copy)]
pub(crate) enum Field {
    Id,
    Kind,
    Cmd,
    Env,
    Dir,
    Exit,
}

impl Field {
    pub(crate) fn from_name(name: &str) -> FsResult<Self> {
        TASK_FIELDS
            .iter()
            .find_map(|(field_name, field)| (*field_name == name).then_some(*field))
            .ok_or(FsError::NotFound)
    }

    pub(crate) fn is_read_only(self) -> bool {
        matches!(self, Self::Id | Self::Kind)
    }

    fn text(self, task: &Task) -> String {
        let value = match self {
            Self::Id => task.id().get().to_string(),
            Self::Kind => task.kind(),
            Self::Cmd => task.cmd(),
            Self::Env => task.env().join("\n"),
            Self::Dir => task.dir().to_string(),
            Self::Exit => task.exit(),
        };
        if value.ends_with('\n') {
            value
        } else {
            format!("{value}\n")
        }
    }

    fn mode(self) -> u32 {
        match self {
            Self::Id | Self::Kind => TASK_FILE_READ_ONLY_MODE,
            Self::Cmd | Self::Env | Self::Dir | Self::Exit => TASK_FILE_READ_WRITE_MODE,
        }
    }

    fn write_text(self, task: &Task, text: String) -> FsResult<()> {
        match self {
            Self::Id | Self::Kind => Err(FsError::PermissionDenied),
            Self::Cmd => task.set_cmd(text),
            Self::Env => task.set_env_lines(text),
            Self::Dir => task.set_dir(text),
            Self::Exit => task.set_exit(text),
        }
    }
}

const TASK_FIELDS: &[(&str, Field)] = &[
    ("id", Field::Id),
    ("kind", Field::Kind),
    ("cmd", Field::Cmd),
    ("env", Field::Env),
    ("dir", Field::Dir),
    ("exit", Field::Exit),
];

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
        let read_data = field.text(&task).into_bytes();
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
        self.field.write_text(&self.task, text)?;
        Ok(buf.len())
    }

    fn metadata(&self) -> FsResult<Metadata> {
        Ok(file_metadata(
            self.read_data.len() as u64,
            self.field.mode(),
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
        self.access.can_read().is_ok()
    }
}

pub(crate) fn field_metadata(task: &Task, field: &str) -> FsResult<Metadata> {
    if field == "ctl" {
        return Ok(file_metadata(0, TASK_FILE_READ_WRITE_MODE));
    }
    if field == "wait" {
        return Ok(file_metadata(0, TASK_FILE_READ_ONLY_MODE));
    }
    let field = Field::from_name(field)?;
    Ok(file_metadata(field.text(task).len() as u64, field.mode()))
}
