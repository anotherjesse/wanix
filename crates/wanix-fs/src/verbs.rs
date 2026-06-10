//! [`VerbBinFs`]: the read-only `bin/` surface a resource ships its verbs in.
//!
//! BinVerbs (ADR 0007 §Confinement contract): a served resource exposes
//! executable verbs as plain files under `bin/`, host-served beside the
//! resource's own tree — never routed through a guest. The view is
//! deliberately narrow: read-only, flat (one level of plain files; nested
//! entries do not exist through it), and size-capped, so mounting a resource's
//! vocabulary can neither mutate the serving host nor ship unbounded bytes
//! disguised as a verb.

use std::sync::Arc;

use crate::{
    DirEntry, File, FileSeekFrom, FileSystem, FileType, FsError, FsResult, Metadata,
    NormalizedPath, OpenOptions,
};

/// The largest file [`VerbBinFs`] will serve.
///
/// Verbs are scripts or small compiled-wasm commands (the bundled `jaq.wasm`
/// is ~1.5 MiB); anything larger than this is refused at open with an honest
/// error instead of streamed.
pub const MAX_VERB_FILE_BYTES: u64 = 4 * 1024 * 1024;
// The bundled jaq.wasm (~1.5 MiB) must always fit under the default cap.
const _: () = assert!(MAX_VERB_FILE_BYTES >= 2 * 1024 * 1024);

const READ_ONLY_FILE_MODE: u32 = 0o555;
const READ_ONLY_DIR_MODE: u32 = 0o555;

/// A read-only, flat, size-capped view over a resource's verb directory.
pub struct VerbBinFs {
    inner: Arc<dyn FileSystem>,
    max_bytes: u64,
}

impl VerbBinFs {
    /// Wraps `inner` (typically a [`LocalFs`](crate::LocalFs) over
    /// `<resource-dir>/bin`) with the default [`MAX_VERB_FILE_BYTES`] cap.
    #[must_use]
    pub fn new(inner: Arc<dyn FileSystem>) -> Self {
        Self::with_max_bytes(inner, MAX_VERB_FILE_BYTES)
    }

    /// Wraps `inner` with an explicit size cap (tests use a small one).
    #[must_use]
    pub fn with_max_bytes(inner: Arc<dyn FileSystem>, max_bytes: u64) -> Self {
        Self { inner, max_bytes }
    }

    /// Resolves a path through the flat-view rules: the root is the verb
    /// directory; anything nested does not exist through this view.
    fn flat_metadata(&self, path: &NormalizedPath) -> FsResult<Metadata> {
        if path.as_str() == "." {
            return Ok(Metadata::new(FileType::Directory, 2, READ_ONLY_DIR_MODE));
        }
        if path.as_str().contains('/') {
            return Err(FsError::NotFound);
        }
        let metadata = self.inner.metadata(path)?;
        if metadata.file_type() != FileType::File {
            // Subdirectories (and symlinks) in the backing dir are not part
            // of the verb surface.
            return Err(FsError::NotFound);
        }
        Ok(read_only_view(&metadata))
    }
}

fn read_only_view(metadata: &Metadata) -> Metadata {
    Metadata::new(FileType::File, metadata.len(), READ_ONLY_FILE_MODE)
}

impl FileSystem for VerbBinFs {
    fn open(&self, path: &NormalizedPath, options: OpenOptions) -> FsResult<Box<dyn File>> {
        if options.write || options.create || options.truncate {
            return Err(FsError::PermissionDenied);
        }
        let metadata = self.flat_metadata(path)?;
        if metadata.file_type() == FileType::Directory {
            return Err(FsError::IsDirectory);
        }
        if metadata.len() > self.max_bytes {
            return Err(FsError::Other(format!(
                "{path}: {} bytes exceeds the {}-byte verb size cap",
                metadata.len(),
                self.max_bytes
            )));
        }
        let file = self.inner.open(path, OpenOptions::read())?;
        Ok(Box::new(CappedReadFile {
            file,
            remaining: self.max_bytes,
        }))
    }

    fn metadata(&self, path: &NormalizedPath) -> FsResult<Metadata> {
        self.flat_metadata(path)
    }

    fn read_dir(&self, path: &NormalizedPath) -> FsResult<Vec<DirEntry>> {
        if path.as_str() != "." {
            self.flat_metadata(path)?;
            return Err(FsError::NotDirectory);
        }
        Ok(self
            .inner
            .read_dir(path)?
            .into_iter()
            .filter(|entry| entry.metadata().file_type() == FileType::File)
            .map(|entry| DirEntry::new(entry.name(), read_only_view(entry.metadata())))
            .collect())
    }
}

/// A read handle that enforces the size cap on the bytes actually served,
/// so a backing file that grows after the open-time check stays bounded.
struct CappedReadFile {
    file: Box<dyn File>,
    remaining: u64,
}

impl File for CappedReadFile {
    fn read(&mut self, buf: &mut [u8]) -> FsResult<usize> {
        if self.remaining == 0 {
            return Err(FsError::Other(
                "verb file read exceeds the verb size cap".to_owned(),
            ));
        }
        let limit = usize::try_from(self.remaining.min(buf.len() as u64)).unwrap_or(buf.len());
        let count = self.file.read(&mut buf[..limit])?;
        self.remaining -= count as u64;
        Ok(count)
    }

    fn seek(&mut self, from: FileSeekFrom) -> FsResult<u64> {
        // Seeking would let a reader re-window past the cap accounting; the
        // verb loaders read sequentially, so refuse rather than under-count.
        let _ = from;
        Err(FsError::NotSupported)
    }

    fn metadata(&self) -> FsResult<Metadata> {
        Ok(read_only_view(&self.file.metadata()?))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::VerbBinFs;
    use crate::{FileSystem, FileType, FsError, MemFs, NormalizedPath, OpenOptions};

    fn np(path: &str) -> NormalizedPath {
        NormalizedPath::new(path).unwrap()
    }

    fn backing() -> Arc<MemFs> {
        let fs = Arc::new(MemFs::new());
        fs.write_file("post.js", b"// post verb").unwrap();
        fs.write_file("watch.wasm", b"\0asm").unwrap();
        fs.create_dir(&np("nested")).unwrap();
        fs.write_file("nested/hidden.js", b"// not a verb").unwrap();
        fs
    }

    #[test]
    fn serves_flat_read_only_files_and_hides_nested_entries() {
        let bin = VerbBinFs::new(backing());
        let mut file = bin.open(&np("post.js"), OpenOptions::read()).unwrap();
        let mut buf = [0u8; 64];
        let n = file.read(&mut buf).unwrap();
        assert_eq!(&buf[..n], b"// post verb");

        let names: Vec<String> = bin
            .read_dir(&np("."))
            .unwrap()
            .into_iter()
            .map(|entry| {
                assert_eq!(entry.metadata().mode() & 0o222, 0, "verbs are read-only");
                assert_eq!(entry.metadata().file_type(), FileType::File);
                entry.name().to_owned()
            })
            .collect();
        assert_eq!(names, ["post.js", "watch.wasm"], "directories are hidden");

        for hidden in ["nested", "nested/hidden.js"] {
            assert_eq!(bin.metadata(&np(hidden)), Err(FsError::NotFound));
            assert!(bin.open(&np(hidden), OpenOptions::read()).is_err());
        }
    }

    #[test]
    fn refuses_writes_and_mutations() {
        let bin = VerbBinFs::new(backing());
        assert_eq!(
            bin.open(&np("post.js"), OpenOptions::read_write())
                .err()
                .unwrap(),
            FsError::PermissionDenied
        );
        assert_eq!(
            bin.open(
                &np("new.js"),
                OpenOptions {
                    write: true,
                    create: true,
                    ..OpenOptions::default()
                }
            )
            .err()
            .unwrap(),
            FsError::PermissionDenied
        );
        // Mutations fall through to the FileSystem trait defaults.
        assert_eq!(bin.remove_file(&np("post.js")), Err(FsError::NotSupported));
    }

    #[test]
    fn oversized_verbs_are_refused_at_open_with_an_honest_error() {
        let bin = VerbBinFs::with_max_bytes(backing(), 8);
        let Err(error) = bin.open(&np("post.js"), OpenOptions::read()).map(|_| ()) else {
            panic!("oversized verb must not open");
        };
        match error {
            FsError::Other(message) => {
                assert!(message.contains("verb size cap"), "{message}");
                assert!(message.contains("post.js"), "{message}");
            }
            other => panic!("expected the size-cap error, got {other:?}"),
        }
        // A small file still serves, fully, through the capped handle.
        let mut file = bin.open(&np("watch.wasm"), OpenOptions::read()).unwrap();
        let mut buf = [0u8; 16];
        assert_eq!(file.read(&mut buf).unwrap(), 4);
        assert_eq!(file.read(&mut buf).unwrap(), 0, "EOF inside the cap");
    }
}

