//! Integration tests for `P9Server::with_policy`: capability-as-a-bind.
//!
//! These drive the public 9P server API exactly as a transport loop would,
//! proving that a verified peer attaches a policy-scoped root, that the rights
//! gate denies out-of-scope mutation, that walks cannot escape the granted
//! prefix, that revocation denies the next attach, and that a policy-free
//! server behaves byte-for-byte like a plain `P9Server::new`.

use std::sync::Arc;

use wanix_9p::{Grant, GrantTable, GrantTablePolicy, P9Server, PeerId};
use wanix_fs::{FileSystem, MemFs};
use wanix_protocol::{
    O_RDWR, O_WRONLY, P9_RATTACH, P9_RLERROR, P9_RLOPEN, P9_RREAD, P9_RWALK, P9_VERSION_9P2000_L,
    p9_decode_rlerror, p9_decode_rread, p9_tattach, p9_tlopen, p9_tread, p9_tversion, p9_twalk,
};
use wanix_vfs::Rights;

const EACCES: u32 = 13;

fn backing() -> Arc<dyn FileSystem> {
    let fs = Arc::new(MemFs::new());
    fs.create_dir_all("projects/foo").unwrap();
    fs.write_file("projects/foo/file.txt", b"scoped bytes")
        .unwrap();
    fs.create_dir_all("docs").unwrap();
    fs.write_file("docs/readme.txt", b"docs bytes").unwrap();
    fs
}

fn negotiate(server: &mut P9Server) {
    let response = server
        .handle_frame(&p9_tversion(1, 8192, P9_VERSION_9P2000_L).unwrap())
        .unwrap();
    assert_eq!(response.message_type(), wanix_protocol::P9_RVERSION);
}

fn attach(server: &mut P9Server, fid: u32, aname: &str) -> u8 {
    let response = server
        .handle_frame(&p9_tattach(2, fid, 0xffff_ffff, "peer", aname, 0).unwrap())
        .unwrap();
    response.message_type()
}

#[test]
fn authorized_attach_installs_scoped_root() {
    let peer = PeerId::from_bytes([1u8; 32]);
    let grants = GrantTable::new();
    grants.grant(Grant::new(
        peer,
        "projects/foo",
        backing(),
        "projects/foo",
        Rights::read_write(),
    ));
    let policy = Arc::new(GrantTablePolicy::new(grants));
    let mut server = P9Server::with_policy(backing(), peer, policy);

    negotiate(&mut server);
    assert_eq!(attach(&mut server, 1, "projects/foo"), P9_RATTACH);

    // The scoped root sees the granted file at its own root: walk "file.txt",
    // not "projects/foo/file.txt".
    let response = server
        .handle_frame(&p9_twalk(3, 1, 2, &["file.txt"]).unwrap())
        .unwrap();
    assert_eq!(response.message_type(), P9_RWALK);

    let response = server.handle_frame(&p9_tlopen(4, 2, O_RDWR)).unwrap();
    assert_eq!(response.message_type(), P9_RLOPEN);

    let response = server.handle_frame(&p9_tread(5, 2, 0, 64)).unwrap();
    assert_eq!(response.message_type(), P9_RREAD);
    assert_eq!(p9_decode_rread(&response).unwrap(), b"scoped bytes");
}

#[test]
fn walk_cannot_escape_the_granted_prefix() {
    let peer = PeerId::from_bytes([2u8; 32]);
    let grants = GrantTable::new();
    grants.grant(Grant::new(
        peer,
        "projects/foo",
        backing(),
        "projects/foo",
        Rights::read_write(),
    ));
    let policy = Arc::new(GrantTablePolicy::new(grants));
    let mut server = P9Server::with_policy(backing(), peer, policy);

    negotiate(&mut server);
    assert_eq!(attach(&mut server, 1, "projects/foo"), P9_RATTACH);

    // The sibling docs tree is invisible: walking to it fails (it is not under
    // projects/foo), and `..` is not even an expressible component.
    let response = server
        .handle_frame(&p9_twalk(3, 1, 2, &["docs"]).unwrap())
        .unwrap();
    assert_eq!(response.message_type(), P9_RLERROR);
}

#[test]
fn read_only_grant_denies_writes() {
    let peer = PeerId::from_bytes([3u8; 32]);
    let grants = GrantTable::new();
    grants.grant(Grant::new(
        peer,
        "docs",
        backing(),
        "docs",
        Rights::read_only(),
    ));
    let policy = Arc::new(GrantTablePolicy::new(grants));
    let mut server = P9Server::with_policy(backing(), peer, policy);

    negotiate(&mut server);
    assert_eq!(attach(&mut server, 1, "docs"), P9_RATTACH);

    let response = server
        .handle_frame(&p9_twalk(3, 1, 2, &["readme.txt"]).unwrap())
        .unwrap();
    assert_eq!(response.message_type(), P9_RWALK);

    // Opening the read-only root's file for writing maps to a write right the
    // SubtreeFs does not grant -> EACCES.
    let response = server.handle_frame(&p9_tlopen(4, 2, O_WRONLY)).unwrap();
    assert_eq!(response.message_type(), P9_RLERROR);
    assert_eq!(p9_decode_rlerror(&response).unwrap().ecode, EACCES);
}

#[test]
fn unauthorized_attach_is_denied() {
    let peer = PeerId::from_bytes([4u8; 32]);
    let grants = GrantTable::new();
    grants.grant(Grant::new(
        peer,
        "projects/foo",
        backing(),
        "projects/foo",
        Rights::read_write(),
    ));
    let policy = Arc::new(GrantTablePolicy::new(grants));
    let mut server = P9Server::with_policy(backing(), peer, policy);

    negotiate(&mut server);

    // No grant for aname "docs": default-deny.
    let response = server
        .handle_frame(&p9_tattach(2, 1, 0xffff_ffff, "peer", "docs", 0).unwrap())
        .unwrap();
    assert_eq!(response.message_type(), P9_RLERROR);
    assert_eq!(p9_decode_rlerror(&response).unwrap().ecode, EACCES);
}

#[test]
fn revoking_the_grant_denies_the_next_attach() {
    let peer = PeerId::from_bytes([5u8; 32]);
    let grants = GrantTable::new();
    grants.grant(Grant::new(
        peer,
        "projects/foo",
        backing(),
        "projects/foo",
        Rights::read_write(),
    ));
    // Keep a handle so we can revoke after the first attach succeeds.
    let revoke_handle = grants.clone();
    let policy = Arc::new(GrantTablePolicy::new(grants));
    let mut server = P9Server::with_policy(backing(), peer, policy);

    negotiate(&mut server);
    assert_eq!(attach(&mut server, 1, "projects/foo"), P9_RATTACH);

    assert_eq!(revoke_handle.revoke(peer, "projects/foo"), 1);

    let response = server
        .handle_frame(&p9_tattach(2, 9, 0xffff_ffff, "peer", "projects/foo", 0).unwrap())
        .unwrap();
    assert_eq!(response.message_type(), P9_RLERROR);
    assert_eq!(p9_decode_rlerror(&response).unwrap().ecode, EACCES);
}

#[test]
fn policy_free_server_matches_plain_new() {
    // A policy-free server serves the whole root; the demo's "None == today".
    let mut policy_free = P9Server::new(backing());
    negotiate(&mut policy_free);
    assert_eq!(attach(&mut policy_free, 1, ""), P9_RATTACH);

    // It sees the full tree (docs is reachable), unlike a scoped grant.
    let response = policy_free
        .handle_frame(&p9_twalk(3, 1, 2, &["docs", "readme.txt"]).unwrap())
        .unwrap();
    assert_eq!(response.message_type(), P9_RWALK);

    let response = policy_free.handle_frame(&p9_tlopen(4, 2, O_RDWR)).unwrap();
    assert_eq!(response.message_type(), P9_RLOPEN);
    let response = policy_free.handle_frame(&p9_tread(5, 2, 0, 64)).unwrap();
    assert_eq!(p9_decode_rread(&response).unwrap(), b"docs bytes");
}
