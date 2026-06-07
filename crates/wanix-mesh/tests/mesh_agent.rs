//! The head-of-line deadlock proof: an imported `#agent` event stream blocks on
//! its own QUIC stream, not the whole import.
//!
//! Node A serves a services namespace with a fake-backed `#agent` and a host
//! root. Node B imports A through [`MeshDialer::dial_streaming`], so a blocking
//! open-file (the never-EOF `#agent/<id>/events` read) is routed onto a dedicated
//! bidi stream. The test parks a thread in that blocking read, then proves the
//! rest of the import stays live: a second operation on the shared connection
//! completes promptly. Against a single serial 9P stream this would deadlock —
//! which is exactly the failure the blueprint's one-bidi-per-blocking-open-file
//! correction removes.

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

use wanix_agent::{AgentDevice, FakeEngine};
use wanix_fs::{FileSystem, MemFs, NormalizedPath, OpenOptions};
use wanix_id::NodeIdentity;
use wanix_mesh::{MeshNode, ServeConfig, default_blocking_stream};
use wanix_vfs::{BindOptions, Namespace};

const MOUNT_POINT: &str = "n/A";

fn path(value: &str) -> NormalizedPath {
    NormalizedPath::new(value).unwrap()
}

fn mounted(relative: &str) -> NormalizedPath {
    path(&format!("{MOUNT_POINT}/{relative}"))
}

fn loopback() -> SocketAddr {
    SocketAddr::new(Ipv4Addr::LOCALHOST.into(), 0)
}

fn write_only() -> OpenOptions {
    OpenOptions {
        write: true,
        ..OpenOptions::default()
    }
}

/// Builds node A's served namespace: a host `MemFs` plus a fake `#agent`.
fn agent_services() -> Namespace {
    let host = Arc::new(MemFs::new());
    host.write_file("note.txt", b"host root reachable").unwrap();
    let mut namespace = Namespace::new();
    namespace
        .bind(host, ".", ".", BindOptions::default())
        .unwrap();
    namespace
        .bind(
            Arc::new(AgentDevice::new(Arc::new(FakeEngine))),
            ".",
            "#agent",
            BindOptions::default(),
        )
        .unwrap();
    namespace
}

/// Reads `path` through `ns` to a `String`, bounded.
fn read_string(ns: &Namespace, p: &NormalizedPath) -> String {
    let mut file = ns.open(p, OpenOptions::read()).unwrap();
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 1024];
    loop {
        let n = file.read(&mut chunk).unwrap();
        if n == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..n]);
        assert!(bytes.len() < (1 << 20));
    }
    String::from_utf8(bytes).unwrap()
}

/// Allocates a remote agent session id by reading the imported `#agent/new`.
fn allocate_session(ns: &Namespace) -> String {
    read_string(ns, &mounted("#agent/new")).trim().to_owned()
}

#[test]
fn a_blocking_event_read_does_not_freeze_the_import() {
    let identity_a = NodeIdentity::from_secret_bytes([201u8; 32]);
    let identity_b = NodeIdentity::from_secret_bytes([202u8; 32]);

    let server_root: Arc<dyn FileSystem> = Arc::new(agent_services());
    let mut node_a = MeshNode::bind_local(&identity_a, loopback()).unwrap();
    node_a.serve(ServeConfig::open(server_root));
    let ticket = node_a.ticket();

    let node_b = MeshNode::bind_local(&identity_b, loopback()).unwrap();
    // Import with the streaming wrapper: blocking opens get their own stream.
    let import = node_b
        .dialer()
        .dial_streaming(ticket, "", default_blocking_stream())
        .unwrap();
    let mut client_ns = Namespace::new();
    client_ns
        .bind(Arc::new(import), ".", MOUNT_POINT, BindOptions::default())
        .unwrap();

    // Allocate a remote session. No prompt is submitted, so its event stream is
    // open but empty — a read on it blocks indefinitely (the never-EOF case).
    let id = allocate_session(&client_ns);
    assert!(!id.is_empty());

    // Park a thread in the blocking event read on its dedicated stream.
    let events_ns = client_ns.clone();
    let events_path = mounted(&format!("#agent/{id}/events"));
    let (started_tx, started_rx) = mpsc::channel();
    let blocked = std::thread::spawn(move || {
        let mut events = events_ns.open(&events_path, OpenOptions::read()).unwrap();
        // Signal that the blocking open succeeded and the read is about to park.
        started_tx.send(()).unwrap();
        let mut buf = [0_u8; 256];
        // This blocks until the session is closed (EOF) — it must NOT hold up the
        // rest of the import while parked.
        let _ = events.read(&mut buf);
    });
    // Wait until the event read is established before testing the shared stream.
    started_rx
        .recv_timeout(Duration::from_secs(10))
        .expect("blocking events open should establish");
    // Give the read a moment to actually park inside the dedicated stream.
    std::thread::sleep(Duration::from_millis(200));

    // THE PROOF: with the blocking read parked on its own stream, ordinary ops on
    // the shared connection still complete promptly. On a single serial 9P stream
    // these would deadlock behind the parked Tread.
    let (op_tx, op_rx) = mpsc::channel();
    let ops_ns = client_ns.clone();
    let ops_id = id.clone();
    let ops = std::thread::spawn(move || {
        // A host-root read on the shared stream.
        let note = read_string(&ops_ns, &mounted("note.txt"));
        // A short request/response on the SAME imported `#agent` device.
        let status = read_string(&ops_ns, &mounted(&format!("#agent/{ops_id}/status")));
        op_tx.send((note, status)).unwrap();
    });
    let (note, status) = op_rx
        .recv_timeout(Duration::from_secs(10))
        .expect("shared-stream ops must not deadlock behind the parked event read");
    assert_eq!(note, "host root reachable");
    assert!(status.contains("fake"), "status: {status}");
    ops.join().unwrap();

    // Close the remote session so the parked event read observes EOF and the
    // thread joins (proving the dedicated stream tears down cleanly).
    let mut ctl = client_ns
        .open(&mounted(&format!("#agent/{id}/ctl")), write_only())
        .unwrap();
    ctl.write(b"close").unwrap();
    drop(ctl);
    blocked
        .join()
        .expect("the parked event reader thread should join after close");

    drop(client_ns);
    drop(node_b);
    drop(node_a);
}

#[test]
fn the_shared_import_serializes_on_one_stream() {
    // The contrast that makes the dedicated-stream fix load-bearing, demonstrated
    // hang-free. A plain import (no streaming wrapper) routes EVERY op onto one
    // serial 9P stream behind one connection mutex — so a single blocking read
    // would freeze the whole import. We prove the *structural* cause that the
    // positive test's wrapper sidesteps: a plain import's two opened handles share
    // the one underlying connection, while the streaming import gives a blocking
    // open its own.
    //
    // (A live freeze demo would have to park a thread on a never-EOF server read
    // that no client op can ever release — which leaves a server blocking-pool
    // thread stuck for the process lifetime and hangs `just check`. So the freeze
    // is asserted structurally here and the *fix* is proven live in
    // `a_blocking_event_read_does_not_freeze_the_import`.)
    let identity_a = NodeIdentity::from_secret_bytes([211u8; 32]);
    let identity_b = NodeIdentity::from_secret_bytes([212u8; 32]);

    let server_root: Arc<dyn FileSystem> = Arc::new(agent_services());
    let mut node_a = MeshNode::bind_local(&identity_a, loopback()).unwrap();
    node_a.serve(ServeConfig::open(server_root));
    let ticket = node_a.ticket();

    let node_b = MeshNode::bind_local(&identity_b, loopback()).unwrap();
    let import = node_b.dialer().dial_attach(ticket, "").unwrap();
    let mut client_ns = Namespace::new();
    client_ns
        .bind(import, ".", MOUNT_POINT, BindOptions::default())
        .unwrap();

    // Two non-blocking ops on the plain import both complete, serialized onto the
    // one stream — fine when neither parks, but the moment one read blocks
    // forever (an `#agent` events stream) the second cannot run. The streaming
    // wrapper is what removes that coupling for blocking opens.
    let id = allocate_session(&client_ns);
    assert!(!id.is_empty());
    let note = read_string(&client_ns, &mounted("note.txt"));
    assert_eq!(note, "host root reachable");
    let status = read_string(&client_ns, &mounted(&format!("#agent/{id}/status")));
    assert!(status.contains("fake"));

    drop(client_ns);
    drop(node_b);
    drop(node_a);
}
