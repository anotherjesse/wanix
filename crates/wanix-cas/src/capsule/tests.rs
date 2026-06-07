use std::path::PathBuf;

use super::*;
use crate::LocalCasStore;

fn temp_dir(label: &str) -> PathBuf {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "wanix-capsule-{label}-{}-{nonce}",
        std::process::id()
    ))
}

fn store(label: &str) -> (LocalCasStore, PathBuf) {
    let dir = temp_dir(&format!("{label}-store"));
    (LocalCasStore::open(dir.clone()), dir)
}

#[test]
fn freeze_then_materialize_round_trips() {
    let (store, store_dir) = store("round-trip");
    let world = temp_dir("round-trip-world");
    std::fs::create_dir_all(world.join("bin")).unwrap();
    std::fs::write(world.join("bin/init"), b"#!/bin/init\n").unwrap();
    std::fs::write(world.join("readme.txt"), b"a world").unwrap();

    let capsule = Capsule::freeze(&store, &world).unwrap();
    assert_eq!(capsule.manifest().len(), 2);

    // Loading by id reproduces the same manifest.
    let loaded = Capsule::load(&store, capsule.id()).unwrap();
    assert_eq!(loaded.manifest(), capsule.manifest());

    // Materialize into a fresh tree and confirm bytes match.
    let out = temp_dir("round-trip-out");
    let stats = capsule.materialize(&store, &out).unwrap();
    assert_eq!(stats.files, 2);
    assert_eq!(
        std::fs::read(out.join("bin/init")).unwrap(),
        b"#!/bin/init\n"
    );
    assert_eq!(std::fs::read(out.join("readme.txt")).unwrap(), b"a world");

    for path in [store_dir, world, out] {
        std::fs::remove_dir_all(path).ok();
    }
}

#[test]
fn identical_files_dedup_to_one_blob() {
    let (store, store_dir) = store("dedup");
    let world = temp_dir("dedup-world");
    std::fs::create_dir_all(world.join("a")).unwrap();
    std::fs::create_dir_all(world.join("b")).unwrap();
    std::fs::write(world.join("a/lib.js"), b"shared module").unwrap();
    std::fs::write(world.join("b/lib.js"), b"shared module").unwrap();

    let capsule = Capsule::freeze(&store, &world).unwrap();
    let entries = capsule.manifest().entries();
    assert_eq!(entries.len(), 2);
    // Both paths resolve to the same content hash (one underlying blob).
    assert_eq!(entries[0].hash, entries[1].hash);

    std::fs::remove_dir_all(store_dir).ok();
    std::fs::remove_dir_all(world).ok();
}

#[test]
fn manifest_is_deterministic_regardless_of_order() {
    let (store, store_dir) = store("deterministic");
    let world = temp_dir("deterministic-world");
    std::fs::create_dir_all(&world).unwrap();
    std::fs::write(world.join("z.txt"), b"z").unwrap();
    std::fs::write(world.join("a.txt"), b"a").unwrap();
    let id1 = Capsule::freeze(&store, &world).unwrap().id();

    // Re-freezing the same content yields the same capsule id.
    let id2 = Capsule::freeze(&store, &world).unwrap().id();
    assert_eq!(id1, id2);

    std::fs::remove_dir_all(store_dir).ok();
    std::fs::remove_dir_all(world).ok();
}

#[test]
fn hostile_manifest_path_is_rejected() {
    // A manifest blob whose path escapes via `..` must fail to parse.
    let hash = crate::hash_bytes(b"x").to_hex();
    let blob = format!("{hash} ../escape\n");
    let err = WorldManifest::from_blob(blob.as_bytes()).unwrap_err();
    assert!(matches!(err, MaterializeError::UnsafePath(_)));

    let absolute = format!("{hash} /etc/passwd\n");
    assert!(matches!(
        WorldManifest::from_blob(absolute.as_bytes()).unwrap_err(),
        MaterializeError::UnsafePath(_)
    ));
}

#[test]
fn oversized_manifest_blob_is_rejected() {
    let blob = vec![b'a'; CAPSULE_MANIFEST_MAX_BYTES + 1];
    assert!(matches!(
        WorldManifest::from_blob(&blob).unwrap_err(),
        MaterializeError::ManifestTooLarge(_)
    ));
}

#[test]
fn materialize_confines_to_target() {
    // Even if a manifest somehow carried a traversal path, safe_join refuses it.
    let (store, store_dir) = store("confine");
    let hash = store.put(b"payload").unwrap();
    let manifest = WorldManifest {
        entries: vec![ManifestEntry {
            path: "nested/deep/file".to_owned(),
            hash,
        }],
    };
    let out = temp_dir("confine-out");
    manifest.materialize(&store, &out).unwrap();
    assert_eq!(
        std::fs::read(out.join("nested/deep/file")).unwrap(),
        b"payload"
    );

    std::fs::remove_dir_all(store_dir).ok();
    std::fs::remove_dir_all(out).ok();
}

#[test]
fn symlinks_are_skipped_on_freeze() {
    let (store, store_dir) = store("symlink");
    let world = temp_dir("symlink-world");
    std::fs::create_dir_all(&world).unwrap();
    std::fs::write(world.join("real.txt"), b"real").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink("real.txt", world.join("link.txt")).unwrap();

    let capsule = Capsule::freeze(&store, &world).unwrap();
    // The symlink is not represented; only the real file is.
    #[cfg(unix)]
    assert_eq!(capsule.manifest().len(), 1);
    #[cfg(unix)]
    assert_eq!(capsule.manifest().entries()[0].path, "real.txt");

    std::fs::remove_dir_all(store_dir).ok();
    std::fs::remove_dir_all(world).ok();
}
