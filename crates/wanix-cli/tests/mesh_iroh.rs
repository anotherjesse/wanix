//! CLI-level mesh truth check: import a remote namespace over real QUIC.
//!
//! This mirrors `mesh_loopback.rs` (which uses a loopback TCP socket) but drives
//! the `wanix-mesh` iroh QUIC transport instead, proving the same namespace
//! operations the `mount-*` verbs run resolve identically when bound through a
//! [`wanix_mesh::MeshNode`] dial. Both nodes bind on loopback with relays/DNS
//! disabled, so the test needs no external network.
//!
//! The `iroh://` mount path imports over the **native** `wanix-mesh-wire` plane
//! (typed `FsError`s, one bidi stream per op / per open file), not 9P, exactly
//! as the CLI's `dial_iroh_remote` does after the native-mesh-wire swap (plan
//! §9). 9P stays at the foreign edge (the `tcp://` mount path).

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;

use wanix_fs::{File, FileSystem, FileType, MemFs, NormalizedPath, OpenOptions};
use wanix_id::NodeIdentity;
use wanix_kv::KvDevice;
use wanix_mesh::{MeshNode, NativeServeConfig};
use wanix_task::TaskTable;
use wanix_term::TermDevice;
use wanix_vfs::{BindOptions, BindPosition, Namespace};

const MOUNT_POINT: &str = "n/remote";

fn path(value: &str) -> NormalizedPath {
    NormalizedPath::new(value).unwrap()
}

fn mounted(relative: &str) -> NormalizedPath {
    path(&format!("{MOUNT_POINT}/{relative}"))
}

fn loopback() -> SocketAddr {
    SocketAddr::new(Ipv4Addr::LOCALHOST.into(), 0)
}

/// Builds a services namespace: a `MemFs` host root plus `#kv`, `#term`, `#task`.
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
    // `#kv` is a `FileSystem`, so it binds beside `#term`/`#task` and imports over
    // the mesh for free: `/n/remote/#kv/<key>` is reachable through the client.
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

/// Serves `root` over the native wire and binds it at `/n/remote` through a
/// dialed mount, returning the client namespace and both nodes (kept alive by
/// the caller).
fn mount_over_quic(root: Arc<dyn FileSystem>) -> (Namespace, MeshNode, MeshNode) {
    let server_identity = NodeIdentity::from_secret_bytes([1u8; 32]);
    let client_identity = NodeIdentity::from_secret_bytes([2u8; 32]);
    let mut server = MeshNode::bind_local(&server_identity, loopback()).unwrap();
    server.serve_native(NativeServeConfig::open(root));
    let ticket = server.ticket();

    let client = MeshNode::bind_local(&client_identity, loopback()).unwrap();
    let remote = client.dialer().dial_native(ticket).unwrap();
    let mut namespace = Namespace::new();
    namespace
        .bind(remote, ".", MOUNT_POINT, BindOptions::default())
        .unwrap();
    (namespace, server, client)
}

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
    read_all(namespace.open(p, OpenOptions::read()).unwrap())
}

#[test]
fn mount_verbs_round_trip_a_file_over_iroh() {
    let (server_ns, host) = services_namespace();
    let (client_ns, _server, _client) = mount_over_quic(Arc::new(server_ns));

    // `mount-write` semantics: create/truncate then write through the namespace.
    let payload = b"written across the QUIC wire";
    client_ns.create_dir(&mounted("docs")).unwrap();
    let mut file = client_ns
        .open(
            &mounted("docs/note.txt"),
            OpenOptions {
                read: false,
                write: true,
                create: true,
                truncate: true,
            },
        )
        .unwrap();
    assert_eq!(file.write(payload).unwrap(), payload.len());
    drop(file);

    // The bytes landed on the server host through the mesh.
    assert_eq!(host.read_file("docs/note.txt").unwrap(), payload);
    // `mount-cat` semantics: read them back verbatim.
    assert_eq!(read_through(&client_ns, &mounted("docs/note.txt")), payload);

    // `mount-ls` semantics: the new entry shows over the wire.
    let mut names: Vec<String> = client_ns
        .read_dir(&mounted("docs"))
        .unwrap()
        .into_iter()
        .map(|entry| entry.name().to_owned())
        .collect();
    names.sort();
    assert_eq!(names, vec!["note.txt".to_owned()]);
}

#[test]
fn service_devices_cross_iroh() {
    let (server_ns, _host) = services_namespace();
    let (client_ns, _server, _client) = mount_over_quic(Arc::new(server_ns));

    // `#`-devices cross the QUIC mount, not just plain files.
    assert_eq!(
        client_ns
            .metadata(&mounted("#task/self/kind"))
            .unwrap()
            .file_type(),
        FileType::File
    );
    assert_eq!(
        read_through(&client_ns, &mounted("#task/self/kind")),
        b"noop\n"
    );
}

#[test]
fn kv_device_operated_over_iroh() {
    // Slice 4 demo: a remote node operates node A's `#kv` store as files over the
    // QUIC mount. We write `#kv/result` and read `#kv/config` purely through the
    // mounted namespace — `mount-write`/`mount-cat` semantics against a service
    // device. The values are non-seekable streams server-side, so this also
    // exercises the honest-seekability read path with no fabricated offset.
    let (server_ns, _host) = services_namespace();
    let (client_ns, _server, _client) = mount_over_quic(Arc::new(server_ns));

    // Seed `config` through the mount (the store starts empty), then mutate
    // `result` — both keys are operated remotely as ordinary files.
    for (key, value) in [
        ("config", b"region=us\n".as_slice()),
        ("result", b"status=ok\n".as_slice()),
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
        assert_eq!(file.write(value).unwrap(), value.len());
        drop(file);
    }

    // Read both keys back over the mesh, byte-exact.
    assert_eq!(
        read_through(&client_ns, &mounted("#kv/config")),
        b"region=us\n"
    );
    assert_eq!(
        read_through(&client_ns, &mounted("#kv/result")),
        b"status=ok\n"
    );

    // The `#kv` directory enumerates both keys through the imported listing.
    let mut keys: Vec<String> = client_ns
        .read_dir(&mounted("#kv"))
        .unwrap()
        .into_iter()
        .map(|entry| entry.name().to_owned())
        .collect();
    keys.sort();
    assert_eq!(keys, vec!["config".to_owned(), "result".to_owned()]);
}

/// A dialed mount that keeps its dialer node alive alongside the namespace, the
/// exact shape the CLI's `IrohMount` keepalive enforces.
struct DialerMount {
    namespace: Namespace,
    /// The dialer node owns the tokio runtime the native import runs every op
    /// on. Held so the runtime is not shut down out from under an in-flight op.
    _dialer: MeshNode,
}

/// Mirrors `wanix-cli`'s `dial_iroh_remote`: bind a *fresh* dialer node, dial
/// over the native wire, bind the remote into a namespace, and return BOTH so
/// the node outlives the mount. Crucially the server-side binding is dropped
/// before the returned mount is used, so this reproduces the real CLI drop path
/// the in-process helpers miss.
fn dial_into_mount(ticket: wanix_mesh::EndpointAddr) -> DialerMount {
    let dialer_identity = NodeIdentity::from_secret_bytes([99u8; 32]);
    let dialer = MeshNode::bind_local(&dialer_identity, loopback()).unwrap();
    let remote = dialer.dialer().dial_native_attach(ticket, "").unwrap();
    let mut namespace = Namespace::new();
    namespace
        .bind(remote, ".", MOUNT_POINT, BindOptions::default())
        .unwrap();
    DialerMount {
        namespace,
        _dialer: dialer,
    }
}

#[test]
fn mount_survives_the_dialer_function_returning() {
    // Regression: the CLI used to return only the `RemoteFs`, dropping the dialer
    // node (which owns the runtime). The first op on the mount then ran inside
    // `block_on` on the shut-down runtime and PANICKED ("a Tokio 1.x context was
    // found, but it is being shutdown"). The fix keeps the node alive alongside
    // the mount. Here the mount is built and the dialing scope returns; the node
    // survives inside the returned struct, so the first op must succeed.
    let (server_ns, host) = services_namespace();
    host.write_file("after.txt", b"runtime still alive")
        .unwrap();
    let server_identity = NodeIdentity::from_secret_bytes([7u8; 32]);
    let mut server = MeshNode::bind_local(&server_identity, loopback()).unwrap();
    server.serve_native(NativeServeConfig::open(Arc::new(server_ns)));
    let ticket = server.ticket();

    // `dial_into_mount` returns; the dialer node lives only inside the mount.
    let mount = dial_into_mount(ticket);

    // The FIRST op after the dialing function returned must not panic.
    assert_eq!(
        read_through(&mount.namespace, &mounted("after.txt")),
        b"runtime still alive"
    );
}
