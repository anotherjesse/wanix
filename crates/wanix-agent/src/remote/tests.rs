use std::sync::Arc;

use wanix_fs::FileSystem;
use wanix_vfs::{BindOptions, Namespace};

use crate::engine::AgentEngine;
use crate::{AgentDevice, FakeEngine, RemoteEngine};

/// Builds a namespace with a fake-backed `#agent` device bound at `#agent`,
/// modeling an imported peer agent reachable as ordinary files.
fn imported_agent_namespace() -> Arc<dyn FileSystem> {
    let device = AgentDevice::new(Arc::new(FakeEngine));
    let mut namespace = Namespace::new();
    namespace
        .bind(Arc::new(device), ".", "#agent", BindOptions::default())
        .unwrap();
    Arc::new(namespace)
}

#[test]
fn remote_engine_drives_a_session_as_files() {
    // The remote engine proxies to the `#agent` device through the namespace,
    // exactly as it would over a `/n/A` mesh import — no transport coupling.
    let fs = imported_agent_namespace();
    let engine = RemoteEngine::new(fs, "#agent", "remote");
    assert_eq!(engine.describe(), "remote");

    let session = engine.start_session().expect("allocate remote session");

    // Submitting a prompt writes the remote `<id>/prompt` file; the fake engine
    // replies `you said: <prompt>`, observable through the remote `<id>/reply`.
    session.submit("hello mesh").expect("submit remote prompt");
    let reply = session.wait_reply().expect("read remote reply");
    assert_eq!(reply, "you said: hello mesh");

    // Status crosses as a snapshot file read.
    assert!(session.status().contains("fake"));
    session.close();
}

#[test]
fn remote_engine_streams_events() {
    let fs = imported_agent_namespace();
    let engine = RemoteEngine::new(fs, "#agent", "remote");
    let session = engine.start_session().unwrap();
    session.submit("hi").unwrap();

    // The remote events stream is drained through the imported `<id>/events`
    // file; the fake engine's normalized JSONL is delivered byte-for-byte.
    let mut buf = [0_u8; 4096];
    let n = session.read_events(&mut buf).unwrap();
    let text = std::str::from_utf8(&buf[..n]).unwrap();
    assert!(text.contains("message"), "events stream: {text}");
}

#[test]
fn remote_engine_reports_an_empty_new_as_error() {
    // A device whose `new` does not yield an id is a protocol error, not a
    // silent empty session.
    let empty = Arc::new(wanix_fs::MemFs::new());
    empty.create_dir_all("#agent").unwrap();
    empty.write_file("#agent/new", b"\n").unwrap();
    let engine = RemoteEngine::new(empty, "#agent", "remote");
    assert!(engine.start_session().is_err());
}
