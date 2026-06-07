//! End-to-end plumber check: a `#plumb` topic carried across real QUIC gossip.
//!
//! Two [`MeshNode`]s bind on loopback (relay/DNS disabled, direct-address only).
//! Each builds a [`GossipPlumbPort`] bootstrapped from the other's ticket and
//! serves the gossip ALPN, then backs a [`wanix_plumb::PlumbDevice`]. Node A
//! subscribes to `#plumb/build/recv`; node B writes a `task.done` envelope to
//! `#plumb/build/send`; the message crosses the gossip swarm and node A reads it.
//! This is the plumber on the mesh: typed, best-effort, broker-less coordination
//! between two machines.

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use wanix_fs::{File, FileSystem, NormalizedPath, OpenOptions};
use wanix_id::NodeIdentity;
use wanix_mesh::{GossipPlumbPort, MeshNode, ServeConfig};
use wanix_plumb::{PlumbDevice, PlumbEnvelope};

fn path(value: &str) -> NormalizedPath {
    NormalizedPath::new(value).unwrap()
}

fn loopback() -> SocketAddr {
    SocketAddr::new(Ipv4Addr::LOCALHOST.into(), 0)
}

fn write_only() -> OpenOptions {
    OpenOptions {
        write: true,
        ..OpenOptions::default()
    }
}

/// Opens `topic/send` on `device` and publishes `envelope`.
fn publish(device: &PlumbDevice, topic: &str, envelope: &PlumbEnvelope) {
    let line = envelope.to_line().unwrap();
    let json = &line[..line.len() - 1];
    let mut send = device
        .open(&path(&format!("{topic}/send")), write_only())
        .unwrap();
    let mut written = 0;
    while written < json.len() {
        written += send.write(&json[written..]).unwrap();
    }
}

/// Blocks (bounded) until `recv` yields a line, retrying the publish so a
/// just-formed gossip swarm that dropped the first best-effort broadcast still
/// converges. Returns the parsed envelope.
fn recv_with_retry(
    recv: &mut Box<dyn File>,
    republish: impl Fn(),
    timeout: Duration,
) -> Option<PlumbEnvelope> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        republish();
        // Give the broadcast a moment to traverse the swarm, then check readiness.
        std::thread::sleep(Duration::from_millis(150));
        if recv.read_ready().unwrap_or(false) {
            let mut buf = [0_u8; 1024];
            let n = recv.read(&mut buf).unwrap();
            if n == 0 {
                return None;
            }
            let line = std::str::from_utf8(&buf[..n]).unwrap();
            return PlumbEnvelope::parse(line.trim_end().as_bytes()).ok();
        }
    }
    None
}

/// Builds a node, its gossip plumb port (bootstrapped from `peers`), serves
/// gossip, and wraps the port in a `#plumb` device.
fn plumb_node(
    seed: u8,
    peers: Vec<iroh::EndpointAddr>,
) -> (MeshNode, GossipPlumbPort, PlumbDevice) {
    let identity = NodeIdentity::from_secret_bytes([seed; 32]);
    let mut node = MeshNode::bind_local(&identity, loopback()).unwrap();
    let port = node.plumb_port(peers);
    // An empty served root is fine: this test exercises only the gossip plane.
    node.serve_with_plumb(ServeConfig::open(Arc::new(wanix_fs::MemFs::new())), &port);
    let device = PlumbDevice::new(Arc::new(port.clone()));
    (node, port, device)
}

#[test]
fn a_topic_message_crosses_the_gossip_mesh() {
    // Node A binds first so node B can bootstrap from A's ticket; A then learns
    // B's ticket and rebuilds its port bootstrapped from B, so the swarm is
    // mutually dialable on a relay-less loopback.
    let identity_a = NodeIdentity::from_secret_bytes([101u8; 32]);
    let mut node_a = MeshNode::bind_local(&identity_a, loopback()).unwrap();
    let ticket_a = node_a.ticket();

    let (node_b, _port_b, device_b) = plumb_node(102, vec![ticket_a.clone()]);
    let ticket_b = node_b.ticket();

    // Build A's port bootstrapped from B and serve gossip on A.
    let port_a = node_a.plumb_port(vec![ticket_b]);
    node_a.serve_with_plumb(ServeConfig::open(Arc::new(wanix_fs::MemFs::new())), &port_a);
    let device_a = PlumbDevice::new(Arc::new(port_a));

    // Node A subscribes to the topic (joins the gossip swarm for it).
    let mut recv_a = device_a
        .open(&path("build/recv"), OpenOptions::read())
        .unwrap();

    let envelope = PlumbEnvelope {
        kind: "task.done".to_owned(),
        from: "node-b".to_owned(),
        to: String::new(),
        body: serde_json::json!({ "out": "/world/result" }),
    };

    // Node B publishes; the message crosses the swarm to A. Best-effort delivery
    // on a freshly formed swarm can drop the first broadcast before membership
    // settles, so retry until A observes it (bounded).
    let republish = || publish(&device_b, "build", &envelope);
    let received = recv_with_retry(&mut recv_a, republish, Duration::from_secs(20))
        .expect("node A must receive node B's #plumb/build message over gossip");
    assert_eq!(received, envelope);

    // Keep the nodes alive until the assertion completes (dropping a node tears
    // down its endpoint, router, and gossip actor).
    drop(node_a);
    drop(node_b);
}

#[test]
fn churned_topics_do_not_accumulate_gossip_memberships() {
    // A client opening and dropping `recv` on many distinct topic names — the
    // walk-path surface a remote 9P importer controls — must not leave a gossip
    // membership and pump task behind for each one. Idle topics are swept on the
    // next topic lookup, so the live-topic count stays bounded, not linear in
    // the number of names ever touched.
    let (node, port, device) = plumb_node(104, Vec::new());
    for i in 0..200 {
        let recv = device
            .open(&path(&format!("ephemeral-{i}/recv")), OpenOptions::read())
            .unwrap();
        // Drop the subscription immediately: the topic is now idle.
        drop(recv);
    }
    // One more open triggers the sweep of all the now-idle topics.
    let _live = device
        .open(&path("survivor/recv"), OpenOptions::read())
        .unwrap();
    let live = port.live_topic_count();
    assert!(
        live <= 2,
        "idle topics must be evicted; {live} gossip memberships were retained"
    );
    drop(node);
}

#[test]
fn same_node_delivery_needs_no_swarm() {
    // A single node's `#plumb` device delivers between its own subscribers
    // immediately: the gossip port's local fan-out does not wait for membership.
    let (node, _port, device) = plumb_node(103, Vec::new());
    let mut recv = device
        .open(&path("local/recv"), OpenOptions::read())
        .unwrap();
    let envelope = PlumbEnvelope::new("ping");
    publish(&device, "local", &envelope);

    // No retry needed: same-node delivery is synchronous through the fan-out.
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut received = None;
    while Instant::now() < deadline {
        if recv.read_ready().unwrap() {
            let mut buf = [0_u8; 256];
            let n = recv.read(&mut buf).unwrap();
            let line = std::str::from_utf8(&buf[..n]).unwrap();
            received = PlumbEnvelope::parse(line.trim_end().as_bytes()).ok();
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(received, Some(envelope));
    drop(node);
}
