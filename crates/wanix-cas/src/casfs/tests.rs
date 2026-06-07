use std::sync::Arc;

use wanix_fs::{CONTENT_HASH_OFFLOAD_THRESHOLD, FileSystem, MemFs, NormalizedPath, OpenOptions};

use super::CasFs;
use crate::hash::hash_bytes;
use crate::store::{CasError, CasResult, ContentStore};

/// In-memory store so the decorator tests do not touch disk.
#[derive(Default)]
struct MemStore {
    blobs: std::sync::Mutex<std::collections::BTreeMap<String, Vec<u8>>>,
}

impl ContentStore for MemStore {
    fn put(&self, bytes: &[u8]) -> CasResult<wanix_fs::ContentHash> {
        let hash = hash_bytes(bytes);
        self.blobs
            .lock()
            .unwrap()
            .insert(hash.to_hex(), bytes.to_vec());
        Ok(hash)
    }

    fn get(&self, hash: &wanix_fs::ContentHash) -> CasResult<Vec<u8>> {
        self.blobs
            .lock()
            .unwrap()
            .get(&hash.to_hex())
            .cloned()
            .ok_or(CasError::NotFound)
    }

    fn has(&self, hash: &wanix_fs::ContentHash) -> CasResult<bool> {
        Ok(self.blobs.lock().unwrap().contains_key(&hash.to_hex()))
    }
}

fn path(raw: &str) -> NormalizedPath {
    NormalizedPath::new(raw).unwrap()
}

fn large_payload() -> Vec<u8> {
    // Comfortably past the offload threshold with position-dependent bytes.
    (0..(CONTENT_HASH_OFFLOAD_THRESHOLD as usize + 4096))
        .map(|i| (i % 251) as u8)
        .collect()
}

fn fixture() -> (CasFs, Arc<MemFs>, Arc<MemStore>) {
    let inner = Arc::new(MemFs::new());
    let store = Arc::new(MemStore::default());
    let fs = CasFs::new(inner.clone(), store.clone());
    (fs, inner, store)
}

#[test]
fn small_file_has_no_offload_hash() {
    let (fs, inner, _store) = fixture();
    inner.write_file("small.txt", b"tiny").unwrap();
    assert_eq!(fs.content_hash(&path("small.txt")).unwrap(), None);
}

#[test]
fn large_file_exposes_hash_and_ingests_blob() {
    let (fs, inner, store) = fixture();
    let payload = large_payload();
    inner.write_file("big.bin", &payload).unwrap();

    let hash = fs.content_hash(&path("big.bin")).unwrap().unwrap();
    assert_eq!(hash, hash_bytes(&payload));
    // The blob is now ingested and fetchable by that content address.
    assert_eq!(store.get(&hash).unwrap(), payload);
}

#[test]
fn hash_is_suppressed_while_open_for_write() {
    let (fs, inner, _store) = fixture();
    inner.write_file("big.bin", large_payload()).unwrap();
    // A large file normally offloads...
    assert!(fs.content_hash(&path("big.bin")).unwrap().is_some());

    // ...but while a write handle is open, the hash is suppressed so a client
    // never fetches a blob for a file mid-write.
    let handle = fs
        .open(&path("big.bin"), OpenOptions::read_write())
        .unwrap();
    assert_eq!(fs.content_hash(&path("big.bin")).unwrap(), None);

    // After close the hash returns, reflecting the committed bytes.
    drop(handle);
    assert!(fs.content_hash(&path("big.bin")).unwrap().is_some());
}

#[test]
fn rewriting_updates_the_offloaded_hash() {
    let (fs, inner, store) = fixture();
    inner.write_file("big.bin", large_payload()).unwrap();
    let first = fs.content_hash(&path("big.bin")).unwrap().unwrap();

    // Overwrite with new large bytes through the CasFs write handle.
    let new_payload: Vec<u8> = large_payload().iter().map(|b| b ^ 0xff).collect();
    {
        let mut handle = fs
            .open(
                &path("big.bin"),
                OpenOptions {
                    read: false,
                    write: true,
                    create: false,
                    truncate: true,
                },
            )
            .unwrap();
        handle.write(&new_payload).unwrap();
    }

    let second = fs.content_hash(&path("big.bin")).unwrap().unwrap();
    assert_ne!(first, second);
    assert_eq!(second, hash_bytes(&new_payload));
    assert_eq!(store.get(&second).unwrap(), new_payload);
}

#[test]
fn reads_pass_through_unchanged() {
    let (fs, inner, _store) = fixture();
    inner.write_file("doc.txt", b"hello").unwrap();
    let mut handle = fs.open(&path("doc.txt"), OpenOptions::read()).unwrap();
    let mut buf = [0u8; 16];
    let n = handle.read(&mut buf).unwrap();
    assert_eq!(&buf[..n], b"hello");
}
