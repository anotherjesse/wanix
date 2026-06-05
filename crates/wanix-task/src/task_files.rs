use wanix_fs::{DirEntry, File, FileSeekFrom, FileType, FsError, FsResult, Metadata};

use crate::{OpenFile, Task};

mod control;
mod new_task;

pub(crate) use control::ControlFile;
pub(crate) use new_task::NewTaskFile;

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

    pub(crate) fn can_seek(self) -> FsResult<()> {
        if self.read || self.write {
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
pub(crate) struct FdProxyFile {
    file: OpenFile,
    access: FileAccess,
}

impl FdProxyFile {
    pub(crate) fn new(file: OpenFile, access: FileAccess) -> Self {
        Self { file, access }
    }
}

impl File for FdProxyFile {
    fn read(&mut self, buf: &mut [u8]) -> FsResult<usize> {
        self.access.can_read()?;
        self.file.read(buf)
    }

    fn write(&mut self, buf: &[u8]) -> FsResult<usize> {
        self.access.can_write()?;
        self.file.write(buf)
    }

    fn seek(&mut self, from: FileSeekFrom) -> FsResult<u64> {
        self.access.can_seek()?;
        self.file.seek(from)
    }

    fn tell(&self) -> FsResult<u64> {
        self.access.can_seek()?;
        self.file.tell()
    }

    fn is_seekable(&self) -> bool {
        self.file.is_seekable().unwrap_or(false)
    }

    fn metadata(&self) -> FsResult<Metadata> {
        self.file.metadata()
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
    if field == "ctl" {
        return Ok(file_metadata(0, 0o755));
    }
    let field = field_from_name(field)?;
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

fn field_from_name(name: &str) -> FsResult<Field> {
    TASK_FIELDS
        .iter()
        .find_map(|(field_name, field)| (*field_name == name).then_some(*field))
        .ok_or(FsError::NotFound)
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
