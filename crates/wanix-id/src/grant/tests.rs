use std::sync::Arc;

use wanix_fs::{FileSystem, FsError, MemFs, NormalizedPath, OpenOptions};
use wanix_vfs::Rights;

use super::{Grant, GrantTable};
use crate::PeerId;

fn backing() -> Arc<dyn FileSystem> {
    let fs = Arc::new(MemFs::new());
    fs.create_dir_all("projects/foo").unwrap();
    fs.write_file("projects/foo/file.txt", b"inside").unwrap();
    fs.create_dir_all("docs").unwrap();
    fs.write_file("docs/readme.txt", b"docs").unwrap();
    fs
}

fn path(value: &str) -> NormalizedPath {
    NormalizedPath::new(value).unwrap()
}

#[test]
fn empty_table_denies_everyone() {
    let table = GrantTable::new();
    assert!(table.is_empty());
    assert!(
        table
            .authorize(PeerId::from_bytes([0u8; 32]), ".")
            .is_none()
    );
}

#[test]
fn grant_authorizes_only_matching_peer_and_aname() {
    let peer = PeerId::from_bytes([1u8; 32]);
    let other = PeerId::from_bytes([2u8; 32]);
    let table = GrantTable::new();
    table.grant(Grant::new(
        peer,
        "projects/foo",
        backing(),
        "projects/foo",
        Rights::read_write(),
    ));

    assert!(table.authorize(peer, "projects/foo").is_some());
    assert!(table.authorize(peer, "docs").is_none());
    assert!(table.authorize(other, "projects/foo").is_none());
}

#[test]
fn authorized_root_is_scoped_and_rights_gated() {
    let peer = PeerId::from_bytes([3u8; 32]);
    let table = GrantTable::new();
    // Read-write grant on projects/foo, read-only grant on docs.
    table.grant(Grant::new(
        peer,
        "projects/foo",
        backing(),
        "projects/foo",
        Rights::read_write(),
    ));
    table.grant(Grant::new(
        peer,
        "docs",
        backing(),
        "docs",
        Rights::read_only(),
    ));

    let rw = table.authorize(peer, "projects/foo").unwrap();
    assert_eq!(rw.rights, Rights::read_write());
    // The scoped root sees the granted file at its own root.
    let mut file = rw
        .root
        .open(&path("file.txt"), OpenOptions::read())
        .unwrap();
    let mut buf = [0u8; 6];
    let read = file.read(&mut buf).unwrap();
    assert_eq!(&buf[..read], b"inside");
    // It cannot see the sibling docs tree.
    assert!(matches!(
        rw.root.metadata(&path("readme.txt")),
        Err(FsError::NotFound)
    ));

    let ro = table.authorize(peer, "docs").unwrap();
    assert_eq!(ro.rights, Rights::read_only());
    // A write to the read-only root is denied inside the filesystem itself.
    assert_eq!(
        ro.root.create_dir(&path("nope")).err(),
        Some(FsError::PermissionDenied)
    );
}

#[test]
fn revoke_removes_the_grant() {
    let peer = PeerId::from_bytes([4u8; 32]);
    let table = GrantTable::new();
    table.grant(Grant::new(
        peer,
        "projects/foo",
        backing(),
        "projects/foo",
        Rights::read_write(),
    ));
    assert_eq!(table.len(), 1);
    assert!(table.authorize(peer, "projects/foo").is_some());

    assert_eq!(table.revoke(peer, "projects/foo"), 1);
    assert!(table.authorize(peer, "projects/foo").is_none());
    assert_eq!(table.revoke(peer, "projects/foo"), 0);
}

#[test]
fn clones_share_one_grant_list() {
    let peer = PeerId::from_bytes([5u8; 32]);
    let table = GrantTable::new();
    let view = table.clone();
    table.grant(Grant::new(peer, ".", backing(), ".", Rights::read_only()));
    // The grant is visible through the clone.
    assert_eq!(view.len(), 1);
    assert!(view.authorize(peer, ".").is_some());
}
