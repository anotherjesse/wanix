//! The host pump: the single reader of the guest→adapter channel.
//!
//! ADR 0010's "the host owns pumping" rule, made literal: one thread per
//! adapter owns the [`AppReceiver`] and drains the guest's output
//! continuously, whether or not a discrete operation is in flight. That one
//! property carries three contracts at once:
//!
//! - **Publishes deliver on arrival.** A `{"publish":{...}}` line emitted
//!   after a reply fans out to stream subscribers immediately, not at the
//!   next discrete op (publishes "may arrive between replies" per the
//!   protocol).
//! - **No mutual stall.** The guest's output channel always has a reader, so
//!   a guest parked writing a large publish burst into a bounded pipe always
//!   makes progress, which in turn keeps the guest reading its input — a
//!   large request line can never deadlock against an undrained backlog.
//! - **Protocol errors are terminal, not desynchronizing.** A guest line the
//!   host cannot honor (parse failure, undeclared-stream publish, a reply
//!   matching no in-flight request) fails the in-flight op with the specific
//!   error and latches the channel down: every later discrete op fails as
//!   [`FsError::Unreachable`] instead of misreading the abandoned reply, and
//!   the stream surface closes (the guest can never publish through this
//!   channel again).

use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;

use wanix_fs::{FsError, FsResult};

use crate::channel::AppReceiver;
use crate::protocol::{AppOk, AppReply, GuestLine};
use crate::service::channel_down;
use crate::streams::StreamTable;
use crate::tree::AppTree;

/// Worst-case re-check interval for a blocked reply wait. Routing always
/// notifies, so this only bounds a missed wakeup (the house condvar style).
const WAIT_RECHECK_INTERVAL: Duration = Duration::from_millis(50);

#[derive(Default)]
struct SlotInner {
    /// The id the single in-flight `transact` awaits (`None` between ops).
    waiting: Option<u64>,
    /// The routed outcome for the in-flight op.
    outcome: Option<FsResult<AppOk>>,
    /// Latched once the pump exits: every later op fails with this.
    down: Option<FsError>,
}

/// The reply mailbox between the pump and the one in-flight `transact`.
#[derive(Default)]
pub(crate) struct ReplySlot {
    inner: Mutex<SlotInner>,
    signal: Condvar,
}

impl ReplySlot {
    fn lock(&self) -> FsResult<MutexGuard<'_, SlotInner>> {
        self.inner
            .lock()
            .map_err(|_| FsError::Other("app reply slot lock poisoned".to_owned()))
    }

    /// Registers the in-flight request id, refusing on a downed channel.
    pub(crate) fn begin(&self, id: u64) -> FsResult<()> {
        let mut inner = self.lock()?;
        if let Some(down) = &inner.down {
            return Err(down.clone());
        }
        inner.waiting = Some(id);
        inner.outcome = None;
        Ok(())
    }

    /// Clears the in-flight registration after a failed send.
    pub(crate) fn abandon(&self) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.waiting = None;
            inner.outcome = None;
        }
    }

    /// Blocks until the pump routes the in-flight outcome or latches down.
    pub(crate) fn wait(&self) -> FsResult<AppOk> {
        let mut inner = self.lock()?;
        loop {
            if let Some(outcome) = inner.outcome.take() {
                inner.waiting = None;
                return outcome;
            }
            if let Some(down) = &inner.down {
                let down = down.clone();
                inner.waiting = None;
                return Err(down);
            }
            let (next, _timed_out) = self
                .signal
                .wait_timeout(inner, WAIT_RECHECK_INTERVAL)
                .map_err(|_| FsError::Other("app reply slot wait poisoned".to_owned()))?;
            inner = next;
        }
    }

    /// Routes one guest reply to the in-flight request.
    ///
    /// # Errors
    ///
    /// Returns [`FsError::Other`] when the reply matches no in-flight id —
    /// a protocol violation the pump treats as terminal.
    fn route(&self, reply: AppReply) -> FsResult<()> {
        let mut inner = self.lock()?;
        match inner.waiting {
            Some(id) if id == reply.id => {
                inner.outcome = Some(reply.into_result());
                drop(inner);
                self.signal.notify_all();
                Ok(())
            }
            Some(id) => Err(FsError::Other(format!(
                "app reply id {} does not match in-flight request {id}",
                reply.id
            ))),
            None => Err(FsError::Other(format!(
                "app reply id {} arrived with no request in flight",
                reply.id
            ))),
        }
    }

    /// Latches the channel permanently down: the in-flight op (if any) fails
    /// with `current`, every later op with `down`.
    fn latch_down(&self, current: FsError, down: FsError) {
        if let Ok(mut inner) = self.inner.lock() {
            if inner.waiting.is_some() && inner.outcome.is_none() {
                inner.outcome = Some(Err(current));
            }
            inner.down = Some(down);
        }
        self.signal.notify_all();
    }
}

/// Spawns the pump thread for one adapter. The pump holds only the receive
/// half, the reply slot, and the stream table — never the sender — so
/// dropping the service still closes a pipe-backed guest's stdin.
pub(crate) fn spawn(
    receiver: Box<dyn AppReceiver>,
    tree: AppTree,
    slot: Arc<ReplySlot>,
    streams: StreamTable,
) {
    std::thread::spawn(move || run(receiver, &tree, &slot, &streams));
}

fn run(
    mut receiver: Box<dyn AppReceiver>,
    tree: &AppTree,
    slot: &ReplySlot,
    streams: &StreamTable,
) {
    let reason = loop {
        let raw = match receiver.recv_line() {
            Ok(raw) => raw,
            Err(err) => break channel_down(&err),
        };
        let line = match GuestLine::parse(&raw) {
            Ok(line) => line,
            Err(err) => break err,
        };
        let handled = match line {
            GuestLine::Publish(publish) => streams.fan_out(tree, &publish),
            GuestLine::Reply(reply) => slot.route(reply),
        };
        if let Err(err) = handled {
            break err;
        }
    };
    let down = match &reason {
        FsError::Unreachable(_) => reason.clone(),
        other => FsError::Unreachable(format!("app channel down: {other}")),
    };
    slot.latch_down(reason, down);
    // The guest can never publish again through this pump: release blocked
    // stream readers with EOF (idempotent with any host-side exit watcher).
    streams.close_all();
}
