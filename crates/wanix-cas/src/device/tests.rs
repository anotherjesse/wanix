use std::sync::Arc;

use wanix_fs::{FileSystem, FsError, NormalizedPath, OpenOptions};

use super::CasDevice;
use crate::hash::hash_bytes;
use crate::store::{CasError, CasResult, ContentStore};

/// A trivial in-memory store so the device tests do not touch disk.
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

fn device() -> CasDevice {
    CasDevice::new(Arc::new(MemStore::default()))
}

fn path(raw: &str) -> NormalizedPath {
    NormalizedPath::new(raw).unwrap()
}

fn read_all(dev: &CasDevice, raw: &str) -> Vec<u8> {
    let mut file = dev.open(&path(raw), OpenOptions::read()).unwrap();
    let mut out = Vec::new();
    let mut buf = [0u8; 64];
    loop {
        let n = file.read(&mut buf).unwrap();
        if n == 0 {
            break;
        }
        out.extend_from_slice(&buf[..n]);
    }
    out
}

#[test]
fn ingest_then_read_hash_then_read_blob() {
    let dev = device();
    let payload = b"content addressed payload";

    // Write to #cas/ingest and close it.
    {
        let mut file = dev
            .open(&path("ingest"), OpenOptions::read_write())
            .unwrap();
        file.write(payload).unwrap();
    }

    // Reading #cas/ingest now returns the hex hash of the ingested blob.
    let hex = String::from_utf8(read_all(&dev, "ingest")).unwrap();
    assert_eq!(hex, hash_bytes(payload).to_hex());

    // The blob is readable at #cas/<hash> and matches the original bytes.
    assert_eq!(read_all(&dev, &hex), payload);
}

#[test]
fn have_reports_presence() {
    let dev = device();
    let hash = hash_bytes(b"present blob");
    let missing = format!("have/{}", hash.to_hex());
    assert_eq!(read_all(&dev, &missing), b"0\n");

    // Ingest it, then the same probe flips to present.
    {
        let mut file = dev
            .open(&path("ingest"), OpenOptions::read_write())
            .unwrap();
        file.write(b"present blob").unwrap();
    }
    assert_eq!(read_all(&dev, &missing), b"1\n");
}

#[test]
fn ingest_rejects_writes_past_the_blob_cap_without_committing() {
    use crate::MAX_BLOB_SIZE;

    let dev = device();
    let mut file = dev
        .open(&path("ingest"), OpenOptions::read_write())
        .unwrap();

    // Stream chunks until a write trips the cap. The buffer is bounded at write
    // time, so the host never holds more than MAX_BLOB_SIZE for this stream —
    // the rejection fires before the over-cap byte is allocated.
    let chunk = vec![0u8; 8 * 1024 * 1024];
    let mut written = 0usize;
    let mut rejected = false;
    // One more chunk than fits, so the loop must hit the cap rather than spin.
    let max_chunks = MAX_BLOB_SIZE / chunk.len() + 2;
    for _ in 0..max_chunks {
        match file.write(&chunk) {
            Ok(n) => written += n,
            Err(FsError::NotSupported) => {
                rejected = true;
                break;
            }
            Err(other) => panic!("unexpected error: {other:?}"),
        }
    }
    assert!(rejected, "ingest must reject the over-cap write");
    assert!(written <= MAX_BLOB_SIZE, "buffer stayed within the cap");

    // Closing the poisoned handle must not publish a truncated blob: a later
    // read of `ingest` returns the empty (never-published) hash slot.
    drop(file);
    assert!(read_all(&dev, "ingest").is_empty());
}

#[test]
fn blob_is_read_only() {
    let dev = device();
    let hash = hash_bytes(b"x");
    // `Box<dyn File>` is not `Debug`, so match the result rather than unwrap_err.
    match dev.open(&path(&hash.to_hex()), OpenOptions::read_write()) {
        // Writing a blob path is refused; the blob is also absent so a read is
        // NotFound — either way it is never writable.
        Err(FsError::PermissionDenied | FsError::NotFound) => {}
        Err(other) => panic!("unexpected error: {other:?}"),
        Ok(_) => panic!("blob path must not be writable"),
    }
}

#[test]
fn malformed_hash_is_rejected() {
    let dev = device();
    let err = dev.metadata(&path("not-a-valid-hash")).unwrap_err();
    assert!(matches!(err, FsError::InvalidPath(_)));
}

#[test]
fn root_lists_control_files_not_blobs() {
    let dev = device();
    {
        let mut file = dev
            .open(&path("ingest"), OpenOptions::read_write())
            .unwrap();
        file.write(b"some blob").unwrap();
    }
    let names: Vec<String> = dev
        .read_dir(&path("."))
        .unwrap()
        .iter()
        .map(|e| e.name().to_owned())
        .collect();
    assert!(names.contains(&"ingest".to_owned()));
    assert!(names.contains(&"have".to_owned()));
    // The blob keyspace is never enumerated.
    assert_eq!(names.len(), 2);
}
