//! [`StreamTable`]: the host-owned stream surface — subscriber registry,
//! publish fan-out, `who` presence, and lifecycle teardown.
//!
//! Stream files never touch the guest: each open registers a bounded, lossy
//! [`LineBuffer`] here, the pump fans every guest publish into the matching
//! buffers, and `close_all` releases blocked readers with EOF once the guest
//! can never publish again. Fan-out touches only these adapter-owned buffers,
//! never another filesystem, so no lock-ordering hazard crosses this module.

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex, MutexGuard};

use wanix_fs::{FsError, FsResult};

use crate::protocol::{AppPublish, decode_data};
use crate::tree::AppTree;
use wanix_fs::LineBuffer;

struct Subscriber {
    stream: String,
    principal: String,
    buffer: Arc<LineBuffer>,
}

#[derive(Default)]
struct Subscribers {
    next_id: u64,
    table: HashMap<u64, Subscriber>,
    /// Set by [`StreamTable::close_all`] once the guest can never publish
    /// again: every current buffer is closed (blocked readers observe EOF)
    /// and every later subscription starts closed, so no stream read can
    /// hang on an app that is gone.
    closed: bool,
}

/// The shared subscriber registry behind every stream file and `who` read.
/// Clone-cheap: every clone shares one registry.
#[derive(Clone, Default)]
pub(crate) struct StreamTable {
    inner: Arc<Mutex<Subscribers>>,
}

impl StreamTable {
    fn lock(&self) -> FsResult<MutexGuard<'_, Subscribers>> {
        self.inner
            .lock()
            .map_err(|_| FsError::Other("app subscriber registry lock poisoned".to_owned()))
    }

    /// Registers one stream subscription and returns its id and buffer.
    pub(crate) fn subscribe(
        &self,
        stream: &str,
        principal: &str,
    ) -> FsResult<(u64, Arc<LineBuffer>)> {
        let mut subscribers = self.lock()?;
        let id = subscribers.next_id;
        subscribers.next_id += 1;
        // Bound each subscription to one maximal publish (`MAX_LINE_LEN`), so
        // a slow reader holds at most one full line's worth of backlog.
        let buffer = Arc::new(LineBuffer::bounded(crate::protocol::MAX_LINE_LEN));
        if subscribers.closed {
            // The guest is gone: the subscription still registers (so the
            // open succeeds and `who` stays truthful) but reads see EOF
            // immediately instead of parking on a stream that can never
            // receive another publish.
            buffer.close();
        }
        subscribers.table.insert(
            id,
            Subscriber {
                stream: stream.to_owned(),
                principal: principal.to_owned(),
                buffer: Arc::clone(&buffer),
            },
        );
        Ok((id, buffer))
    }

    /// Removes one stream subscription (called when its open file drops).
    pub(crate) fn unsubscribe(&self, id: u64) {
        if let Ok(mut subscribers) = self.inner.lock() {
            subscribers.table.remove(&id);
        }
    }

    /// Appends one guest publish to every subscriber buffer of its stream.
    ///
    /// # Errors
    ///
    /// Returns [`FsError::Other`] for a publish naming an undeclared stream
    /// or carrying invalid base64 — a guest protocol violation the pump
    /// treats as terminal.
    pub(crate) fn fan_out(&self, tree: &AppTree, publish: &AppPublish) -> FsResult<()> {
        if !tree.is_stream(&publish.stream) {
            return Err(FsError::Other(format!(
                "app publish names undeclared stream {:?}",
                publish.stream
            )));
        }
        let bytes = decode_data(&publish.data)?;
        let subscribers = self.lock()?;
        for subscriber in subscribers.table.values() {
            if subscriber.stream == publish.stream {
                subscriber.buffer.push(&bytes);
            }
        }
        Ok(())
    }

    /// Renders the `who` presence snapshot: the unique principals currently
    /// holding open stream subscriptions, sorted, one per line.
    pub(crate) fn who_snapshot(&self) -> FsResult<Vec<u8>> {
        let subscribers = self.lock()?;
        let principals: BTreeSet<&str> = subscribers
            .table
            .values()
            .map(|subscriber| subscriber.principal.as_str())
            .collect();
        let mut bytes = Vec::new();
        for principal in principals {
            bytes.extend_from_slice(principal.as_bytes());
            bytes.push(b'\n');
        }
        Ok(bytes)
    }

    /// Closes every current subscription buffer and marks the stream surface
    /// permanently down. Idempotent.
    pub(crate) fn close_all(&self) {
        if let Ok(mut subscribers) = self.inner.lock() {
            subscribers.closed = true;
            for subscriber in subscribers.table.values() {
                subscriber.buffer.close();
            }
        }
    }
}
