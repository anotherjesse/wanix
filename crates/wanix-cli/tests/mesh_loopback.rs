//! End-to-end mesh truth check: Plan 9 import over a real loopback socket.
//!
//! A `P9Server` serves a Wanix namespace (a `MemFs` host root plus the `#term`
//! and `#task` service devices) over an accepted loopback TCP connection on a
//! background thread. From the main thread we dial the listener, build a
//! [`RemoteFs`] with the real `wanix-9p-client` keystone, bind it at `/n/remote`
//! in a fresh [`Namespace`], and drive a full round trip through that namespace:
//!
//! - write a regular file and read it back byte-identical;
//! - `create_dir` then `read_dir` and see the new directory;
//! - read the `#task` and `#term` service files across the wire, proving the
//!   `#`-devices cross the socket and not just plain files;
//! - read a streamed service file with no fabricated offset, and confirm the
//!   bytes are identical to operating the same devices through a local
//!   namespace built the same way.
//!
//! Everything is deterministic and self-contained: threads plus loopback TCP,
//! no external processes.

use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::thread;

use wanix_9p::P9Server;
use wanix_9p_client::RemoteFs;
use wanix_fs::{File, FileSystem, FileType, MemFs, NormalizedPath, OpenOptions};
use wanix_kv::KvDevice;
use wanix_task::TaskTable;
use wanix_term::TermDevice;
use wanix_vfs::{BindOptions, BindPosition, Namespace};

/// Relative Wanix path the remote filesystem is bound at, matching the CLI.
const MOUNT_POINT: &str = "n/remote";

fn path(value: &str) -> NormalizedPath {
    NormalizedPath::new(value).unwrap()
}

/// Joins a remote-relative path under the mount point.
fn mounted(relative: &str) -> NormalizedPath {
    path(&format!("{MOUNT_POINT}/{relative}"))
}

/// Builds a services namespace: a `MemFs` host root plus `#kv`, `#term`, `#task`.
///
/// The host `MemFs` is returned so the caller can inspect server-side state. The
/// `#task` root task is allocated with kind `noop`, so `#task/self/kind` is the
/// deterministic string `noop` and a fresh `#term` allocates id `1` first.
fn services_namespace() -> (Namespace, Arc<MemFs>) {
    let host = Arc::new(MemFs::new());
    let mut namespace = Namespace::new();
    namespace
        .bind(host.clone(), ".", ".", BindOptions::default())
        .unwrap();
    namespace
        .bind(
            Arc::new(TermDevice::new()),
            ".",
            "#term",
            BindOptions::default(),
        )
        .unwrap();
    // `#kv` binds as a service device beside `#term`/`#task` and imports for free.
    namespace
        .bind(
            Arc::new(KvDevice::new()),
            ".",
            "#kv",
            BindOptions::default(),
        )
        .unwrap();

    let table = TaskTable::new();
    table.register_noop_driver("noop").unwrap();
    let root_task = table
        .allocate_root_with_namespace("noop", namespace.clone())
        .unwrap();
    namespace
        .bind(
            Arc::new(table.filesystem_for(root_task.id())),
            ".",
            "#task",
            BindOptions {
                position: BindPosition::Replace,
            },
        )
        .unwrap();
    (namespace, host)
}

/// Serves `root` over one accepted loopback connection and returns the dialed,
/// negotiated client filesystem bound at `/n/remote`.
fn mount_over_loopback(root: Arc<dyn FileSystem>) -> (Namespace, Arc<RemoteFs>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let read = stream.try_clone().unwrap();
        let mut server = P9Server::new(root);
        // The client closes the socket at teardown; ignore the transport result.
        let _ = server.serve_stream(read, stream);
    });
    let stream = TcpStream::connect(addr).unwrap();
    let remote = Arc::new(RemoteFs::connect(Box::new(stream)).unwrap());
    let mut namespace = Namespace::new();
    namespace
        .bind(remote.clone(), ".", MOUNT_POINT, BindOptions::default())
        .unwrap();
    (namespace, remote)
}

/// Reads a file handle to EOF with a bounded loop.
fn read_all(mut file: Box<dyn File>) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 4096];
    loop {
        let read = file.read(&mut chunk).unwrap();
        if read == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..read]);
        assert!(bytes.len() < (1 << 20), "stream grew unbounded");
    }
    bytes
}

/// Reads a path through a namespace and returns its bytes.
fn read_through(namespace: &Namespace, p: &NormalizedPath) -> Vec<u8> {
    let file = namespace.open(p, OpenOptions::read()).unwrap();
    read_all(file)
}

#[test]
fn regular_file_round_trips_through_mounted_remote() {
    let (server_ns, host) = services_namespace();
    let (client_ns, _remote) = mount_over_loopback(Arc::new(server_ns));

    let payload = b"plan9 import over a real socket";
    // Create the parent directory over the wire, then the file inside it.
    client_ns.create_dir(&mounted("notes")).unwrap();
    let mut file = client_ns
        .open(
            &mounted("notes/hello.txt"),
            OpenOptions {
                read: true,
                write: true,
                create: true,
                truncate: true,
            },
        )
        .unwrap();
    assert_eq!(file.write(payload).unwrap(), payload.len());
    assert!(file.is_seekable(), "a regular file must report seekable");
    drop(file);

    // The server's MemFs observed the exact bytes through the wire.
    assert_eq!(host.read_file("notes/hello.txt").unwrap(), payload);

    // Read back through the client; bytes are identical.
    let read_back = read_through(&client_ns, &mounted("notes/hello.txt"));
    assert_eq!(read_back, payload);

    // Metadata round trip reports a regular file of the right length.
    let metadata = client_ns.metadata(&mounted("notes/hello.txt")).unwrap();
    assert_eq!(metadata.file_type(), FileType::File);
    assert_eq!(metadata.len(), payload.len() as u64);
}

#[test]
fn create_dir_then_read_dir_shows_entry_over_wire() {
    let (server_ns, host) = services_namespace();
    let (client_ns, _remote) = mount_over_loopback(Arc::new(server_ns));

    client_ns.create_dir(&mounted("made")).unwrap();
    assert_eq!(
        host.metadata(&path("made")).unwrap().file_type(),
        FileType::Directory
    );

    client_ns
        .open(
            &mounted("made/a.txt"),
            OpenOptions {
                read: true,
                write: true,
                create: true,
                truncate: true,
            },
        )
        .unwrap()
        .write(b"a")
        .unwrap();

    let mut names: Vec<String> = client_ns
        .read_dir(&mounted("made"))
        .unwrap()
        .into_iter()
        .map(|entry| entry.name().to_owned())
        .collect();
    names.sort();
    assert_eq!(names, vec!["a.txt".to_owned()]);
}

#[test]
fn task_service_files_cross_the_wire_identically_to_local() {
    let (server_ns, _host) = services_namespace();
    let (client_ns, _remote) = mount_over_loopback(Arc::new(server_ns));

    // A local reference namespace built the same way: the root task is `noop`.
    let (local_ns, _local_host) = services_namespace();

    // `#task/self/kind` is the deterministic root-task kind, end to end.
    let remote_kind = read_through(&client_ns, &mounted("#task/self/kind"));
    let local_kind = read_through(&local_ns, &path("#task/self/kind"));
    assert_eq!(String::from_utf8_lossy(&remote_kind).trim(), "noop");
    assert_eq!(remote_kind, local_kind);

    // `#task/self/id` is the root task id; both report task 1.
    let remote_id = read_through(&client_ns, &mounted("#task/self/id"));
    let local_id = read_through(&local_ns, &path("#task/self/id"));
    assert_eq!(remote_id, local_id);
    assert_eq!(String::from_utf8_lossy(&remote_id).trim(), "1");

    // The `#task/new` listing advertises the registered driver kinds, proving the
    // service directory (not just a file) crosses the wire.
    let mut remote_kinds: Vec<String> = client_ns
        .read_dir(&mounted("#task/new"))
        .unwrap()
        .into_iter()
        .map(|entry| entry.name().to_owned())
        .collect();
    remote_kinds.sort();
    assert_eq!(remote_kinds, vec!["auto".to_owned(), "noop".to_owned()]);
}

#[test]
fn term_service_stream_reads_exact_server_bytes() {
    let (server_ns, _host) = services_namespace();
    let (client_ns, _remote) = mount_over_loopback(Arc::new(server_ns));
    let (local_ns, _local_host) = services_namespace();

    // Reading `#term/new` allocates a terminal id server-side and returns it.
    // A fresh device allocates id `1`, then `2`: each read reflects live server
    // state, so there is no cached or fabricated buffer. The bytes are exactly
    // the server's bytes, matching a locally driven device.
    let remote_first = read_through(&client_ns, &mounted("#term/new"));
    let local_first = read_through(&local_ns, &path("#term/new"));
    assert_eq!(remote_first, b"1\n");
    assert_eq!(remote_first, local_first);

    let remote_second = read_through(&client_ns, &mounted("#term/new"));
    let local_second = read_through(&local_ns, &path("#term/new"));
    assert_eq!(remote_second, b"2\n");
    assert_eq!(remote_second, local_second);

    // The freshly allocated terminals appear in the device listing over the wire.
    let mut term_entries: Vec<String> = client_ns
        .read_dir(&mounted("#term"))
        .unwrap()
        .into_iter()
        .map(|entry| entry.name().to_owned())
        .collect();
    term_entries.sort();
    assert_eq!(
        term_entries,
        vec!["1".to_owned(), "2".to_owned(), "new".to_owned()]
    );

    // A streamed control file is opened and read across the wire without a
    // fabricated offset: a `#term/<id>/ctl` read yields zero bytes (no queued
    // input) rather than corrupting on a phantom seek. Reading it twice keeps
    // returning the server's honest empty stream.
    let ctl = read_through(&client_ns, &mounted("#term/1/ctl"));
    assert!(
        ctl.is_empty(),
        "control stream returned phantom bytes: {ctl:?}"
    );
    let ctl_again = read_through(&client_ns, &mounted("#term/1/ctl"));
    assert!(ctl_again.is_empty());
}

#[test]
fn kv_service_operated_over_the_wire() {
    // Slice 4: a remote node operates node A's `#kv` store as files. We write
    // `#kv/result` and `#kv/config` through the mount, then read them back — the
    // `mount-write`/`mount-cat` verbs against a service device. A `#kv` value is a
    // non-seekable stream server-side, so a large value also proves the read path
    // stays byte-exact across many sequential `Tread`s with no fabricated offset.
    let (server_ns, _host) = services_namespace();
    let (client_ns, _remote) = mount_over_loopback(Arc::new(server_ns));

    // A value larger than one read chunk, with position-dependent bytes so any
    // off-by-N in the streaming reassembly fails the comparison.
    let blob: Vec<u8> = (0..20_000u32).map(|i| (i % 251) as u8).collect();
    for (key, value) in [
        ("config", b"region=us\n".to_vec()),
        ("result", b"status=ok\n".to_vec()),
        ("blob", blob.clone()),
    ] {
        let mut file = client_ns
            .open(
                &mounted(&format!("#kv/{key}")),
                OpenOptions {
                    read: false,
                    write: true,
                    create: true,
                    truncate: true,
                },
            )
            .unwrap();
        let mut written = 0;
        while written < value.len() {
            let n = file.write(&value[written..]).unwrap();
            assert!(n > 0, "short write to #kv/{key}");
            written += n;
        }
        drop(file);
    }

    assert_eq!(
        read_through(&client_ns, &mounted("#kv/config")),
        b"region=us\n"
    );
    assert_eq!(
        read_through(&client_ns, &mounted("#kv/result")),
        b"status=ok\n"
    );
    assert_eq!(
        read_through(&client_ns, &mounted("#kv/blob")),
        blob,
        "a large non-seekable #kv value streamed back corrupted"
    );
}

#[test]
fn mounted_remote_bytes_match_a_purely_local_namespace() {
    // The strongest truth check: operate the same workload through the mounted
    // remote and through a purely local namespace, and assert byte-identical
    // results for files and service streams alike.
    let (server_ns, server_host) = services_namespace();
    let (client_ns, _remote) = mount_over_loopback(Arc::new(server_ns));
    let (local_ns, local_host) = services_namespace();

    for (ns, target) in [
        (&client_ns, mounted("shared/data.bin")),
        (&local_ns, path("shared/data.bin")),
    ] {
        let parent = target.parent().unwrap();
        ns.create_dir(&parent).unwrap();
        let mut file = ns
            .open(
                &target,
                OpenOptions {
                    read: true,
                    write: true,
                    create: true,
                    truncate: true,
                },
            )
            .unwrap();
        file.write(b"0123456789").unwrap();
    }

    assert_eq!(
        server_host.read_file("shared/data.bin").unwrap(),
        local_host.read_file("shared/data.bin").unwrap()
    );
    assert_eq!(
        read_through(&client_ns, &mounted("shared/data.bin")),
        read_through(&local_ns, &path("shared/data.bin"))
    );

    // Service reads are identical too.
    assert_eq!(
        read_through(&client_ns, &mounted("#task/self/kind")),
        read_through(&local_ns, &path("#task/self/kind"))
    );
}
