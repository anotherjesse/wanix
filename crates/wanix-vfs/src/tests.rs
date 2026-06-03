use std::sync::Arc;

use super::{BindOptions, BindPosition, CRATE_PURPOSE, Namespace};
use wanix_fs::{FileSystem, FileType, FsError, MemFs, Metadata, NormalizedPath, OpenOptions};

fn fixture(entries: &[(&str, &[u8])]) -> Arc<MemFs> {
    let fs = Arc::new(MemFs::new());
    for (path, data) in entries {
        fs.write_file(path, data).unwrap();
    }
    fs
}

fn read_file(fs: &dyn FileSystem, path: &str) -> Vec<u8> {
    let path = NormalizedPath::new(path).unwrap();
    let mut file = fs.open(&path, OpenOptions::read()).unwrap();
    let mut out = Vec::new();
    let mut buf = [0; 16];
    loop {
        let n = file.read(&mut buf).unwrap();
        if n == 0 {
            return out;
        }
        out.extend_from_slice(&buf[..n]);
    }
}

fn entry_names(fs: &dyn FileSystem, path: &str) -> Vec<String> {
    fs.read_dir(&NormalizedPath::new(path).unwrap())
        .unwrap()
        .into_iter()
        .map(|entry| entry.name().to_owned())
        .collect()
}

fn entry_metadata(fs: &dyn FileSystem, path: &str, name: &str) -> Metadata {
    fs.read_dir(&NormalizedPath::new(path).unwrap())
        .unwrap()
        .into_iter()
        .find(|entry| entry.name() == name)
        .map(|entry| entry.metadata().clone())
        .unwrap()
}

fn path(value: &str) -> NormalizedPath {
    NormalizedPath::new(value).unwrap()
}

#[test]
fn purpose_is_declared() {
    assert!(!CRATE_PURPOSE.is_empty());
}

#[test]
fn empty_namespace_root_is_a_directory() {
    let ns = Namespace::new();

    assert_eq!(
        ns.metadata(&NormalizedPath::new(".").unwrap())
            .unwrap()
            .file_type(),
        FileType::Directory
    );
    assert_eq!(entry_names(&ns, "."), Vec::<String>::new());
}

#[test]
fn direct_file_and_directory_binds_resolve() {
    let fs = fixture(&[
        ("file1.txt", b"content1"),
        ("dir/file2.txt", b"content2"),
        ("dir/subdir/file", b"content3"),
    ]);
    let mut ns = Namespace::new();

    ns.bind(
        fs.clone(),
        "file1.txt",
        "bound-file.txt",
        BindOptions::default(),
    )
    .unwrap();
    ns.bind(fs, ".", "bound-dir", BindOptions::default())
        .unwrap();

    assert_eq!(read_file(&ns, "bound-file.txt"), b"content1");
    assert_eq!(read_file(&ns, "bound-dir/dir/file2.txt"), b"content2");
    assert_eq!(entry_names(&ns, "bound-dir/dir"), ["file2.txt", "subdir"]);
    let metadata = entry_metadata(&ns, ".", "bound-file.txt");
    assert_eq!(metadata.file_type(), FileType::File);
    assert_eq!(metadata.len(), 8);
    assert_eq!(metadata.mode(), 0o644);
    assert!(matches!(
        ns.open(
            &NormalizedPath::new("nonexistent").unwrap(),
            OpenOptions::read()
        ),
        Err(FsError::NotFound)
    ));
}

#[test]
fn subpath_bind_maps_source_subtree() {
    let fs = fixture(&[
        ("src/inner/app.js", b"app"),
        ("src/inner/lib/mod.js", b"mod"),
        ("src/outer.js", b"outer"),
    ]);
    let mut ns = Namespace::new();

    ns.bind(fs, "src/inner", "app", BindOptions::default())
        .unwrap();

    assert_eq!(read_file(&ns, "app/app.js"), b"app");
    assert_eq!(read_file(&ns, "app/lib/mod.js"), b"mod");
    assert_eq!(entry_names(&ns, "app"), ["app.js", "lib"]);
    assert!(matches!(
        ns.open(
            &NormalizedPath::new("app/outer.js").unwrap(),
            OpenOptions::read()
        ),
        Err(FsError::NotFound)
    ));
}

#[test]
fn longest_component_subpath_bind_wins() {
    let root = fixture(&[
        ("lib/shared.txt", b"root-lib"),
        ("apple/shared.txt", b"apple"),
    ]);
    let lib = fixture(&[("shared.txt", b"specific-lib")]);
    let mut ns = Namespace::new();

    ns.bind(root, ".", ".", BindOptions::default()).unwrap();
    ns.bind(lib, ".", "lib", BindOptions::default()).unwrap();

    assert_eq!(read_file(&ns, "lib/shared.txt"), b"specific-lib");
    assert_eq!(read_file(&ns, "apple/shared.txt"), b"apple");
}

#[test]
fn bind_order_and_replace_are_encoded() {
    let fs1 = fixture(&[("file.txt", b"fs1")]);
    let fs2 = fixture(&[("file.txt", b"fs2")]);
    let mut ns = Namespace::new();

    ns.bind(fs1.clone(), ".", "test", BindOptions::default())
        .unwrap();
    ns.bind(fs2.clone(), ".", "test", BindOptions::default())
        .unwrap();
    assert_eq!(read_file(&ns, "test/file.txt"), b"fs2");

    ns.bind(
        fs1,
        ".",
        "test",
        BindOptions {
            position: BindPosition::Last,
        },
    )
    .unwrap();
    assert_eq!(read_file(&ns, "test/file.txt"), b"fs2");

    ns.bind(
        fs2,
        ".",
        "test",
        BindOptions {
            position: BindPosition::Replace,
        },
    )
    .unwrap();
    assert_eq!(ns.bindings().len(), 1);
    assert_eq!(read_file(&ns, "test/file.txt"), b"fs2");
}

#[test]
fn replace_clears_only_same_destination() {
    let fs1 = fixture(&[("file.txt", b"fs1")]);
    let fs2 = fixture(&[("file.txt", b"fs2")]);
    let fs3 = fixture(&[("file.txt", b"fs3")]);
    let mut ns = Namespace::new();

    ns.bind(fs1, ".", "mnt", BindOptions::default()).unwrap();
    ns.bind(fs2, ".", "other", BindOptions::default()).unwrap();
    ns.bind(
        fs3,
        ".",
        "mnt",
        BindOptions {
            position: BindPosition::Replace,
        },
    )
    .unwrap();

    assert_eq!(ns.binding_count(), 2);
    assert_eq!(read_file(&ns, "mnt/file.txt"), b"fs3");
    assert_eq!(read_file(&ns, "other/file.txt"), b"fs2");
}

#[test]
fn namespace_clones_are_isolated_but_share_backing_filesystems() {
    let fs = fixture(&[("file.txt", b"root")]);
    let mut parent = Namespace::new();
    parent
        .bind(fs.clone(), ".", ".", BindOptions::default())
        .unwrap();
    let mut child = parent.clone();

    child
        .bind(
            fixture(&[("child.txt", b"child")]),
            ".",
            "child",
            BindOptions::default(),
        )
        .unwrap();

    assert_eq!(read_file(&parent, "file.txt"), b"root");
    assert!(matches!(
        parent.metadata(&NormalizedPath::new("child").unwrap()),
        Err(FsError::NotFound)
    ));
    assert_eq!(read_file(&child, "child/child.txt"), b"child");

    fs.write_file("after-clone.txt", b"shared").unwrap();
    assert_eq!(read_file(&child, "after-clone.txt"), b"shared");
}

#[test]
fn create_dir_routes_to_resolved_backing_filesystem() {
    let root = fixture(&[("mnt/existing.txt", b"file")]);
    let mut ns = Namespace::new();
    ns.bind(root.clone(), "mnt", "app", BindOptions::default())
        .unwrap();

    ns.create_dir(&NormalizedPath::new("app/newdir").unwrap())
        .unwrap();

    assert_eq!(
        root.metadata(&NormalizedPath::new("mnt/newdir").unwrap())
            .unwrap()
            .file_type(),
        FileType::Directory
    );
    assert_eq!(
        ns.create_dir(&NormalizedPath::new("app/newdir").unwrap()),
        Err(FsError::AlreadyExists)
    );
    assert_eq!(
        ns.create_dir(&NormalizedPath::new("app/missing/child").unwrap()),
        Err(FsError::NotFound)
    );
    assert_eq!(
        ns.create_dir(&NormalizedPath::new("app/existing.txt/child").unwrap()),
        Err(FsError::NotDirectory)
    );
}

#[test]
fn create_dir_reports_synthetic_namespace_directories_as_existing() {
    let fs = fixture(&[("file.txt", b"content")]);
    let mut ns = Namespace::new();
    ns.bind(fs, "file.txt", "a/b/file.txt", BindOptions::default())
        .unwrap();

    assert_eq!(
        ns.create_dir(&NormalizedPath::new("a").unwrap()),
        Err(FsError::AlreadyExists)
    );
    assert_eq!(
        ns.create_dir(&NormalizedPath::new("a/c").unwrap()),
        Err(FsError::NotFound)
    );
}

#[test]
fn remove_dir_routes_to_resolved_backing_filesystem() {
    let root = fixture(&[
        ("mnt/nonempty/file.txt", b"file"),
        ("mnt/file.txt", b"file"),
    ]);
    root.create_dir_all("mnt/empty").unwrap();
    let mut ns = Namespace::new();
    ns.bind(root.clone(), "mnt", "app", BindOptions::default())
        .unwrap();

    ns.remove_dir(&NormalizedPath::new("app/empty").unwrap())
        .unwrap();

    assert_eq!(
        root.metadata(&NormalizedPath::new("mnt/empty").unwrap()),
        Err(FsError::NotFound)
    );
    assert_eq!(
        ns.remove_dir(&NormalizedPath::new("app/nonempty").unwrap()),
        Err(FsError::NotEmpty)
    );
    assert_eq!(
        ns.remove_dir(&NormalizedPath::new("app/file.txt").unwrap()),
        Err(FsError::NotDirectory)
    );
    assert_eq!(
        ns.remove_dir(&NormalizedPath::new("app/missing").unwrap()),
        Err(FsError::NotFound)
    );
    assert_eq!(
        ns.remove_dir(&NormalizedPath::new(".").unwrap()),
        Err(FsError::PermissionDenied)
    );
}

#[test]
fn remove_dir_reports_synthetic_namespace_directories_as_not_empty() {
    let fs = fixture(&[("file.txt", b"content")]);
    let mut ns = Namespace::new();
    ns.bind(fs, "file.txt", "a/b/file.txt", BindOptions::default())
        .unwrap();

    assert_eq!(
        ns.remove_dir(&NormalizedPath::new("a").unwrap()),
        Err(FsError::NotEmpty)
    );
}

#[test]
fn rename_routes_to_same_backing_filesystem() {
    let root = fixture(&[
        ("mnt/old.txt", b"old"),
        ("mnt/target.txt", b"target"),
        ("mnt/dir/sub/file.txt", b"nested"),
        ("mnt/nonempty/file.txt", b"busy"),
    ]);
    root.create_dir_all("mnt/empty").unwrap();
    let mut ns = Namespace::new();
    ns.bind(root.clone(), "mnt", "app", BindOptions::default())
        .unwrap();

    ns.rename(&path("app/old.txt"), &path("app/renamed.txt"))
        .unwrap();
    assert_eq!(root.metadata(&path("mnt/old.txt")), Err(FsError::NotFound));
    assert_eq!(root.read_file("mnt/renamed.txt").unwrap(), b"old");

    ns.rename(&path("app/renamed.txt"), &path("app/target.txt"))
        .unwrap();
    assert_eq!(root.read_file("mnt/target.txt").unwrap(), b"old");

    ns.rename(&path("app/dir"), &path("app/empty")).unwrap();
    assert_eq!(root.metadata(&path("mnt/dir")), Err(FsError::NotFound));
    assert_eq!(root.read_file("mnt/empty/sub/file.txt").unwrap(), b"nested");

    assert_eq!(
        ns.rename(&path("app/missing"), &path("app/missing")),
        Err(FsError::NotFound)
    );
    assert_eq!(
        ns.rename(&path("app/empty"), &path("app/nonempty")),
        Err(FsError::NotEmpty)
    );
}

#[test]
fn rename_rejects_synthetic_targets_and_cross_filesystem_moves() {
    let fs = fixture(&[("old.txt", b"old"), ("synthetic.txt", b"synthetic")]);
    let mut synthetic = Namespace::new();
    synthetic
        .bind(fs.clone(), "old.txt", "old.txt", BindOptions::default())
        .unwrap();
    synthetic
        .bind(
            fs,
            "synthetic.txt",
            "a/b/synthetic.txt",
            BindOptions::default(),
        )
        .unwrap();

    assert_eq!(
        synthetic.rename(&path("old.txt"), &path("a")),
        Err(FsError::AlreadyExists)
    );

    let left = fixture(&[("old.txt", b"old")]);
    let right = fixture(&[("target.txt", b"target")]);
    let mut ns = Namespace::new();
    ns.bind(left.clone(), ".", "left", BindOptions::default())
        .unwrap();
    ns.bind(right.clone(), ".", "right", BindOptions::default())
        .unwrap();

    assert_eq!(
        ns.rename(&path("left/old.txt"), &path("right/new.txt")),
        Err(FsError::NotSupported)
    );
    assert_eq!(left.read_file("old.txt").unwrap(), b"old");
    assert_eq!(right.metadata(&path("new.txt")), Err(FsError::NotFound));
}

#[test]
fn set_times_routes_to_highest_priority_backing_filesystem() {
    let lower = fixture(&[("same.txt", b"lower"), ("synthetic.txt", b"synthetic")]);
    let upper = fixture(&[("same.txt", b"upper")]);
    let mut ns = Namespace::new();
    ns.bind(lower.clone(), ".", ".", BindOptions::default())
        .unwrap();
    ns.bind(upper.clone(), ".", ".", BindOptions::default())
        .unwrap();
    ns.bind(
        lower.clone(),
        "synthetic.txt",
        "a/b/synthetic.txt",
        BindOptions::default(),
    )
    .unwrap();

    ns.set_times(&path("same.txt"), 1_000_000_000, 2_000_000_000)
        .unwrap();

    assert_eq!(
        upper
            .metadata(&path("same.txt"))
            .unwrap()
            .modified_time_ns(),
        2_000_000_000
    );
    assert_eq!(
        lower
            .metadata(&path("same.txt"))
            .unwrap()
            .modified_time_ns(),
        0
    );
    assert_eq!(ns.set_times(&path("a"), 1, 2), Err(FsError::NotSupported));
    assert_eq!(
        ns.set_times(&path("missing.txt"), 1, 2),
        Err(FsError::NotFound)
    );
}

#[test]
fn read_dir_unions_direct_and_subpath_bindings() {
    let fs1 = fixture(&[
        ("file1.txt", b"content1"),
        ("dir/file2.txt", b"content2"),
        ("dir/subdir/file", b"content3"),
    ]);
    let fs2 = fixture(&[("fs2.txt", b"fs2")]);
    let mut ns = Namespace::new();

    ns.bind(fs1, ".", ".", BindOptions::default()).unwrap();
    ns.bind(fs2.clone(), ".", ".", BindOptions::default())
        .unwrap();
    ns.bind(fs2, ".", "dir", BindOptions::default()).unwrap();

    assert_eq!(entry_names(&ns, "."), ["dir", "file1.txt", "fs2.txt"]);
    assert_eq!(entry_names(&ns, "dir"), ["file2.txt", "fs2.txt", "subdir"]);
}

#[test]
fn synthesized_directories_are_visible_for_deep_binds() {
    let fs = fixture(&[("file.txt", b"content")]);
    let mut ns = Namespace::new();

    ns.bind(fs, "file.txt", "a/b/c/file.txt", BindOptions::default())
        .unwrap();

    assert_eq!(entry_names(&ns, "."), ["a"]);
    assert_eq!(entry_names(&ns, "a"), ["b"]);
    assert_eq!(entry_names(&ns, "a/b"), ["c"]);
    assert_eq!(entry_names(&ns, "a/b/c"), ["file.txt"]);
    assert_eq!(
        ns.metadata(&NormalizedPath::new("a/b").unwrap())
            .unwrap()
            .file_type(),
        FileType::Directory
    );
    let metadata = entry_metadata(&ns, "a/b/c", "file.txt");
    assert_eq!(metadata.file_type(), FileType::File);
    assert_eq!(metadata.len(), 7);
    assert_eq!(metadata.mode(), 0o644);
    assert_eq!(read_file(&ns, "a/b/c/file.txt"), b"content");
}

#[test]
fn hidden_entries_are_reachable_but_not_listed() {
    let fs = fixture(&[("visible", b"visible"), ("#hidden", b"hidden")]);
    let mut ns = Namespace::new();

    ns.bind(fs, ".", "#svc", BindOptions::default()).unwrap();

    assert_eq!(entry_names(&ns, "."), Vec::<String>::new());
    assert_eq!(entry_names(&ns, "#svc"), ["visible"]);
    assert_eq!(read_file(&ns, "#svc/#hidden"), b"hidden");
}

#[test]
fn read_dir_deduplicates_sorts_and_hides_hash_entries() {
    let lower = fixture(&[
        ("dup", b"lower"),
        ("zeta", b"zeta"),
        ("#lower-hidden", b"hidden"),
    ]);
    let upper = fixture(&[
        ("alpha", b"alpha"),
        ("dup", b"upper"),
        ("#upper-hidden", b"hidden"),
    ]);
    let mut ns = Namespace::new();

    ns.bind(lower, ".", ".", BindOptions::default()).unwrap();
    ns.bind(upper, ".", ".", BindOptions::default()).unwrap();

    assert_eq!(entry_names(&ns, "."), ["alpha", "dup", "zeta"]);
    assert_eq!(read_file(&ns, "dup"), b"upper");
}

#[test]
fn high_priority_file_masks_lower_directory_in_readdir() {
    let lower = fixture(&[("x/lower.txt", b"lower")]);
    let upper = fixture(&[("file", b"upper-file")]);
    let mut ns = Namespace::new();

    ns.bind(lower, ".", ".", BindOptions::default()).unwrap();
    ns.bind(upper, "file", "x", BindOptions::default()).unwrap();

    assert_eq!(read_file(&ns, "x"), b"upper-file");
    assert_eq!(entry_metadata(&ns, ".", "x").file_type(), FileType::File);
    assert!(matches!(
        ns.read_dir(&NormalizedPath::new("x").unwrap()),
        Err(FsError::NotDirectory)
    ));
}

#[test]
fn create_and_write_flow_to_bound_filesystem() {
    let fs = Arc::new(MemFs::new());
    let mut ns = Namespace::new();
    ns.bind(fs.clone(), ".", ".", BindOptions::default())
        .unwrap();
    let path = NormalizedPath::new("created.txt").unwrap();

    let mut file = ns
        .open(
            &path,
            OpenOptions {
                read: true,
                write: true,
                create: true,
                truncate: false,
            },
        )
        .unwrap();
    file.write(b"created").unwrap();

    assert_eq!(fs.read_file("created.txt").unwrap(), b"created");
    assert_eq!(entry_names(&ns, "."), ["created.txt"]);
}

#[test]
fn create_uses_highest_priority_existing_parent() {
    let lower = fixture(&[("a", b"content1"), ("b", b"content2")]);
    let upper = Arc::new(MemFs::new());
    let mut ns = Namespace::new();

    ns.bind(lower, ".", ".", BindOptions::default()).unwrap();
    ns.bind(upper.clone(), ".", ".", BindOptions::default())
        .unwrap();

    let mut file = ns
        .open(
            &NormalizedPath::new("c").unwrap(),
            OpenOptions {
                read: false,
                write: true,
                create: true,
                truncate: false,
            },
        )
        .unwrap();
    file.write(b"content3").unwrap();

    assert_eq!(upper.read_file("c").unwrap(), b"content3");
    assert_eq!(entry_names(&ns, "."), ["a", "b", "c"]);
}

#[test]
fn remove_file_flows_to_highest_priority_bound_filesystem() {
    let lower = fixture(&[("same.txt", b"lower"), ("dir/file.txt", b"nested")]);
    let upper = fixture(&[("same.txt", b"upper")]);
    let mut ns = Namespace::new();

    ns.bind(lower.clone(), ".", ".", BindOptions::default())
        .unwrap();
    ns.bind(upper.clone(), ".", ".", BindOptions::default())
        .unwrap();

    ns.remove_file(&NormalizedPath::new("same.txt").unwrap())
        .unwrap();

    assert_eq!(
        upper.metadata(&NormalizedPath::new("same.txt").unwrap()),
        Err(FsError::NotFound)
    );
    assert_eq!(lower.read_file("same.txt").unwrap(), b"lower");
    assert_eq!(read_file(&ns, "same.txt"), b"lower");
    assert_eq!(
        ns.remove_file(&NormalizedPath::new("dir").unwrap()),
        Err(FsError::IsDirectory)
    );
}
