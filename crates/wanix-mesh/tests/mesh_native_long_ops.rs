//! Open-file reply waits must outlive the per-op deadline.
//!
//! After the open reply, an open file's replies may legitimately take longer
//! than any fixed per-op deadline: a never-EOF `#plumb/<topic>/recv` read
//! parks until something is published, and a synchronous `ctl run` write on a
//! job device replies only when the job finishes. The client therefore bounds
//! only the FIRST reply (the open response) with the per-op deadline and lets
//! QUIC connection liveness bound a dead peer afterwards (ADR 0008) — a
//! healthy provider that is simply *busy* must never be torn down mid-op.
//! Before this contract, any mesh-mounted tool job longer than the CLI's 5s
//! mount deadline failed client-side as "resource unreachable" while the job
//! kept running server-side.

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use wanix_fs::{FileSystem, NormalizedPath, OpenOptions};
use wanix_id::NodeIdentity;
use wanix_mesh::{MeshNode, NativeServeConfig};
use wanix_plumb::{PlumbDevice, PlumbEnvelope};
use wanix_vfs::{BindOptions, Namespace};

fn loopback() -> SocketAddr {
    SocketAddr::new(Ipv4Addr::LOCALHOST.into(), 0)
}

fn path(value: &str) -> NormalizedPath {
    NormalizedPath::new(value).unwrap()
}

#[test]
fn parked_open_file_read_outlives_a_short_per_op_deadline() {
    const DEADLINE: Duration = Duration::from_millis(250);
    const PARK: Duration = Duration::from_millis(1_000);

    let plumb = PlumbDevice::local();
    let mut served = Namespace::new();
    served
        .bind(
            Arc::new(plumb.clone()),
            ".",
            "#plumb",
            BindOptions::default(),
        )
        .unwrap();

    let mut server = MeshNode::bind_local(&NodeIdentity::from_secret_bytes([81u8; 32]), loopback())
        .unwrap()
        .with_deadline(DEADLINE);
    server.serve_native(NativeServeConfig::open(
        Arc::new(served) as Arc<dyn FileSystem>
    ));

    let client = MeshNode::bind_local(&NodeIdentity::from_secret_bytes([82u8; 32]), loopback())
        .unwrap()
        .with_deadline(DEADLINE);
    let remote = client.dialer().dial_native(server.ticket()).unwrap();
    let mut namespace = Namespace::new();
    namespace
        .bind(remote, ".", "n/A", BindOptions::default())
        .unwrap();

    // Park a never-EOF read on its own thread for several deadlines.
    let recv_ns = namespace.clone();
    let reader = std::thread::spawn(move || {
        let mut recv = recv_ns
            .open(&path("n/A/#plumb/build/recv"), OpenOptions::read())
            .unwrap();
        let mut buf = [0u8; 4096];
        let started = Instant::now();
        let n = recv.read(&mut buf).unwrap();
        (started.elapsed(), buf[..n].to_vec())
    });

    // Hold the provider silent well past the per-op deadline, then publish
    // over a sibling stream so the parked read finally has its bytes.
    std::thread::sleep(PARK);
    let envelope = PlumbEnvelope {
        kind: "job.done".to_owned(),
        from: "node-a".to_owned(),
        to: String::new(),
        body: serde_json::json!({ "took": "longer than the deadline" }),
    };
    let line = envelope.to_line().unwrap();
    let json = &line[..line.len() - 1];
    let mut send = namespace
        .open(
            &path("n/A/#plumb/build/send"),
            OpenOptions {
                write: true,
                ..OpenOptions::default()
            },
        )
        .unwrap();
    let mut written = 0;
    while written < json.len() {
        written += send.write(&json[written..]).unwrap();
    }
    drop(send);
    let _ = plumb; // keep the server-side device handle alive.

    let (waited, bytes) = reader
        .join()
        .expect("the parked read must survive the per-op deadline on a healthy provider");
    assert!(
        waited >= PARK - Duration::from_millis(50),
        "the read returned in {waited:?}, before anything was published"
    );
    let parsed =
        PlumbEnvelope::parse(String::from_utf8(bytes).unwrap().trim_end().as_bytes()).unwrap();
    assert_eq!(parsed, envelope);

    drop(namespace);
    drop(client);
    drop(server);
}
