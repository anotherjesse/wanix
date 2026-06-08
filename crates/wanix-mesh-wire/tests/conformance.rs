//! Phase 3 proof: the reusable conformance suite runs against any `FileSystem`.
//!
//! This drives `wanix_mesh_wire::conformance::run` against two implementations of
//! the *same* contract:
//!
//! 1. a `MemFs` **directly** — proving the suite is satisfiable by a conformant
//!    backing (so a later failure indicts the wire, not the suite);
//! 2. a `NativeFs` import of a `MemFs` over the in-process loopback harness —
//!    proving the native wire faithfully re-presents the whole contract.
//!
//! This is the early, transport-free rehearsal of Phase 5's differential test,
//! which will additionally fold in a 9P import of the same `MemFs`.

use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use wanix_fs::{FileSystem, MemFs};
use wanix_mesh_wire::conformance;
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

impl StreamFactory for LoopbackFactory {
    fn open_stream(&self) -> std::io::Result<Box<dyn wanix_mesh_wire::Duplex>> {
        let (client, server) = pipe_pair();
        let root = Arc::clone(&self.root);
        let handle = std::thread::spawn(move || {
            serve_one(&root, server, None);
        });
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

#[test]
fn conformance_suite_passes_against_a_direct_memfs() {
    let mem = MemFs::new();
    // The suite must be satisfiable by a conformant backing; running it directly
    // proves any later native-import failure is the wire's fault, not the suite's.
    conformance::run(&mem, "memfs");
}

#[test]
fn conformance_suite_passes_against_a_native_import() {
    let mem: Arc<dyn FileSystem> = Arc::new(MemFs::new());
    let native = NativeFs::new(LoopbackFactory {
        root: mem,
        threads: Mutex::new(Vec::new()),
    });
    // The full FileSystem contract must survive the native wire intact.
    conformance::run(&native, "native");
}
