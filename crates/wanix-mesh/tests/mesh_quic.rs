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
use wanix_kv::KvDevice;
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

/// The served services namespace and the live device handles backing it.
///
/// The `host` and `kv` handles alias the filesystems bound into the namespace,
/// so a test can observe server-side state directly after the client mutates it
/// over the wire — proving the bytes that crossed QUIC are the bytes the server
/// stored, with nothing lost or corrupted in transit.
struct Services {
    namespace: Namespace,
    host: Arc<MemFs>,
    kv: KvDevice,
}

/// Builds a services namespace: a `MemFs` host root plus `#kv`, `#term`, `#task`.
///
/// This is the Slice 4 served shape: the same service-device namespace that
/// `wanix mesh-serve --wanix-services` exports, so importing `/n/A/#kv` over the
/// mesh exercises exactly the device a remote node operates as files.
fn services_namespace() -> Services {
    let host = Arc::new(MemFs::new());
    let kv = KvDevice::new();
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
    // `#kv` is a `FileSystem`, so it binds into the served namespace exactly like
    // `#term`/`#task` and imports over the mesh for free: each key is a file under
    // `/n/A/#kv/<key>` reachable through the unchanged 9P client.
    namespace
        .bind(Arc::new(kv.clone()), ".", "#kv", BindOptions::default())
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
    Services {
        namespace,
        host,
        kv,
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
    let services = services_namespace();
    services.host.write_file("note.txt", b"still here").unwrap();
    let server_root: Arc<dyn FileSystem> = Arc::new(services.namespace);

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
    let services = services_namespace();
    let host = services.host.clone();
    let server_root: Arc<dyn FileSystem> = Arc::new(services.namespace);
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
    let services = services_namespace();
    let server_root: Arc<dyn FileSystem> = Arc::new(services.namespace);
    let (client_ns, _a, _b) = connect(ServeConfig::open(server_root), "");

    // The `#task` device crosses the wire, not just plain files.
    assert_eq!(
        read_through(&client_ns, &mounted("#task/self/kind")),
        b"noop\n"
    );
    // A streamed `#term/new` read returns live server state (id 1 first).
    assert_eq!(read_through(&client_ns, &mounted("#term/new")), b"1\n");
}

/// Writes `value` to a `#kv` key over the mounted namespace by creating the key
/// file, writing it, then dropping the handle so the close-time `Tclunk` commits
/// the buffered value on the server.
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
    // Drop sends Tclunk; the server drops the KvWriteFile, committing the buffer.
    drop(file);
}

#[test]
fn kv_service_operated_over_quic_mount() {
    // Slice 4: a remote node operates node A's `#kv` device as ordinary files
    // over QUIC. Node B mounts `/n/A`, reads `/n/A/#kv/config`, and writes
    // `/n/A/#kv/result` — no special-cased code, just the unchanged 9P client
    // resolving into the imported `#kv` FileSystem. Mount-there, run-here.
    let services = services_namespace();
    let kv = services.kv.clone();
    // Node A seeds `config`; the value lands in the server's live store.
    kv.open(
        &path("config"),
        OpenOptions {
            write: true,
            create: true,
            ..OpenOptions::default()
        },
    )
    .unwrap()
    .write(b"region=us\nreplicas=3\n")
    .unwrap();
    let server_root: Arc<dyn FileSystem> = Arc::new(services.namespace);
    let (client_ns, _a, _b) = connect(ServeConfig::open(server_root), "");

    // Node B reads node A's `config` key over the mesh. The value is a streamed,
    // non-seekable `#kv` file on the server (it never honors `Tread.offset`), so
    // this is exactly where Slice 1's honest-seekability correction earns its
    // keep: a purely sequential read returns the exact bytes, uncorrupted.
    assert_eq!(
        read_through(&client_ns, &mounted("#kv/config")),
        b"region=us\nreplicas=3\n"
    );

    // Node B writes a result back into node A's key store over the mesh.
    kv_write(&client_ns, "result", b"status=ok\nbuilt=42\n");

    // The bytes that crossed QUIC are the bytes node A's live store now holds.
    let mut stored = kv.open(&path("result"), OpenOptions::read()).unwrap();
    let mut buf = Vec::new();
    let mut chunk = [0u8; 256];
    loop {
        let n = stored.read(&mut chunk).unwrap();
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    assert_eq!(buf, b"status=ok\nbuilt=42\n");

    // The new key is also visible to node B reading it straight back over the
    // mesh, and `#kv` enumerates both keys through the imported directory.
    assert_eq!(
        read_through(&client_ns, &mounted("#kv/result")),
        b"status=ok\nbuilt=42\n"
    );
    let mut keys: Vec<String> = client_ns
        .read_dir(&mounted("#kv"))
        .unwrap()
        .into_iter()
        .map(|entry| entry.name().to_owned())
        .collect();
    keys.sort();
    assert_eq!(keys, vec!["config".to_owned(), "result".to_owned()]);
}

#[test]
fn large_kv_value_streams_across_quic_without_corruption() {
    // The seekability correction is only honest if a value larger than one read
    // chunk still streams byte-exact. A `#kv` value is non-seekable on the
    // server, so each `Tread` is served sequentially from the server's own
    // cursor; the client must drive purely sequential reads and reassemble the
    // value with nothing dropped, duplicated, or reordered across chunk
    // boundaries — the precise failure a fictional client-side offset would hide.
    let services = services_namespace();
    let kv = services.kv.clone();

    // A multi-kilobyte value with position-dependent bytes: any off-by-N in the
    // streaming reassembly shifts the pattern and fails the comparison.
    let value: Vec<u8> = (0..40_000u32).map(|i| (i % 251) as u8).collect();
    {
        let mut seed = kv
            .open(
                &path("blob"),
                OpenOptions {
                    write: true,
                    create: true,
                    ..OpenOptions::default()
                },
            )
            .unwrap();
        seed.write(&value).unwrap();
    }

    let server_root: Arc<dyn FileSystem> = Arc::new(services.namespace);
    let (client_ns, _a, _b) = connect(ServeConfig::open(server_root), "");

    // Read with a small buffer so the value spans many sequential reads.
    let mut reader = client_ns
        .open(&mounted("#kv/blob"), OpenOptions::read())
        .unwrap();
    let mut got = Vec::new();
    let mut chunk = [0u8; 100];
    loop {
        let n = reader.read(&mut chunk).unwrap();
        if n == 0 {
            break;
        }
        got.extend_from_slice(&chunk[..n]);
        assert!(got.len() <= value.len(), "stream over-read its value");
    }
    assert_eq!(got, value, "the streamed #kv value crossed QUIC corrupted");
}

#[test]
fn verified_peer_id_keys_a_read_only_grant() {
    // Node B's identity is fixed, so node A can grant exactly that peer.
    let peer_b = NodeIdentity::from_secret_bytes([22u8; 32]).peer_id();
    let server_root: Arc<dyn FileSystem> = Arc::new(services_namespace().namespace);

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
    let server_root: Arc<dyn FileSystem> = Arc::new(services_namespace().namespace);
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
