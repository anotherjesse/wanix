//! End-to-end Phase 3 proof: generate a site (the SSG wasm task) → freeze it to
//! `#cas` → bind a host to the root hash → GET pages byte-identical to the live
//! output; then mutate + publish again and confirm the new hash serves new
//! content while the old hash still serves the old (atomic deploy + rollback).
//!
//! This exercises the real pipeline: the checked-in `site-gen.wasm` artifact
//! runs as a Wanix `.wasm` task into a `MemFs`, that `MemFs` is frozen through
//! the `FileSystem` trait into an in-memory content store, and the resulting
//! `CasSiteFs` is served through the same `wanix_site_fs::read_site_file`
//! handler the HTTP gateway uses — so "served by hash" and "served live" go
//! through one code path.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use wanix_cas::{CasError, CasResult, ContentStore};
use wanix_fs::{ContentHash, FileSystem, MemFs};
use wanix_site_fs::{SiteFile, read_site_file};
use wanix_sites::{Host, SiteSource, SitesDevice};
use wanix_task::TaskTable;
use wanix_vfs::{BindOptions, Namespace};
use wanix_wasm::WasmTaskDriver;

/// The vendored SSG artifact (rebuild via `wasm-src/README.md`).
const SITE_GEN_WASM: &[u8] = include_bytes!("../fixtures/site-gen.wasm");

/// Fixture corpus pages seeded into the in-memory namespace for v1.
const CORPUS_V1: &[(&str, &str)] = &[
    (
        "content/home.md",
        "---\ntitle: \"Wanix Home\"\n---\n# Wanix\n\nGo to [EIAF](/concepts/everything-is-a-file).\n",
    ),
    (
        "content/concepts/everything-is-a-file.md",
        "---\ntitle: Everything Is a File\n---\n# Everything Is a File\n\nBack [home](/).\n",
    ),
];

/// A trivial in-memory [`ContentStore`] — the whole flow runs with no host disk.
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

/// Runs the SSG wasm task over `corpus`, returning the generated output `MemFs`
/// (the `site/` subtree is the served site root).
fn generate(corpus: &[(&str, &str)]) -> Arc<MemFs> {
    let fs = Arc::new(MemFs::new());
    fs.write_file("site-gen.wasm", SITE_GEN_WASM)
        .expect("seed wasm module");
    for (path, body) in corpus {
        if let Some(i) = path.rfind('/') {
            fs.create_dir_all(&path[..i]).expect("seed corpus dir");
        }
        fs.write_file(path, body.as_bytes()).expect("seed corpus");
    }

    let mut ns = Namespace::new();
    ns.bind(fs.clone(), ".", ".", BindOptions::default())
        .expect("bind shared fs at root");

    let table = TaskTable::new();
    table
        .register_driver("wasm", Arc::new(WasmTaskDriver::new()))
        .expect("register wasm driver");
    let task = table
        .allocate_root_with_namespace("auto", ns)
        .expect("allocate task");
    task.set_cmd("site-gen.wasm content site").expect("set cmd");
    table.start(task.id()).expect("auto-start wasm SSG task");
    assert_eq!(task.exit(), "0", "SSG task should exit 0");
    fs
}

/// A read-only view of the `site/` subtree of a generated output `MemFs`, so the
/// served-root paths line up with what is frozen.
struct SiteRoot(Arc<MemFs>);

impl SiteRoot {
    fn live_bytes(&self, url: &str) -> Vec<u8> {
        // Read through the same handler the gateway uses, against the `site/`
        // subtree, so "live output" means exactly what a live serve would send.
        let prefixed = if url.is_empty() {
            "site".to_owned()
        } else {
            format!("site/{url}")
        };
        match read_site_file(self.0.as_ref(), &prefixed) {
            SiteFile::Found { bytes, .. } => bytes,
            other => panic!("live read of {url:?} failed: {other:?}"),
        }
    }
}

/// Reads `url` from a served (CAS-backed) filesystem through the gateway handler.
fn served_bytes(fs: &dyn FileSystem, url: &str) -> Vec<u8> {
    match read_site_file(fs, url) {
        SiteFile::Found { bytes, .. } => bytes,
        other => panic!("served read of {url:?} failed: {other:?}"),
    }
}

#[test]
fn generate_freeze_publish_serves_by_hash_with_rollback() {
    let store: Arc<dyn ContentStore> = Arc::new(MemStore::default());
    let sites = SitesDevice::with_store(Arc::clone(&store));
    let host = Host::registered("docs.localhost").unwrap();

    // --- v1: generate, freeze the `site/` subtree, bind the host to its hash. ---
    let v1_out = SiteRoot(generate(CORPUS_V1));
    let v1_root = sites
        .publish(host.clone(), v1_out.0.as_ref(), "site")
        .expect("publish v1");

    let v1_served = sites
        .resolve(&Host::parse("docs.localhost").unwrap())
        .expect("v1 host resolves to a served filesystem");

    // Every generated page is byte-identical served-by-hash vs. served-live.
    for url in ["", "concepts/everything-is-a-file"] {
        assert_eq!(
            served_bytes(v1_served.as_ref(), url),
            v1_out.live_bytes(url),
            "byte-identical for {url:?}"
        );
    }
    // Sanity: the home page carries the rewritten internal link.
    let home = String::from_utf8(served_bytes(v1_served.as_ref(), "")).unwrap();
    assert!(
        home.contains("href=\"/concepts/everything-is-a-file/\""),
        "served home keeps the rewritten link: {home}"
    );

    // --- v2: mutate the corpus, regenerate, publish again (a new hash). ---
    let corpus_v2: &[(&str, &str)] = &[
        (
            "content/home.md",
            "---\ntitle: \"Wanix Home\"\n---\n# Wanix v2\n\nNow with more words.\n",
        ),
        (
            "content/concepts/everything-is-a-file.md",
            "---\ntitle: Everything Is a File\n---\n# Everything Is a File\n\nBack [home](/).\n",
        ),
    ];
    let v2_out = SiteRoot(generate(corpus_v2));
    let v2_root = sites
        .publish(host.clone(), v2_out.0.as_ref(), "site")
        .expect("publish v2");

    assert_ne!(v1_root, v2_root, "v2 has a distinct root hash");

    // The host now serves v2's new content.
    let v2_served = sites
        .resolve(&Host::parse("docs.localhost").unwrap())
        .expect("v2 host resolves");
    let v2_home = String::from_utf8(served_bytes(v2_served.as_ref(), "")).unwrap();
    assert!(
        v2_home.contains("<h1>Wanix v2</h1>"),
        "v2 content: {v2_home}"
    );
    assert_eq!(served_bytes(v2_served.as_ref(), ""), v2_out.live_bytes(""));

    // The old hash still serves the old bytes (immutability) — rolling the host
    // back to v1 restores the prior site with no file copying.
    sites.bind_site(host, SiteSource::Cas(v1_root.to_hex()));
    let rolled = sites
        .resolve(&Host::parse("docs.localhost").unwrap())
        .expect("rolled-back host resolves");
    assert_eq!(
        served_bytes(rolled.as_ref(), ""),
        v1_out.live_bytes(""),
        "rollback to v1 serves the original bytes"
    );
    let rolled_home = String::from_utf8(served_bytes(rolled.as_ref(), "")).unwrap();
    assert!(
        rolled_home.contains("<h1>Wanix</h1>") && !rolled_home.contains("Wanix v2"),
        "rollback restores v1: {rolled_home}"
    );
}
