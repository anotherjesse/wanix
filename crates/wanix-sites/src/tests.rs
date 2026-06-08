use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use wanix_cas::{CasError, CasResult, ContentStore};
use wanix_fs::{ContentHash, FileSystem, MemFs, NormalizedPath, OpenOptions};

use crate::{Host, SiteSource, SitesDevice};

/// A trivial in-memory [`ContentStore`] for the publish/rollback tests, so a
/// site can be frozen and served by hash without any host disk.
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

fn read_control(device: &SitesDevice, host: &str) -> String {
    let mut file = device.open(&path(host), OpenOptions::read()).unwrap();
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 64];
    loop {
        let read = file.read(&mut chunk).unwrap();
        if read == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..read]);
    }
    String::from_utf8(bytes).unwrap()
}

fn read_file(fs: &dyn FileSystem, p: &str) -> Vec<u8> {
    let mut file = fs.open(&path(p), OpenOptions::read()).unwrap();
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 64];
    loop {
        let read = file.read(&mut chunk).unwrap();
        if read == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..read]);
    }
    bytes
}

#[test]
fn host_parse_strips_port_and_lowercases() {
    assert_eq!(
        Host::parse("Blog.LocalHost:7654").unwrap().as_str(),
        "blog.localhost"
    );
    assert_eq!(
        Host::parse("docs.localhost").unwrap().as_str(),
        "docs.localhost"
    );
    // Bare localhost, an IP literal, and empty fall through (no site).
    assert!(Host::parse("localhost").is_none());
    assert!(Host::parse("localhost:7654").is_none());
    assert!(Host::parse("127.0.0.1:7654").is_none());
    assert!(Host::parse("[::1]:7654").is_none());
    assert!(Host::parse("").is_none());
}

#[test]
fn resolve_routes_distinct_hosts_to_distinct_filesystems() {
    let blog = MemFs::new();
    blog.write_file("index.html", b"<h1>blog</h1>").unwrap();
    let docs = MemFs::new();
    docs.write_file("index.html", b"<h1>docs</h1>").unwrap();

    let device = SitesDevice::new();
    device.bind_site(
        Host::registered("blog.localhost").unwrap(),
        SiteSource::Memory(Arc::new(blog)),
    );
    device.bind_site(
        Host::registered("docs.localhost").unwrap(),
        SiteSource::Memory(Arc::new(docs)),
    );

    let blog_fs = device
        .resolve(&Host::parse("blog.localhost:7654").unwrap())
        .unwrap();
    let docs_fs = device
        .resolve(&Host::parse("docs.localhost").unwrap())
        .unwrap();
    assert_eq!(read_file(blog_fs.as_ref(), "index.html"), b"<h1>blog</h1>");
    assert_eq!(read_file(docs_fs.as_ref(), "index.html"), b"<h1>docs</h1>");
    // An unbound host resolves to nothing (gateway falls back to --root).
    assert!(
        device
            .resolve(&Host::parse("absent.localhost").unwrap())
            .is_none()
    );
}

#[test]
fn listing_enumerates_bound_hosts_and_control_file_shows_binding() {
    let device = SitesDevice::new();
    device.bind_site(
        Host::registered("blog.localhost").unwrap(),
        SiteSource::Memory(Arc::new(MemFs::new())),
    );
    device.bind_site(
        Host::registered("docs.localhost").unwrap(),
        SiteSource::Cas("deadbeef".to_owned()),
    );

    let mut hosts: Vec<String> = device
        .read_dir(&path("."))
        .unwrap()
        .into_iter()
        .map(|entry| entry.name().to_owned())
        .collect();
    hosts.sort();
    assert_eq!(hosts, vec!["blog.localhost", "docs.localhost"]);

    assert_eq!(read_control(&device, "blog.localhost"), "memory\n");
    assert_eq!(read_control(&device, "docs.localhost"), "cas deadbeef\n");
}

#[test]
fn writing_descriptor_creates_and_repoints_binding() {
    let device = SitesDevice::new();
    // Create a binding by writing a `cas <hash>` descriptor.
    {
        let mut file = device
            .open(&path("new.localhost"), OpenOptions::read_write())
            .unwrap();
        file.write(b"cas abc123").unwrap();
    }
    assert_eq!(read_control(&device, "new.localhost"), "cas abc123\n");

    // Repoint the same host to a different hash.
    {
        let mut file = device
            .open(&path("new.localhost"), OpenOptions::read_write())
            .unwrap();
        file.write(b"cas def456").unwrap();
    }
    assert_eq!(read_control(&device, "new.localhost"), "cas def456\n");
}

#[test]
fn removing_a_host_unbinds_it() {
    let device = SitesDevice::new();
    device.bind_site(
        Host::registered("gone.localhost").unwrap(),
        SiteSource::Memory(Arc::new(MemFs::new())),
    );
    assert!(
        device
            .resolve(&Host::parse("gone.localhost").unwrap())
            .is_some()
    );

    device.remove_file(&path("gone.localhost")).unwrap();
    assert!(
        device
            .resolve(&Host::parse("gone.localhost").unwrap())
            .is_none()
    );
    assert!(device.read_dir(&path(".")).unwrap().is_empty());
}

#[test]
fn cas_source_resolves_to_none_without_a_store() {
    // A device built with `new()` records a `cas` binding but has no store to
    // load blobs from, so it cannot serve it.
    let device = SitesDevice::new();
    device.bind_site(
        Host::registered("docs.localhost").unwrap(),
        SiteSource::Cas("00".repeat(32)),
    );
    assert!(
        device
            .resolve(&Host::parse("docs.localhost").unwrap())
            .is_none()
    );
}

#[test]
fn publish_freezes_serves_by_hash_and_rolls_back() {
    let store: Arc<dyn ContentStore> = Arc::new(MemStore::default());
    let device = SitesDevice::with_store(Arc::clone(&store));
    let host = Host::registered("docs.localhost").unwrap();

    // Publish v1.
    let v1_site = MemFs::new();
    v1_site.write_file("index.html", b"<h1>v1</h1>").unwrap();
    let v1 = device.publish(host.clone(), &v1_site, ".").unwrap();

    // The host now serves the frozen v1 by hash.
    let served = device
        .resolve(&Host::parse("docs.localhost").unwrap())
        .unwrap();
    assert_eq!(read_file(served.as_ref(), "index.html"), b"<h1>v1</h1>");

    // Mutate the source and publish v2: a new hash serves new content.
    let v2_site = MemFs::new();
    v2_site.write_file("index.html", b"<h1>v2</h1>").unwrap();
    let v2 = device.publish(host.clone(), &v2_site, ".").unwrap();
    assert_ne!(v1, v2);
    let served_v2 = device
        .resolve(&Host::parse("docs.localhost").unwrap())
        .unwrap();
    assert_eq!(read_file(served_v2.as_ref(), "index.html"), b"<h1>v2</h1>");

    // The old hash still serves the old content (immutability) — binding the
    // host back to v1 is an instant rollback with no file copying.
    device.bind_site(host, SiteSource::Cas(v1.to_hex()));
    let rolled_back = device
        .resolve(&Host::parse("docs.localhost").unwrap())
        .unwrap();
    assert_eq!(
        read_file(rolled_back.as_ref(), "index.html"),
        b"<h1>v1</h1>"
    );
}

#[test]
fn publish_without_store_is_an_error() {
    let device = SitesDevice::new();
    let site = MemFs::new();
    site.write_file("index.html", b"x").unwrap();
    assert!(matches!(
        device.publish(Host::registered("x.localhost").unwrap(), &site, "."),
        Err(crate::PublishError::NoStore)
    ));
}

#[test]
fn dir_source_descriptor_round_trips() {
    // `dir <path>` is the file-driven way to serve a host directory as a site,
    // replacing the old `--site` CLI flag.
    let source = SiteSource::parse_descriptor("dir /srv/blog").expect("parse dir descriptor");
    assert!(matches!(&source, SiteSource::Dir(path) if path == std::path::Path::new("/srv/blog")));
    assert_eq!(source.descriptor(), "dir /srv/blog\n");
}

#[test]
fn dir_source_resolves_to_the_host_directory() {
    // Writing `dir <path>` to `#sites/<host>` makes the gateway serve that
    // directory through a `LocalFs` — no flag, no in-process registration.
    let dir = std::env::temp_dir().join(format!(
        "wanix-sites-dir-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create site dir");
    std::fs::write(dir.join("index.html"), b"<h1>dir site</h1>").expect("write index");

    let device = SitesDevice::new();
    let host = Host::registered("blog.localhost").unwrap();
    device.bind_site(host.clone(), SiteSource::Dir(dir.clone()));

    let fs = device.resolve(&host).expect("dir source resolves");
    assert_eq!(read_file(fs.as_ref(), "index.html"), b"<h1>dir site</h1>");

    let _ = std::fs::remove_dir_all(&dir);
}
