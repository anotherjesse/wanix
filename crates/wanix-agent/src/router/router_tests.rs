use std::sync::Arc;

use wanix_fs::FileSystem;
use wanix_vfs::{BindOptions, Namespace};

use crate::engine::AgentEngine;
use crate::{AgentDevice, FakeEngine, RemoteEngine, RouterEngine};

/// An imported peer `#agent`, reachable as files through a namespace.
fn imported_agent() -> Arc<dyn FileSystem> {
    let mut namespace = Namespace::new();
    namespace
        .bind(
            Arc::new(AgentDevice::new(Arc::new(FakeEngine))),
            ".",
            "#agent",
            BindOptions::default(),
        )
        .unwrap();
    Arc::new(namespace)
}

#[test]
fn default_route_runs_the_local_engine() {
    // The bare start_session dispatches to the default (local) engine, so a
    // router substitutes for a single engine anywhere one is expected.
    let router = RouterEngine::new(Arc::new(FakeEngine));
    assert!(router.describe().contains("default=fake"));
    let session = router.start_session().unwrap();
    session.submit("local turn").unwrap();
    assert_eq!(session.wait_reply().unwrap(), "you said: local turn");
}

#[test]
fn a_named_route_runs_a_remote_engine() {
    // Routing to "A" runs the session on node A's imported `#agent`, while the
    // default still runs locally — the same call, a different machine.
    let remote = Arc::new(RemoteEngine::new(imported_agent(), "#agent", "remote-A"));
    let router = RouterEngine::new(Arc::new(FakeEngine)).route("A", remote);

    assert_eq!(router.route_names(), vec!["A".to_owned()]);
    let session = router.start_session_on("A").expect("route to A");
    session.submit("remote turn").unwrap();
    assert_eq!(session.wait_reply().unwrap(), "you said: remote turn");
}

#[test]
fn an_unknown_route_is_not_found() {
    let router = RouterEngine::new(Arc::new(FakeEngine));
    assert!(router.start_session_on("nope").is_err());
}
