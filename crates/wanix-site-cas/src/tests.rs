use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use wanix_cas::{CasError, CasResult, ContentStore};
use wanix_fs::{ContentHash, FileSystem, MemFs, NormalizedPath, OpenOptions};
use wanix_site_fs::{SiteFile, read_site_file};

use crate::{CasSiteFs, freeze_fs};

/// A trivial in-memory [`ContentStore`] for tests — no host disk involved, so a
/// site can be frozen and served entirely in memory.
#[derive(Default)]
struct MemStore {
    blobs: Mutex<HashMap<ContentHash, Vec<u8>>>,
}

impl ContentStore for MemStore {
    fn put(&self, bytes: &[u8]) -> CasResult<ContentHash> {
        let hash = wanix_cas::hash_bytes(bytes);
        self.blobs.lock().unwrap().insert(hash, bytes.to_vec());
        Ok(hash)
    }

    fn get(&self, hash: &ContentHash) -> CasResult<Vec<u8>> {
        self.blobs
            .lock()
            .unwrap()
            .get(hash)
            .cloned()
            .ok_or(CasError::NotFound)
    }

    fn has(&self, hash: &ContentHash) -> CasResult<bool> {
        Ok(self.blobs.lock().unwrap().contains_key(hash))
    }
}

fn path(raw: &str) -> NormalizedPath {
    NormalizedPath::new(raw).unwrap()
}

/// Builds a small in-memory site filesystem with a nested directory tree.
fn sample_site() -> MemFs {
    let fs = MemFs::new();
    fs.write_file("index.html", b"<h1>home</h1>").unwrap();
    fs.write_file("style.css", b"body{}").unwrap();
    fs.create_dir_all("concepts/foo").unwrap();
    fs.write_file("concepts/foo/index.html", b"<h1>foo</h1>")
        .unwrap();
    fs.create_dir_all("concepts/bar").unwrap();
    fs.write_file("concepts/bar/index.html", b"<h1>bar</h1>")
        .unwrap();
    fs
}

#[test]
fn freeze_then_cas_fs_roundtrip() {
    let store: Arc<dyn ContentStore> = Arc::new(MemStore::default());
    let site = sample_site();

    let root = freeze_fs(store.as_ref(), &site, ".").unwrap();
    let cas = CasSiteFs::open_root(Arc::clone(&store), root).unwrap();

    // Every leaf reads back byte-identical to the live site.
    for p in ["index.html", "style.css", "concepts/foo/index.html"] {
        let mut original = site.open(&path(p), OpenOptions::read()).unwrap();
        let mut want = Vec::new();
        let mut chunk = [0u8; 32];
        loop {
            let n = original.read(&mut chunk).unwrap();
            if n == 0 {
                break;
            }
            want.extend_from_slice(&chunk[..n]);
        }
        match read_site_file(&cas, p) {
            SiteFile::Found { bytes, .. } => assert_eq!(bytes, want, "path {p}"),
            other => panic!("expected file for {p}, got {other:?}"),
        }
    }

    // Directory-index resolution composes with the Phase 0 handler.
    match read_site_file(&cas, "concepts/foo") {
        SiteFile::Found { bytes, extension } => {
            assert_eq!(bytes, b"<h1>foo</h1>");
            assert_eq!(extension.as_deref(), Some("html"));
        }
        other => panic!("expected directory-index file, got {other:?}"),
    }

    // read_dir synthesizes the directory tree from manifest prefixes.
    let mut root_entries: Vec<String> = cas
        .read_dir(&path("."))
        .unwrap()
        .into_iter()
        .map(|e| e.name().to_owned())
        .collect();
    root_entries.sort();
    assert_eq!(root_entries, vec!["concepts", "index.html", "style.css"]);

    let mut concepts: Vec<String> = cas
        .read_dir(&path("concepts"))
        .unwrap()
        .into_iter()
        .map(|e| e.name().to_owned())
        .collect();
    concepts.sort();
    assert_eq!(concepts, vec!["bar", "foo"]);
}

#[test]
fn writes_are_rejected() {
    let store: Arc<dyn ContentStore> = Arc::new(MemStore::default());
    let root = freeze_fs(store.as_ref(), &sample_site(), ".").unwrap();
    let cas = CasSiteFs::open_root(store, root).unwrap();

    assert!(matches!(
        cas.open(&path("index.html"), OpenOptions::read_write()),
        Err(wanix_fs::FsError::PermissionDenied)
    ));
    assert!(matches!(
        cas.create_dir(&path("new")),
        Err(wanix_fs::FsError::NotSupported)
    ));
    assert!(matches!(
        cas.remove_file(&path("index.html")),
        Err(wanix_fs::FsError::NotSupported)
    ));
}

#[test]
fn freeze_is_deterministic() {
    let store_a: Arc<dyn ContentStore> = Arc::new(MemStore::default());
    let store_b: Arc<dyn ContentStore> = Arc::new(MemStore::default());

    let root_a = freeze_fs(store_a.as_ref(), &sample_site(), ".").unwrap();
    let root_b = freeze_fs(store_b.as_ref(), &sample_site(), ".").unwrap();
    assert_eq!(root_a, root_b);
    assert_eq!(root_a.to_hex(), root_b.to_hex());

    // A different tree freezes to a different root hash.
    let other = MemFs::new();
    other.write_file("index.html", b"<h1>changed</h1>").unwrap();
    let root_c = freeze_fs(store_a.as_ref(), &other, ".").unwrap();
    assert_ne!(root_a, root_c);
}

#[test]
fn content_hash_offload_returns_entry_hash() {
    let store: Arc<dyn ContentStore> = Arc::new(MemStore::default());
    let site = sample_site();
    let root = freeze_fs(store.as_ref(), &site, ".").unwrap();
    let cas = CasSiteFs::open_root(Arc::clone(&store), root).unwrap();

    let want = wanix_cas::hash_bytes(b"<h1>home</h1>");
    assert_eq!(cas.content_hash(&path("index.html")).unwrap(), Some(want));
    // Directories have no offloadable hash.
    assert_eq!(cas.content_hash(&path("concepts")).unwrap(), None);
}

#[test]
fn root_hash_hex_round_trips() {
    let store: Arc<dyn ContentStore> = Arc::new(MemStore::default());
    let root = freeze_fs(store.as_ref(), &sample_site(), ".").unwrap();
    let hex = root.to_hex();
    assert_eq!(crate::CasRootHash::from_hex(&hex), Some(root));
    assert_eq!(crate::CasRootHash::from_hex("notahash"), None);
}

#[test]
fn missing_root_hash_is_a_load_error() {
    let store: Arc<dyn ContentStore> = Arc::new(MemStore::default());
    let bogus = crate::CasRootHash::from_hash(wanix_cas::hash_bytes(b"absent"));
    assert!(CasSiteFs::open_root(store, bogus).is_err());
}
