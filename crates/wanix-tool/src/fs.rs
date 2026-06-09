//! `ToolFs`: the principal-scoped filesystem view of one [`ToolService`].
//!
//! The view implements the job-protocol surface (ADR 0009): root metadata
//! files, `new`, and the per-job directories. Privacy is enforced here only
//! in the sense that the bound principal scopes every service call; the
//! `FileSystem` impl itself never sees a caller-claimed identity.

use wanix_fs::{
    DirEntry, File, FileSystem, FileType, FsError, FsResult, Metadata, NormalizedPath, OpenOptions,
};

use crate::files::{AppendFile, BytesFile, CtlFile, NewJobFile, require_read_only};
use crate::principal::ToolPrincipal;
use crate::service::{JobField, ToolService};

pub(crate) mod modes {
    pub(crate) const DIRECTORY: u32 = 0o555;
    pub(crate) const READ_FILE: u32 = 0o444;
    pub(crate) const DATA_FILE: u32 = 0o666;
    pub(crate) const CTL_FILE: u32 = 0o222;
}

/// One principal's view of a ToolFS resource.
#[derive(Debug, Clone)]
pub struct ToolFs {
    service: ToolService,
    principal: ToolPrincipal,
}

impl ToolFs {
    pub(crate) fn new(service: ToolService, principal: ToolPrincipal) -> Self {
        Self { service, principal }
    }

    /// The principal this view acts as.
    #[must_use]
    pub fn principal(&self) -> &ToolPrincipal {
        &self.principal
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ToolPath<'a> {
    Root,
    Spec,
    Schema,
    Health,
    Usage,
    New,
    JobsDir,
    JobDir(&'a str),
    JobFile(&'a str, JobNode),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum JobNode {
    Field(JobField),
    Ctl,
}

const JOB_FILES: [(&str, JobNode); 7] = [
    ("in", JobNode::Field(JobField::In)),
    ("params.json", JobNode::Field(JobField::Params)),
    ("ctl", JobNode::Ctl),
    ("out", JobNode::Field(JobField::Out)),
    ("err", JobNode::Field(JobField::Err)),
    ("status", JobNode::Field(JobField::Status)),
    ("result.json", JobNode::Field(JobField::Result)),
];

fn job_node(name: &str) -> FsResult<JobNode> {
    JOB_FILES
        .iter()
        .find(|(file, _)| *file == name)
        .map(|(_, node)| *node)
        .ok_or(FsError::NotFound)
}

fn parse_path(path: &NormalizedPath) -> FsResult<ToolPath<'_>> {
    let raw = path.as_str();
    if raw == "." {
        return Ok(ToolPath::Root);
    }
    let mut parts = raw.split('/');
    match (parts.next(), parts.next(), parts.next(), parts.next()) {
        (Some("spec.json"), None, _, _) => Ok(ToolPath::Spec),
        (Some("params.schema.json"), None, _, _) => Ok(ToolPath::Schema),
        (Some("health"), None, _, _) => Ok(ToolPath::Health),
        (Some("usage"), None, _, _) => Ok(ToolPath::Usage),
        (Some("new"), None, _, _) => Ok(ToolPath::New),
        (Some("jobs"), None, _, _) => Ok(ToolPath::JobsDir),
        (Some("jobs"), Some(id), None, _) => Ok(ToolPath::JobDir(id)),
        (Some("jobs"), Some(id), Some(file), None) => Ok(ToolPath::JobFile(id, job_node(file)?)),
        _ => Err(FsError::NotFound),
    }
}

impl ToolFs {
    fn open_job_file(
        &self,
        id: &str,
        node: JobNode,
        options: OpenOptions,
    ) -> FsResult<Box<dyn File>> {
        match node {
            JobNode::Ctl => {
                if !options.write {
                    return Err(FsError::PermissionDenied);
                }
                self.service.check_job(&self.principal, id)?;
                Ok(Box::new(CtlFile::new(
                    self.service.clone(),
                    self.principal.clone(),
                    id.to_owned(),
                )))
            }
            JobNode::Field(field @ (JobField::In | JobField::Params))
                if options.write || options.create || options.truncate =>
            {
                self.service.check_job(&self.principal, id)?;
                if options.truncate {
                    self.service.reset_field(&self.principal, id, field)?;
                }
                Ok(Box::new(AppendFile::new(
                    self.service.clone(),
                    self.principal.clone(),
                    id.to_owned(),
                    field,
                )))
            }
            JobNode::Field(field) => {
                if !options.read || options.write {
                    return Err(FsError::PermissionDenied);
                }
                let bytes = self.service.read_field(&self.principal, id, field)?;
                let mode = match field {
                    JobField::In | JobField::Params => modes::DATA_FILE,
                    _ => modes::READ_FILE,
                };
                Ok(Box::new(BytesFile::new(bytes, mode)))
            }
        }
    }

    fn job_file_metadata(&self, id: &str, node: JobNode) -> FsResult<Metadata> {
        match node {
            JobNode::Ctl => {
                self.service.check_job(&self.principal, id)?;
                Ok(file_metadata(0, modes::CTL_FILE))
            }
            JobNode::Field(field) => {
                let len = self.service.field_len(&self.principal, id, field)?;
                let mode = match field {
                    JobField::In | JobField::Params => modes::DATA_FILE,
                    _ => modes::READ_FILE,
                };
                Ok(file_metadata(len, mode))
            }
        }
    }
}

impl FileSystem for ToolFs {
    fn open(&self, path: &NormalizedPath, options: OpenOptions) -> FsResult<Box<dyn File>> {
        match parse_path(path)? {
            ToolPath::Root | ToolPath::JobsDir | ToolPath::JobDir(_) => Err(FsError::IsDirectory),
            ToolPath::Spec => {
                require_read_only(options)?;
                Ok(Box::new(BytesFile::new(
                    self.service.spec_json(),
                    modes::READ_FILE,
                )))
            }
            ToolPath::Schema => {
                require_read_only(options)?;
                let bytes = self.service.schema_json().ok_or(FsError::NotFound)?;
                Ok(Box::new(BytesFile::new(bytes, modes::READ_FILE)))
            }
            ToolPath::Health => {
                require_read_only(options)?;
                Ok(Box::new(BytesFile::new(
                    self.service.health_json(),
                    modes::READ_FILE,
                )))
            }
            ToolPath::Usage => {
                require_read_only(options)?;
                Ok(Box::new(BytesFile::new(
                    self.service.usage_json(&self.principal)?,
                    modes::READ_FILE,
                )))
            }
            ToolPath::New => {
                require_read_only(options)?;
                Ok(Box::new(NewJobFile::new(
                    self.service.clone(),
                    self.principal.clone(),
                )))
            }
            ToolPath::JobFile(id, node) => self.open_job_file(id, node, options),
        }
    }

    fn metadata(&self, path: &NormalizedPath) -> FsResult<Metadata> {
        match parse_path(path)? {
            ToolPath::Root | ToolPath::JobsDir => Ok(directory_metadata()),
            ToolPath::JobDir(id) => {
                self.service.check_job(&self.principal, id)?;
                Ok(directory_metadata())
            }
            ToolPath::Spec => Ok(file_metadata(
                self.service.spec_json().len() as u64,
                modes::READ_FILE,
            )),
            ToolPath::Schema => {
                let bytes = self.service.schema_json().ok_or(FsError::NotFound)?;
                Ok(file_metadata(bytes.len() as u64, modes::READ_FILE))
            }
            ToolPath::Health => Ok(file_metadata(
                self.service.health_json().len() as u64,
                modes::READ_FILE,
            )),
            ToolPath::Usage => Ok(file_metadata(
                self.service.usage_json(&self.principal)?.len() as u64,
                modes::READ_FILE,
            )),
            ToolPath::New => Ok(file_metadata(0, modes::READ_FILE)),
            ToolPath::JobFile(id, node) => self.job_file_metadata(id, node),
        }
    }

    fn read_dir(&self, path: &NormalizedPath) -> FsResult<Vec<DirEntry>> {
        match parse_path(path)? {
            ToolPath::Root => {
                let mut entries = Vec::new();
                for name in ["spec.json", "params.schema.json", "health", "usage", "new"] {
                    match self.metadata(&NormalizedPath::new(name)?) {
                        Ok(meta) => entries.push(DirEntry::new(name, meta)),
                        // params.schema.json exists only when configured.
                        Err(FsError::NotFound) => {}
                        Err(err) => return Err(err),
                    }
                }
                entries.push(DirEntry::new("jobs", directory_metadata()));
                Ok(entries)
            }
            ToolPath::JobsDir => Ok(self
                .service
                .job_ids(&self.principal)?
                .into_iter()
                .map(|id| DirEntry::new(id, directory_metadata()))
                .collect()),
            ToolPath::JobDir(id) => {
                self.service.check_job(&self.principal, id)?;
                JOB_FILES
                    .iter()
                    .map(|(name, node)| {
                        Ok(DirEntry::new(*name, self.job_file_metadata(id, *node)?))
                    })
                    .collect()
            }
            _ => Err(FsError::NotDirectory),
        }
    }
}

fn directory_metadata() -> Metadata {
    Metadata::new(FileType::Directory, 2, modes::DIRECTORY)
}

pub(crate) fn file_metadata(len: u64, mode: u32) -> Metadata {
    Metadata::new(FileType::File, len, mode)
}
