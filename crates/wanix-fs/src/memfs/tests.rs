use super::MemFs;
use crate::{FileSeekFrom, FileSystem, FileType, FsError, NormalizedPath, OpenOptions};

#[test]
fn new_memfs_has_empty_root_directory() {
    let fs = MemFs::new();
    let root = NormalizedPath::new(".").unwrap();
    let metadata = fs.metadata(&root).unwrap();

    assert_eq!(metadata.file_type(), FileType::Directory);
    assert_eq!(metadata.mode(), 0o755);
    assert_eq!(fs.read_dir(&root).unwrap(), []);
}

#[test]
fn write_file_creates_implicit_parent_directories() {
    let fs = MemFs::new();
    fs.write_file("dir/sub/file.txt", b"hello").unwrap();

    let dir = NormalizedPath::new("dir").unwrap();
    let sub = NormalizedPath::new("dir/sub").unwrap();
    assert_eq!(fs.metadata(&dir).unwrap().file_type(), FileType::Directory);
    assert_eq!(fs.metadata(&sub).unwrap().file_type(), FileType::Directory);
    assert_eq!(fs.read_file("dir/sub/file.txt").unwrap(), b"hello");
}

#[test]
fn read_dir_returns_sorted_direct_children() {
    let fs = MemFs::new();
    fs.write_file("b.txt", b"b").unwrap();
    fs.write_file("a.txt", b"a").unwrap();
    fs.write_file("dir/nested.txt", b"n").unwrap();

    let root = NormalizedPath::new(".").unwrap();
    let names = fs
        .read_dir(&root)
        .unwrap()
        .into_iter()
        .map(|entry| entry.name().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(names, ["a.txt", "b.txt", "dir"]);
}

#[test]
fn remove_file_removes_only_files() {
    let fs = MemFs::new();
    fs.write_file("file.txt", b"hello").unwrap();
    fs.create_dir_all("dir").unwrap();
    let file = NormalizedPath::new("file.txt").unwrap();

    fs.remove_file(&file).unwrap();
    assert_eq!(fs.metadata(&file), Err(FsError::NotFound));
    assert_eq!(fs.remove_file(&file), Err(FsError::NotFound));
    assert_eq!(
        fs.remove_file(&NormalizedPath::new("dir").unwrap()),
        Err(FsError::IsDirectory)
    );
}

#[test]
fn remove_dir_removes_only_empty_non_root_directories() {
    let fs = MemFs::new();
    fs.create_dir_all("empty").unwrap();
    fs.write_file("nonempty/file.txt", b"file").unwrap();
    fs.write_file("file.txt", b"file").unwrap();

    fs.remove_dir(&NormalizedPath::new("empty").unwrap())
        .unwrap();

    assert_eq!(
        fs.metadata(&NormalizedPath::new("empty").unwrap()),
        Err(FsError::NotFound)
    );
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
}

#[test]
fn create_dir_creates_one_directory_with_existing_directory_parent() {
    let fs = MemFs::new();
    fs.create_dir_all("parent").unwrap();
    let child = NormalizedPath::new("parent/child").unwrap();

    fs.create_dir(&child).unwrap();

    assert_eq!(
        fs.metadata(&child).unwrap().file_type(),
        FileType::Directory
    );
    assert_eq!(
        fs.read_dir(&NormalizedPath::new("parent").unwrap())
            .unwrap()
            .len(),
        1
    );
    assert_eq!(fs.create_dir(&child), Err(FsError::AlreadyExists));
    assert_eq!(
        fs.create_dir(&NormalizedPath::new("missing/child").unwrap()),
        Err(FsError::NotFound)
    );
    fs.write_file("file.txt", b"file").unwrap();
    assert_eq!(
        fs.create_dir(&NormalizedPath::new("file.txt/child").unwrap()),
        Err(FsError::NotDirectory)
    );
    assert_eq!(
        fs.create_dir(&NormalizedPath::new(".").unwrap()),
        Err(FsError::AlreadyExists)
    );
}

#[test]
fn open_read_and_write_round_trip() {
    let fs = MemFs::new();
    fs.write_file("file.txt", b"hello").unwrap();
    let path = NormalizedPath::new("file.txt").unwrap();
    let metadata = fs.metadata(&path).unwrap();
    assert_eq!(metadata.mode(), 0o644);
    assert_eq!(metadata.len(), 5);

    let mut file = fs.open(&path, OpenOptions::read()).unwrap();
    let mut buf = [0; 8];
    let n = file.read(&mut buf).unwrap();
    assert_eq!(&buf[..n], b"hello");

    let mut file = fs.open(&path, OpenOptions::read_write()).unwrap();
    assert_eq!(file.write(b"HELLO").unwrap(), 5);
    assert_eq!(fs.read_file("file.txt").unwrap(), b"HELLO");
}

#[test]
fn create_succeeds_in_existing_parent_and_truncate_clears_file() {
    let fs = MemFs::new();
    fs.create_dir_all("dir").unwrap();
    let path = NormalizedPath::new("dir/new.txt").unwrap();

    let mut file = fs
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
    assert_eq!(fs.read_file("dir/new.txt").unwrap(), b"created");

    fs.open(
        &path,
        OpenOptions {
            read: true,
            write: true,
            create: false,
            truncate: true,
        },
    )
    .unwrap();
    assert_eq!(fs.read_file("dir/new.txt").unwrap(), b"");
}

#[test]
fn open_create_reports_missing_parent() {
    let fs = MemFs::new();
    let missing_parent = NormalizedPath::new("missing/file.txt").unwrap();

    assert!(matches!(
        fs.open(
            &missing_parent,
            OpenOptions {
                read: true,
                write: true,
                create: true,
                truncate: false,
            },
        ),
        Err(FsError::NotFound)
    ));
}

#[test]
fn open_create_reports_non_directory_parent() {
    let fs = MemFs::new();
    fs.write_file("file.txt", b"file").unwrap();
    let file_parent = NormalizedPath::new("file.txt/child").unwrap();

    assert!(matches!(
        fs.open(
            &file_parent,
            OpenOptions {
                read: true,
                write: true,
                create: true,
                truncate: false,
            },
        ),
        Err(FsError::NotDirectory)
    ));
}

#[test]
fn file_handles_have_independent_offsets_and_writes_persist_immediately() {
    let fs = MemFs::new();
    fs.write_file("file.txt", b"hello").unwrap();
    let path = NormalizedPath::new("file.txt").unwrap();
    let mut first = fs.open(&path, OpenOptions::read()).unwrap();
    let mut second = fs.open(&path, OpenOptions::read_write()).unwrap();

    let mut first_buf = [0; 1];
    let mut second_buf = [0; 2];
    assert_eq!(first.read(&mut first_buf).unwrap(), 1);
    assert_eq!(second.read(&mut second_buf).unwrap(), 2);
    assert_eq!(&first_buf, b"h");
    assert_eq!(&second_buf, b"he");

    second.write(b"ZZ").unwrap();
    assert_eq!(fs.read_file("file.txt").unwrap(), b"heZZo");

    let mut first_next = [0; 2];
    assert_eq!(first.read(&mut first_next).unwrap(), 2);
    assert_eq!(&first_next, b"eZ");

    second.write(b"!").unwrap();
    assert_eq!(fs.read_file("file.txt").unwrap(), b"heZZ!");
}

#[test]
fn file_handles_support_seek_and_tell() {
    let fs = MemFs::new();
    fs.write_file("file.txt", b"abcdef").unwrap();
    let path = NormalizedPath::new("file.txt").unwrap();
    let mut file = fs.open(&path, OpenOptions::read_write()).unwrap();

    assert!(file.is_seekable());
    assert_eq!(file.tell().unwrap(), 0);
    assert_eq!(file.seek(FileSeekFrom::Start(2)).unwrap(), 2);
    assert_eq!(file.tell().unwrap(), 2);
    assert_eq!(file.write(b"XY").unwrap(), 2);
    assert_eq!(fs.read_file("file.txt").unwrap(), b"abXYef");
    assert_eq!(file.seek(FileSeekFrom::Current(-3)).unwrap(), 1);

    let mut buf = [0; 2];
    assert_eq!(file.read(&mut buf).unwrap(), 2);
    assert_eq!(&buf, b"bX");
    assert_eq!(file.seek(FileSeekFrom::End(-1)).unwrap(), 5);
    assert_eq!(file.seek(FileSeekFrom::End(2)).unwrap(), 8);
    assert_eq!(file.write(b"!").unwrap(), 1);
    assert_eq!(fs.read_file("file.txt").unwrap(), b"abXYef\0\0!");
}

#[test]
fn file_seek_rejects_negative_offsets() {
    let fs = MemFs::new();
    fs.write_file("file.txt", b"abc").unwrap();
    let path = NormalizedPath::new("file.txt").unwrap();
    let mut file = fs.open(&path, OpenOptions::read()).unwrap();

    assert_eq!(
        file.seek(FileSeekFrom::Current(-1)),
        Err(FsError::InvalidOffset)
    );
    assert_eq!(
        file.seek(FileSeekFrom::End(-4)),
        Err(FsError::InvalidOffset)
    );
    assert_eq!(file.tell().unwrap(), 0);
}

#[test]
fn errors_are_pinned_for_missing_files_directories_and_directory_io() {
    let fs = MemFs::new();
    fs.write_file("file.txt", b"hello").unwrap();
    fs.create_dir_all("dir").unwrap();

    assert!(matches!(fs.read_file("."), Err(FsError::IsDirectory)));
    assert!(matches!(
        fs.write_file("dir", b"x"),
        Err(FsError::IsDirectory)
    ));
    assert!(matches!(fs.read_file("missing"), Err(FsError::NotFound)));
    assert!(matches!(
        fs.read_dir(&NormalizedPath::new("file.txt").unwrap()),
        Err(FsError::NotDirectory)
    ));
    assert!(matches!(
        fs.open(
            &NormalizedPath::new("dir").unwrap(),
            OpenOptions {
                read: true,
                write: true,
                create: true,
                truncate: false,
            },
        ),
        Err(FsError::IsDirectory)
    ));
}

#[test]
fn open_options_are_enforced_as_capabilities() {
    let fs = MemFs::new();
    fs.write_file("file.txt", b"hello").unwrap();
    let path = NormalizedPath::new("file.txt").unwrap();

    let mut write_only = fs
        .open(
            &path,
            OpenOptions {
                read: false,
                write: true,
                create: false,
                truncate: false,
            },
        )
        .unwrap();
    assert!(matches!(
        write_only.read(&mut [0; 1]),
        Err(FsError::PermissionDenied)
    ));
    assert!(matches!(write_only.write(b"ok"), Ok(2)));

    let mut read_only = fs.open(&path, OpenOptions::read()).unwrap();
    assert!(matches!(
        read_only.write(b"no"),
        Err(FsError::PermissionDenied)
    ));

    assert!(matches!(
        fs.open(
            &NormalizedPath::new("new.txt").unwrap(),
            OpenOptions {
                read: true,
                write: false,
                create: true,
                truncate: false,
            },
        ),
        Err(FsError::PermissionDenied)
    ));
    assert!(matches!(
        fs.open(
            &path,
            OpenOptions {
                read: true,
                write: false,
                create: false,
                truncate: true,
            },
        ),
        Err(FsError::PermissionDenied)
    ));
}

#[test]
fn hash_named_entries_are_visible_in_core_memfs() {
    let fs = MemFs::new();
    fs.write_file("#task/id", b"1").unwrap();

    let root = NormalizedPath::new(".").unwrap();
    let names = fs
        .read_dir(&root)
        .unwrap()
        .into_iter()
        .map(|entry| entry.name().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(names, ["#task"]);
}
