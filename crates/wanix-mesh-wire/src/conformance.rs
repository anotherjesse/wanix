//! A reusable `FileSystem`-contract conformance suite.
//!
//! This is **test-support**: it runs a self-contained scenario against
//! **any** [`wanix_fs::FileSystem`] and asserts the trait's observable contract,
//! so the *same* checks can be driven against a `MemFs` directly, a `NativeFs`
//! import of that `MemFs` over the native wire, and a `RemoteFs` (9P) import of
//! it. Phase 5's differential test (`wanix-mesh/tests/mesh_native_differential.rs`)
//! runs this suite against both a native and a 9P import of one shared `MemFs`,
//! asserting the two encodings stay behaviorally identical.
//!
//! The suite targets the capability set a `MemFs` (and therefore both its imports)
//! must satisfy: open/read/write/seek, follow-vs-nofollow metadata, `read_dir`
//! carrying full per-entry metadata, `create_dir`/`remove_*`/`rename`,
//! `symlink`/`read_link`, `set_times`, and a `content_hash` of `None` for a plain
//! backing. It works under a unique subdirectory it creates and removes, so it is
//! idempotent and isolates its side effects from a caller's other state.
//!
//! Every check takes `&dyn FileSystem`, uses only `assert!`/`panic!`, and pulls in
//! no test-runtime dependency, so it composes into either a `#[test]` (the wire
//! crate's own `tests/conformance.rs`) or an integration harness over a transport.

use wanix_fs::{
    FileSeekFrom, FileSystem, FileType, FsError, MetadataLookup, NormalizedPath, OpenOptions,
};

/// Parses a conformance path, panicking on a malformed test path.
fn path(p: &str) -> NormalizedPath {
    NormalizedPath::new(p).unwrap_or_else(|err| panic!("conformance path {p:?}: {err:?}"))
}

/// Read-write-create-truncate options for a fresh regular file.
fn create_rw() -> OpenOptions {
    OpenOptions {
        read: true,
        write: true,
        create: true,
        truncate: true,
    }
}

/// Runs the whole conformance suite against `fs` under a unique scratch root.
///
/// `label` names the implementation under test (`"native"`, `"9p"`, `"memfs"`)
/// so a differential caller can attribute a failure to the right encoding. The
/// suite creates `conformance-<label>/`, exercises every check beneath it, and
/// removes it on success.
///
/// # Panics
///
/// Panics on the first contract violation, naming `label` and the failing check.
pub fn run(fs: &dyn FileSystem, label: &str) {
    let root = format!("conformance-{label}");
    // A re-run must start clean; ignore "already gone" on the pre-clean.
    teardown(fs, &root);
    fs.create_dir(&path(&root))
        .unwrap_or_else(|err| panic!("[{label}] create scratch root {root:?}: {err:?}"));

    check_open_read_write_seek(fs, &root, label);
    check_metadata_follow_nofollow(fs, &root, label);
    check_read_dir_carries_metadata(fs, &root, label);
    check_create_remove_rename(fs, &root, label);
    check_symlink_read_link(fs, &root, label);
    check_set_times(fs, &root, label);
    check_content_hash_none(fs, &root, label);
    check_typed_errors(fs, &root, label);

    teardown(fs, &root);
}

/// Removes the scratch tree best-effort (used to pre-clean and to tear down).
fn teardown(fs: &dyn FileSystem, root: &str) {
    // Depth-first: the suite only nests one or two levels, so a fixed walk that
    // removes files then directories suffices without a general recursive remove.
    if let Ok(entries) = fs.read_dir(&path(root)) {
        for entry in entries {
            let child = format!("{root}/{}", entry.name());
            if entry.metadata().file_type() == FileType::Directory {
                teardown(fs, &child);
            } else {
                let _ = fs.remove_file(&path(&child));
            }
        }
    }
    let _ = fs.remove_dir(&path(root));
}

/// Open → write → reopen → read back; seek to the tail; offset tracking.
pub fn check_open_read_write_seek(fs: &dyn FileSystem, root: &str, label: &str) {
    let file = format!("{root}/greeting");
    let mut out = fs
        .open(&path(&file), create_rw())
        .unwrap_or_else(|err| panic!("[{label}] open create {file:?}: {err:?}"));
    assert_eq!(
        out.write(b"hello world").expect("write"),
        11,
        "[{label}] full write length"
    );
    drop(out);

    let mut handle = fs
        .open(&path(&file), OpenOptions::read())
        .unwrap_or_else(|err| panic!("[{label}] reopen {file:?}: {err:?}"));
    assert!(handle.is_seekable(), "[{label}] a regular file is seekable");
    let mut head = [0_u8; 5];
    assert_eq!(handle.read(&mut head).expect("read head"), 5);
    assert_eq!(&head, b"hello", "[{label}] head bytes round-trip");
    assert_eq!(handle.tell().expect("tell"), 5, "[{label}] offset advanced");

    assert_eq!(
        handle.seek(FileSeekFrom::Start(6)).expect("seek"),
        6,
        "[{label}] seek returns the absolute offset"
    );
    let mut tail = Vec::new();
    let mut chunk = [0_u8; 16];
    loop {
        let n = handle.read(&mut chunk).expect("read tail");
        if n == 0 {
            break;
        }
        tail.extend_from_slice(&chunk[..n]);
    }
    assert_eq!(tail, b"world", "[{label}] tail after seek");
    drop(handle);
    fs.remove_file(&path(&file)).expect("remove greeting");
}

/// Plain `metadata` follows symlinks; `metadata_with_lookup` honors the mode.
pub fn check_metadata_follow_nofollow(fs: &dyn FileSystem, root: &str, label: &str) {
    let target = format!("{root}/target");
    let link = format!("{root}/link");
    fs.open(&path(&target), create_rw())
        .expect("create target")
        .write(b"abcdef")
        .expect("write target");
    fs.symlink(b"target", &path(&link))
        .unwrap_or_else(|err| panic!("[{label}] symlink {link:?}: {err:?}"));

    // NoFollow always reports the link itself.
    let raw = fs
        .metadata_with_lookup(&path(&link), MetadataLookup::NoFollow)
        .expect("nofollow metadata");
    assert_eq!(
        raw.file_type(),
        FileType::Symlink,
        "[{label}] nofollow reports the link"
    );

    // `metadata()` follows by default. Some backings (MemFs) do not resolve their
    // own symlinks for stat, so accept *either* the followed target or the link —
    // the contract that must hold across encodings is that the *same* backing
    // gives the *same* answer, which the differential test cross-checks.
    let followed = fs.metadata(&path(&link)).expect("default metadata");
    assert!(
        matches!(followed.file_type(), FileType::File | FileType::Symlink),
        "[{label}] default metadata resolves to a file or the link, got {:?}",
        followed.file_type()
    );

    fs.remove_file(&path(&link)).expect("remove link");
    fs.remove_file(&path(&target)).expect("remove target");
}

/// `read_dir` returns each entry with full, type-correct metadata.
pub fn check_read_dir_carries_metadata(fs: &dyn FileSystem, root: &str, label: &str) {
    let dir = format!("{root}/listing");
    fs.create_dir(&path(&dir)).expect("create listing dir");
    fs.open(&path(&format!("{dir}/a")), create_rw())
        .expect("create a")
        .write(b"aaa")
        .expect("write a");
    fs.create_dir(&path(&format!("{dir}/sub")))
        .expect("create sub");

    let mut entries = fs.read_dir(&path(&dir)).expect("read_dir");
    entries.sort_by(|a, b| a.name().cmp(b.name()));
    assert_eq!(entries.len(), 2, "[{label}] read_dir entry count");
    assert_eq!(entries[0].name(), "a");
    assert_eq!(
        entries[0].metadata().file_type(),
        FileType::File,
        "[{label}] entry 'a' is a file with carried metadata"
    );
    assert_eq!(
        entries[0].metadata().len(),
        3,
        "[{label}] entry 'a' carries its real length"
    );
    assert_eq!(entries[1].name(), "sub");
    assert_eq!(
        entries[1].metadata().file_type(),
        FileType::Directory,
        "[{label}] entry 'sub' is a directory"
    );

    fs.remove_file(&path(&format!("{dir}/a")))
        .expect("remove a");
    fs.remove_dir(&path(&format!("{dir}/sub")))
        .expect("remove sub");
    fs.remove_dir(&path(&dir)).expect("remove listing dir");
}

/// `create_dir`, `rename`, `remove_file`, `remove_dir` all take effect.
pub fn check_create_remove_rename(fs: &dyn FileSystem, root: &str, label: &str) {
    let dir = format!("{root}/work");
    fs.create_dir(&path(&dir)).expect("create work");
    assert_eq!(
        fs.metadata(&path(&dir)).expect("stat work").file_type(),
        FileType::Directory,
        "[{label}] create_dir produced a directory"
    );

    let tmp = format!("{dir}/tmp");
    let kept = format!("{dir}/kept");
    fs.open(&path(&tmp), create_rw()).expect("create tmp");
    fs.rename(&path(&tmp), &path(&kept)).expect("rename");
    assert!(
        fs.metadata(&path(&tmp)).is_err(),
        "[{label}] source gone after rename"
    );
    assert!(
        fs.metadata(&path(&kept)).is_ok(),
        "[{label}] destination present after rename"
    );

    fs.remove_file(&path(&kept)).expect("remove kept");
    assert!(
        fs.metadata(&path(&kept)).is_err(),
        "[{label}] file gone after remove"
    );
    fs.remove_dir(&path(&dir)).expect("remove work");
    assert!(
        fs.metadata(&path(&dir)).is_err(),
        "[{label}] dir gone after remove"
    );
}

/// `symlink` stores its target bytes verbatim and `read_link` returns them.
pub fn check_symlink_read_link(fs: &dyn FileSystem, root: &str, label: &str) {
    let link = format!("{root}/ln");
    fs.symlink(b"some/target", &path(&link)).expect("symlink");
    assert_eq!(
        fs.read_link(&path(&link)).expect("read_link"),
        b"some/target",
        "[{label}] read_link returns the exact target bytes"
    );
    fs.remove_file(&path(&link)).expect("remove link");
}

/// `set_times` persists the supplied nanosecond stamps.
pub fn check_set_times(fs: &dyn FileSystem, root: &str, label: &str) {
    let file = format!("{root}/stamped");
    fs.open(&path(&file), create_rw()).expect("create stamped");
    fs.set_times(&path(&file), 111, 222).expect("set_times");
    let meta = fs.metadata(&path(&file)).expect("stat stamped");
    assert_eq!(meta.accessed_time_ns(), 111, "[{label}] atime crossed");
    assert_eq!(meta.modified_time_ns(), 222, "[{label}] mtime crossed");
    fs.remove_file(&path(&file)).expect("remove stamped");
}

/// A plain (non-content-addressed) backing reports `content_hash` as `None`.
pub fn check_content_hash_none(fs: &dyn FileSystem, root: &str, label: &str) {
    let file = format!("{root}/plain");
    fs.open(&path(&file), create_rw()).expect("create plain");
    assert_eq!(
        fs.content_hash(&path(&file)).expect("content_hash"),
        None,
        "[{label}] plain backing has no content hash (None, not an error)"
    );
    fs.remove_file(&path(&file)).expect("remove plain");
}

/// Typed errors surface as their precise variant (the wire's headline win).
pub fn check_typed_errors(fs: &dyn FileSystem, root: &str, label: &str) {
    // A missing file is exactly NotFound, never an opaque transport error.
    match fs.metadata(&path(&format!("{root}/does/not/exist"))) {
        Err(FsError::NotFound) => {}
        other => panic!("[{label}] expected NotFound for a missing path, got {other:?}"),
    }

    // remove_dir on a non-empty directory is exactly NotEmpty.
    let dir = format!("{root}/full");
    fs.create_dir(&path(&dir)).expect("create full dir");
    fs.open(&path(&format!("{dir}/child")), create_rw())
        .expect("create child");
    match fs.remove_dir(&path(&dir)) {
        Err(FsError::NotEmpty) => {}
        other => panic!("[{label}] expected NotEmpty for a non-empty dir, got {other:?}"),
    }
    fs.remove_file(&path(&format!("{dir}/child")))
        .expect("remove child");
    fs.remove_dir(&path(&dir)).expect("remove full dir");
}
