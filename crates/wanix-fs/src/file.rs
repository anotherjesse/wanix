use crate::{FsError, FsResult, Metadata};

/// File seek origin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileSeekFrom {
    /// Seek to an absolute byte offset from the beginning of the file.
    Start(u64),
    /// Seek relative to the current file offset.
    Current(i64),
    /// Seek relative to the current end of file.
    End(i64),
}

/// Open options used by filesystem implementations.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OpenOptions {
    /// Open for reading.
    pub read: bool,
    /// Open for writing.
    pub write: bool,
    /// Create the file if it is missing.
    pub create: bool,
    /// Truncate the file after opening.
    pub truncate: bool,
}

impl OpenOptions {
    /// Read-only open options.
    #[must_use]
    pub fn read() -> Self {
        Self {
            read: true,
            ..Self::default()
        }
    }

    /// Read-write open options.
    #[must_use]
    pub fn read_write() -> Self {
        Self {
            read: true,
            write: true,
            ..Self::default()
        }
    }
}

/// Open file behavior.
pub trait File: Send {
    /// Reads bytes into `buf`.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when the file cannot be read.
    fn read(&mut self, buf: &mut [u8]) -> FsResult<usize>;

    /// Writes bytes from `buf`.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when the file cannot be written.
    fn write(&mut self, _buf: &[u8]) -> FsResult<usize> {
        Err(FsError::NotSupported)
    }

    /// Seeks to a new file offset and returns the resulting absolute offset.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when the file cannot seek or the requested
    /// offset is invalid.
    fn seek(&mut self, _from: FileSeekFrom) -> FsResult<u64> {
        Err(FsError::NotSupported)
    }

    /// Returns the current file offset.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when the file cannot report its offset.
    fn tell(&self) -> FsResult<u64> {
        Err(FsError::NotSupported)
    }

    /// Returns whether this handle supports seek/tell operations.
    #[must_use]
    fn is_seekable(&self) -> bool {
        false
    }

    /// Returns whether a nonblocking read would produce data now.
    ///
    /// Regular byte files are ready by default, including at EOF. Devices with
    /// queued input can override this to avoid reporting readiness while empty.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when readiness cannot be determined.
    fn read_ready(&self) -> FsResult<bool> {
        Ok(true)
    }

    /// Returns whether a nonblocking write can be attempted now.
    ///
    /// Most Wanix files and devices currently accept writes synchronously, so
    /// the default is ready.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when readiness cannot be determined.
    fn write_ready(&self) -> FsResult<bool> {
        Ok(true)
    }

    /// Sets the file length in bytes.
    ///
    /// Implementations should preserve the current file offset when possible,
    /// matching `ftruncate`-style behavior.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when the handle cannot change file size.
    fn set_len(&mut self, _len: u64) -> FsResult<()> {
        Err(FsError::NotSupported)
    }

    /// Returns file metadata.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when metadata cannot be produced.
    fn metadata(&self) -> FsResult<Metadata>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FileType, Metadata};

    #[test]
    fn open_options_constructors_set_expected_capabilities() {
        assert_eq!(
            OpenOptions::read(),
            OpenOptions {
                read: true,
                ..OpenOptions::default()
            }
        );
        assert_eq!(
            OpenOptions::read_write(),
            OpenOptions {
                read: true,
                write: true,
                ..OpenOptions::default()
            }
        );
    }

    #[test]
    fn default_file_operations_report_core_capabilities() {
        let mut file = MinimalFile;

        assert_eq!(file.write(b"data"), Err(FsError::NotSupported));
        assert_eq!(
            file.seek(FileSeekFrom::Start(0)),
            Err(FsError::NotSupported)
        );
        assert_eq!(file.tell(), Err(FsError::NotSupported));
        assert!(!file.is_seekable());
        assert_eq!(file.read_ready(), Ok(true));
        assert_eq!(file.write_ready(), Ok(true));
        assert_eq!(file.set_len(0), Err(FsError::NotSupported));
        assert_eq!(file.metadata().unwrap().len(), 0);
    }

    struct MinimalFile;

    impl File for MinimalFile {
        fn read(&mut self, _buf: &mut [u8]) -> FsResult<usize> {
            Ok(0)
        }

        fn metadata(&self) -> FsResult<Metadata> {
            Ok(Metadata::new(FileType::File, 0, 0o644))
        }
    }
}
