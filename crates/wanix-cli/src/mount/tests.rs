use std::ffi::OsString;
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::thread;

use wanix_9p::P9Server;
use wanix_9p_client::RemoteFs;
use wanix_fs::{FileSystem, MemFs};
use wanix_vfs::{BindOptions, Namespace};

use super::{MOUNT_POINT, MountCommand, parse_mount_command, run_mount_op_for_tests};

fn args(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}

#[test]
fn parses_each_verb_with_operands() {
    assert_eq!(
        parse_mount_command("mount-ls", &args(&["tcp://127.0.0.1:9999"])).unwrap(),
        MountCommand::Ls {
            addr: "tcp://127.0.0.1:9999".to_owned(),
            path: ".".to_owned(),
        }
    );
    assert_eq!(
        parse_mount_command("mount-cat", &args(&["tcp://h:1", "a/b.txt"])).unwrap(),
        MountCommand::Cat {
            addr: "tcp://h:1".to_owned(),
            path: "a/b.txt".to_owned(),
        }
    );
    assert_eq!(
        parse_mount_command("mount-write", &args(&["tcp://h:1", "f.txt", "hi"])).unwrap(),
        MountCommand::Write {
            addr: "tcp://h:1".to_owned(),
            path: "f.txt".to_owned(),
            text: "hi".to_owned(),
        }
    );
}

#[test]
fn rejects_missing_operands() {
    assert!(parse_mount_command("mount-cat", &args(&["tcp://h:1"])).is_err());
    assert!(parse_mount_command("mount-write", &args(&["tcp://h:1", "f.txt"])).is_err());
    assert!(parse_mount_command("mount-ls", &[]).is_err());
    assert!(parse_mount_command("mount-bogus", &[]).is_err());
}

/// Spawns a serial 9P server over one accepted loopback connection and returns a
/// namespace with the dialed remote bound at `/n/remote`.
fn mount_local(fs: Arc<MemFs>) -> Namespace {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let read = stream.try_clone().unwrap();
        let mut server = P9Server::new(fs as Arc<dyn FileSystem>);
        let _ = server.serve_stream(read, stream);
    });
    let stream = TcpStream::connect(addr).unwrap();
    let remote = Arc::new(RemoteFs::connect(Box::new(stream)).unwrap());
    let mut namespace = Namespace::new();
    namespace
        .bind(remote, ".", MOUNT_POINT, BindOptions::default())
        .unwrap();
    namespace
}

#[test]
fn write_then_cat_round_trips_through_namespace() {
    let fs = Arc::new(MemFs::new());
    let namespace = mount_local(fs.clone());

    let written = run_mount_op_for_tests(
        &namespace,
        &MountCommand::Write {
            addr: String::new(),
            path: "note.txt".to_owned(),
            text: "mounted bytes".to_owned(),
        },
    )
    .unwrap();
    assert_eq!(written.exit_code(), 0);
    assert_eq!(fs.read_file("note.txt").unwrap(), b"mounted bytes");

    let read = run_mount_op_for_tests(
        &namespace,
        &MountCommand::Cat {
            addr: String::new(),
            path: "note.txt".to_owned(),
        },
    )
    .unwrap();
    assert_eq!(read.stdout(), b"mounted bytes");
}

#[test]
fn ls_lists_remote_directory_entries() {
    let fs = Arc::new(MemFs::new());
    fs.create_dir_all("d").unwrap();
    fs.write_file("d/a.txt", b"a").unwrap();
    fs.write_file("d/b.txt", b"b").unwrap();
    let namespace = mount_local(fs);

    let listed = run_mount_op_for_tests(
        &namespace,
        &MountCommand::Ls {
            addr: String::new(),
            path: "d".to_owned(),
        },
    )
    .unwrap();
    assert_eq!(listed.stdout(), b"a.txt\nb.txt\n");
}
