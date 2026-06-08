//! Phase 2 proof: round-trip a whole `FileSystem` over the native wire.
//!
//! There is **no transport** here — the client [`NativeFs`] and the server
//! [`serve_one`] talk over an in-process bidirectional pipe pair, exactly the
//! plan's "fully unit-testable over an in-memory `Duplex`" surface. Each op opens
//! a fresh stream (a fresh pipe pair + a server thread running `serve_one`), so
//! the harness mirrors the real per-op / per-open-file stream discipline. The
//! backing filesystem is a `MemFs` for the structural ops and a small never-EOF
//! blocking device for the streaming proof.

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;

use wanix_fs::{
    DirEntry, File, FileSeekFrom, FileSystem, FsError, FsResult, MemFs, Metadata, MetadataLookup,
    MetadataTimes, NormalizedPath, OpenOptions,
};
use wanix_mesh_wire::{NativeFs, StreamFactory, serve_one};

/// A bidirectional in-memory stream: read from one pipe, write to the other.
struct PipeDuplex {
    reader: std::io::PipeReader,
    writer: std::io::PipeWriter,
}

impl Read for PipeDuplex {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.reader.read(buf)
    }
}

impl Write for PipeDuplex {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.writer.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.writer.flush()
    }
}

/// Builds a connected pair of [`PipeDuplex`]es (client end, server end).
fn pipe_pair() -> (PipeDuplex, PipeDuplex) {
    // client -> server pipe (c2s) and server -> client pipe (s2c).
    let (c2s_r, c2s_w) = std::io::pipe().expect("c2s pipe");
    let (s2c_r, s2c_w) = std::io::pipe().expect("s2c pipe");
    let client = PipeDuplex {
        reader: s2c_r,
        writer: c2s_w,
    };
    let server = PipeDuplex {
        reader: c2s_r,
        writer: s2c_w,
    };
    (client, server)
}

/// A [`StreamFactory`] that serves each new stream against a shared root by
/// spawning a `serve_one` thread per op. Threads are joined on drop.
struct LoopbackFactory {
    root: Arc<dyn FileSystem>,
    threads: Mutex<Vec<JoinHandle<()>>>,
}

impl LoopbackFactory {
    fn new(root: Arc<dyn FileSystem>) -> Self {
        Self {
            root,
            threads: Mutex::new(Vec::new()),
        }
    }
}

impl StreamFactory for LoopbackFactory {
    fn open_stream(&self) -> std::io::Result<Box<dyn wanix_mesh_wire::Duplex>> {
        let (client, server) = pipe_pair();
        let root = Arc::clone(&self.root);
        let handle = std::thread::spawn(move || {
            serve_one(&root, server, None);
        });
        // Reap any finished server threads so the vector does not grow unbounded
        // across thousands of ops; live ones (open files) stay until close.
        let mut threads = self.threads.lock().expect("threads lock");
        threads.retain(|t| !t.is_finished());
        threads.push(handle);
        Ok(Box::new(client))
    }
}

impl Drop for LoopbackFactory {
    fn drop(&mut self) {
        let handles = std::mem::take(&mut *self.threads.lock().expect("threads lock"));
        for handle in handles {
            let _ = handle.join();
        }
    }
}

/// Builds a `NativeFs` whose loopback server serves `root`.
fn native_over(root: Arc<dyn FileSystem>) -> NativeFs<LoopbackFactory> {
    NativeFs::new(LoopbackFactory::new(root))
}

/// Parses a path for the tests.
fn path(p: &str) -> NormalizedPath {
    NormalizedPath::new(p).expect("path")
}

#[test]
fn open_read_write_seek_round_trips() {
    let mem = Arc::new(MemFs::new());
    mem.open(
        &path("greeting"),
        OpenOptions {
            read: true,
            write: true,
            create: true,
            truncate: true,
        },
    )
    .unwrap()
    .write(b"hello world")
    .unwrap();
    let native = native_over(mem);

    // Read it back over the wire.
    let mut file = native.open(&path("greeting"), OpenOptions::read()).unwrap();
    assert!(file.is_seekable());
    let mut buf = [0_u8; 5];
    assert_eq!(file.read(&mut buf).unwrap(), 5);
    assert_eq!(&buf, b"hello");
    assert_eq!(file.tell().unwrap(), 5);

    // Seek over the wire (server-authoritative), then read the tail.
    assert_eq!(file.seek(FileSeekFrom::Start(6)).unwrap(), 6);
    let mut tail = Vec::new();
    let mut chunk = [0_u8; 16];
    loop {
        let n = file.read(&mut chunk).unwrap();
        if n == 0 {
            break;
        }
        tail.extend_from_slice(&chunk[..n]);
    }
    assert_eq!(tail, b"world");
    drop(file);

    // Write a new file over the wire, observe it server-side via a fresh open.
    let mut out = native
        .open(
            &path("note"),
            OpenOptions {
                read: true,
                write: true,
                create: true,
                truncate: true,
            },
        )
        .unwrap();
    assert_eq!(out.write(b"native bytes").unwrap(), 12);
    drop(out);
    let mut readback = native.open(&path("note"), OpenOptions::read()).unwrap();
    let mut got = String::new();
    let mut tmp = [0_u8; 32];
    let n = readback.read(&mut tmp).unwrap();
    got.push_str(std::str::from_utf8(&tmp[..n]).unwrap());
    assert_eq!(got, "native bytes");
}

#[test]
fn metadata_follow_versus_nofollow() {
    // `MemFs` does not resolve symlinks in metadata, so a follow/nofollow
    // *difference* needs a backing that honors the lookup mode. `LocalFs` does,
    // which proves the `follow_symlink` flag genuinely crosses the wire and
    // selects `metadata_with_lookup` rather than being silently dropped.
    use std::os::unix::fs::symlink;

    let root = unique_temp_dir();
    std::fs::write(root.join("target.txt"), b"abcdef").unwrap();
    symlink("target.txt", root.join("link")).unwrap();
    let local: Arc<dyn FileSystem> = Arc::new(wanix_fs::LocalFs::new(&root).unwrap());
    let native = native_over(local);

    // FollowSymlink reports the target (a regular file of length 6).
    let followed = native
        .metadata_with_lookup(&path("link"), MetadataLookup::FollowSymlink)
        .unwrap();
    assert_eq!(followed.file_type(), wanix_fs::FileType::File);
    assert_eq!(followed.len(), 6);

    // NoFollow reports the symlink itself.
    let raw = native
        .metadata_with_lookup(&path("link"), MetadataLookup::NoFollow)
        .unwrap();
    assert_eq!(raw.file_type(), wanix_fs::FileType::Symlink);

    // Plain metadata() follows by default and equals the followed lookup.
    assert_eq!(native.metadata(&path("link")).unwrap().len(), 6);

    std::fs::remove_dir_all(&root).ok();
}

/// Creates a unique empty directory under the system temp dir for `LocalFs`.
fn unique_temp_dir() -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nonce = COUNTER.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    let dir = std::env::temp_dir().join(format!("wanix-mesh-wire-{pid}-{nonce}"));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

#[test]
fn bulk_readdir_carries_full_metadata() {
    let mem = Arc::new(MemFs::new());
    mem.create_dir(&path("dir")).unwrap();
    mem.open(
        &path("dir/a"),
        OpenOptions {
            read: true,
            write: true,
            create: true,
            truncate: true,
        },
    )
    .unwrap()
    .write(b"aaa")
    .unwrap();
    mem.create_dir(&path("dir/sub")).unwrap();
    let native = native_over(mem);

    let mut entries = native.read_dir(&path("dir")).unwrap();
    entries.sort_by(|a, b| a.name().cmp(b.name()));
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].name(), "a");
    assert_eq!(entries[0].metadata().file_type(), wanix_fs::FileType::File);
    assert_eq!(entries[0].metadata().len(), 3);
    assert_eq!(entries[1].name(), "sub");
    assert_eq!(
        entries[1].metadata().file_type(),
        wanix_fs::FileType::Directory
    );
}

#[test]
fn create_dir_remove_and_rename() {
    let mem = Arc::new(MemFs::new());
    let native = native_over(mem.clone());

    native.create_dir(&path("work")).unwrap();
    assert_eq!(
        mem.metadata(&path("work")).unwrap().file_type(),
        wanix_fs::FileType::Directory
    );

    // Create a file, rename it, remove it, all over the wire.
    native
        .open(
            &path("work/tmp"),
            OpenOptions {
                read: true,
                write: true,
                create: true,
                truncate: true,
            },
        )
        .unwrap();
    native
        .rename(&path("work/tmp"), &path("work/kept"))
        .unwrap();
    assert!(mem.metadata(&path("work/tmp")).is_err());
    assert!(mem.metadata(&path("work/kept")).is_ok());

    native.remove_file(&path("work/kept")).unwrap();
    assert!(mem.metadata(&path("work/kept")).is_err());

    native.remove_dir(&path("work")).unwrap();
    assert!(mem.metadata(&path("work")).is_err());
}

#[test]
fn symlink_and_read_link() {
    let mem = Arc::new(MemFs::new());
    let native = native_over(mem.clone());

    native.symlink(b"some/target", &path("ln")).unwrap();
    assert_eq!(native.read_link(&path("ln")).unwrap(), b"some/target");
    // The same bytes are visible server-side.
    assert_eq!(mem.read_link(&path("ln")).unwrap(), b"some/target");
}

#[test]
fn set_permissions_and_set_times_surface_backing_result() {
    let mem = Arc::new(MemFs::new());
    mem.open(
        &path("f"),
        OpenOptions {
            read: true,
            write: true,
            create: true,
            truncate: true,
        },
    )
    .unwrap();
    let native = native_over(mem.clone());

    // MemFs supports set_times; assert the stamps cross the wire and stick.
    native.set_times(&path("f"), 111, 222).unwrap();
    let meta = mem.metadata(&path("f")).unwrap();
    assert_eq!(meta.accessed_time_ns(), 111);
    assert_eq!(meta.modified_time_ns(), 222);

    // set_permissions over the wire returns whatever the backing returns; assert
    // the call completes and the typed result matches the direct call.
    let direct = mem.set_permissions(&path("f"), 0o600);
    let over_wire = native.set_permissions(&path("f"), 0o600);
    assert_eq!(direct.is_ok(), over_wire.is_ok());
}

#[test]
fn content_hash_is_none_for_a_plain_backing() {
    let mem = Arc::new(MemFs::new());
    mem.open(
        &path("plain"),
        OpenOptions {
            read: true,
            write: true,
            create: true,
            truncate: true,
        },
    )
    .unwrap();
    let native = native_over(mem);
    // MemFs is not content-addressed, so the typed Option is None (not an error).
    assert_eq!(native.content_hash(&path("plain")).unwrap(), None);
}

#[test]
fn typed_invalid_path_error_surfaces_as_the_same_variant() {
    // A path that normalizes but escapes the root is rejected by NormalizedPath
    // server-side as InvalidPath; assert the *typed* variant and its String
    // survive the wire (the 9P errno table would lose both).
    let mem = Arc::new(MemFs::new());
    let native = native_over(mem);
    // NotFound is the canonical typed error for a missing file; assert it
    // crosses as exactly NotFound, not Other("remote errno ..").
    match native.metadata(&path("does/not/exist")) {
        Err(FsError::NotFound) => {}
        other => panic!("expected NotFound, got {other:?}"),
    }
    // remove_dir on a non-empty directory surfaces the precise typed variant.
    let mem = Arc::new(MemFs::new());
    mem.create_dir(&path("d")).unwrap();
    mem.open(
        &path("d/child"),
        OpenOptions {
            read: true,
            write: true,
            create: true,
            truncate: true,
        },
    )
    .unwrap();
    let native = native_over(mem);
    match native.remove_dir(&path("d")) {
        Err(FsError::NotEmpty) => {}
        other => panic!("expected NotEmpty, got {other:?}"),
    }

    // The headline typed-error proof: a server that returns
    // `InvalidPath("a/../b")` must surface on the client as the *same*
    // `InvalidPath` variant with the *same* String — the exact case the 9P
    // `FsError -> errno -> FsError` table collapses to `Other("remote errno 22")`.
    let native = native_over(Arc::new(InvalidPathFs));
    match native.metadata(&path("whatever")) {
        Err(FsError::InvalidPath(message)) => assert_eq!(message, "a/../b"),
        other => panic!("expected InvalidPath(\"a/../b\"), got {other:?}"),
    }
}

/// A filesystem whose `metadata` always returns a `String`-carrying typed error,
/// used to prove the message survives the wire intact.
struct InvalidPathFs;

impl FileSystem for InvalidPathFs {
    fn open(&self, _p: &NormalizedPath, _options: OpenOptions) -> FsResult<Box<dyn File>> {
        Err(FsError::NotSupported)
    }

    fn metadata(&self, _p: &NormalizedPath) -> FsResult<Metadata> {
        Err(FsError::InvalidPath("a/../b".to_owned()))
    }

    fn read_dir(&self, _p: &NormalizedPath) -> FsResult<Vec<DirEntry>> {
        Err(FsError::NotDirectory)
    }
}

/// A never-EOF blocking device: `read` parks until a producer pushes a chunk,
/// returns it, and reports `Ok(0)` (a clean device close) only once the producer
/// has signalled close *and* drained its queue.
#[derive(Default)]
struct DeviceState {
    queue: VecDeque<Vec<u8>>,
    closed: bool,
}

struct BlockingDevice {
    shared: Arc<(Mutex<DeviceState>, Condvar)>,
}

impl File for BlockingDevice {
    fn read(&mut self, buf: &mut [u8]) -> FsResult<usize> {
        let (lock, cvar) = &*self.shared;
        let mut state = lock.lock().expect("device lock");
        // Park until there is a chunk to deliver or the device has closed: this
        // is the never-EOF blocking read that, on the real wire, parks only this
        // one stream's blocking-pool thread.
        while state.queue.is_empty() && !state.closed {
            state = cvar.wait(state).expect("device wait");
        }
        match state.queue.pop_front() {
            Some(chunk) => {
                let n = chunk.len().min(buf.len());
                buf[..n].copy_from_slice(&chunk[..n]);
                Ok(n)
            }
            // Drained and closed: clean device close, surfaced as a 0-byte read.
            None => Ok(0),
        }
    }

    fn metadata(&self) -> FsResult<Metadata> {
        Ok(Metadata::new_with_links(
            wanix_fs::FileType::File,
            0,
            0o444,
            1,
            MetadataTimes::new(0, 0, 0),
        ))
    }
}

/// A one-file filesystem exposing a single never-EOF [`BlockingDevice`] at
/// `events`, with a producer handle to push chunks and close it.
struct DeviceFs {
    shared: Arc<(Mutex<DeviceState>, Condvar)>,
}

impl DeviceFs {
    fn new() -> Self {
        Self {
            shared: Arc::new((Mutex::new(DeviceState::default()), Condvar::new())),
        }
    }

    fn producer(&self) -> Arc<(Mutex<DeviceState>, Condvar)> {
        Arc::clone(&self.shared)
    }
}

impl FileSystem for DeviceFs {
    fn open(&self, p: &NormalizedPath, _options: OpenOptions) -> FsResult<Box<dyn File>> {
        if p.as_str() == "events" {
            Ok(Box::new(BlockingDevice {
                shared: Arc::clone(&self.shared),
            }))
        } else {
            Err(FsError::NotFound)
        }
    }

    fn metadata(&self, p: &NormalizedPath) -> FsResult<Metadata> {
        if p.as_str() == "events" {
            Ok(Metadata::new_with_links(
                wanix_fs::FileType::File,
                0,
                0o444,
                1,
                MetadataTimes::new(0, 0, 0),
            ))
        } else {
            Err(FsError::NotFound)
        }
    }

    fn read_dir(&self, _p: &NormalizedPath) -> FsResult<Vec<DirEntry>> {
        Err(FsError::NotDirectory)
    }
}

/// Pushes one chunk to a blocking device and wakes its reader.
fn push_chunk(shared: &Arc<(Mutex<DeviceState>, Condvar)>, bytes: &[u8]) {
    let (lock, cvar) = &**shared;
    lock.lock()
        .expect("device lock")
        .queue
        .push_back(bytes.to_vec());
    cvar.notify_all();
}

/// Marks a blocking device closed and wakes its reader.
fn close_device(shared: &Arc<(Mutex<DeviceState>, Condvar)>) {
    let (lock, cvar) = &**shared;
    lock.lock().expect("device lock").closed = true;
    cvar.notify_all();
}

#[test]
fn never_eof_device_streams_incremental_chunks_then_closes_cleanly() {
    let device_fs = DeviceFs::new();
    let producer = device_fs.producer();
    let native = native_over(Arc::new(device_fs));

    // The open-file stream is dedicated to this handle; its blocking read parks
    // only this stream. No dedicated-stream machinery (StreamingImportFs) exists.
    let mut events = native.open(&path("events"), OpenOptions::read()).unwrap();
    assert!(!events.is_seekable());
    assert!(events.seek(FileSeekFrom::Start(0)).is_err());

    // Producer thread delivers two chunks with a gap, then closes the device.
    let driver = std::thread::spawn(move || {
        push_chunk(&producer, b"first");
        std::thread::sleep(std::time::Duration::from_millis(20));
        push_chunk(&producer, b"second");
        std::thread::sleep(std::time::Duration::from_millis(20));
        close_device(&producer);
    });

    // The client reads chunks incrementally as they arrive, never hitting EOF
    // until the real device close arrives as a 0-byte read.
    let mut buf = [0_u8; 64];
    let n1 = events.read(&mut buf).unwrap();
    assert_eq!(&buf[..n1], b"first");
    let n2 = events.read(&mut buf).unwrap();
    assert_eq!(&buf[..n2], b"second");
    let n3 = events.read(&mut buf).unwrap();
    assert_eq!(n3, 0, "clean device close arrives as a 0-byte read");

    driver.join().expect("producer thread");
}
