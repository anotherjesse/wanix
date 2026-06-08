//! End-to-end data-plane truth check: ship a world by content address.
//!
//! Two [`MeshNode`]s bind on loopback (relay/DNS disabled, direct-address only).
//! Node A serves the blob plane on [`iroh_blobs::ALPN`] from the same endpoint as
//! 9P; it freezes a directory tree into a [`Capsule`] (each file a blob, the
//! sorted manifest a blob whose hash is the capsule id). Node B, configured with
//! A as a provider peer, loads the capsule by id over QUIC — the manifest blob
//! and every file blob are fetched and BLAKE3-verified end to end — and
//! materializes the world locally. Shared files deduplicate to one blob.
//!
//! This exercises the *real* iroh-blobs 0.102 API (`add_bytes`, `get_bytes`,
//! `has`, and `endpoint.connect(peer, ALPN)` + `store.remote().fetch(conn,
//! HashAndFormat::raw(hash))` for the cross-node pull) reached through the
//! synchronous [`wanix_cas::ContentStore`] trait, with no async leaking into the
//! capsule logic.

use std::net::{Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use wanix_cas::{Capsule, ContentStore};
use wanix_fs::{FileSystem, MemFs};
use wanix_id::NodeIdentity;
use wanix_mesh::{MeshNode, NativeServeConfig};

fn loopback() -> SocketAddr {
    SocketAddr::new(Ipv4Addr::LOCALHOST.into(), 0)
}

fn temp_dir(label: &str) -> PathBuf {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "wanix-mesh-blobs-{label}-{}-{nonce}",
        std::process::id()
    ))
}

/// Builds a small world tree with two directories sharing one identical file, so
/// dedup is observable: `a/lib.js` and `b/lib.js` are byte-identical.
fn build_world(root: &Path) {
    std::fs::create_dir_all(root.join("bin")).unwrap();
    std::fs::create_dir_all(root.join("a")).unwrap();
    std::fs::create_dir_all(root.join("b")).unwrap();
    std::fs::write(root.join("bin/init"), b"#!/bin/init\nexec qjs main.js\n").unwrap();
    std::fs::write(root.join("a/lib.js"), b"export const shared = 1;\n").unwrap();
    std::fs::write(root.join("b/lib.js"), b"export const shared = 1;\n").unwrap();
}

#[test]
fn ship_a_world_by_capsule_id_over_the_blob_plane() {
    let identity_a = NodeIdentity::from_secret_bytes([101u8; 32]);
    let identity_b = NodeIdentity::from_secret_bytes([202u8; 32]);

    // Node A serves both planes; its CAS store fetches from no one (it is the
    // origin), so its peer list is empty. The bundled control plane is the
    // native wire (the mesh path), not 9P; the blobs data plane it rides beside
    // is a distinct ALPN, unchanged by the wire choice.
    let mut node_a = MeshNode::bind_local(&identity_a, loopback()).unwrap();
    let cas_a = node_a.cas_store(vec![]);
    let server_root: Arc<dyn FileSystem> = Arc::new(MemFs::new());
    node_a.serve_native_with_blobs(NativeServeConfig::open(server_root), &cas_a);
    // Node B fetches with A's full ticket (direct loopback addresses), since
    // relay/DNS discovery is disabled in the local test binding.
    let ticket_a = node_a.ticket();

    // Node A freezes a world into its blob store and gets a capsule id.
    let world = temp_dir("origin-world");
    build_world(&world);
    let capsule = Capsule::freeze(&cas_a, &world).unwrap();
    let capsule_id = capsule.id();
    // Dedup: three files but the two identical libs share one blob, so the
    // manifest has 3 entries with a repeated hash.
    assert_eq!(capsule.manifest().len(), 3);

    // Node B knows A as a provider peer and fetches the capsule by id alone.
    let node_b = MeshNode::bind_local(&identity_b, loopback()).unwrap();
    let cas_b = node_b.cas_store(vec![ticket_a]);

    // Loading the capsule fetches the manifest blob over the blob plane, then
    // every referenced file blob is pulled and verified on materialize.
    let loaded = Capsule::load(&cas_b, capsule_id).unwrap();
    assert_eq!(loaded.manifest(), capsule.manifest());

    let out = temp_dir("materialized-world");
    let stats = loaded.materialize(&cas_b, &out).unwrap();
    assert_eq!(stats.files, 3);

    // The world arrived byte-for-byte over QUIC's data plane.
    assert_eq!(
        std::fs::read(out.join("bin/init")).unwrap(),
        b"#!/bin/init\nexec qjs main.js\n"
    );
    assert_eq!(
        std::fs::read(out.join("a/lib.js")).unwrap(),
        b"export const shared = 1;\n"
    );
    assert_eq!(
        std::fs::read(out.join("b/lib.js")).unwrap(),
        std::fs::read(out.join("a/lib.js")).unwrap()
    );

    std::fs::remove_dir_all(world).ok();
    std::fs::remove_dir_all(out).ok();
}

#[test]
fn put_and_get_round_trip_through_one_node_store() {
    // A single node's blob store is a working local CAS: put returns the content
    // hash, has reports presence, and get returns the verified bytes.
    let identity = NodeIdentity::from_secret_bytes([55u8; 32]);
    let node = MeshNode::bind_local(&identity, loopback()).unwrap();
    let cas = node.cas_store(vec![]);

    let payload = b"venti blob over iroh";
    let hash = cas.put(payload).unwrap();
    assert!(cas.has(&hash).unwrap());
    assert_eq!(cas.get(&hash).unwrap(), payload);

    // An absent blob with no peers to fetch from is NotFound, not a hang.
    let absent = wanix_cas::hash_bytes(b"never stored");
    assert!(!cas.has(&absent).unwrap());
    assert!(cas.get(&absent).is_err());
}
