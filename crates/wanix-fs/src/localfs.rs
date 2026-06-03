use std::fs::{self, File as StdFile};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::{
    DirEntry, File, FileSeekFrom, FileSystem, FileType, FsError, FsResult, Metadata,
    NormalizedPath, OpenOptions,
};

/// Host-directory-backed filesystem rooted at a single local directory.
#[derive(Debug, Clone)]
pub struct LocalFs {
    root: Arc<PathBuf>,
}

impl LocalFs {
    /// Creates a local filesystem rooted at `root`.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when `root` cannot be resolved or is not a
    /// directory.
    pub fn new(root: impl AsRef<Path>) -> FsResult<Self> {
        let root = fs::canonicalize(root.as_ref()).map_err(map_io_error)?;
        let metadata = fs::metadata(&root).map_err(map_io_error)?;
        if !metadata.is_dir() {
            return Err(FsError::NotDirectory);
        }
        Ok(Self {
            root: Arc::new(root),
        })
    }

    /// Returns the canonical host root path.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    fn existing_host_path(&self, path: &NormalizedPath) -> FsResult<PathBuf> {
        let host_path = self.raw_host_path(path);
        let resolved = fs::canonicalize(&host_path).map_err(map_io_error)?;
        if resolved.starts_with(&*self.root) {
            Ok(resolved)
        } else {
            Err(FsError::PermissionDenied)
        }
    }

    fn host_path_for_open(&self, path: &NormalizedPath, options: OpenOptions) -> FsResult<PathBuf> {
        let host_path = self.raw_host_path(path);
        if options.create {
            match fs::symlink_metadata(&host_path) {
                Ok(_) => return self.existing_host_path(path),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    let parent = host_path.parent().ok_or(FsError::IsDirectory)?;
                    let parent = fs::canonicalize(parent).map_err(map_io_error)?;
                    if parent.starts_with(&*self.root) {
                        return Ok(host_path);
                    }
                    return Err(FsError::PermissionDenied);
                }
                Err(error) => return Err(map_io_error(error)),
            }
        }
        self.existing_host_path(path)
    }

    fn raw_host_path(&self, path: &NormalizedPath) -> PathBuf {
        let mut host_path = (*self.root).clone();
        if path.as_str() != "." {
            for component in path.as_str().split('/') {
                host_path.push(component);
            }
        }
        host_path
    }
}

impl FileSystem for LocalFs {
    fn open(&self, path: &NormalizedPath, options: OpenOptions) -> FsResult<Box<dyn File>> {
        if options.create && !options.write {
            return Err(FsError::PermissionDenied);
        }
        if options.truncate && !options.write {
            return Err(FsError::PermissionDenied);
        }

        let host_path = self.host_path_for_open(path, options)?;
        let file = fs::OpenOptions::new()
            .read(options.read)
            .write(options.write)
            .create(options.create)
            .truncate(options.truncate)
            .open(&host_path)
            .map_err(map_io_error)?;
        let metadata = file.metadata().map_err(map_io_error)?;
        if metadata.is_dir() {
            return Err(FsError::IsDirectory);
        }
        Ok(Box::new(LocalFile {
            file,
            offset: 0,
            readable: options.read,
            writable: options.write,
        }))
    }

    fn metadata(&self, path: &NormalizedPath) -> FsResult<Metadata> {
        let host_path = self.existing_host_path(path)?;
        let metadata = fs::symlink_metadata(host_path).map_err(map_io_error)?;
        Ok(metadata_from_host(&metadata))
    }

    fn read_dir(&self, path: &NormalizedPath) -> FsResult<Vec<DirEntry>> {
        let host_path = self.existing_host_path(path)?;
        let metadata = fs::metadata(&host_path).map_err(map_io_error)?;
        if !metadata.is_dir() {
            return Err(FsError::NotDirectory);
        }

        let mut entries = Vec::new();
        for entry in fs::read_dir(host_path).map_err(map_io_error)? {
            let entry = entry.map_err(map_io_error)?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| FsError::InvalidPath("<non-utf8 host path>".to_owned()))?;
            if NormalizedPath::new(&name).is_err() {
                continue;
            }
            let child_path = if path.as_str() == "." {
                NormalizedPath::new(&name)?
            } else {
                NormalizedPath::new(format!("{path}/{name}"))?
            };
            let metadata = match self.metadata(&child_path) {
                Ok(metadata) => metadata,
                Err(FsError::NotFound | FsError::PermissionDenied) => continue,
                Err(err) => return Err(err),
            };
            entries.push(DirEntry::new(name, metadata));
        }
        entries.sort_by(|left, right| left.name().cmp(right.name()));
        Ok(entries)
    }

    fn create_dir(&self, path: &NormalizedPath) -> FsResult<()> {
        if path.as_str() == "." {
            return Err(FsError::AlreadyExists);
        }
        let host_path = self.raw_host_path(path);
        let parent = host_path.parent().ok_or(FsError::AlreadyExists)?;
        let parent = fs::canonicalize(parent).map_err(map_io_error)?;
        if !parent.starts_with(&*self.root) {
            return Err(FsError::PermissionDenied);
        }
        fs::create_dir(host_path).map_err(map_io_error)
    }

    fn remove_file(&self, path: &NormalizedPath) -> FsResult<()> {
        let host_path = self.raw_host_path(path);
        let metadata = fs::symlink_metadata(&host_path).map_err(map_io_error)?;
        if metadata.is_dir() {
            return Err(FsError::IsDirectory);
        }
        let resolved = fs::canonicalize(&host_path).map_err(map_io_error)?;
        if !resolved.starts_with(&*self.root) {
            return Err(FsError::PermissionDenied);
        }
        fs::remove_file(host_path).map_err(map_io_error)
    }

    fn remove_dir(&self, path: &NormalizedPath) -> FsResult<()> {
        if path.as_str() == "." {
            return Err(FsError::PermissionDenied);
        }
        let host_path = self.raw_host_path(path);
        let metadata = fs::symlink_metadata(&host_path).map_err(map_io_error)?;
        if !metadata.is_dir() {
            return Err(FsError::NotDirectory);
        }
        let resolved = fs::canonicalize(&host_path).map_err(map_io_error)?;
        if !resolved.starts_with(&*self.root) {
            return Err(FsError::PermissionDenied);
        }
        fs::remove_dir(host_path).map_err(map_io_error)
    }

    fn rename(&self, old_path: &NormalizedPath, new_path: &NormalizedPath) -> FsResult<()> {
        if old_path.as_str() == "." || new_path.as_str() == "." {
            return Err(FsError::PermissionDenied);
        }

        let old_host_path = self.raw_host_path(old_path);
        let old_metadata = fs::symlink_metadata(&old_host_path).map_err(map_io_error)?;
        let old_resolved = fs::canonicalize(&old_host_path).map_err(map_io_error)?;
        if !old_resolved.starts_with(&*self.root) {
            return Err(FsError::PermissionDenied);
        }

        let new_host_path = self.raw_host_path(new_path);
        let new_parent = new_host_path.parent().ok_or(FsError::PermissionDenied)?;
        let new_parent = fs::canonicalize(new_parent).map_err(map_io_error)?;
        if !new_parent.starts_with(&*self.root) {
            return Err(FsError::PermissionDenied);
        }
        match fs::symlink_metadata(&new_host_path) {
            Ok(new_metadata) => {
                let new_resolved = fs::canonicalize(&new_host_path).map_err(map_io_error)?;
                if !new_resolved.starts_with(&*self.root) {
                    return Err(FsError::PermissionDenied);
                }
                match (old_metadata.is_dir(), new_metadata.is_dir()) {
                    (true, true) => {
                        if fs::read_dir(&new_host_path)
                            .map_err(map_io_error)?
                            .next()
                            .transpose()
                            .map_err(map_io_error)?
                            .is_some()
                        {
                            return Err(FsError::NotEmpty);
                        }
                    }
                    (true, false) => return Err(FsError::NotDirectory),
                    (false, true) => return Err(FsError::IsDirectory),
                    (false, false) => {}
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(map_io_error(error)),
        }

        fs::rename(old_host_path, new_host_path).map_err(map_io_error)
    }
}

#[derive(Debug)]
struct LocalFile {
    file: StdFile,
    offset: u64,
    readable: bool,
    writable: bool,
}

impl File for LocalFile {
    fn read(&mut self, buf: &mut [u8]) -> FsResult<usize> {
        if !self.readable {
            return Err(FsError::PermissionDenied);
        }
        let count = self.file.read(buf).map_err(map_io_error)?;
        self.offset = self
            .offset
            .checked_add(u64::try_from(count).map_err(|_| FsError::InvalidOffset)?)
            .ok_or(FsError::InvalidOffset)?;
        Ok(count)
    }

    fn write(&mut self, buf: &[u8]) -> FsResult<usize> {
        if !self.writable {
            return Err(FsError::PermissionDenied);
        }
        let count = self.file.write(buf).map_err(map_io_error)?;
        self.offset = self
            .offset
            .checked_add(u64::try_from(count).map_err(|_| FsError::InvalidOffset)?)
            .ok_or(FsError::InvalidOffset)?;
        Ok(count)
    }

    fn seek(&mut self, from: FileSeekFrom) -> FsResult<u64> {
        let offset = self.file.seek(host_seek_from(from)).map_err(map_io_error)?;
        self.offset = offset;
        Ok(offset)
    }

    fn tell(&self) -> FsResult<u64> {
        Ok(self.offset)
    }

    fn is_seekable(&self) -> bool {
        true
    }

    fn metadata(&self) -> FsResult<Metadata> {
        self.file
            .metadata()
            .map(|metadata| metadata_from_host(&metadata))
            .map_err(map_io_error)
    }
}

fn host_seek_from(from: FileSeekFrom) -> SeekFrom {
    match from {
        FileSeekFrom::Start(offset) => SeekFrom::Start(offset),
        FileSeekFrom::Current(offset) => SeekFrom::Current(offset),
        FileSeekFrom::End(offset) => SeekFrom::End(offset),
    }
}

fn metadata_from_host(metadata: &fs::Metadata) -> Metadata {
    let file_type = if metadata.is_dir() {
        FileType::Directory
    } else if metadata.is_file() {
        FileType::File
    } else if metadata.file_type().is_symlink() {
        FileType::Symlink
    } else {
        FileType::File
    };
    Metadata::new(file_type, metadata.len(), metadata_mode(metadata))
}

#[cfg(unix)]
fn metadata_mode(metadata: &fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;

    metadata.permissions().mode()
}

#[cfg(not(unix))]
fn metadata_mode(metadata: &fs::Metadata) -> u32 {
    if metadata.permissions().readonly() {
        0o444
    } else if metadata.is_dir() {
        0o755
    } else {
        0o644
    }
}

fn map_io_error(error: std::io::Error) -> FsError {
    match error.kind() {
        std::io::ErrorKind::NotFound => FsError::NotFound,
        std::io::ErrorKind::PermissionDenied => FsError::PermissionDenied,
        std::io::ErrorKind::AlreadyExists => FsError::AlreadyExists,
        std::io::ErrorKind::NotADirectory => FsError::NotDirectory,
        std::io::ErrorKind::IsADirectory => FsError::IsDirectory,
        std::io::ErrorKind::DirectoryNotEmpty => FsError::NotEmpty,
        std::io::ErrorKind::InvalidInput => FsError::InvalidOffset,
        _ => FsError::Other(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    use crate::{
        FileSeekFrom, FileSystem, FileType, FsError, LocalFs, NormalizedPath, OpenOptions,
    };

    static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

    fn path(value: &str) -> NormalizedPath {
        NormalizedPath::new(value).unwrap()
    }

    #[test]
    fn localfs_reads_lists_and_writes_inside_root() {
        let root = temp_root();
        fs::write(root.join("input.txt"), "hello host").unwrap();
        fs::create_dir(root.join("dir")).unwrap();

        let fs = LocalFs::new(&root).unwrap();
        assert!(fs.root().is_absolute());
        assert_eq!(
            fs.metadata(&NormalizedPath::new(".").unwrap())
                .unwrap()
                .file_type(),
            FileType::Directory
        );
        let entries = fs.read_dir(&NormalizedPath::new(".").unwrap()).unwrap();
        assert_eq!(
            entries.iter().map(|entry| entry.name()).collect::<Vec<_>>(),
            vec!["dir", "input.txt"]
        );

        let mut input = fs
            .open(
                &NormalizedPath::new("input.txt").unwrap(),
                OpenOptions::read(),
            )
            .unwrap();
        let mut buf = [0; 32];
        let count = input.read(&mut buf).unwrap();
        assert_eq!(&buf[..count], b"hello host");
        assert!(input.is_seekable());
        assert_eq!(input.tell().unwrap(), 10);
        assert_eq!(input.seek(FileSeekFrom::Start(6)).unwrap(), 6);

        let mut output = fs
            .open(
                &NormalizedPath::new("dir/out.txt").unwrap(),
                OpenOptions {
                    write: true,
                    create: true,
                    truncate: true,
                    ..OpenOptions::default()
                },
            )
            .unwrap();
        assert_eq!(output.write(b"from wanix").unwrap(), 10);
        assert_eq!(fs::read(root.join("dir/out.txt")).unwrap(), b"from wanix");

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn localfs_remove_file_deletes_files_inside_root() {
        let root = temp_root();
        fs::write(root.join("input.txt"), "hello host").unwrap();
        fs::create_dir(root.join("dir")).unwrap();
        let fs = LocalFs::new(&root).unwrap();

        fs.remove_file(&NormalizedPath::new("input.txt").unwrap())
            .unwrap();

        assert!(!root.join("input.txt").exists());
        assert_eq!(
            fs.remove_file(&NormalizedPath::new("input.txt").unwrap()),
            Err(FsError::NotFound)
        );
        assert_eq!(
            fs.remove_file(&NormalizedPath::new("dir").unwrap()),
            Err(FsError::IsDirectory)
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn localfs_remove_dir_deletes_empty_directories_inside_root() {
        let root = temp_root();
        fs::write(root.join("file.txt"), "hello host").unwrap();
        fs::create_dir(root.join("empty")).unwrap();
        fs::create_dir(root.join("nonempty")).unwrap();
        fs::write(root.join("nonempty/file.txt"), "nested").unwrap();
        let fs = LocalFs::new(&root).unwrap();

        fs.remove_dir(&NormalizedPath::new("empty").unwrap())
            .unwrap();

        assert!(!root.join("empty").exists());
        assert_eq!(
            fs.remove_dir(&NormalizedPath::new("nonempty").unwrap()),
            Err(FsError::NotEmpty)
        );
        assert_eq!(
            fs.remove_dir(&NormalizedPath::new("file.txt").unwrap()),
            Err(FsError::NotDirectory)
        );
        assert_eq!(
            fs.remove_dir(&NormalizedPath::new("missing").unwrap()),
            Err(FsError::NotFound)
        );
        assert_eq!(
            fs.remove_dir(&NormalizedPath::new(".").unwrap()),
            Err(FsError::PermissionDenied)
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn localfs_create_dir_creates_one_directory_inside_root() {
        let root = temp_root();
        fs::write(root.join("file.txt"), "hello host").unwrap();
        fs::create_dir(root.join("parent")).unwrap();
        let fs = LocalFs::new(&root).unwrap();

        fs.create_dir(&NormalizedPath::new("parent/child").unwrap())
            .unwrap();

        assert!(root.join("parent/child").is_dir());
        assert_eq!(
            fs.create_dir(&NormalizedPath::new("parent/child").unwrap()),
            Err(FsError::AlreadyExists)
        );
        assert_eq!(
            fs.create_dir(&NormalizedPath::new("missing/child").unwrap()),
            Err(FsError::NotFound)
        );
        assert_eq!(
            fs.create_dir(&NormalizedPath::new("file.txt/child").unwrap()),
            Err(FsError::NotDirectory)
        );
        assert_eq!(
            fs.create_dir(&NormalizedPath::new(".").unwrap()),
            Err(FsError::AlreadyExists)
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn localfs_rename_moves_files_and_directory_subtrees_inside_root() {
        let root = temp_root();
        fs::write(root.join("old.txt"), "old").unwrap();
        fs::write(root.join("target.txt"), "target").unwrap();
        fs::create_dir_all(root.join("dir/sub")).unwrap();
        fs::write(root.join("dir/sub/file.txt"), "nested").unwrap();
        fs::create_dir(root.join("empty")).unwrap();
        fs::write(root.join("file.txt"), "file").unwrap();
        fs::create_dir(root.join("nonempty")).unwrap();
        fs::write(root.join("nonempty/file.txt"), "busy").unwrap();
        let fs = LocalFs::new(&root).unwrap();

        fs.rename(&path("old.txt"), &path("renamed.txt")).unwrap();
        assert!(!root.join("old.txt").exists());
        assert_eq!(std::fs::read(root.join("renamed.txt")).unwrap(), b"old");

        fs.rename(&path("renamed.txt"), &path("target.txt"))
            .unwrap();
        assert_eq!(std::fs::read(root.join("target.txt")).unwrap(), b"old");

        fs.rename(&path("dir"), &path("empty")).unwrap();
        assert!(!root.join("dir").exists());
        assert_eq!(
            std::fs::read(root.join("empty/sub/file.txt")).unwrap(),
            b"nested"
        );

        assert_eq!(
            fs.rename(&path("file.txt"), &path("empty")),
            Err(FsError::IsDirectory)
        );
        assert_eq!(
            fs.rename(&path("empty"), &path("file.txt")),
            Err(FsError::NotDirectory)
        );
        assert_eq!(
            fs.rename(&path("empty"), &path("nonempty")),
            Err(FsError::NotEmpty)
        );
        assert_eq!(
            fs.rename(&path("missing"), &path("missing")),
            Err(FsError::NotFound)
        );
        assert_eq!(
            fs.rename(&path("."), &path("root")),
            Err(FsError::PermissionDenied)
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn localfs_create_requires_existing_parent_inside_root() {
        let root = temp_root();
        let fs = LocalFs::new(&root).unwrap();

        let error = fs
            .open(
                &NormalizedPath::new("missing/out.txt").unwrap(),
                OpenOptions {
                    write: true,
                    create: true,
                    ..OpenOptions::default()
                },
            )
            .err()
            .unwrap();
        assert_eq!(error, FsError::NotFound);

        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn localfs_rejects_symlink_escape_from_root() {
        use std::os::unix::fs::symlink;

        let root = temp_root();
        let outside = temp_root();
        fs::write(outside.join("secret.txt"), "outside").unwrap();
        symlink(outside.join("secret.txt"), root.join("secret-link")).unwrap();
        symlink(outside.join("missing.txt"), root.join("broken-link")).unwrap();

        let fs = LocalFs::new(&root).unwrap();
        let error = fs
            .open(
                &NormalizedPath::new("secret-link").unwrap(),
                OpenOptions::read(),
            )
            .err()
            .unwrap();
        assert_eq!(error, FsError::PermissionDenied);
        assert!(
            fs.read_dir(&NormalizedPath::new(".").unwrap())
                .unwrap()
                .iter()
                .all(|entry| entry.name() != "secret-link")
        );
        let error = fs
            .open(
                &NormalizedPath::new("broken-link").unwrap(),
                OpenOptions {
                    write: true,
                    create: true,
                    ..OpenOptions::default()
                },
            )
            .err()
            .unwrap();
        assert_eq!(error, FsError::NotFound);
        assert!(!outside.join("missing.txt").exists());

        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(outside).unwrap();
    }

    fn temp_root() -> std::path::PathBuf {
        let mut path = std::env::temp_dir();
        let nonce = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
        path.push(format!("wanix-localfs-test-{}-{nonce}", std::process::id()));
        fs::create_dir_all(&path).unwrap();
        path
    }
}
