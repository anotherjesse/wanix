//! End-to-end mesh truth check: Plan 9 import over a real QUIC connection.
//!
//! Two [`MeshNode`]s are bound on loopback (relay/DNS disabled, direct-address
//! only — no external network). Node A serves a Wanix namespace; node B dials
//! A's direct [`EndpointAddr`] ticket, builds a [`wanix_9p_client::RemoteFs`]
//! over the bridged QUIC stream, binds it at `/n/A`, and drives a full round
//! trip. The unchanged synchronous 9P server and client run over QUIC, with the
//! peer authenticated by its ed25519 key and grants enforced by that identity.

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use wanix_fs::{File, FileSystem, FileType, MemFs, NormalizedPath, OpenOptions};
use wanix_id::{Grant, GrantTable, GrantTablePolicy, NodeIdentity};
use wanix_mesh::{MeshNode, ServeConfig};
use wanix_task::TaskTable;
use wanix_term::TermDevice;
use wanix_vfs::{BindOptions, BindPosition, Namespace, Rights};

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

/// Builds a services namespace: a `MemFs` host root plus `#term` and `#task`.
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

/// Binds node A serving `config` and node B dialing it, returning B's namespace
/// with A's namespace bound at `/n/A`, plus both nodes (kept alive by the
/// caller so the QUIC connection and router are not torn down).
fn connect(config: ServeConfig, attach: &str) -> (Namespace, MeshNode, MeshNode) {
    let identity_a = NodeIdentity::from_secret_bytes([11u8; 32]);
    let identity_b = NodeIdentity::from_secret_bytes([22u8; 32]);
    let mut node_a = MeshNode::bind_local(&identity_a, loopback()).unwrap();
    node_a.serve(config);
    let ticket = node_a.ticket();

    let node_b = MeshNode::bind_local(&identity_b, loopback()).unwrap();
    let remote = node_b.dialer().dial_attach(ticket, attach).unwrap();

    let mut namespace = Namespace::new();
    namespace
        .bind(remote, ".", MOUNT_POINT, BindOptions::default())
        .unwrap();
    (namespace, node_a, node_b)
}

/// Like [`connect`], but pins both nodes to a short per-op deadline so a test
/// can exercise the idle-vs-in-flight distinction without waiting 30s.
fn connect_with_deadline(
    config: ServeConfig,
    deadline: Duration,
) -> (Namespace, MeshNode, MeshNode) {
    let identity_a = NodeIdentity::from_secret_bytes([33u8; 32]);
    let identity_b = NodeIdentity::from_secret_bytes([44u8; 32]);
    let mut node_a = MeshNode::bind_local(&identity_a, loopback())
        .unwrap()
        .with_deadline(deadline);
    node_a.serve(config);
    let ticket = node_a.ticket();

    let node_b = MeshNode::bind_local(&identity_b, loopback())
        .unwrap()
        .with_deadline(deadline);
    let remote = node_b.dialer().dial_attach(ticket, "").unwrap();

    let mut namespace = Namespace::new();
    namespace
        .bind(remote, ".", MOUNT_POINT, BindOptions::default())
        .unwrap();
    (namespace, node_a, node_b)
}

#[test]
fn idle_mount_survives_past_the_op_deadline() {
    // The per-op deadline bounds in-flight work, not the idle steady state of a
    // mounted namespace. A healthy session that sits idle longer than the
    // deadline (9P has no keepalive: the server simply blocks on its next-request
    // read) must NOT be torn down, or the "open a mount, walk away" demo dies.
    let (server_ns, host) = services_namespace();
    let server_root: Arc<dyn FileSystem> = Arc::new(server_ns);
    host.write_file("note.txt", b"still here").unwrap();

    let deadline = Duration::from_millis(300);
    let (client_ns, _a, _b) = connect_with_deadline(ServeConfig::open(server_root), deadline);

    // First op completes within the deadline.
    assert_eq!(
        read_through(&client_ns, &mounted("note.txt")),
        b"still here"
    );

    // Idle well past the deadline with no in-flight request.
    std::thread::sleep(deadline * 4);

    // The session is still alive: a second op succeeds over the same mount.
    assert_eq!(
        read_through(&client_ns, &mounted("note.txt")),
        b"still here"
    );
}

#[test]
fn regular_file_round_trips_through_quic_mount() {
    let (server_ns, host) = services_namespace();
    let server_root: Arc<dyn FileSystem> = Arc::new(server_ns);
    let (client_ns, _a, _b) = connect(ServeConfig::open(server_root), "");

    let payload = b"plan9 import over real QUIC";
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

    // The server's MemFs observed the exact bytes through the QUIC stream.
    assert_eq!(host.read_file("notes/hello.txt").unwrap(), payload);

    // Read back through the client; bytes are identical.
    assert_eq!(
        read_through(&client_ns, &mounted("notes/hello.txt")),
        payload
    );
    let metadata = client_ns.metadata(&mounted("notes/hello.txt")).unwrap();
    assert_eq!(metadata.file_type(), FileType::File);
    assert_eq!(metadata.len(), payload.len() as u64);
}

#[test]
fn service_devices_cross_quic_identically() {
    let (server_ns, _host) = services_namespace();
    let server_root: Arc<dyn FileSystem> = Arc::new(server_ns);
    let (client_ns, _a, _b) = connect(ServeConfig::open(server_root), "");

    // The `#task` device crosses the wire, not just plain files.
    assert_eq!(
        read_through(&client_ns, &mounted("#task/self/kind")),
        b"noop\n"
    );
    // A streamed `#term/new` read returns live server state (id 1 first).
    assert_eq!(read_through(&client_ns, &mounted("#term/new")), b"1\n");
}

#[test]
fn verified_peer_id_keys_a_read_only_grant() {
    // Node B's identity is fixed, so node A can grant exactly that peer.
    let peer_b = NodeIdentity::from_secret_bytes([22u8; 32]).peer_id();
    let (server_ns, _host) = services_namespace();
    let server_root: Arc<dyn FileSystem> = Arc::new(server_ns);

    // Seed a read-only subtree the grant scopes to.
    let backing = Arc::new(MemFs::new());
    backing.create_dir_all("docs").unwrap();
    backing.write_file("docs/readme.txt", b"read me").unwrap();
    let backing_fs: Arc<dyn FileSystem> = backing;

    let grants = GrantTable::new();
    grants.grant(Grant::new(
        peer_b,
        "docs",
        Arc::clone(&backing_fs),
        "docs",
        Rights::read_only(),
    ));
    let policy = Arc::new(GrantTablePolicy::new(grants));
    let config = ServeConfig::guarded(server_root, policy);

    // Node B attaches aname=docs and gets the scoped, read-only subtree.
    let (client_ns, _a, _b) = connect(config, "docs");
    assert_eq!(read_through(&client_ns, &mounted("readme.txt")), b"read me");

    // A write to the read-only grant is denied (the rights gate fires).
    let write = client_ns.open(
        &mounted("readme.txt"),
        OpenOptions {
            read: false,
            write: true,
            create: false,
            truncate: false,
        },
    );
    assert!(
        write.is_err(),
        "a write-mode open on a read-only grant must be denied"
    );
}

#[test]
fn default_deny_denies_an_ungranted_peer() {
    let (server_ns, _host) = services_namespace();
    let server_root: Arc<dyn FileSystem> = Arc::new(server_ns);
    // An empty grant table denies everyone, regardless of attach name.
    let policy = Arc::new(GrantTablePolicy::new(GrantTable::new()));
    let config = ServeConfig::guarded(server_root.clone(), policy);

    let identity_a = NodeIdentity::from_secret_bytes([11u8; 32]);
    let identity_b = NodeIdentity::from_secret_bytes([22u8; 32]);
    let mut node_a = MeshNode::bind_local(&identity_a, loopback()).unwrap();
    node_a.serve(config);
    let ticket = node_a.ticket();
    let node_b = MeshNode::bind_local(&identity_b, loopback()).unwrap();

    // The QUIC connection succeeds, but the 9P attach is default-denied, so
    // session negotiation (Tattach) fails.
    let dialed = node_b.dialer().dial_attach(ticket, "docs");
    assert!(
        dialed.is_err(),
        "default-deny must reject an ungranted peer's attach"
    );
}
