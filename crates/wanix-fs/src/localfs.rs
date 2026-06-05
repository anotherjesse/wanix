use std::fs::{self, File as StdFile, FileTimes};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::{
    DirEntry, File, FileSystem, FsError, FsResult, Metadata, MetadataLookup, NormalizedPath,
    OpenOptions,
};

mod file;
mod host;
mod paths;
mod readdir;

use file::LocalFile;
use host::{
    create_symlink, metadata_from_host, pathbuf_into_bytes, set_host_permissions,
    system_time_from_ns,
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
        Ok(Box::new(LocalFile::new(file, options)))
    }

    fn metadata(&self, path: &NormalizedPath) -> FsResult<Metadata> {
        self.metadata_with_lookup(path, MetadataLookup::FollowSymlink)
    }

    fn metadata_with_lookup(
        &self,
        path: &NormalizedPath,
        lookup: MetadataLookup,
    ) -> FsResult<Metadata> {
        let host_path = self.host_path_for_metadata(path, lookup)?;
        let metadata = fs::symlink_metadata(host_path).map_err(map_io_error)?;
        Ok(metadata_from_host(&metadata))
    }

    fn read_dir(&self, path: &NormalizedPath) -> FsResult<Vec<DirEntry>> {
        self.read_host_dir(path)
    }

    fn read_link(&self, path: &NormalizedPath) -> FsResult<Vec<u8>> {
        let host_path = self.host_path_for_final_component_operation(path)?;
        let metadata = fs::symlink_metadata(&host_path).map_err(map_io_error)?;
        if !metadata.file_type().is_symlink() {
            return Err(FsError::InvalidPath(format!("{path} is not a symlink")));
        }
        pathbuf_into_bytes(fs::read_link(host_path).map_err(map_io_error)?)
    }

    fn symlink(&self, target: &[u8], path: &NormalizedPath) -> FsResult<()> {
        let host_path = self.host_path_for_final_component_operation(path)?;
        create_symlink(target, &host_path)
    }

    fn hard_link(&self, old_path: &NormalizedPath, new_path: &NormalizedPath) -> FsResult<()> {
        if old_path.as_str() == "." || new_path.as_str() == "." {
            return Err(FsError::PermissionDenied);
        }

        let old_host_path = self.existing_host_path(old_path)?;
        if fs::metadata(&old_host_path).map_err(map_io_error)?.is_dir() {
            return Err(FsError::IsDirectory);
        }

        let new_host_path = self.host_path_for_final_component_operation(new_path)?;
        match fs::symlink_metadata(&new_host_path) {
            Ok(_) => return Err(FsError::AlreadyExists),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(map_io_error(error)),
        }
        fs::hard_link(old_host_path, new_host_path).map_err(map_io_error)
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

        let (old_host_path, old_metadata) = self.rename_source(old_path)?;
        let new_host_path = self.rename_destination(new_path, &old_metadata)?;

        fs::rename(old_host_path, new_host_path).map_err(map_io_error)
    }

    fn set_permissions(&self, path: &NormalizedPath, permissions: u32) -> FsResult<()> {
        let host_path = self.existing_host_path(path)?;
        set_host_permissions(&host_path, permissions)
    }

    fn set_times(
        &self,
        path: &NormalizedPath,
        accessed_time_ns: u64,
        modified_time_ns: u64,
    ) -> FsResult<()> {
        let host_path = self.existing_host_path(path)?;
        let file = StdFile::open(&host_path).map_err(map_io_error)?;
        let times = FileTimes::new()
            .set_accessed(system_time_from_ns(accessed_time_ns)?)
            .set_modified(system_time_from_ns(modified_time_ns)?);
        file.set_times(times).map_err(map_timestamp_io_error)
    }
}

pub(super) fn map_io_error(error: std::io::Error) -> FsError {
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

fn map_timestamp_io_error(error: std::io::Error) -> FsError {
    match map_io_error(error) {
        FsError::InvalidOffset => FsError::InvalidTime,
        error => error,
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    use crate::{
        FileSeekFrom, FileSystem, FileType, FsError, LocalFs, MetadataLookup, NormalizedPath,
        OpenOptions,
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
    #[cfg(unix)]
    fn localfs_hard_link_creates_second_name_inside_root() {
        let root = temp_root();
        fs::write(root.join("source.txt"), b"source").unwrap();
        fs::create_dir(root.join("dir")).unwrap();
        fs::write(root.join("existing.txt"), b"existing").unwrap();
        let fs = LocalFs::new(&root).unwrap();

        fs.hard_link(&path("source.txt"), &path("hard.txt"))
            .unwrap();

        assert_eq!(std::fs::read(root.join("hard.txt")).unwrap(), b"source");
        assert_eq!(fs.metadata(&path("source.txt")).unwrap().link_count(), 2);
        assert_eq!(fs.metadata(&path("hard.txt")).unwrap().link_count(), 2);
        std::fs::write(root.join("hard.txt"), b"updated").unwrap();
        assert_eq!(std::fs::read(root.join("source.txt")).unwrap(), b"updated");

        assert_eq!(
            fs.hard_link(&path("missing.txt"), &path("missing-hard.txt")),
            Err(FsError::NotFound)
        );
        assert_eq!(
            fs.hard_link(&path("dir"), &path("dir-hard")),
            Err(FsError::IsDirectory)
        );
        assert_eq!(
            fs.hard_link(&path("source.txt"), &path("existing.txt")),
            Err(FsError::AlreadyExists)
        );
        assert_eq!(
            fs.hard_link(&path("source.txt"), &path(".")),
            Err(FsError::PermissionDenied)
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn localfs_set_times_updates_host_file_metadata() {
        let root = temp_root();
        fs::write(root.join("stamp.txt"), "stamp").unwrap();
        let fs = LocalFs::new(&root).unwrap();

        fs.set_times(&path("stamp.txt"), 1_000_000_000, 2_000_000_000)
            .unwrap();

        let metadata = fs.metadata(&path("stamp.txt")).unwrap();
        assert_eq!(metadata.accessed_time_ns(), 1_000_000_000);
        assert_eq!(metadata.modified_time_ns(), 2_000_000_000);
        assert_eq!(
            fs.set_times(&path("missing.txt"), 1_000_000_000, 2_000_000_000),
            Err(FsError::NotFound)
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[cfg(unix)]
    fn localfs_set_permissions_updates_host_file_mode() {
        use std::os::unix::fs::PermissionsExt;

        let root = temp_root();
        fs::write(root.join("mode.txt"), "mode").unwrap();
        let fs = LocalFs::new(&root).unwrap();

        fs.set_permissions(&path("mode.txt"), 0o100600).unwrap();

        assert_eq!(
            fs::metadata(root.join("mode.txt"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert_eq!(
            fs.metadata(&path("mode.txt")).unwrap().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs.set_permissions(&path("missing.txt"), 0o600),
            Err(FsError::NotFound)
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn localfs_file_set_len_updates_host_file() {
        let root = temp_root();
        fs::write(root.join("file.txt"), "hello").unwrap();
        let fs = LocalFs::new(&root).unwrap();
        let path = path("file.txt");

        let mut file = fs.open(&path, OpenOptions::read_write()).unwrap();
        assert_eq!(file.seek(FileSeekFrom::Start(4)).unwrap(), 4);
        file.set_len(8).unwrap();
        assert_eq!(file.tell().unwrap(), 4);
        assert_eq!(fs::read(root.join("file.txt")).unwrap(), b"hello\0\0\0");

        file.set_len(2).unwrap();
        assert_eq!(file.tell().unwrap(), 4);
        assert_eq!(fs::read(root.join("file.txt")).unwrap(), b"he");

        let mut read_only = fs.open(&path, OpenOptions::read()).unwrap();
        assert_eq!(read_only.set_len(1), Err(FsError::PermissionDenied));

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

    #[cfg(unix)]
    #[test]
    fn localfs_metadata_lookup_controls_final_symlink_following() {
        use std::os::unix::fs::symlink;

        let root = temp_root();
        let outside = temp_root();
        fs::write(root.join("target.txt"), "inside").unwrap();
        fs::write(outside.join("secret.txt"), "outside").unwrap();
        symlink("target.txt", root.join("inside-link")).unwrap();
        symlink(outside.join("secret.txt"), root.join("outside-link")).unwrap();
        symlink(outside.join("missing.txt"), root.join("broken-link")).unwrap();
        symlink(&outside, root.join("dir-link")).unwrap();

        let fs = LocalFs::new(&root).unwrap();

        assert_eq!(
            fs.metadata(&path("inside-link")).unwrap().file_type(),
            FileType::File
        );
        assert_eq!(
            fs.metadata_with_lookup(&path("inside-link"), MetadataLookup::NoFollow)
                .unwrap()
                .file_type(),
            FileType::Symlink
        );
        assert_eq!(
            fs.metadata_with_lookup(&path("outside-link"), MetadataLookup::NoFollow)
                .unwrap()
                .file_type(),
            FileType::Symlink
        );
        assert_eq!(
            fs.metadata_with_lookup(&path("outside-link"), MetadataLookup::FollowSymlink),
            Err(FsError::PermissionDenied)
        );
        assert_eq!(
            fs.metadata_with_lookup(&path("broken-link"), MetadataLookup::NoFollow)
                .unwrap()
                .file_type(),
            FileType::Symlink
        );
        assert_eq!(
            fs.metadata_with_lookup(&path("broken-link"), MetadataLookup::FollowSymlink),
            Err(FsError::NotFound)
        );
        assert_eq!(
            fs.metadata_with_lookup(&path("dir-link/secret.txt"), MetadataLookup::NoFollow),
            Err(FsError::PermissionDenied)
        );

        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(outside).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn localfs_read_link_and_symlink_preserve_mount_boundary() {
        use std::os::unix::fs::symlink;

        let root = temp_root();
        let outside = temp_root();
        fs::write(root.join("target.txt"), "inside").unwrap();
        fs::write(outside.join("secret.txt"), "outside").unwrap();
        symlink(outside.join("secret.txt"), root.join("outside-link")).unwrap();
        symlink(&outside, root.join("dir-link")).unwrap();

        let fs = LocalFs::new(&root).unwrap();

        fs.symlink(b"target.txt", &path("created-link")).unwrap();
        assert_eq!(fs.read_link(&path("created-link")).unwrap(), b"target.txt");
        assert_eq!(
            fs.metadata_with_lookup(&path("created-link"), MetadataLookup::NoFollow)
                .unwrap()
                .file_type(),
            FileType::Symlink
        );
        let mut opened = fs.open(&path("created-link"), OpenOptions::read()).unwrap();
        let mut bytes = [0; 16];
        let count = opened.read(&mut bytes).unwrap();
        assert_eq!(&bytes[..count], b"inside");

        let outside_target = fs.read_link(&path("outside-link")).unwrap();
        assert!(String::from_utf8_lossy(&outside_target).contains("secret.txt"));
        assert_eq!(
            fs.open(&path("outside-link"), OpenOptions::read()).err(),
            Some(FsError::PermissionDenied)
        );
        assert_eq!(
            fs.read_link(&path("dir-link/secret.txt")),
            Err(FsError::PermissionDenied)
        );
        assert_eq!(
            fs.symlink(b"new.txt", &path("dir-link/new-link")),
            Err(FsError::PermissionDenied)
        );
        assert_eq!(
            fs.read_link(&path("target.txt")),
            Err(FsError::InvalidPath(
                "target.txt is not a symlink".to_owned()
            ))
        );

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
