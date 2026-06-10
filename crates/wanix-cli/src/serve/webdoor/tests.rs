use std::fs;
use std::io::{Read as _, Write as _};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use wanix_fs::{
    DirEntry, File, FileSystem, FileType, FsError, FsResult, MemFs, Metadata, NormalizedPath,
    OpenOptions,
};
use wanix_id::NodeIdentity;
use wanix_sites::Host;
use wanix_vfs::Namespace;

use super::super::connection::serve_connection;
use super::super::roots::ServeRoots;
use super::super::{ServeCommand, run_serve_with_listener};
use super::{WebBind, WebBindSource, WebDoor, bind_last};

fn temp_root(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("wanix-webdoor-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn host(name: &str) -> Host {
    Host::parse(name).unwrap()
}

/// Serves `door` on a loopback listener, handling each accepted connection on
/// its own thread (so a live-streamed response never blocks the next accept).
fn spawn_gateway(door: WebDoor, connections: usize, name: &str) -> SocketAddr {
    let root = temp_root(name);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let roots = ServeRoots::new(&root, addr, None, false)
        .unwrap()
        .with_webdoor(Arc::new(door));
    thread::spawn(move || {
        for _ in 0..connections {
            let (stream, peer_addr) = listener.accept().unwrap();
            let connection_roots = roots.clone();
            thread::spawn(move || {
                let _ = serve_connection(&connection_roots, stream, peer_addr);
            });
        }
    });
    addr
}

fn http(addr: SocketAddr, raw: &str) -> String {
    let mut stream = TcpStream::connect(addr).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    stream.write_all(raw.as_bytes()).unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).unwrap();
    String::from_utf8_lossy(&response).into_owned()
}

fn site_fs(index_body: &str) -> Arc<dyn FileSystem> {
    let fs = MemFs::new();
    fs.write_file("index.html", index_body.as_bytes()).unwrap();
    Arc::new(fs)
}

// ---------------------------------------------------------------------------
// An in-process device filesystem standing in for a mesh-mounted room: a
// sized read file, a discrete-write file, a never-EOF stream file fed by a
// channel, and an unreachable file (a dead provider).
// ---------------------------------------------------------------------------

type RecordedPosts = Arc<Mutex<Vec<Vec<u8>>>>;

struct DeviceFs {
    posts: RecordedPosts,
    stream_feed: Mutex<Option<mpsc::Receiver<Vec<u8>>>>,
}

impl DeviceFs {
    fn new() -> (Self, RecordedPosts, mpsc::Sender<Vec<u8>>) {
        let posts = Arc::new(Mutex::new(Vec::new()));
        let (sender, receiver) = mpsc::channel();
        let device = Self {
            posts: Arc::clone(&posts),
            stream_feed: Mutex::new(Some(receiver)),
        };
        (device, posts, sender)
    }

    fn file_metadata(name: &str) -> FsResult<Metadata> {
        match name {
            // A sized file: the gateway sends it whole with Content-Length.
            "latest" => Ok(Metadata::new(FileType::File, 11, 0o444)),
            // Zero-length device files: the gateway streams them chunked.
            "post" | "stream" => Ok(Metadata::new(FileType::File, 0, 0o666)),
            // Declares 4 bytes but produces bytes forever (a misbehaving
            // origin lying about a never-EOF read).
            "liar" => Ok(Metadata::new(FileType::File, 4, 0o444)),
            "dead" => Err(FsError::Unreachable("room offline".to_owned())),
            _ => Err(FsError::NotFound),
        }
    }
}

impl FileSystem for DeviceFs {
    fn open(&self, path: &NormalizedPath, options: OpenOptions) -> FsResult<Box<dyn File>> {
        match (path.as_str(), options.write) {
            ("latest", false) => Ok(Box::new(BytesFile::new(b"hello room\n".to_vec()))),
            ("latest", true) => Err(FsError::NotSupported),
            ("post", true) => Ok(Box::new(PostFile {
                posts: Arc::clone(&self.posts),
            })),
            ("stream", false) => {
                let receiver = self
                    .stream_feed
                    .lock()
                    .unwrap()
                    .take()
                    .ok_or_else(|| FsError::Other("stream already subscribed".to_owned()))?;
                Ok(Box::new(StreamFile { receiver }))
            }
            ("liar", false) => Ok(Box::new(EndlessFile)),
            ("dead", _) => Err(FsError::Unreachable("room offline".to_owned())),
            _ => Err(FsError::NotFound),
        }
    }

    fn metadata(&self, path: &NormalizedPath) -> FsResult<Metadata> {
        match path.as_str() {
            "." => Ok(Metadata::new(FileType::Directory, 2, 0o555)),
            name => Self::file_metadata(name),
        }
    }

    fn read_dir(&self, path: &NormalizedPath) -> FsResult<Vec<DirEntry>> {
        if path.as_str() != "." {
            return Err(FsError::NotDirectory);
        }
        Ok(["latest", "post", "stream"]
            .into_iter()
            .map(|name| DirEntry::new(name, Self::file_metadata(name).unwrap()))
            .collect())
    }
}

struct BytesFile {
    bytes: Vec<u8>,
    offset: usize,
}

impl BytesFile {
    fn new(bytes: Vec<u8>) -> Self {
        Self { bytes, offset: 0 }
    }
}

impl File for BytesFile {
    fn read(&mut self, buf: &mut [u8]) -> FsResult<usize> {
        let remaining = &self.bytes[self.offset.min(self.bytes.len())..];
        let len = remaining.len().min(buf.len());
        buf[..len].copy_from_slice(&remaining[..len]);
        self.offset += len;
        Ok(len)
    }

    fn metadata(&self) -> FsResult<Metadata> {
        Ok(Metadata::new(
            FileType::File,
            self.bytes.len() as u64,
            0o444,
        ))
    }
}

/// Never EOFs and never blocks: every read fills the whole buffer.
struct EndlessFile;

impl File for EndlessFile {
    fn read(&mut self, buf: &mut [u8]) -> FsResult<usize> {
        buf.fill(b'x');
        Ok(buf.len())
    }

    fn metadata(&self) -> FsResult<Metadata> {
        Ok(Metadata::new(FileType::File, 4, 0o444))
    }
}

struct PostFile {
    posts: Arc<Mutex<Vec<Vec<u8>>>>,
}

impl File for PostFile {
    fn read(&mut self, _buf: &mut [u8]) -> FsResult<usize> {
        Err(FsError::NotSupported)
    }

    fn write(&mut self, buf: &[u8]) -> FsResult<usize> {
        self.posts.lock().unwrap().push(buf.to_vec());
        Ok(buf.len())
    }

    fn metadata(&self) -> FsResult<Metadata> {
        Ok(Metadata::new(FileType::File, 0, 0o222))
    }
}

/// Never-EOF until the feeding sender drops: each `read` blocks for the next
/// published line, mirroring an AppFS `stream` subscription.
struct StreamFile {
    receiver: mpsc::Receiver<Vec<u8>>,
}

impl File for StreamFile {
    fn read(&mut self, buf: &mut [u8]) -> FsResult<usize> {
        match self.receiver.recv() {
            Ok(bytes) => {
                let len = bytes.len().min(buf.len());
                buf[..len].copy_from_slice(&bytes[..len]);
                Ok(len)
            }
            Err(_) => Ok(0),
        }
    }

    fn metadata(&self) -> FsResult<Metadata> {
        Ok(Metadata::new(FileType::File, 0, 0o444))
    }
}

/// One origin composing a static site dir and the device room, the shape the
/// chat demo binds (`--bind chat=WEBDIR --bind chat=iroh://ROOM`).
fn composed_origin(device: DeviceFs) -> Arc<dyn FileSystem> {
    let mut namespace = Namespace::new();
    namespace
        .bind(site_fs("<h1>chat</h1>"), ".", ".", bind_last())
        .unwrap();
    namespace
        .bind(Arc::new(device), ".", ".", bind_last())
        .unwrap();
    Arc::new(namespace)
}

// ---------------------------------------------------------------------------
// Binding grammar
// ---------------------------------------------------------------------------

#[test]
fn bind_parse_qualifies_bare_names_and_detects_tickets() {
    let dir = WebBind::parse("chat=/srv/chat-web").unwrap();
    assert_eq!(dir.host, host("chat.localhost"));
    assert_eq!(
        dir.source,
        WebBindSource::Dir(PathBuf::from("/srv/chat-web"))
    );

    // A source spelled like a catalog name resolves at startup: catalog entry
    // first, relative directory fallback.
    let qualified = WebBind::parse("chat.localhost=web").unwrap();
    assert_eq!(qualified.host, host("chat.localhost"));
    assert_eq!(qualified.source, WebBindSource::NameOrDir("web".to_owned()));

    // `./web` forces the directory reading of the same spelling.
    let forced_dir = WebBind::parse("chat=./web").unwrap();
    assert_eq!(
        forced_dir.source,
        WebBindSource::Dir(PathBuf::from("./web"))
    );

    let peer_hex = NodeIdentity::from_secret_bytes([7u8; 32])
        .peer_id()
        .to_hex();
    let ticket = format!("room=iroh://{peer_hex}?addr=127.0.0.1:5000");
    let mesh = WebBind::parse(&ticket).unwrap();
    assert_eq!(mesh.host, host("room.localhost"));
    assert_eq!(
        mesh.source,
        WebBindSource::Mesh(format!("iroh://{peer_hex}?addr=127.0.0.1:5000"))
    );
}

#[test]
fn bind_parse_rejects_malformed_specs() {
    for raw in [
        "no-equals",
        "chat=",
        "=dir",
        "localhost=/srv",       // bare localhost is the index, not a name
        "127.0.0.1=/srv",       // IP literals never name an origin
        "room=iroh://tooshort", // ticket validated at parse time
    ] {
        assert!(WebBind::parse(raw).is_err(), "{raw:?} must be rejected");
    }
}

/// `--bind NAME=CATALOG_NAME`: a catalog entry wins (the gateway dials the
/// registered live serve and the origin reads the served bytes); the same
/// spelling with no entry falls back to a relative directory.
#[test]
fn bind_name_sources_resolve_through_the_catalog_with_dir_fallback() {
    use crate::catalog::{CatalogEntry, write_entry};
    use crate::mesh::mounts::test_support::serve_native;

    fn read_through(fs: &Arc<dyn FileSystem>, path: &str) -> Vec<u8> {
        let mut file = fs
            .open(&NormalizedPath::new(path).unwrap(), OpenOptions::read())
            .unwrap();
        let mut bytes = Vec::new();
        let mut chunk = [0_u8; 4096];
        loop {
            let read = file.read(&mut chunk).unwrap();
            if read == 0 {
                break;
            }
            bytes.extend_from_slice(&chunk[..read]);
        }
        bytes
    }

    let served = MemFs::new();
    served.write_file("index.html", b"served room").unwrap();
    let (server, url) = serve_native(Arc::new(served) as Arc<dyn FileSystem>, 11);
    let catalog = temp_root("name-catalog");
    write_entry(
        &catalog,
        &CatalogEntry {
            name: "room".to_owned(),
            description: None,
            tags: Vec::new(),
            address: url,
        },
        false,
    )
    .unwrap();

    let binds = [WebBind {
        host: host("chat.localhost"),
        source: WebBindSource::NameOrDir("room".to_owned()),
    }];
    let door = WebDoor::build_in(Some(&catalog), &binds).unwrap();
    let origin = door.resolve(&host("chat.localhost")).unwrap();
    assert_eq!(read_through(&origin, "index.html"), b"served room");
    drop(door);
    drop(server);

    // No catalog entry: the same spelling is a relative directory.
    let fallback_dir = format!("wanix-webdoor-fallback-{}", std::process::id());
    let _ = fs::remove_dir_all(&fallback_dir);
    fs::create_dir_all(&fallback_dir).unwrap();
    fs::write(format!("{fallback_dir}/index.html"), b"plain dir").unwrap();
    let binds = [WebBind {
        host: host("chat.localhost"),
        source: WebBindSource::NameOrDir(fallback_dir.clone()),
    }];
    let door = WebDoor::build_in(Some(&catalog), &binds).unwrap();
    let origin = door.resolve(&host("chat.localhost")).unwrap();
    assert_eq!(read_through(&origin, "index.html"), b"plain dir");

    // Neither an entry nor a directory: the error names both readings.
    let binds = [WebBind {
        host: host("chat.localhost"),
        source: WebBindSource::NameOrDir("absent".to_owned()),
    }];
    let Err(error) = WebDoor::build_in(Some(&catalog), &binds) else {
        panic!("a bind that is neither entry nor directory must fail");
    };
    assert!(
        error
            .to_string()
            .contains("neither a catalog entry nor an openable directory"),
        "{error}"
    );

    let _ = fs::remove_dir_all(&fallback_dir);
    let _ = fs::remove_dir_all(&catalog);
}

// ---------------------------------------------------------------------------
// Trust boundary
// ---------------------------------------------------------------------------

#[test]
fn serve_refuses_bind_on_non_loopback_http_door() {
    let root = temp_root("refusal");
    let listener = TcpListener::bind("0.0.0.0:0").unwrap();
    let command = ServeCommand {
        root_path: root.clone(),
        addr: listener.local_addr().unwrap().to_string(),
        bundle: None,
        wanix_services: false,
        once: true,
        p9_addr: None,
        peer: None,
        grants: Vec::new(),
        binds: vec![WebBind {
            host: host("chat.localhost"),
            source: WebBindSource::Dir(root),
        }],
    };
    let mut stderr = Vec::new();
    let error = run_serve_with_listener(command, listener, &mut stderr).unwrap_err();
    let message = error.to_string();
    assert!(message.contains("non-loopback"), "{message}");
    assert!(message.contains("--bind"), "{message}");
}

// ---------------------------------------------------------------------------
// Host routing, the bare-host index, and the unknown-name 404
// ---------------------------------------------------------------------------

#[test]
fn gateway_routes_names_serves_index_and_404s_unknown_names() {
    let mut door = WebDoor::empty();
    door.bind_origin(host("alpha.localhost"), site_fs("site alpha"));
    door.bind_origin(host("beta.localhost"), site_fs("site beta"));
    let addr = spawn_gateway(door, 4, "routing");

    let alpha = http(addr, "GET / HTTP/1.1\r\nHost: alpha.localhost\r\n\r\n");
    assert!(alpha.starts_with("HTTP/1.1 200 OK\r\n"), "{alpha}");
    assert!(alpha.ends_with("site alpha"), "{alpha}");

    let beta = http(addr, "GET / HTTP/1.1\r\nHost: beta.localhost:7654\r\n\r\n");
    assert!(beta.ends_with("site beta"), "{beta}");

    let unknown = http(addr, "GET / HTTP/1.1\r\nHost: gamma.localhost\r\n\r\n");
    assert!(unknown.starts_with("HTTP/1.1 404"), "{unknown}");
    assert!(unknown.contains("bound names are listed at"), "{unknown}");

    let index = http(addr, "GET / HTTP/1.1\r\nHost: localhost\r\n\r\n");
    assert!(index.starts_with("HTTP/1.1 200 OK\r\n"), "{index}");
    assert!(index.contains("alpha.localhost"), "{index}");
    assert!(index.contains("beta.localhost"), "{index}");
}

// ---------------------------------------------------------------------------
// GET/POST round trip beside a static site under one origin
// ---------------------------------------------------------------------------

#[test]
fn gateway_round_trips_device_files_beside_static_site_on_one_origin() {
    let (device, posts, _feed) = DeviceFs::new();
    let mut door = WebDoor::empty();
    door.bind_origin(host("chat.localhost"), composed_origin(device));
    let addr = spawn_gateway(door, 3, "roundtrip");

    // Same-origin static webapp: the composed namespace serves index.html.
    // Gateway responses carry no CORS grant — the namespace door must not be
    // readable by other web origins in the operator's browser.
    let page = http(addr, "GET / HTTP/1.1\r\nHost: chat.localhost\r\n\r\n");
    assert!(page.contains("Content-Type: text/html"), "{page}");
    assert!(page.ends_with("<h1>chat</h1>"), "{page}");
    assert!(!page.contains("Access-Control-Allow-Origin"), "{page}");

    // POST writes the body into the device file (the page's own Origin, as a
    // browser fetch stamps it, passes).
    let post = http(
        addr,
        "POST /post HTTP/1.1\r\nHost: chat.localhost\r\n\
         Origin: http://chat.localhost:8080\r\nContent-Length: 5\r\n\r\nhi yo",
    );
    assert!(post.starts_with("HTTP/1.1 200 OK\r\n"), "{post}");
    assert_eq!(posts.lock().unwrap().as_slice(), &[b"hi yo".to_vec()]);

    // GET a sized device file returns its whole body in one response.
    let latest = http(addr, "GET /latest HTTP/1.1\r\nHost: chat.localhost\r\n\r\n");
    assert!(latest.starts_with("HTTP/1.1 200 OK\r\n"), "{latest}");
    assert!(latest.contains("Content-Length: 11\r\n"), "{latest}");
    assert!(!latest.contains("Access-Control-Allow-Origin"), "{latest}");
    assert!(latest.ends_with("hello room\n"), "{latest}");
}

#[test]
fn gateway_refuses_cross_site_writes_and_caps_lying_sized_reads() {
    let (device, posts, _feed) = DeviceFs::new();
    let mut door = WebDoor::empty();
    door.bind_origin(host("chat.localhost"), composed_origin(device));
    let addr = spawn_gateway(door, 3, "boundary");

    // A drive-by write from another web page (browsers always stamp the
    // page's Origin onto a POST) is refused; nothing reaches the device.
    let drive_by = http(
        addr,
        "POST /post HTTP/1.1\r\nHost: chat.localhost\r\n\
         Origin: http://evil.example\r\nContent-Length: 3\r\n\r\npwn",
    );
    assert!(drive_by.starts_with("HTTP/1.1 403"), "{drive_by}");
    assert!(drive_by.contains("cross-site write refused"), "{drive_by}");
    assert!(posts.lock().unwrap().is_empty());

    // An opaque "null" Origin (sandboxed iframe, data: page) is refused too.
    let opaque = http(
        addr,
        "POST /post HTTP/1.1\r\nHost: chat.localhost\r\n\
         Origin: null\r\nContent-Length: 3\r\n\r\npwn",
    );
    assert!(opaque.starts_with("HTTP/1.1 403"), "{opaque}");

    // A sized GET buffers at most the declared (stat) size, even when the
    // file produces bytes forever — a misbehaving mesh origin cannot grow
    // host memory without bound by lying about its size.
    let liar = http(addr, "GET /liar HTTP/1.1\r\nHost: chat.localhost\r\n\r\n");
    assert!(liar.starts_with("HTTP/1.1 200 OK\r\n"), "{liar}");
    assert!(liar.contains("Content-Length: 4\r\n"), "{liar}");
    assert!(liar.ends_with("xxxx"), "{liar}");
}

// ---------------------------------------------------------------------------
// Never-EOF streaming: chunked transfer and SSE
// ---------------------------------------------------------------------------

fn streamed_response(addr: SocketAddr, request: &str, feed: mpsc::Sender<Vec<u8>>) -> String {
    let mut stream = TcpStream::connect(addr).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    stream.write_all(request.as_bytes()).unwrap();
    // Publish only after the request is in flight, proving the gateway holds
    // the connection open across the blocking device read. Dropping the
    // sender afterwards EOFs the stream so the response terminates.
    let publisher = thread::spawn(move || {
        thread::sleep(Duration::from_millis(200));
        feed.send(b"live-line\n".to_vec()).unwrap();
        drop(feed);
    });
    let started = Instant::now();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).unwrap();
    publisher.join().unwrap();
    assert!(
        started.elapsed() >= Duration::from_millis(200),
        "response ended before the publish: {:?}",
        started.elapsed()
    );
    String::from_utf8_lossy(&response).into_owned()
}

#[test]
fn gateway_streams_never_eof_device_file_as_chunked_transfer() {
    let (device, _posts, feed) = DeviceFs::new();
    let mut door = WebDoor::empty();
    door.bind_origin(host("chat.localhost"), composed_origin(device));
    let addr = spawn_gateway(door, 1, "chunked");

    let response = streamed_response(
        addr,
        "GET /stream HTTP/1.1\r\nHost: chat.localhost\r\n\r\n",
        feed,
    );
    assert!(response.starts_with("HTTP/1.1 200 OK\r\n"), "{response}");
    assert!(
        response.contains("Transfer-Encoding: chunked\r\n"),
        "{response}"
    );
    assert!(!response.contains("Content-Length"), "{response}");
    // a-byte hex chunk frame for the 10-byte line, then the terminal chunk.
    assert!(response.contains("a\r\nlive-line\n\r\n"), "{response}");
    assert!(response.ends_with("0\r\n\r\n"), "{response}");
}

#[test]
fn gateway_streams_sse_events_when_accept_asks_for_event_stream() {
    let (device, _posts, feed) = DeviceFs::new();
    let mut door = WebDoor::empty();
    door.bind_origin(host("chat.localhost"), composed_origin(device));
    let addr = spawn_gateway(door, 1, "sse");

    let response = streamed_response(
        addr,
        "GET /stream HTTP/1.1\r\nHost: chat.localhost\r\nAccept: text/event-stream\r\n\r\n",
        feed,
    );
    assert!(
        response.contains("Content-Type: text/event-stream"),
        "{response}"
    );
    assert!(response.contains("data: live-line\n\n"), "{response}");
    assert!(response.ends_with("0\r\n\r\n"), "{response}");
}

// ---------------------------------------------------------------------------
// Error mapping
// ---------------------------------------------------------------------------

#[test]
fn gateway_maps_filesystem_errors_to_http_statuses() {
    let (device, _posts, _feed) = DeviceFs::new();
    let mut door = WebDoor::empty();
    door.bind_origin(host("chat.localhost"), composed_origin(device));
    let addr = spawn_gateway(door, 4, "errors");

    // A dead provider is an outage: 503 with a retry hint, never 404.
    let dead = http(addr, "GET /dead HTTP/1.1\r\nHost: chat.localhost\r\n\r\n");
    assert!(dead.starts_with("HTTP/1.1 503"), "{dead}");
    assert!(dead.contains("Retry-After: 5\r\n"), "{dead}");
    assert!(dead.contains("room offline"), "{dead}");

    let missing = http(addr, "GET /nope HTTP/1.1\r\nHost: chat.localhost\r\n\r\n");
    assert!(missing.starts_with("HTTP/1.1 404"), "{missing}");

    // The device refuses writes on a read-only file: NotSupported -> 405.
    let readonly = http(
        addr,
        "POST /latest HTTP/1.1\r\nHost: chat.localhost\r\nContent-Length: 2\r\n\r\nhi",
    );
    assert!(readonly.starts_with("HTTP/1.1 405"), "{readonly}");

    let verb = http(
        addr,
        "DELETE /post HTTP/1.1\r\nHost: chat.localhost\r\n\r\n",
    );
    assert!(verb.starts_with("HTTP/1.1 405"), "{verb}");
}
