//! Resilience checks for `iroh://` native mesh mounts.
//!
//! These tests pin the agent-facing contract for live volume/resource mounts:
//! a mounted resource is a durable namespace binding, but any single in-flight
//! operation is bounded. If the provider disappears, fresh operations fail with
//! transport `EIO`-class errors until the same peer identity comes back; old
//! open file handles are not transparently resurrected.

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use wanix_fs::{File, FileSystem, FsError, MemFs, NormalizedPath, OpenOptions};
use wanix_id::NodeIdentity;
use wanix_mesh::{MeshNode, NativeServeConfig, endpoint_id_for};
use wanix_vfs::{BindOptions, Namespace};

const MOUNT_POINT: &str = "vol/notes";
const SHORT_DEADLINE: Duration = Duration::from_millis(250);

fn path(value: &str) -> NormalizedPath {
    NormalizedPath::new(value).unwrap()
}

fn mounted(relative: &str) -> NormalizedPath {
    path(&format!("{MOUNT_POINT}/{relative}"))
}

fn loopback() -> SocketAddr {
    SocketAddr::new(Ipv4Addr::LOCALHOST.into(), 0)
}

fn unused_loopback_addr() -> SocketAddr {
    let socket = std::net::UdpSocket::bind(loopback()).unwrap();
    socket.local_addr().unwrap()
}

fn mem_root(marker: &[u8]) -> Arc<MemFs> {
    let root = Arc::new(MemFs::new());
    root.write_file("marker.txt", marker).unwrap();
    root
}

fn serve(identity: &NodeIdentity, root: Arc<MemFs>) -> MeshNode {
    let mut server = MeshNode::bind_local(identity, loopback())
        .unwrap()
        .with_deadline(SHORT_DEADLINE);
    server.serve_native(NativeServeConfig::open(root as Arc<dyn FileSystem>));
    server
}

fn client(identity: &NodeIdentity) -> MeshNode {
    MeshNode::bind_local(identity, loopback())
        .unwrap()
        .with_deadline(SHORT_DEADLINE)
}

fn read_all(mut file: Box<dyn File>) -> Result<Vec<u8>, FsError> {
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 4096];
    loop {
        let read = file.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..read]);
        assert!(bytes.len() < (1 << 20), "stream grew unbounded");
    }
    Ok(bytes)
}

fn read_through(namespace: &Namespace, p: &NormalizedPath) -> Result<Vec<u8>, FsError> {
    read_all(namespace.open(p, OpenOptions::read())?)
}

fn bind_remote(client: &MeshNode, ticket: wanix_mesh::EndpointAddr) -> Namespace {
    let remote = client.dialer().dial_native(ticket).unwrap();
    let mut namespace = Namespace::new();
    namespace
        .bind(remote, ".", MOUNT_POINT, BindOptions::default())
        .unwrap();
    namespace
}

fn assert_transport_error(error: FsError) {
    match error {
        FsError::Other(message) => assert!(
            message.contains("mesh:"),
            "transport errors should stay visibly mesh-scoped: {message}"
        ),
        other => panic!("expected mesh transport error, got {other:?}"),
    }
}

#[test]
fn missing_provider_dial_fails_within_the_mesh_deadline() {
    let missing_identity = NodeIdentity::from_secret_bytes([61u8; 32]);
    let importer = client(&NodeIdentity::from_secret_bytes([62u8; 32]));
    let missing_ticket =
        wanix_mesh::EndpointAddr::new(endpoint_id_for(missing_identity.peer_id()).unwrap())
            .with_ip_addr(unused_loopback_addr());

    let started = Instant::now();
    let error = match importer.dialer().dial_native(missing_ticket) {
        Ok(_) => panic!("missing provider must not mount"),
        Err(error) => error,
    };
    let elapsed = started.elapsed();

    assert!(
        elapsed < SHORT_DEADLINE * 6,
        "missing provider dial took {elapsed:?}, expected bounded failure"
    );
    assert!(
        error.to_string().contains("native connect"),
        "error should name the bounded connect path: {error}"
    );
}

#[test]
fn fresh_ops_fail_fast_while_provider_is_down_then_recover_without_rebind() {
    let provider_identity = NodeIdentity::from_secret_bytes([63u8; 32]);
    let importer_identity = NodeIdentity::from_secret_bytes([64u8; 32]);

    let provider_v1 = serve(&provider_identity, mem_root(b"v1"));
    let ticket = provider_v1.ticket();
    let importer = client(&importer_identity);
    let namespace = bind_remote(&importer, ticket);

    assert_eq!(
        read_through(&namespace, &mounted("marker.txt")).unwrap(),
        b"v1"
    );

    drop(provider_v1);
    let started = Instant::now();
    let error = read_through(&namespace, &mounted("marker.txt"))
        .expect_err("down provider must fail the foreground op");
    let elapsed = started.elapsed();

    assert!(
        elapsed < SHORT_DEADLINE * 8,
        "down-provider op took {elapsed:?}, expected bounded failure"
    );
    assert_transport_error(error);

    let _provider_v2 = serve(&provider_identity, mem_root(b"v2"));
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut recovered = None;
    while Instant::now() < deadline {
        if let Ok(bytes) = read_through(&namespace, &mounted("marker.txt"))
            && bytes == b"v2"
        {
            recovered = Some(bytes);
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    assert_eq!(
        recovered.as_deref(),
        Some(b"v2".as_slice()),
        "held mount did not reconnect after the same provider identity returned"
    );
}

#[test]
fn stale_direct_hint_recovers_when_same_peer_returns_on_a_new_port() {
    let provider_identity = NodeIdentity::from_secret_bytes([65u8; 32]);
    let importer_identity = NodeIdentity::from_secret_bytes([66u8; 32]);

    let provider_v1 = serve(&provider_identity, mem_root(b"old-route"));
    let stale_hint_ticket = provider_v1.ticket();
    let importer = client(&importer_identity);
    let namespace = bind_remote(&importer, stale_hint_ticket);

    assert_eq!(
        read_through(&namespace, &mounted("marker.txt")).unwrap(),
        b"old-route"
    );

    drop(provider_v1);
    let _provider_v2 = serve(&provider_identity, mem_root(b"new-route"));
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut recovered = None;
    while Instant::now() < deadline {
        if let Ok(bytes) = read_through(&namespace, &mounted("marker.txt"))
            && bytes == b"new-route"
        {
            recovered = Some(bytes);
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    assert_eq!(
        recovered.as_deref(),
        Some(b"new-route".as_slice()),
        "stale addr= hint should not pin the mount to the old port"
    );
}

#[test]
fn open_file_handle_errors_after_provider_death_but_fresh_open_recovers() {
    let provider_identity = NodeIdentity::from_secret_bytes([67u8; 32]);
    let importer_identity = NodeIdentity::from_secret_bytes([68u8; 32]);

    let provider_v1 = serve(&provider_identity, mem_root(b"before"));
    let ticket = provider_v1.ticket();
    let importer = client(&importer_identity);
    let namespace = bind_remote(&importer, ticket);
    let mut old_handle = namespace
        .open(&mounted("marker.txt"), OpenOptions::read())
        .unwrap();

    drop(provider_v1);
    let started = Instant::now();
    let mut buf = [0_u8; 16];
    let error = old_handle
        .read(&mut buf)
        .expect_err("stale open handle must not return EOF or reconnect");
    let elapsed = started.elapsed();

    assert!(
        elapsed < SHORT_DEADLINE * 6,
        "stale open-file read took {elapsed:?}, expected bounded failure"
    );
    assert_transport_error(error);

    let _provider_v2 = serve(&provider_identity, mem_root(b"after"));
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut recovered = None;
    while Instant::now() < deadline {
        if let Ok(bytes) = read_through(&namespace, &mounted("marker.txt"))
            && bytes == b"after"
        {
            recovered = Some(bytes);
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    assert_eq!(
        recovered.as_deref(),
        Some(b"after".as_slice()),
        "fresh open should recover after the provider identity returns"
    );
}
