//! [`RemoteFs`] drives a `#kv` service device over loopback TCP, sync and
//! iroh-free, proving Slice 4's keystone property: a service-device value file
//! crosses 9P byte-exact even though it is **non-seekable** on the server.
//!
//! A [`wanix_kv::KvDevice`]'s value files report [`wanix_fs::FileType::File`], so
//! the client marks them seekable and tracks a local offset. But the server's
//! read handle does not honor `Tread.offset` (a `#kv` value streams from its own
//! cursor), so for a purely sequential read the client offset and server cursor
//! advance in lockstep and the bytes are exact. This is the honest-seekability
//! contract from Slice 1 (correction #2): a sequential stream of a non-seekable
//! file is correct; the moment a client *seeks* such a file, a fictional local
//! offset would silently corrupt the read. These tests pin both halves of that
//! contract directly against the real [`P9Server`], with no async runtime.

use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::thread;

use wanix_9p::P9Server;
use wanix_9p_client::RemoteFs;
use wanix_fs::{FileSystem, FileType, NormalizedPath, OpenOptions};
use wanix_kv::KvDevice;

/// Spawns a serial 9P server exporting `fs` over one accepted loopback socket,
/// returning the client end of the connection.
fn spawn_server(fs: Arc<dyn FileSystem>) -> TcpStream {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let read = stream.try_clone().unwrap();
        let mut server = P9Server::new(fs);
        let _ = server.serve_stream(read, stream);
    });
    TcpStream::connect(addr).unwrap()
}

fn path(value: &str) -> NormalizedPath {
    NormalizedPath::new(value).unwrap()
}

/// Reads a remote file to EOF with a small buffer so a multi-read value forces
/// the streaming path across several sequential `Tread`s.
fn read_to_end(remote: &RemoteFs, p: &NormalizedPath) -> Vec<u8> {
    let mut file = remote.open(p, OpenOptions::read()).unwrap();
    let mut out = Vec::new();
    let mut chunk = [0u8; 64];
    loop {
        let n = file.read(&mut chunk).unwrap();
        if n == 0 {
            break;
        }
        out.extend_from_slice(&chunk[..n]);
        assert!(out.len() < (1 << 20), "stream grew unbounded");
    }
    out
}

/// Seeds `key` with `value` on the server-side device before the client mounts.
fn seed(kv: &KvDevice, key: &str, value: &[u8]) {
    let mut file = kv
        .open(
            &path(key),
            OpenOptions {
                write: true,
                create: true,
                ..OpenOptions::default()
            },
        )
        .unwrap();
    file.write(value).unwrap();
}

#[test]
fn kv_value_streams_back_byte_exact() {
    // The demo path: read a remote `#kv` value sequentially. A small value that
    // fits one read still proves the device file crosses 9P as a service file,
    // not just a plain on-disk file.
    let kv = KvDevice::new();
    seed(&kv, "config", b"region=us\nreplicas=3\n");
    let client = spawn_server(Arc::new(kv) as Arc<dyn FileSystem>);
    let remote = RemoteFs::connect(Box::new(client)).unwrap();

    assert_eq!(
        read_to_end(&remote, &path("config")),
        b"region=us\nreplicas=3\n"
    );
}

#[test]
fn large_kv_value_streams_without_fake_offset_corruption() {
    // The corruption-proof: a value larger than one read chunk. The server never
    // honors the client's `Tread.offset` for a `#kv` value (it is non-seekable),
    // so any reliance on a fictional client offset would reorder or drop bytes
    // across chunk boundaries. Position-dependent contents make any such slip
    // fail the comparison.
    let value: Vec<u8> = (0..30_000u32).map(|i| (i % 251) as u8).collect();
    let kv = KvDevice::new();
    seed(&kv, "blob", &value);
    let client = spawn_server(Arc::new(kv) as Arc<dyn FileSystem>);
    let remote = RemoteFs::connect(Box::new(client)).unwrap();

    assert_eq!(
        read_to_end(&remote, &path("blob")),
        value,
        "a non-seekable #kv value streamed back corrupted"
    );
}

#[test]
fn kv_write_commits_on_close_over_9p() {
    // Writing `#kv/<key>` buffers server-side and commits when the fid is clunked.
    // The client's drop sends Tclunk, so the value must be observable afterward.
    let kv = KvDevice::new();
    let observe = kv.clone();
    let client = spawn_server(Arc::new(kv) as Arc<dyn FileSystem>);
    let remote = RemoteFs::connect(Box::new(client)).unwrap();

    let mut file = remote
        .open(
            &path("result"),
            OpenOptions {
                read: false,
                write: true,
                create: true,
                truncate: true,
            },
        )
        .unwrap();
    file.write(b"status=ok\n").unwrap();
    // Drop the handle: the close-time Tclunk commits the buffered value.
    drop(file);

    // The server-side device now holds the bytes that crossed the wire.
    assert_eq!(
        read_to_end(&remote, &path("result")),
        b"status=ok\n",
        "the written #kv value was not committed on close"
    );

    // The same key is visible directly on the server-side device handle.
    let mut local = observe.open(&path("result"), OpenOptions::read()).unwrap();
    let mut buf = Vec::new();
    let mut chunk = [0u8; 64];
    loop {
        let n = local.read(&mut chunk).unwrap();
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    assert_eq!(buf, b"status=ok\n");
}

#[test]
fn kv_value_reports_file_type_over_9p() {
    // A `#kv` value statted over the wire is a regular file: this is exactly why
    // the client marks it seekable, and exactly why the streaming-read honesty
    // matters. The metadata len mirrors the stored value length.
    let kv = KvDevice::new();
    seed(&kv, "k", b"twelve bytes");
    let client = spawn_server(Arc::new(kv) as Arc<dyn FileSystem>);
    let remote = RemoteFs::connect(Box::new(client)).unwrap();

    let metadata = remote.metadata(&path("k")).unwrap();
    assert_eq!(metadata.file_type(), FileType::File);
    assert_eq!(metadata.len(), b"twelve bytes".len() as u64);
}
