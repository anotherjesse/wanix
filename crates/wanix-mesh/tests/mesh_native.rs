//! End-to-end native-wire truth check: Plan 9 import over real QUIC, no 9P.
//!
//! Two [`MeshNode`]s bind on loopback (relay/DNS disabled, direct-address only —
//! no external network). Node A serves a Wanix `FileSystem` over the **native**
//! `wanix-mesh-wire` plane ([`MeshNode::serve_native`]); node B dials A's direct
//! [`EndpointAddr`] ticket over [`wanix_mesh::WANIX_FS_ALPN`], builds a
//! [`wanix_mesh::NativeFs`] over the held QUIC connection (one bidi stream per op
//! / per open file), binds it at `/n/A`, and drives a full round trip — typed
//! errors, not an errno round-trip, with the peer authenticated by its ed25519
//! key.
//!
//! The second test is the **head-of-line proof**: a never-EOF `#plumb/<topic>/
//! recv` read parks only its own stream and cannot stall a concurrent op on a
//! sibling stream — the property `StreamingImportFs` hand-discovers on the 9P
//! plane, here structural and so `StreamingImportFs` is absent from the native
//! import path entirely.

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use wanix_fs::{File, FileSystem, FileType, MemFs, NormalizedPath, OpenOptions};
use wanix_id::NodeIdentity;
use wanix_kv::KvDevice;
use wanix_mesh::{MeshNode, NativeServeConfig};
use wanix_plumb::{PlumbDevice, PlumbEnvelope};
use wanix_vfs::{BindOptions, Namespace};

/// Relative Wanix path the remote filesystem is bound at.
const MOUNT_POINT: &str = "n/A";

fn path(value: &str) -> NormalizedPath {
    NormalizedPath::new(value).unwrap()
}

fn mounted(relative: &str) -> NormalizedPath {
    path(&format!("{MOUNT_POINT}/{relative}"))
}

fn loopback() -> SocketAddr {
    // Port 0: the OS assigns a free loopback port for the test endpoint.
    SocketAddr::new(Ipv4Addr::LOCALHOST.into(), 0)
}

/// The served namespace and the live device handles backing it.
///
/// The `host`, `kv`, and `plumb` handles alias the filesystems bound into the
/// namespace, so a test can observe server-side state directly after the client
/// mutates it over the native wire — proving the bytes that crossed QUIC are the
/// bytes the server stored, with nothing lost or corrupted in transit.
struct Services {
    namespace: Namespace,
    host: Arc<MemFs>,
    kv: KvDevice,
    plumb: PlumbDevice,
}

/// Builds a services namespace: a `MemFs` host root plus `#kv` and `#plumb`.
///
/// `#plumb` is a single-node [`wanix_plumb::LocalPlumbPort`] device (no gossip
/// swarm needed): its `recv` is a real blocking, never-EOF read, the
/// deterministic primitive the head-of-line proof parks on.
fn services_namespace() -> Services {
    let host = Arc::new(MemFs::new());
    let kv = KvDevice::new();
    let plumb = PlumbDevice::local();
    let mut namespace = Namespace::new();
    namespace
        .bind(host.clone(), ".", ".", BindOptions::default())
        .unwrap();
    // `#kv` and `#plumb` are plain `FileSystem`s, so they bind into the served
    // namespace and import over the native wire for free.
    namespace
        .bind(Arc::new(kv.clone()), ".", "#kv", BindOptions::default())
        .unwrap();
    namespace
        .bind(
            Arc::new(plumb.clone()),
            ".",
            "#plumb",
            BindOptions::default(),
        )
        .unwrap();
    Services {
        namespace,
        host,
        kv,
        plumb,
    }
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

fn read_through(namespace: &Namespace, p: &NormalizedPath) -> Vec<u8> {
    let file = namespace.open(p, OpenOptions::read()).unwrap();
    read_all(file)
}

/// Binds node A serving `config` over the native wire and node B dialing it,
/// returning B's namespace with A's namespace bound at `/n/A`, plus both nodes
/// (kept alive by the caller so the QUIC connection and router are not torn
/// down — the [`wanix_mesh::IrohStreamFactory`] holds a non-owning runtime
/// handle, so dropping node B would panic the next op).
fn connect(config: NativeServeConfig) -> (Namespace, MeshNode, MeshNode) {
    let identity_a = NodeIdentity::from_secret_bytes([11u8; 32]);
    let identity_b = NodeIdentity::from_secret_bytes([22u8; 32]);
    let mut node_a = MeshNode::bind_local(&identity_a, loopback()).unwrap();
    node_a.serve_native(config);
    let ticket = node_a.ticket();

    let node_b = MeshNode::bind_local(&identity_b, loopback()).unwrap();
    let remote = node_b.dialer().dial_native(ticket).unwrap();

    let mut namespace = Namespace::new();
    namespace
        .bind(remote, ".", MOUNT_POINT, BindOptions::default())
        .unwrap();
    (namespace, node_a, node_b)
}

/// Writes `value` to a `#kv` key over the mounted namespace, dropping the handle
/// so the close-time half-close commits the buffered value on the server.
fn kv_write(namespace: &Namespace, key: &str, value: &[u8]) {
    let mut file = namespace
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
        assert!(n > 0, "unexpected short write to #kv/{key}");
        written += n;
    }
    // Drop finishes the send half; the server drops the KvWriteFile, committing.
    drop(file);
}

#[test]
fn full_round_trip_and_kv_write_cross_the_native_wire() {
    let services = services_namespace();
    let host = services.host.clone();
    let kv = services.kv.clone();
    let server_root: Arc<dyn FileSystem> = Arc::new(services.namespace);
    let (client_ns, _a, _b) = connect(NativeServeConfig::open(server_root));

    // A regular file round-trips: write through the native wire, observe the
    // exact bytes server-side, read them back, and confirm typed metadata.
    let payload = b"plan9 import over the native wire";
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
    assert_eq!(host.read_file("notes/hello.txt").unwrap(), payload);
    assert_eq!(
        read_through(&client_ns, &mounted("notes/hello.txt")),
        payload
    );
    let metadata = client_ns.metadata(&mounted("notes/hello.txt")).unwrap();
    assert_eq!(metadata.file_type(), FileType::File);
    assert_eq!(metadata.len(), payload.len() as u64);

    // The `#kv` device crosses the wire as ordinary files: node B writes a key
    // and the bytes that crossed QUIC are the bytes node A's live store holds.
    //
    // The native close is a fire-and-forget half-close (Plan §3.1, "replaces
    // Tclunk"): the client's Drop finishes its send half and returns without
    // waiting for the server to observe it, so the server's close-time commit of
    // a `#kv` value lands asynchronously. A caller observing the commit must wait
    // for it; this test polls the server-side store (bounded) for the committed
    // bytes — the honest read-after-half-close contract.
    kv_write(&client_ns, "result", b"status=ok\nbuilt=42\n");
    assert_eq!(
        await_kv_value(&kv, "result", b"status=ok\nbuilt=42\n"),
        b"status=ok\nbuilt=42\n",
        "the bytes that crossed QUIC must be the bytes node A's #kv store committed"
    );
    // The same key reads straight back over the native wire (its own op stream),
    // also bounded-polled since the read may race the still-in-flight half-close.
    let read_back = await_wire_value(&client_ns, "#kv/result", b"status=ok\nbuilt=42\n");
    assert_eq!(read_back, b"status=ok\nbuilt=42\n");
}

/// Polls node A's live `#kv` store (bounded) until `key` holds `expected`,
/// accommodating the asynchronous commit of the fire-and-forget half-close.
fn await_kv_value(kv: &KvDevice, key: &str, expected: &[u8]) -> Vec<u8> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let mut got = Vec::new();
        let mut stored = kv.open(&path(key), OpenOptions::read()).unwrap();
        let mut chunk = [0u8; 256];
        loop {
            let n = stored.read(&mut chunk).unwrap();
            if n == 0 {
                break;
            }
            got.extend_from_slice(&chunk[..n]);
        }
        if got == expected || Instant::now() >= deadline {
            return got;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Reads `relative` over the native wire (bounded retry) until it yields
/// `expected`, accommodating a read that races the still-in-flight half-close.
fn await_wire_value(namespace: &Namespace, relative: &str, expected: &[u8]) -> Vec<u8> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let got = read_through(namespace, &mounted(relative));
        if got == expected || Instant::now() >= deadline {
            return got;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn typed_not_found_error_crosses_the_native_wire() {
    // A typed application error crosses the wire as the *same* FsError variant,
    // not an errno round-trip: opening a missing path is NotFound, not a lossy
    // Other("remote errno 2").
    let services = services_namespace();
    let server_root: Arc<dyn FileSystem> = Arc::new(services.namespace);
    let (client_ns, _a, _b) = connect(NativeServeConfig::open(server_root));

    let result = client_ns.metadata(&mounted("does/not/exist"));
    match result {
        Err(wanix_fs::FsError::NotFound) => {}
        other => panic!("expected typed NotFound across the native wire, got {other:?}"),
    }
}

#[test]
fn a_never_eof_recv_does_not_stall_a_concurrent_op() {
    // The head-of-line proof. A `#plumb/<topic>/recv` open is a never-EOF
    // blocking read: it parks server-side in the plumb LineBuffer until a message
    // arrives. On the native wire it rides its OWN bidi stream, so it cannot
    // freeze a sibling op — exactly what StreamingImportFs hand-discovers on the
    // 9P plane, here structural (no StreamingImportFs on the native import path).
    let services = services_namespace();
    services
        .host
        .write_file("sibling.txt", b"unblocked")
        .unwrap();
    let plumb = services.plumb.clone();
    let server_root: Arc<dyn FileSystem> = Arc::new(services.namespace);
    let (client_ns, _a, _b) = connect(NativeServeConfig::open(server_root));

    // Open and read `#plumb/topic/recv` on a background thread. The read parks
    // server-side (no message yet) on its own QUIC stream.
    let recv_ns = client_ns.clone();
    let recv_started = Arc::new(AtomicBool::new(false));
    let recv_started_writer = Arc::clone(&recv_started);
    let recv_thread = std::thread::spawn(move || {
        let mut recv = recv_ns
            .open(&mounted("#plumb/build/recv"), OpenOptions::read())
            .unwrap();
        // Signal the open completed and we are about to park on the read.
        recv_started_writer.store(true, Ordering::SeqCst);
        let mut buf = [0_u8; 1024];
        let n = recv.read(&mut buf).unwrap();
        String::from_utf8(buf[..n].to_vec()).unwrap()
    });

    // Wait until the recv stream has opened and is parked on its read.
    let deadline = Instant::now() + Duration::from_secs(5);
    while !recv_started.load(Ordering::SeqCst) {
        assert!(Instant::now() < deadline, "recv open never completed");
        std::thread::sleep(Duration::from_millis(5));
    }
    // Give the parked read a beat to actually reach the blocking server-side read.
    std::thread::sleep(Duration::from_millis(100));

    // While recv is parked, a concurrent op on a SIBLING stream must complete
    // promptly. If the never-EOF read held the head of the line, this would hang.
    let concurrent = Instant::now();
    assert_eq!(
        read_through(&client_ns, &mounted("sibling.txt")),
        b"unblocked",
        "a sibling op must cross the native wire while recv is parked"
    );
    let stat = client_ns.metadata(&mounted("sibling.txt")).unwrap();
    assert_eq!(stat.file_type(), FileType::File);
    assert!(
        concurrent.elapsed() < Duration::from_secs(2),
        "the concurrent op took {:?}; the parked recv stalled the head of the line",
        concurrent.elapsed()
    );

    // The recv thread is still parked (nothing published yet): publish, and it
    // wakes and returns the message — proving the streaming read works end to end
    // on its own stream alongside the sibling traffic.
    let envelope = PlumbEnvelope {
        kind: "task.done".to_owned(),
        from: "node-b".to_owned(),
        to: String::new(),
        body: serde_json::json!({ "out": "/world/result" }),
    };
    let line = envelope.to_line().unwrap();
    let json = &line[..line.len() - 1];
    let mut send = client_ns
        .open(
            &mounted("#plumb/build/send"),
            OpenOptions {
                write: true,
                ..OpenOptions::default()
            },
        )
        .unwrap();
    let mut written = 0;
    while written < json.len() {
        written += send.write(&json[written..]).unwrap();
    }
    drop(send);
    let _ = plumb; // keep the server-side device handle alive for the assertion.

    let received = recv_thread
        .join()
        .expect("recv thread panicked while parked on its own stream");
    let parsed = PlumbEnvelope::parse(received.trim_end().as_bytes()).unwrap();
    assert_eq!(parsed, envelope);
}
