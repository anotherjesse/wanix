use std::sync::Arc;

use wanix_fs::{FileSystem, MemFs, NormalizedPath, OpenOptions};

use crate::{Host, SiteSource, SitesDevice};

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
