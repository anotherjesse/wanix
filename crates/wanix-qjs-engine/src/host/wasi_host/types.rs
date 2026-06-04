/// WASI Preview 1 errno values used by the QuickJS engine host boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuickJsWasiErrno {
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
    /// Generic I/O error.
    Io,
    /// Path names a directory where a file was expected.
    Isdir,
    /// Path component was not a directory.
    Notdir,
    /// Directory is not empty.
    Notempty,
    /// Operation not supported.
    Nosys,
    /// Capability rights are insufficient.
    Notcapable,
}

impl QuickJsWasiErrno {
    pub(crate) const fn preview1_result(self) -> i32 {
        match self {
            Self::Badf => 8,
            Self::Exist => 20,
            Self::Inval => 28,
            Self::Io => 29,
            Self::Isdir => 31,
            Self::Nametoolong => 37,
            Self::Noent => 44,
            Self::Nosys => 52,
            Self::Notdir => 54,
            Self::Notempty => 55,
            Self::Notcapable => 76,
        }
    }
}

/// WASI Preview 1 file type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuickJsWasiFileType {
    /// Unknown file type.
    Unknown,
    /// Character device.
    CharacterDevice,
    /// Directory.
    Directory,
    /// Regular file.
    RegularFile,
    /// Symbolic link.
    SymbolicLink,
}

impl QuickJsWasiFileType {
    /// Creates a file type from a Preview 1 numeric code.
    #[must_use]
    pub const fn from_preview1_code(code: u8) -> Option<Self> {
        match code {
            0 => Some(Self::Unknown),
            2 => Some(Self::CharacterDevice),
            3 => Some(Self::Directory),
            4 => Some(Self::RegularFile),
            7 => Some(Self::SymbolicLink),
            _ => None,
        }
    }

    pub(crate) const fn preview1_code(self) -> u8 {
        match self {
            Self::Unknown => 0,
            Self::CharacterDevice => 2,
            Self::Directory => 3,
            Self::RegularFile => 4,
            Self::SymbolicLink => 7,
        }
    }
}

/// WASI Preview 1 seek origin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuickJsWasiWhence {
    /// Seek relative to the start.
    Set,
    /// Seek relative to the current offset.
    Cur,
    /// Seek relative to the end.
    End,
}

impl QuickJsWasiWhence {
    pub(crate) const fn from_preview1(code: i32) -> Result<Self, QuickJsWasiErrno> {
        match code {
            0 => Ok(Self::Set),
            1 => Ok(Self::Cur),
            2 => Ok(Self::End),
            _ => Err(QuickJsWasiErrno::Inval),
        }
    }
}

/// WASI Preview 1 prestat metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuickJsWasiPrestat {
    dir_name: String,
}

impl QuickJsWasiPrestat {
    /// Creates prestat metadata for a preopened directory name.
    #[must_use]
    pub fn new(dir_name: impl Into<String>) -> Self {
        Self {
            dir_name: dir_name.into(),
        }
    }

    pub(crate) fn dir_name(&self) -> &str {
        &self.dir_name
    }
}

/// WASI Preview 1 directory entry metadata returned by a live host provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuickJsWasiDirEntry {
    name: String,
    file_type: QuickJsWasiFileType,
}

impl QuickJsWasiDirEntry {
    /// Creates directory entry metadata.
    #[must_use]
    pub fn new(name: impl Into<String>, file_type: QuickJsWasiFileType) -> Self {
        Self {
            name: name.into(),
            file_type,
        }
    }

    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    pub(crate) const fn file_type(&self) -> QuickJsWasiFileType {
        self.file_type
    }
}

/// WASI Preview 1 fdstat metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuickJsWasiFdStat {
    file_type: QuickJsWasiFileType,
    fdflags: u16,
    rights_base: u64,
    rights_inheriting: u64,
}

impl QuickJsWasiFdStat {
    /// Creates fdstat metadata.
    #[must_use]
    pub const fn new(
        file_type: QuickJsWasiFileType,
        rights_base: u64,
        rights_inheriting: u64,
    ) -> Self {
        Self::new_with_fdflags(file_type, 0, rights_base, rights_inheriting)
    }

    /// Creates fdstat metadata with Preview 1 fdflags.
    #[must_use]
    pub const fn new_with_fdflags(
        file_type: QuickJsWasiFileType,
        fdflags: u16,
        rights_base: u64,
        rights_inheriting: u64,
    ) -> Self {
        Self {
            file_type,
            fdflags,
            rights_base,
            rights_inheriting,
        }
    }

    pub(crate) const fn file_type(self) -> QuickJsWasiFileType {
        self.file_type
    }

    pub(crate) const fn fdflags(self) -> u16 {
        self.fdflags
    }

    pub(crate) const fn rights_base(self) -> u64 {
        self.rights_base
    }

    pub(crate) const fn rights_inheriting(self) -> u64 {
        self.rights_inheriting
    }
}

/// WASI Preview 1 filestat metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuickJsWasiFileStat {
    file_type: QuickJsWasiFileType,
    size: u64,
    accessed_time_ns: u64,
    modified_time_ns: u64,
    changed_time_ns: u64,
}

impl QuickJsWasiFileStat {
    /// Creates filestat metadata.
    #[must_use]
    pub const fn new(file_type: QuickJsWasiFileType, size: u64) -> Self {
        Self::new_with_times(file_type, size, 0, 0, 0)
    }

    /// Creates filestat metadata with explicit nanosecond timestamps.
    #[must_use]
    pub const fn new_with_times(
        file_type: QuickJsWasiFileType,
        size: u64,
        accessed_time_ns: u64,
        modified_time_ns: u64,
        changed_time_ns: u64,
    ) -> Self {
        Self {
            file_type,
            size,
            accessed_time_ns,
            modified_time_ns,
            changed_time_ns,
        }
    }

    pub(crate) const fn file_type(self) -> QuickJsWasiFileType {
        self.file_type
    }

    pub(crate) const fn size(self) -> u64 {
        self.size
    }

    pub(crate) const fn accessed_time_ns(self) -> u64 {
        self.accessed_time_ns
    }

    pub(crate) const fn modified_time_ns(self) -> u64 {
        self.modified_time_ns
    }

    pub(crate) const fn changed_time_ns(self) -> u64 {
        self.changed_time_ns
    }
}

#[cfg(test)]
mod tests {
    use super::{QuickJsWasiErrno, QuickJsWasiFileType};

    #[test]
    fn preview1_errno_codes_are_pinned() {
        let cases = [
            (QuickJsWasiErrno::Badf, 8),
            (QuickJsWasiErrno::Exist, 20),
            (QuickJsWasiErrno::Inval, 28),
            (QuickJsWasiErrno::Io, 29),
            (QuickJsWasiErrno::Isdir, 31),
            (QuickJsWasiErrno::Nametoolong, 37),
            (QuickJsWasiErrno::Noent, 44),
            (QuickJsWasiErrno::Nosys, 52),
            (QuickJsWasiErrno::Notdir, 54),
            (QuickJsWasiErrno::Notempty, 55),
            (QuickJsWasiErrno::Notcapable, 76),
        ];

        for (errno, code) in cases {
            assert_eq!(errno.preview1_result(), code);
        }
    }

    #[test]
    fn preview1_file_type_codes_round_trip_known_values() {
        let cases = [
            (0, QuickJsWasiFileType::Unknown),
            (2, QuickJsWasiFileType::CharacterDevice),
            (3, QuickJsWasiFileType::Directory),
            (4, QuickJsWasiFileType::RegularFile),
            (7, QuickJsWasiFileType::SymbolicLink),
        ];

        for (code, file_type) in cases {
            assert_eq!(
                QuickJsWasiFileType::from_preview1_code(code),
                Some(file_type)
            );
            assert_eq!(file_type.preview1_code(), code);
        }
        assert_eq!(QuickJsWasiFileType::from_preview1_code(1), None);
        assert_eq!(QuickJsWasiFileType::from_preview1_code(u8::MAX), None);
    }
}
