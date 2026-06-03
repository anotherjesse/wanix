use wanix_fs::FsError;

/// WASI errno values used by the initial adapter contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Errno {
    /// Operation succeeded.
    Success,
    /// Bad file descriptor.
    Badf,
    /// Invalid input.
    Inval,
    /// Path or name is too long.
    Nametoolong,
    /// File or directory missing.
    Noent,
    /// File exists.
    Exist,
    /// Path component was not a directory.
    Notdir,
    /// Path names a directory where a file was expected.
    Isdir,
    /// Operation not supported.
    Nosys,
    /// Capability rights are insufficient.
    Notcapable,
    /// Unknown host filesystem error.
    Io,
}

impl From<&FsError> for Errno {
    fn from(error: &FsError) -> Self {
        match error {
            FsError::InvalidPath(_) => Self::Inval,
            FsError::NotFound => Self::Noent,
            FsError::NotSupported => Self::Nosys,
            FsError::PermissionDenied => Self::Notcapable,
            FsError::AlreadyExists => Self::Exist,
            FsError::NotDirectory => Self::Notdir,
            FsError::IsDirectory => Self::Isdir,
            FsError::InvalidFd => Self::Badf,
            FsError::NotEmpty | FsError::Other(_) => Self::Io,
        }
    }
}

impl From<FsError> for Errno {
    fn from(error: FsError) -> Self {
        Self::from(&error)
    }
}
