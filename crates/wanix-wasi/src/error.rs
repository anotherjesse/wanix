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
    /// Directory is not empty.
    Notempty,
    /// Operation not supported.
    Nosys,
    /// Interrupted: the task was cancelled (killed) while blocked.
    Intr,
    /// Capability rights are insufficient.
    Notcapable,
    /// Unknown host filesystem error.
    Io,
}

impl Errno {
    /// Returns the WASI Preview 1 numeric errno code.
    #[must_use]
    pub const fn preview1_code(self) -> u16 {
        match self {
            Self::Success => 0,
            Self::Badf => 8,
            Self::Inval => 28,
            Self::Intr => 27,
            Self::Io => 29,
            Self::Isdir => 31,
            Self::Nametoolong => 37,
            Self::Noent => 44,
            Self::Nosys => 52,
            Self::Notdir => 54,
            Self::Notempty => 55,
            Self::Exist => 20,
            Self::Notcapable => 76,
        }
    }

    /// Returns the WASI Preview 1 numeric errno code as an import result.
    #[must_use]
    pub const fn preview1_result(self) -> i32 {
        self.preview1_code() as i32
    }
}

impl From<&FsError> for Errno {
    fn from(error: &FsError) -> Self {
        match error {
            FsError::InvalidPath(_) => Self::Inval,
            FsError::InvalidArgument(_) => Self::Inval,
            FsError::NotFound => Self::Noent,
            FsError::NotSupported => Self::Nosys,
            FsError::PermissionDenied => Self::Notcapable,
            FsError::AlreadyExists => Self::Exist,
            FsError::NotDirectory => Self::Notdir,
            FsError::IsDirectory => Self::Isdir,
            FsError::InvalidFd => Self::Badf,
            FsError::InvalidOffset => Self::Inval,
            FsError::InvalidTime => Self::Inval,
            FsError::NotEmpty => Self::Notempty,
            // ADR 0008: a live resource outage is EIO-class, never ENOENT —
            // agents must not mistake a missing provider for a missing file.
            FsError::Unreachable(_) => Self::Io,
            FsError::Other(_) => Self::Io,
        }
    }
}

impl From<FsError> for Errno {
    fn from(error: FsError) -> Self {
        Self::from(&error)
    }
}
