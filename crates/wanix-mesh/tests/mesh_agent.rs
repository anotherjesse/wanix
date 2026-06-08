//! The head-of-line proof on the native wire: an imported `#agent` event stream
//! blocks on its own QUIC stream, not the whole import.
//!
//! Node A serves a services namespace with a fake-backed `#agent` and a host
//! root over the **native** `wanix-mesh-wire` plane ([`MeshNode::serve_native`]).
//! Node B imports A with [`MeshDialer::dial_native`] — a plain [`NativeFs`], no
//! streaming wrapper. On the native wire every open file rides its own bidi
//! stream **by construction**, so a never-EOF `#agent/<id>/events` read parks
//! only its own stream and cannot freeze any sibling op. The test parks a thread
//! in that blocking read, then proves the rest of the import stays live: a
//! second operation completes promptly. On the 9P plane this needed the
//! dedicated-stream `StreamingImportFs` wrapper to dodge a single-serial-stream
//! deadlock; on the native plane the property is structural and the wrapper is
//! absent from the import path entirely.

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

use wanix_agent::{AgentDevice, FakeEngine};
use wanix_fs::{FileSystem, MemFs, NormalizedPath, OpenOptions};
use wanix_id::NodeIdentity;
use wanix_mesh::{MeshNode, NativeServeConfig};
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

/// Imports node A's native serve at `/n/A` in node B, returning B's namespace
/// and both nodes (kept alive by the caller: the native `IrohStreamFactory`
/// holds a non-owning runtime handle, so dropping node B would panic the next
/// op).
fn import_native(server_root: Arc<dyn FileSystem>) -> (Namespace, MeshNode, MeshNode) {
    let identity_a = NodeIdentity::from_secret_bytes([201u8; 32]);
    let identity_b = NodeIdentity::from_secret_bytes([202u8; 32]);

    let mut node_a = MeshNode::bind_local(&identity_a, loopback()).unwrap();
    node_a.serve_native(NativeServeConfig::open(server_root));
    let ticket = node_a.ticket();

    let node_b = MeshNode::bind_local(&identity_b, loopback()).unwrap();
    // A plain native import: no streaming wrapper. Every open file gets its own
    // bidi stream by construction.
    let import = node_b.dialer().dial_native(ticket).unwrap();
    let mut client_ns = Namespace::new();
    client_ns
        .bind(import, ".", MOUNT_POINT, BindOptions::default())
        .unwrap();
    (client_ns, node_a, node_b)
}

#[test]
fn a_blocking_event_read_does_not_freeze_the_import() {
    let server_root: Arc<dyn FileSystem> = Arc::new(agent_services());
    let (client_ns, node_a, node_b) = import_native(server_root);

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
    // Wait until the event read is established before testing the sibling stream.
    started_rx
        .recv_timeout(Duration::from_secs(10))
        .expect("blocking events open should establish");
    // Give the read a moment to actually park inside the dedicated stream.
    std::thread::sleep(Duration::from_millis(200));

    // THE PROOF: with the blocking read parked on its own stream, ordinary ops on
    // sibling streams still complete promptly. On a single serial 9P stream these
    // would deadlock behind the parked Tread; on the native wire each op is its
    // own stream, so the parked read holds the head of nothing.
    let (op_tx, op_rx) = mpsc::channel();
    let ops_ns = client_ns.clone();
    let ops_id = id.clone();
    let ops = std::thread::spawn(move || {
        // A host-root read on its own stream.
        let note = read_string(&ops_ns, &mounted("note.txt"));
        // A short request/response on the SAME imported `#agent` device.
        let status = read_string(&ops_ns, &mounted(&format!("#agent/{ops_id}/status")));
        op_tx.send((note, status)).unwrap();
    });
    let (note, status) = op_rx
        .recv_timeout(Duration::from_secs(10))
        .expect("sibling-stream ops must not deadlock behind the parked event read");
    assert_eq!(note, "host root reachable");
    assert!(status.contains("fake"), "status: {status}");
    ops.join().unwrap();

    // Close the remote session so the parked event read observes EOF and the
    // thread joins (proving the dedicated stream tears down cleanly). The `ctl`
    // write commits server-side before `write` returns (the native write waits
    // for the server's `Wrote` reply), so the close wakes the parked read.
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
fn a_plain_native_import_needs_no_streaming_wrapper() {
    // The native counterpart to the 9P plane's `the_shared_import_serializes_on_
    // one_stream`. On the 9P plane a plain import routes every op onto one serial
    // stream behind one connection mutex, so a blocking read would freeze the
    // whole import — which is why the 9P path needs the dedicated-stream
    // `StreamingImportFs` wrapper. On the native plane there is no serial
    // connection: a plain `NativeFs` already gives every op / every open file its
    // own bidi stream, so the same `#agent` ops run with no wrapper at all.
    let server_root: Arc<dyn FileSystem> = Arc::new(agent_services());
    let (client_ns, node_a, node_b) = import_native(server_root);

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
