//! Differential proof: the native wire and the 9P wire stay behaviorally
//! equivalent for one shared backing `FileSystem` (Plan §10 Phase 5, risk
//! register #5).
//!
//! One backing services namespace (a `MemFs` host root plus `#kv`) is served by
//! **two** loopback [`MeshNode`]s from the *same* [`Arc<dyn FileSystem>`]: node
//! `A_native` over [`wanix_mesh::WANIX_FS_ALPN`] ([`MeshNode::serve_native`]) and
//! node `A_9p` over [`wanix_mesh::WANIX_9P_ALPN`] ([`MeshNode::serve`]). A single
//! dialer node `B` imports both — a [`wanix_mesh::NativeFs`] and a
//! [`wanix_9p_client::RemoteFs`] — and the reusable
//! [`wanix_mesh_wire::conformance`] suite runs across both imports of the same
//! backing.
//!
//! Two distinct claims are proved, kept deliberately separate because the native
//! wire is a *superset* of the 9P-bridged surface, not a re-encoding of it:
//!
//! 1. **Faithful native encoding** — the *full* conformance suite (open/read/
//!    write/seek, follow-vs-nofollow metadata, full-metadata readdir, create/
//!    remove/rename, symlink/read_link, set_times, content_hash, typed errors)
//!    passes against the native import. These are the methods the design's §4
//!    op-by-op mapping carries faithfully, including the ones the 9P client never
//!    bridged (`symlink`, `set_times`, `metadata_with_lookup` nofollow, full
//!    per-entry readdir metadata).
//!
//! 2. **Two encodings stay behaviorally equivalent on the shared surface** — the
//!    subset of the contract that *both* planes bridge (open/read/write/seek,
//!    create/remove/rename, content_hash-`None`, typed `NotFound`/`NotEmpty`)
//!    runs against the native AND the 9P import of the *same* backing and both
//!    pass identically. The native-only additions (symlink creation, `set_times`,
//!    real readdir entry sizes) are *not* part of this differential because the
//!    9P plane genuinely does not carry them (`FsError::NotSupported` /
//!    placeholder readdir sizes, per the design's §4 note); a differential there
//!    would assert a divergence the design intends, not a regression.
//!
//! Both server nodes share one `Arc<dyn FileSystem>` rather than one node serving
//! both ALPNs, because [`MeshNode::serve`]/[`MeshNode::serve_native`] each install
//! their own [`Router`] and the second would replace the first — and this keeps
//! the proof to test code, touching no production serve path (Phase 5 is
//! proofs-only).

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;

use wanix_9p_client::RemoteFs;
use wanix_fs::{FileSystem, MemFs};
use wanix_id::NodeIdentity;
use wanix_kv::KvDevice;
use wanix_mesh::{MeshNode, NativeFs, NativeServeConfig, ServeConfig};
use wanix_mesh_wire::conformance;
use wanix_vfs::{BindOptions, Namespace};

fn loopback() -> SocketAddr {
    // Port 0: the OS assigns a free loopback port for the test endpoint.
    SocketAddr::new(Ipv4Addr::LOCALHOST.into(), 0)
}

/// Builds a services namespace shared by both planes: a `MemFs` host root plus a
/// `#kv` device, both plain `FileSystem`s so they import over either wire for
/// free. Returned as an `Arc<dyn FileSystem>` so the *same* backing instance is
/// handed to both server nodes.
fn shared_backing() -> Arc<dyn FileSystem> {
    let host = Arc::new(MemFs::new());
    let kv = KvDevice::new();
    let mut namespace = Namespace::new();
    namespace
        .bind(host, ".", ".", BindOptions::default())
        .unwrap();
    namespace
        .bind(Arc::new(kv), ".", "#kv", BindOptions::default())
        .unwrap();
    Arc::new(namespace)
}

/// Holds the live mesh nodes so the QUIC connections, routers, and the dialer's
/// non-owning runtime handle stay alive for the test's whole body. Dropping any
/// of these would tear down a connection the imports still ride.
struct MeshPair {
    _server_native: MeshNode,
    _server_9p: MeshNode,
    _client: MeshNode,
    native: Arc<NativeFs<wanix_mesh::IrohStreamFactory>>,
    nine_p: Arc<RemoteFs>,
}

/// Serves `backing` over both wires from two nodes and imports each from one
/// dialer node, returning both imports plus the live nodes that back them.
fn serve_both(backing: Arc<dyn FileSystem>) -> MeshPair {
    let identity_native = NodeIdentity::from_secret_bytes([11u8; 32]);
    let identity_9p = NodeIdentity::from_secret_bytes([22u8; 32]);
    let identity_client = NodeIdentity::from_secret_bytes([33u8; 32]);

    // Node A_native serves the shared backing over the native wire.
    let mut server_native = MeshNode::bind_local(&identity_native, loopback()).unwrap();
    server_native.serve_native(NativeServeConfig::open(Arc::clone(&backing)));
    let ticket_native = server_native.ticket();

    // Node A_9p serves the SAME backing over the 9P wire.
    let mut server_9p = MeshNode::bind_local(&identity_9p, loopback()).unwrap();
    server_9p.serve(ServeConfig::open(Arc::clone(&backing)));
    let ticket_9p = server_9p.ticket();

    // One dialer imports both planes.
    let client = MeshNode::bind_local(&identity_client, loopback()).unwrap();
    let native = client.dialer().dial_native(ticket_native).unwrap();
    let nine_p = client.dialer().dial_attach(ticket_9p, "").unwrap();

    MeshPair {
        _server_native: server_native,
        _server_9p: server_9p,
        _client: client,
        native,
        nine_p,
    }
}

#[test]
fn native_import_is_a_faithful_filesystem_encoding() {
    // Claim 1: the full FileSystem contract crosses the native wire intact —
    // including the symlink/set_times/nofollow/full-readdir-metadata surface the
    // 9P client never bridged. This is the native wire's headline correctness
    // property, run against a real import over QUIC of the shared backing.
    let pair = serve_both(shared_backing());
    conformance::run(pair.native.as_ref(), "native");
}

#[test]
fn native_and_9p_agree_on_the_shared_filesystem_surface() {
    // Claim 2: the two encodings of the SAME backing are behaviorally identical
    // on the surface both planes bridge. Each check runs against both imports;
    // if either plane diverged, the corresponding check would panic for that
    // label, naming the offending encoding.
    let pair = serve_both(shared_backing());
    let native: &dyn FileSystem = pair.native.as_ref();
    let nine_p: &dyn FileSystem = pair.nine_p.as_ref();

    differential_shared_surface(native, "native");
    differential_shared_surface(nine_p, "9p");
}

/// Runs the conformance checks that *both* planes bridge identically against one
/// `fs`, under a per-label scratch root so the native and 9P passes over one
/// shared backing do not collide.
///
/// Deliberately excludes the native-only additions: `check_symlink_read_link`
/// and `check_set_times` (9P returns `FsError::NotSupported`), the symlink-driven
/// half of `check_metadata_follow_nofollow`, and `check_read_dir_carries_metadata`
/// (9P readdir carries placeholder entry sizes, not the real length — the design's
/// §4 "no 9P placeholder-size problem" is a native-only win, not a shared
/// invariant).
fn differential_shared_surface(fs: &dyn FileSystem, label: &str) {
    use wanix_fs::NormalizedPath;

    let root = format!("differential-{label}");
    let path = |p: &str| NormalizedPath::new(p).expect("conformance path");

    // Clean any prior run, then create the per-label scratch root.
    let _ = fs.remove_dir(&path(&root));
    fs.create_dir(&path(&root))
        .unwrap_or_else(|err| panic!("[{label}] create scratch root {root:?}: {err:?}"));

    conformance::check_open_read_write_seek(fs, &root, label);
    conformance::check_create_remove_rename(fs, &root, label);
    conformance::check_content_hash_none(fs, &root, label);
    conformance::check_typed_errors(fs, &root, label);

    fs.remove_dir(&path(&root))
        .unwrap_or_else(|err| panic!("[{label}] remove scratch root {root:?}: {err:?}"));
}
