//! End-to-end test: [`RemoteFs`] drives a real [`P9Server`] over loopback TCP.
//!
//! A `P9Server` serving an in-memory [`MemFs`] runs on a background thread bound
//! to a loopback `TcpListener`. The client connects over the accepted socket and
//! exercises the round trip the crate must satisfy: write a file, read it back,
//! list a directory, stat, create and remove directories, and rename. This is
//! the mirror contract the server's `serve_stream` test inverts.

use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::thread;

use wanix_9p::P9Server;
use wanix_9p_client::RemoteFs;
use wanix_fs::{FileSystem, FileType, MemFs, NormalizedPath, OpenOptions};

/// Spawns a serial 9P server over one accepted loopback connection.
fn spawn_server(fs: Arc<MemFs>) -> TcpStream {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let read = stream.try_clone().unwrap();
        let mut server = P9Server::new(fs as Arc<dyn FileSystem>);
        // Ignore the transport result; the client closes the socket at teardown.
        let _ = server.serve_stream(read, stream);
    });
    TcpStream::connect(addr).unwrap()
}

fn path(value: &str) -> NormalizedPath {
    NormalizedPath::new(value).unwrap()
}

#[test]
fn write_read_and_list_round_trip() {
    let fs = Arc::new(MemFs::new());
    fs.create_dir_all("dir").unwrap();
    let client = spawn_server(fs.clone());

    let remote = RemoteFs::connect(Box::new(client)).unwrap();

    // Write a new file through the client.
    let mut file = remote
        .open(
            &path("dir/hello.txt"),
            OpenOptions {
                read: true,
                write: true,
                create: true,
                truncate: true,
            },
        )
        .unwrap();
    let payload = b"hello over 9P";
    assert_eq!(file.write(payload).unwrap(), payload.len());
    drop(file);

    // The server's MemFs observed the write.
    assert_eq!(fs.read_file("dir/hello.txt").unwrap(), payload);

    // Read it back through the client.
    let mut reader = remote
        .open(&path("dir/hello.txt"), OpenOptions::read())
        .unwrap();
    let mut buf = vec![0_u8; payload.len()];
    let mut filled = 0;
    while filled < buf.len() {
        let n = reader.read(&mut buf[filled..]).unwrap();
        assert!(n > 0, "unexpected short read");
        filled += n;
    }
    assert_eq!(&buf, payload);
    assert!(reader.is_seekable(), "a regular file must be seekable");
    drop(reader);

    // Metadata round trip.
    let metadata = remote.metadata(&path("dir/hello.txt")).unwrap();
    assert_eq!(metadata.file_type(), FileType::File);
    assert_eq!(metadata.len(), payload.len() as u64);

    // Directory listing round trip.
    let mut names: Vec<String> = remote
        .read_dir(&path("dir"))
        .unwrap()
        .into_iter()
        .map(|entry| entry.name().to_owned())
        .collect();
    names.sort();
    assert_eq!(names, vec!["hello.txt".to_owned()]);
}

#[test]
fn create_remove_and_rename_round_trip() {
    let fs = Arc::new(MemFs::new());
    let client = spawn_server(fs.clone());
    let remote = RemoteFs::connect(Box::new(client)).unwrap();

    remote.create_dir(&path("made")).unwrap();
    assert_eq!(
        fs.metadata(&path("made")).unwrap().file_type(),
        FileType::Directory
    );

    // Create a file, rename it, then remove it.
    let mut file = remote
        .open(
            &path("made/a.txt"),
            OpenOptions {
                read: true,
                write: true,
                create: true,
                truncate: true,
            },
        )
        .unwrap();
    file.write(b"x").unwrap();
    drop(file);

    remote
        .rename(&path("made/a.txt"), &path("made/b.txt"))
        .unwrap();
    assert!(fs.metadata(&path("made/a.txt")).is_err());
    assert!(fs.metadata(&path("made/b.txt")).is_ok());

    remote.remove_file(&path("made/b.txt")).unwrap();
    assert!(fs.metadata(&path("made/b.txt")).is_err());

    remote.remove_dir(&path("made")).unwrap();
    assert!(fs.metadata(&path("made")).is_err());
}

#[test]
fn missing_path_reports_not_found() {
    let fs = Arc::new(MemFs::new());
    let client = spawn_server(fs.clone());
    let remote = RemoteFs::connect(Box::new(client)).unwrap();

    let error = remote.metadata(&path("nope")).unwrap_err();
    assert_eq!(error, wanix_fs::FsError::NotFound);
}

#[test]
fn deep_path_walk_chunks_past_maxwelem() {
    // A path with more than MAXWELEM (16) components forces the walk to span
    // multiple Twalk chunks. The deepest file must still resolve.
    let fs = Arc::new(MemFs::new());
    let deep_dir = (0..20)
        .map(|i| format!("d{i}"))
        .collect::<Vec<_>>()
        .join("/");
    fs.create_dir_all(&deep_dir).unwrap();
    let deep_file = format!("{deep_dir}/leaf.txt");
    fs.write_file(&deep_file, b"deep").unwrap();

    let client = spawn_server(fs.clone());
    let remote = RemoteFs::connect(Box::new(client)).unwrap();

    let metadata = remote.metadata(&path(&deep_file)).unwrap();
    assert_eq!(metadata.file_type(), FileType::File);
    assert_eq!(metadata.len(), 4);
}

#[test]
fn read_back_via_seek_reports_offset() {
    let fs = Arc::new(MemFs::new());
    fs.write_file("data.bin", b"0123456789").unwrap();
    let client = spawn_server(fs.clone());
    let remote = RemoteFs::connect(Box::new(client)).unwrap();

    let mut file = remote.open(&path("data.bin"), OpenOptions::read()).unwrap();
    assert!(file.is_seekable());

    use wanix_fs::FileSeekFrom;
    assert_eq!(file.seek(FileSeekFrom::Start(5)).unwrap(), 5);
    assert_eq!(file.tell().unwrap(), 5);

    let mut buf = [0_u8; 3];
    let n = file.read(&mut buf).unwrap();
    assert_eq!(&buf[..n], b"567");
}
