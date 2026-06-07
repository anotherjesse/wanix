//! End-to-end test: a [`RemoteFs`] learns a file's content hash over 9P.
//!
//! This is the control/data split crossing the wire. A `P9Server` serves a
//! filesystem whose `content_hash` hook returns a known hash for a large file
//! and `None` for a small one. The client, over loopback TCP, calls
//! `content_hash` and receives:
//!
//! - the exact [`ContentHash`] for the large file (carried as the `cas.hash`
//!   synthetic xattr — a genuine `Tread`, not appended `Rgetattr` bytes), so a
//!   CAS-aware caller can offload the bulk read to the blob plane; and
//! - `None` for the small file (the server replied `ENODATA`), so the caller
//!   falls back to a plain read.
//!
//! Without this round trip the most novel Slice-5 correction would be a pure
//! local-Rust API that never crosses the wire.

use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::thread;

use wanix_9p::P9Server;
use wanix_9p_client::RemoteFs;
use wanix_fs::{
    ContentHash, DirEntry, File, FileSystem, FileType, FsError, FsResult, Metadata, NormalizedPath,
    OpenOptions,
};

/// A filesystem that advertises a fixed content hash for `big`, none for `small`.
#[derive(Debug)]
struct HashFs {
    hash: ContentHash,
    big_len: u64,
}

impl FileSystem for HashFs {
    fn open(&self, path: &NormalizedPath, _options: OpenOptions) -> FsResult<Box<dyn File>> {
        match path.as_str() {
            "big" => Ok(Box::new(ZeroFile {
                remaining: self.big_len as usize,
            })),
            "small" => Ok(Box::new(ZeroFile { remaining: 4 })),
            _ => Err(FsError::NotFound),
        }
    }

    fn metadata(&self, path: &NormalizedPath) -> FsResult<Metadata> {
        match path.as_str() {
            "." => Ok(Metadata::new(FileType::Directory, 0, 0o755)),
            "big" => Ok(Metadata::new(FileType::File, self.big_len, 0o644)),
            "small" => Ok(Metadata::new(FileType::File, 4, 0o644)),
            _ => Err(FsError::NotFound),
        }
    }

    fn content_hash(&self, path: &NormalizedPath) -> FsResult<Option<ContentHash>> {
        Ok((path.as_str() == "big").then_some(self.hash))
    }

    fn read_dir(&self, path: &NormalizedPath) -> FsResult<Vec<DirEntry>> {
        if path.as_str() != "." {
            return Err(FsError::NotDirectory);
        }
        Ok(vec![
            DirEntry::new("big", Metadata::new(FileType::File, self.big_len, 0o644)),
            DirEntry::new("small", Metadata::new(FileType::File, 4, 0o644)),
        ])
    }
}

/// A read handle that yields a fixed number of zero bytes (the file body the
/// control plane would otherwise crawl, but which the client offloads).
struct ZeroFile {
    remaining: usize,
}

impl File for ZeroFile {
    fn read(&mut self, buf: &mut [u8]) -> FsResult<usize> {
        let len = self.remaining.min(buf.len());
        buf[..len].fill(0);
        self.remaining -= len;
        Ok(len)
    }

    fn metadata(&self) -> FsResult<Metadata> {
        Ok(Metadata::new(FileType::File, self.remaining as u64, 0o644))
    }
}

fn spawn_server(fs: Arc<HashFs>) -> TcpStream {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let read = stream.try_clone().unwrap();
        let mut server = P9Server::new(fs as Arc<dyn FileSystem>);
        let _ = server.serve_stream(read, stream);
    });
    TcpStream::connect(addr).unwrap()
}

fn path(value: &str) -> NormalizedPath {
    NormalizedPath::new(value).unwrap()
}

#[test]
fn content_hash_crosses_the_wire_for_a_large_file() {
    let hash = ContentHash::from_bytes([0x42; 32]);
    let fs = Arc::new(HashFs {
        hash,
        big_len: 1 << 20,
    });
    let client = spawn_server(fs);
    let remote = RemoteFs::connect(Box::new(client)).unwrap();

    // The large file's hash arrives over 9P, so a CAS-aware client offloads.
    let learned = remote.content_hash(&path("big")).unwrap();
    assert_eq!(learned, Some(hash));

    // The small file has no offloadable hash; the client falls back to a read.
    let none = remote.content_hash(&path("small")).unwrap();
    assert_eq!(none, None);
}

#[test]
fn content_hash_is_none_for_a_missing_path_via_walk_error() {
    let hash = ContentHash::from_bytes([0x42; 32]);
    let fs = Arc::new(HashFs { hash, big_len: 16 });
    let client = spawn_server(fs);
    let remote = RemoteFs::connect(Box::new(client)).unwrap();

    // A path that does not resolve at all is an error, not a silent None: the
    // walk fails before any xattr probe.
    assert!(remote.content_hash(&path("nope")).is_err());
}
