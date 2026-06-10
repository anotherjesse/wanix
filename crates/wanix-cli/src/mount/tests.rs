use std::ffi::OsString;
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::thread;

use wanix_9p::P9Server;
use wanix_9p_client::RemoteFs;
use wanix_fs::{FileSystem, MemFs};
use wanix_vfs::{BindOptions, Namespace};

use super::{
    MOUNT_POINT, MountCommand, mount_namespace_in, parse_mount_command, run_mount_op_for_tests,
};

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
            follow: false,
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
fn parses_follow_flag_anywhere_in_the_cat_operands() {
    for follow_args in [
        ["--follow", "tcp://h:1", "a/b.txt"],
        ["tcp://h:1", "--follow", "a/b.txt"],
        ["tcp://h:1", "a/b.txt", "--follow"],
    ] {
        assert_eq!(
            parse_mount_command("mount-cat", &args(&follow_args)).unwrap(),
            MountCommand::Cat {
                addr: "tcp://h:1".to_owned(),
                path: "a/b.txt".to_owned(),
                follow: true,
            },
            "args: {follow_args:?}"
        );
    }
    // --follow alone still misses its operands.
    assert!(parse_mount_command("mount-cat", &args(&["--follow", "tcp://h:1"])).is_err());
}

/// Mount-by-name for the one-shot verbs: a live loopback serve registered via
/// the `--register` machinery is dialed by its bare catalog NAME — the address
/// is resolved at invocation time and the verb runs over the real native-wire
/// mount. An unknown name is a clear error naming the catalog and catalog add.
#[test]
fn mount_verbs_resolve_catalog_names_at_invocation_time() {
    use wanix_fs::NormalizedPath;

    use crate::catalog::register_served;
    use crate::mesh::mounts::test_support::serve_native;

    let fs = Arc::new(MemFs::new());
    fs.write_file("hello.txt", b"hi from the catalog").unwrap();
    let (server, url) = serve_native(fs.clone() as Arc<dyn FileSystem>, 8);
    let catalog =
        std::env::temp_dir().join(format!("wanix-mount-verb-names-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&catalog);
    register_served(&catalog, "shared", "volume", &[("shared".to_owned(), url)]).unwrap();

    // `mount-cat shared hello.txt`, with the catalog dir injected: the session
    // is the same one `run_mount_command` builds for a ticket.
    let session = mount_namespace_in(&catalog, "shared").unwrap();
    let read = super::ops::mount_cat(
        &session.namespace,
        &NormalizedPath::new(format!("{MOUNT_POINT}/hello.txt")).unwrap(),
    )
    .unwrap();
    assert_eq!(read.stdout(), b"hi from the catalog");

    let Err(unknown) = mount_namespace_in(&catalog, "absent") else {
        panic!("an unknown name must not mount");
    };
    let message = unknown.to_string();
    assert!(message.contains("no catalog entry \"absent\""), "{message}");
    assert!(message.contains("catalog add absent"), "{message}");

    // A non-name spelling passes through to transport dispatch untouched.
    let Err(not_a_name) = mount_namespace_in(&catalog, "ftp://host:1") else {
        panic!("a bogus scheme must not mount");
    };
    assert!(
        not_a_name.to_string().contains("tcp://HOST:PORT"),
        "{not_a_name}"
    );

    drop(session);
    drop(server);
    let _ = std::fs::remove_dir_all(&catalog);
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
fn mount_local(fs: Arc<dyn FileSystem>) -> Namespace {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let read = stream.try_clone().unwrap();
        let mut server = P9Server::new(fs);
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
            follow: false,
        },
    )
    .unwrap();
    assert_eq!(read.stdout(), b"mounted bytes");
}

#[test]
fn cat_follow_streams_a_never_eof_pipe_incrementally_then_exits_at_eof() {
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    use wanix_fs::{NormalizedPath, OpenOptions};
    use wanix_pipe::PipeDevice;

    /// A `Write` sink shared with the asserting thread.
    #[derive(Clone, Default)]
    struct SharedSink(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for SharedSink {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn wait_for(sink: &SharedSink, expected: &[u8]) {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if sink.0.lock().unwrap().as_slice() == expected {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "follow output never reached {:?}: {:?}",
                String::from_utf8_lossy(expected),
                String::from_utf8_lossy(&sink.0.lock().unwrap())
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    // Serve a namespace holding a #pipe device over loopback 9P; the pipe is
    // a real never-EOF stream while its writer lives.
    let pipe = Arc::new(PipeDevice::new());
    let pipe_id = pipe.alloc().unwrap();
    let mut served = Namespace::new();
    served
        .bind(pipe.clone(), ".", "#pipe", BindOptions::default())
        .unwrap();
    let mut writer = pipe
        .open(
            &NormalizedPath::new(format!("{pipe_id}/data")).unwrap(),
            OpenOptions {
                write: true,
                ..OpenOptions::default()
            },
        )
        .unwrap();
    writer.write(b"alpha ").unwrap();

    let namespace = mount_local(Arc::new(served));
    let sink = SharedSink::default();
    let stream_sink = sink.clone();
    let path = format!("#pipe/{pipe_id}/data");
    let follower = thread::spawn(move || {
        let mut stdout = stream_sink;
        super::run_mount_cat_follow_for_tests(&namespace, &path, &mut stdout)
    });

    // The pre-fed chunk arrives without any EOF; later chunks stream as the
    // writer produces them (incremental write-through, no byte cap heuristics).
    wait_for(&sink, b"alpha ");
    writer.write(b"beta").unwrap();
    wait_for(&sink, b"alpha beta");

    // Dropping the last writer is EOF: the follow loop exits cleanly.
    drop(writer);
    follower
        .join()
        .expect("follower thread")
        .expect("follow ended at EOF");
    assert_eq!(sink.0.lock().unwrap().as_slice(), b"alpha beta");
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
