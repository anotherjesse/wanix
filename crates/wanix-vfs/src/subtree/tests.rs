use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use wanix_fs::{FileSystem, FsError, LocalFs, MemFs, NormalizedPath, OpenOptions};

use super::{Rights, SubtreeFs};

fn path(value: &str) -> NormalizedPath {
    NormalizedPath::new(value).unwrap()
}

fn backing() -> Arc<MemFs> {
    let fs = Arc::new(MemFs::new());
    fs.create_dir_all("projects/foo").unwrap();
    fs.write_file("projects/foo/file.txt", b"inside").unwrap();
    fs.create_dir_all("docs").unwrap();
    fs.write_file("docs/secret.txt", b"outside").unwrap();
    fs
}

#[test]
fn read_only_subtree_reads_inside_prefix() {
    let fs: Arc<dyn FileSystem> = backing();
    let subtree = SubtreeFs::new(fs, "projects/foo", Rights::read_only()).unwrap();

    let mut file = subtree
        .open(&path("file.txt"), OpenOptions::read())
        .unwrap();
    let mut buf = [0u8; 6];
    let read = file.read(&mut buf).unwrap();
    assert_eq!(&buf[..read], b"inside");

    let entries = subtree.read_dir(&NormalizedPath::root()).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name(), "file.txt");
}

#[test]
fn rebase_keeps_paths_inside_the_prefix() {
    let fs: Arc<dyn FileSystem> = backing();
    let subtree = SubtreeFs::new(fs, "projects/foo", Rights::read_only()).unwrap();

    // The subtree root maps to the prefix, not the backing root.
    assert_eq!(
        subtree.rebase(&NormalizedPath::root()).unwrap().as_str(),
        "projects/foo"
    );
    assert_eq!(
        subtree.rebase(&path("file.txt")).unwrap().as_str(),
        "projects/foo/file.txt"
    );

    // A NormalizedPath can never carry `..`, so escaping the prefix is
    // unrepresentable: the sibling `docs` tree is simply not addressable.
    assert!(NormalizedPath::new("../docs/secret.txt").is_err());
    assert!(matches!(
        subtree.metadata(&path("docs/secret.txt")),
        Err(FsError::NotFound)
    ));
}

#[test]
fn dot_prefix_reroots_at_backing_root() {
    let fs: Arc<dyn FileSystem> = backing();
    let subtree = SubtreeFs::new(fs, ".", Rights::read_only()).unwrap();

    assert_eq!(
        subtree.rebase(&NormalizedPath::root()).unwrap().as_str(),
        "."
    );
    assert_eq!(
        subtree.rebase(&path("docs/secret.txt")).unwrap().as_str(),
        "docs/secret.txt"
    );
    assert!(subtree.metadata(&path("docs/secret.txt")).is_ok());
}

#[test]
fn invalid_prefix_is_rejected() {
    let fs: Arc<dyn FileSystem> = backing();
    assert!(matches!(
        SubtreeFs::new(fs, "../escape", Rights::read_write()),
        Err(FsError::InvalidPath(_))
    ));
}

#[test]
fn every_mutation_on_read_only_subtree_is_permission_denied() {
    let fs: Arc<dyn FileSystem> = backing();
    let subtree = SubtreeFs::new(fs, "projects/foo", Rights::read_only()).unwrap();

    // Opening for write requires the write right.
    assert_eq!(
        subtree
            .open(&path("file.txt"), OpenOptions::read_write())
            .err(),
        Some(FsError::PermissionDenied)
    );
    assert_eq!(
        subtree
            .open(
                &path("new.txt"),
                OpenOptions {
                    create: true,
                    write: true,
                    ..OpenOptions::default()
                }
            )
            .err(),
        Some(FsError::PermissionDenied)
    );

    assert_eq!(
        subtree.symlink(b"file.txt", &path("link")).err(),
        Some(FsError::PermissionDenied)
    );
    assert_eq!(
        subtree
            .hard_link(&path("file.txt"), &path("hard.txt"))
            .err(),
        Some(FsError::PermissionDenied)
    );
    assert_eq!(
        subtree.create_dir(&path("sub")).err(),
        Some(FsError::PermissionDenied)
    );
    assert_eq!(
        subtree.remove_file(&path("file.txt")).err(),
        Some(FsError::PermissionDenied)
    );
    assert_eq!(
        subtree.remove_dir(&path("sub")).err(),
        Some(FsError::PermissionDenied)
    );
    assert_eq!(
        subtree
            .rename(&path("file.txt"), &path("renamed.txt"))
            .err(),
        Some(FsError::PermissionDenied)
    );
    assert_eq!(
        subtree.set_permissions(&path("file.txt"), 0o600).err(),
        Some(FsError::PermissionDenied)
    );
    assert_eq!(
        subtree.set_times(&path("file.txt"), 1, 2).err(),
        Some(FsError::PermissionDenied)
    );

    // The backing store must be untouched.
    assert_eq!(
        backing_unchanged(&subtree),
        b"inside",
        "read-only subtree must not have mutated the backing file"
    );
}

fn backing_unchanged(subtree: &SubtreeFs) -> Vec<u8> {
    let mut file = subtree
        .open(&path("file.txt"), OpenOptions::read())
        .unwrap();
    let mut buf = Vec::new();
    let mut chunk = [0u8; 32];
    loop {
        let read = file.read(&mut chunk).unwrap();
        if read == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..read]);
    }
    buf
}

#[test]
fn read_write_subtree_allows_scoped_mutation() {
    let fs: Arc<dyn FileSystem> = backing();
    let subtree = SubtreeFs::new(fs, "projects/foo", Rights::read_write()).unwrap();

    subtree.create_dir(&path("sub")).unwrap();
    let mut file = subtree
        .open(
            &path("sub/new.txt"),
            OpenOptions {
                create: true,
                write: true,
                read: true,
                ..OpenOptions::default()
            },
        )
        .unwrap();
    assert_eq!(file.write(b"hi").unwrap(), 2);
    assert_eq!(subtree.metadata(&path("sub/new.txt")).unwrap().len(), 2);
}

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

fn temp_root() -> std::path::PathBuf {
    let mut dir = std::env::temp_dir();
    let nonce = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
    dir.push(format!("wanix-subtree-test-{}-{nonce}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[cfg(unix)]
#[test]
fn localfs_backed_subtree_denies_symlink_escape_of_prefix() {
    use std::os::unix::fs::symlink;

    // POC layout: a single LocalFs root that contains both the granted
    // `projects/foo` subtree and a sibling `docs/secret.txt` that is NOT
    // granted. A symlink planted inside the granted prefix points up and over
    // into the sibling — staying inside the LocalFs root, so LocalFs's own
    // root confinement does not catch it, but escaping the SubtreeFs prefix.
    let root = temp_root();
    std::fs::create_dir_all(root.join("projects/foo")).unwrap();
    std::fs::write(root.join("projects/foo/file.txt"), b"inside").unwrap();
    std::fs::create_dir_all(root.join("docs")).unwrap();
    std::fs::write(root.join("docs/secret.txt"), b"TOP SECRET - not granted").unwrap();
    symlink("../../docs/secret.txt", root.join("projects/foo/escape")).unwrap();

    let local: Arc<dyn FileSystem> = Arc::new(LocalFs::new(&root).unwrap());
    let subtree = SubtreeFs::new(local, "projects/foo", Rights::read_only()).unwrap();

    // The in-prefix file is reachable; the escape symlink is not.
    let mut file = subtree
        .open(&path("file.txt"), OpenOptions::read())
        .unwrap();
    let mut buf = [0u8; 6];
    let read = file.read(&mut buf).unwrap();
    assert_eq!(&buf[..read], b"inside");

    // Opening the escape link must NOT yield the out-of-prefix secret.
    assert_eq!(
        subtree.open(&path("escape"), OpenOptions::read()).err(),
        Some(FsError::PermissionDenied),
        "symlink whose target leaves the granted prefix must be denied"
    );
    assert_eq!(
        subtree.metadata(&path("escape")).err(),
        Some(FsError::PermissionDenied),
        "following metadata through the escape link must be denied"
    );

    // The opaque link target is still readable (it does not dereference), and a
    // NoFollow stat reports the link itself without escaping.
    assert_eq!(
        subtree.read_link(&path("escape")).unwrap(),
        b"../../docs/secret.txt"
    );
    assert_eq!(
        subtree
            .metadata_with_lookup(&path("escape"), wanix_fs::MetadataLookup::NoFollow)
            .unwrap()
            .file_type(),
        wanix_fs::FileType::Symlink
    );

    std::fs::remove_dir_all(&root).unwrap();
}

#[cfg(unix)]
#[test]
fn localfs_backed_subtree_allows_in_prefix_symlink() {
    use std::os::unix::fs::symlink;

    // A symlink that stays inside the granted prefix is followed normally.
    let root = temp_root();
    std::fs::create_dir_all(root.join("projects/foo")).unwrap();
    std::fs::write(root.join("projects/foo/file.txt"), b"inside").unwrap();
    symlink("file.txt", root.join("projects/foo/alias")).unwrap();

    let local: Arc<dyn FileSystem> = Arc::new(LocalFs::new(&root).unwrap());
    let subtree = SubtreeFs::new(local, "projects/foo", Rights::read_only()).unwrap();

    let mut file = subtree.open(&path("alias"), OpenOptions::read()).unwrap();
    let mut buf = [0u8; 6];
    let read = file.read(&mut buf).unwrap();
    assert_eq!(&buf[..read], b"inside");

    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn no_rights_denies_even_reads() {
    let fs: Arc<dyn FileSystem> = backing();
    let subtree = SubtreeFs::new(fs, "projects/foo", Rights::none()).unwrap();

    assert_eq!(
        subtree.metadata(&path("file.txt")).err(),
        Some(FsError::PermissionDenied)
    );
    assert_eq!(
        subtree.read_dir(&NormalizedPath::root()).err(),
        Some(FsError::PermissionDenied)
    );
    assert_eq!(
        subtree.open(&path("file.txt"), OpenOptions::read()).err(),
        Some(FsError::PermissionDenied)
    );
}
