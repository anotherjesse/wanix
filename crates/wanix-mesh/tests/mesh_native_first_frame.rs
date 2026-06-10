//! The first-frame guard over real QUIC: a stream that never completes its
//! request frame must be torn down by the server within the per-op deadline,
//! releasing its session permit — it must not pin a permit and blocking-pool
//! thread until the connection dies (the silent-stream wedge any ticket
//! holder could otherwise inflict on every other peer).

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use wanix_fs::{FileSystem, MemFs, NormalizedPath, OpenOptions};
use wanix_id::NodeIdentity;
use wanix_mesh::{MeshNode, NativeServeConfig};

fn loopback() -> SocketAddr {
    SocketAddr::new(Ipv4Addr::LOCALHOST.into(), 0)
}

#[test]
fn partial_first_frame_stream_is_torn_down_and_serving_continues() {
    let server_identity = NodeIdentity::from_secret_bytes([71u8; 32]);
    let root = Arc::new(MemFs::new());
    root.write_file("marker.txt", b"alive").unwrap();
    let mut server = MeshNode::bind_local(&server_identity, loopback())
        .unwrap()
        .with_deadline(Duration::from_millis(500));
    server.serve_native(NativeServeConfig::open(root as Arc<dyn FileSystem>));
    let ticket = server.ticket();

    // A raw QUIC client (not the wire client) so the test can stall
    // mid-frame: write a partial length prefix and then go silent.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    let started = Instant::now();
    runtime.block_on(async move {
        let endpoint = iroh::Endpoint::builder(iroh::endpoint::presets::Minimal)
            .relay_mode(iroh::RelayMode::Disabled)
            .bind_addr(loopback())
            .unwrap()
            .bind()
            .await
            .unwrap();
        let connection = endpoint
            .connect(ticket, wanix_mesh::WANIX_FS_ALPN)
            .await
            .unwrap();
        let (mut send, mut recv) = connection.open_bi().await.unwrap();
        // Two bytes of the four-byte length prefix, then silence. The write
        // is what makes the server's accept_bi see the stream at all.
        send.write_all(&[7, 0]).await.unwrap();

        // The server must abandon the stream once the first-frame deadline
        // fires; the client observes the teardown as a read error or EOF.
        let teardown = tokio::time::timeout(Duration::from_secs(10), async {
            let mut buf = [0u8; 16];
            loop {
                match recv.read(&mut buf).await {
                    Ok(Some(_)) => continue,
                    Ok(None) | Err(_) => return,
                }
            }
        })
        .await;
        assert!(
            teardown.is_ok(),
            "server never tore down the silent stream within 10s"
        );
    });
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "teardown took {:?}",
        started.elapsed()
    );

    // The endpoint still serves: a real dial after the stalled stream works.
    let client = MeshNode::bind_local(&NodeIdentity::from_secret_bytes([72u8; 32]), loopback())
        .unwrap()
        .with_deadline(Duration::from_secs(5));
    let remote = client.dialer().dial_native(server.ticket()).unwrap();
    let mut file = remote
        .open(
            &NormalizedPath::new("marker.txt").unwrap(),
            OpenOptions::read(),
        )
        .unwrap();
    let mut buf = [0u8; 16];
    let read = file.read(&mut buf).unwrap();
    assert_eq!(&buf[..read], b"alive");
}
