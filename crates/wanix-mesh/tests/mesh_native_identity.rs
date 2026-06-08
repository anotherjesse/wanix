//! Per-principal identity proof for the native wire (Plan §6, §10 Phase 5).
//!
//! The native wire binds the principal **once per connection** from the
//! cryptographically verified `remote_id()` (an ed25519 pubkey), never a
//! client-claimed name, and resolves the per-connection root through the existing
//! [`wanix_id::AttachPolicy`] — the non-no-op `NamespaceProvider` seam ADR 0006/
//! 0007 need. This test pins three properties:
//!
//! 1. **Granted peer sees the scoped root.** Peer A holds a grant scoping the
//!    backing to `projects/foo`; dialing one [`MeshNode::serve_native`] under that
//!    [`wanix_id::GrantTablePolicy`], peer A imports exactly the re-rooted
//!    [`wanix_vfs::SubtreeFs`]: it reads the in-scope file at the subtree root and
//!    cannot see anything above the prefix (`projects/bar` is invisible).
//!
//! 2. **Ungranted peer is default-denied.** Peer B holds no grant; the policy
//!    returns `None`, so the native handler serves it no streams at all (the
//!    native EACCES) and the first op the import attempts — including
//!    [`wanix_vfs::Namespace::bind`]'s own eager `metadata` probe — fails as a
//!    transport fault.
//!
//! 3. **Both planes grant identical roots for the same peer.** The *same*
//!    `GrantTablePolicy` drives a 9P [`MeshNode::serve`] beside the native one;
//!    peer A imports both planes and observes byte-identical content through each,
//!    so neither plane grants wider (or narrower) access than the other for one
//!    verified peer — risk register #5.
//!
//! The native wire resolves its connection root at the empty attach name
//! (`ROOT_ANAME`; v1 carries no `aname` on the wire), so the grant is keyed at
//! `aname = ""` and the 9P plane attaches the same empty name, which is what makes
//! the two planes resolve the *same* grant for the same peer.

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use wanix_fs::{File, FileSystem, FileType, MemFs, NormalizedPath, OpenOptions};
use wanix_id::{Grant, GrantTable, GrantTablePolicy, NodeIdentity};
use wanix_mesh::{MeshNode, NativeServeConfig, ServeConfig};
use wanix_vfs::{BindOptions, Namespace, Rights};

/// Peer A's fixed secret: the granted principal.
const PEER_A_SECRET: [u8; 32] = [0xA1; 32];
/// Peer B's fixed secret: the ungranted (default-denied) principal.
const PEER_B_SECRET: [u8; 32] = [0xB2; 32];
/// The native and 9P server identities (distinct endpoints, one shared policy).
const SERVER_NATIVE_SECRET: [u8; 32] = [0x11; 32];
const SERVER_9P_SECRET: [u8; 32] = [0x99; 32];

/// A short deadline so a default-denied native op (served no stream at all)
/// surfaces as a bounded transport fault instead of waiting out the 30s default.
const DENY_DEADLINE: Duration = Duration::from_secs(3);

/// Relative path the imported namespace is bound at on the client.
const MOUNT_POINT: &str = "n/A";

fn loopback() -> SocketAddr {
    SocketAddr::new(Ipv4Addr::LOCALHOST.into(), 0)
}

fn path(value: &str) -> NormalizedPath {
    NormalizedPath::new(value).unwrap()
}

fn mounted(relative: &str) -> NormalizedPath {
    path(&format!("{MOUNT_POINT}/{relative}"))
}

/// Builds the shared backing: an in-scope file under `projects/foo` and an
/// out-of-scope file under `projects/bar`, so a `projects/foo` grant proves both
/// what a peer *can* and *cannot* see through the re-rooted subtree.
fn shared_backing() -> Arc<dyn FileSystem> {
    let backing = Arc::new(MemFs::new());
    backing.create_dir_all("projects/foo").unwrap();
    backing
        .write_file("projects/foo/scoped.txt", b"in scope")
        .unwrap();
    backing.create_dir_all("projects/bar").unwrap();
    backing
        .write_file("projects/bar/other.txt", b"out of scope")
        .unwrap();
    backing
}

/// One policy granting peer A read-only access to `projects/foo` at the empty
/// attach name, scoping `backing` to that prefix. Peer B is never added, so the
/// default-deny table denies it. The *same* policy instance drives both planes.
fn shared_policy(backing: &Arc<dyn FileSystem>) -> Arc<GrantTablePolicy> {
    let peer_a = NodeIdentity::from_secret_bytes(PEER_A_SECRET).peer_id();
    let grants = GrantTable::new();
    grants.grant(Grant::new(
        peer_a,
        // The native wire resolves at the empty attach name; the 9P plane attaches
        // the same empty name, so both planes key on this one grant.
        "",
        Arc::clone(backing),
        "projects/foo",
        Rights::read_only(),
    ));
    Arc::new(GrantTablePolicy::new(grants))
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

/// Binds `remote` (any imported `FileSystem`) at `/n/A` on a fresh namespace.
fn mount(remote: Arc<dyn FileSystem>) -> Namespace {
    let mut namespace = Namespace::new();
    namespace
        .bind(remote, ".", MOUNT_POINT, BindOptions::default())
        .unwrap();
    namespace
}

#[test]
fn granted_peer_sees_scoped_subtree_over_the_native_wire() {
    // Property 1: peer A imports the re-rooted SubtreeFs and sees exactly the
    // `projects/foo` scope — the in-scope file at the subtree root, with the
    // out-of-scope sibling and the prefix itself invisible.
    let backing = shared_backing();
    let policy = shared_policy(&backing);

    let identity_server = NodeIdentity::from_secret_bytes(SERVER_NATIVE_SECRET);
    let mut server = MeshNode::bind_local(&identity_server, loopback()).unwrap();
    server.serve_native(NativeServeConfig::guarded(Arc::clone(&backing), policy));
    let ticket = server.ticket();

    let identity_a = NodeIdentity::from_secret_bytes(PEER_A_SECRET);
    let peer_a = MeshNode::bind_local(&identity_a, loopback()).unwrap();
    let remote = peer_a.dialer().dial_native(ticket).unwrap();
    let ns = mount(remote);

    // The scoped file is at the subtree root (re-rooted past `projects/foo`).
    assert_eq!(read_through(&ns, &mounted("scoped.txt")), b"in scope");
    let meta = ns.metadata(&mounted("scoped.txt")).unwrap();
    assert_eq!(meta.file_type(), FileType::File);

    // Confinement: nothing above the prefix is reachable. `projects` and the
    // out-of-scope sibling do not exist in the re-rooted view.
    assert!(
        ns.metadata(&mounted("projects")).is_err(),
        "the prefix must not be visible inside the re-rooted subtree"
    );
    assert!(
        ns.metadata(&mounted("other.txt")).is_err(),
        "the out-of-scope sibling must not be reachable"
    );

    // The grant is read-only: a write-mode open is denied by the rights gate.
    let write = ns.open(
        &mounted("scoped.txt"),
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

    drop(ns);
    drop(peer_a);
    drop(server);
}

#[test]
fn ungranted_peer_is_default_denied_over_the_native_wire() {
    // Property 2: peer B holds no grant. The policy returns None, so the native
    // handler serves no streams (the native EACCES) and the first op over the
    // import fails as a transport fault, bounded by the short deadline.
    let backing = shared_backing();
    let policy = shared_policy(&backing);

    let identity_server = NodeIdentity::from_secret_bytes(SERVER_NATIVE_SECRET);
    let mut server = MeshNode::bind_local(&identity_server, loopback())
        .unwrap()
        .with_deadline(DENY_DEADLINE);
    server.serve_native(NativeServeConfig::guarded(Arc::clone(&backing), policy));
    let ticket = server.ticket();

    // Peer B's QUIC handshake succeeds (identity is verified post-handshake), so
    // the dial returns an import handle; the default-deny surfaces on the first op
    // the import attempts, because the server installed no scoped root for this
    // principal and so its accept handler serves it no stream at all.
    let identity_b = NodeIdentity::from_secret_bytes(PEER_B_SECRET);
    let peer_b = MeshNode::bind_local(&identity_b, loopback())
        .unwrap()
        .with_deadline(DENY_DEADLINE);
    let remote: Arc<dyn FileSystem> = peer_b.dialer().dial_native(ticket).unwrap();

    // The first op an ungranted peer attempts fails. `Namespace::bind` itself
    // probes the source with `metadata` (binding.rs:69), so binding the denied
    // import is the first op and already fails — default-deny refuses the peer
    // before its namespace can even mount. A direct op on the import fails the same
    // way; both are asserted so the surface is unambiguous.
    let direct = remote.metadata(&path("scoped.txt"));
    assert!(
        direct.is_err(),
        "default-deny must refuse an ungranted peer's native op, got {direct:?}"
    );
    // It is a transport fault (no stream served), not a typed application error: a
    // denied peer never reaches the server's FileSystem to receive a typed reply.
    match direct {
        Err(wanix_fs::FsError::Other(_)) => {}
        other => panic!("expected a transport fault for a denied native peer, got {other:?}"),
    }

    let mut namespace = Namespace::new();
    let bound = namespace.bind(remote, ".", MOUNT_POINT, BindOptions::default());
    assert!(
        bound.is_err(),
        "binding a default-denied import must fail on its eager metadata probe, got {bound:?}"
    );

    drop(peer_b);
    drop(server);
}

#[test]
fn native_and_9p_grant_identical_roots_for_the_same_peer() {
    // Property 3: one policy, two planes, one peer. Peer A imports the native and
    // the 9P plane of the same backing under the same grant and observes
    // byte-identical scoped content through each — neither plane grants wider or
    // narrower access than the other for the verified peer.
    let backing = shared_backing();
    let policy = shared_policy(&backing);

    // Native plane.
    let identity_native = NodeIdentity::from_secret_bytes(SERVER_NATIVE_SECRET);
    let mut server_native = MeshNode::bind_local(&identity_native, loopback()).unwrap();
    server_native.serve_native(NativeServeConfig::guarded(
        Arc::clone(&backing),
        Arc::clone(&policy) as Arc<_>,
    ));
    let ticket_native = server_native.ticket();

    // 9P plane, same policy.
    let identity_9p = NodeIdentity::from_secret_bytes(SERVER_9P_SECRET);
    let mut server_9p = MeshNode::bind_local(&identity_9p, loopback()).unwrap();
    server_9p.serve(ServeConfig::guarded(
        Arc::clone(&backing),
        Arc::clone(&policy) as Arc<_>,
    ));
    let ticket_9p = server_9p.ticket();

    // Peer A dials both planes from one node.
    let identity_a = NodeIdentity::from_secret_bytes(PEER_A_SECRET);
    let peer_a = MeshNode::bind_local(&identity_a, loopback()).unwrap();
    let native = peer_a.dialer().dial_native(ticket_native).unwrap();
    // The 9P plane attaches the same empty aname the native plane resolves at, so
    // both planes evaluate the same `(peer_a, "")` grant.
    let nine_p = peer_a.dialer().dial_attach(ticket_9p, "").unwrap();

    let native_ns = mount(native);
    let nine_p_ns = mount(nine_p);

    // Both planes surface the scoped file with identical bytes.
    let native_bytes = read_through(&native_ns, &mounted("scoped.txt"));
    let nine_p_bytes = read_through(&nine_p_ns, &mounted("scoped.txt"));
    assert_eq!(
        native_bytes, b"in scope",
        "native plane must surface the scoped file"
    );
    assert_eq!(
        native_bytes, nine_p_bytes,
        "the native and 9P planes must grant identical content for the same peer"
    );

    // Both planes confine identically: the out-of-scope sibling is invisible on
    // each, so neither plane grants wider access than the other.
    assert!(
        native_ns.metadata(&mounted("other.txt")).is_err(),
        "native plane must not expose the out-of-scope file"
    );
    assert!(
        nine_p_ns.metadata(&mounted("other.txt")).is_err(),
        "9P plane must not expose the out-of-scope file"
    );
    assert!(
        native_ns.metadata(&mounted("projects")).is_err(),
        "native plane must not expose the prefix above the scope"
    );
    assert!(
        nine_p_ns.metadata(&mounted("projects")).is_err(),
        "9P plane must not expose the prefix above the scope"
    );

    drop(native_ns);
    drop(nine_p_ns);
    drop(peer_a);
    drop(server_native);
    drop(server_9p);
}
