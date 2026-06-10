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
//! - **Op-fatal vs channel-fatal.** A complete, newline-framed line the host
//!   cannot parse (malformed JSON, or a line over the size ceiling) does not
//!   desynchronize the channel — framing is line-based — so it is *op-fatal*:
//!   it is taken as the failed reply to the in-flight op, which fails with
//!   the specific error while the channel stays up. Only errors that
//!   genuinely break the channel are *channel-fatal* and latch it down: a
//!   broken/EOF'd receive half, an unparseable line with no op in flight to
//!   attribute it to, and post-framing protocol violations (an undeclared-
//!   stream publish, a reply matching no in-flight id, a second hello). Once
//!   latched, every later discrete op fails as [`FsError::Unreachable`] and
//!   the stream surface closes (the guest can never publish through this
//!   channel again). A reply that never arrives is bounded separately by the
//!   per-request deadline on [`ReplySlot::wait`], whose expiry also latches
//!   down — an unresponsive guest is a dead guest.

use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use wanix_fs::{FsError, FsResult};

use crate::channel::AppReceiver;
use crate::protocol::{AppOk, AppOp, AppReply, GuestLine};
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

    /// Blocks until the pump routes the in-flight outcome, the channel
    /// latches down, or `deadline` expires.
    ///
    /// Deadline expiry latches the channel down: a guest that did not answer
    /// within the (generous) per-request deadline is unresponsive, and later
    /// ops must fail honestly as [`FsError::Unreachable`] instead of queueing
    /// behind a wedged actor.
    pub(crate) fn wait(&self, op: AppOp, deadline: Duration) -> FsResult<AppOk> {
        let start = Instant::now();
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
            let elapsed = start.elapsed();
            if elapsed >= deadline {
                let down = FsError::Unreachable(format!(
                    "app guest did not reply to {} within {deadline:?}",
                    op.name()
                ));
                inner.waiting = None;
                inner.down = Some(down.clone());
                drop(inner);
                self.signal.notify_all();
                return Err(down);
            }
            let (next, _timed_out) = self
                .signal
                .wait_timeout(inner, WAIT_RECHECK_INTERVAL.min(deadline - elapsed))
                .map_err(|_| FsError::Other("app reply slot wait poisoned".to_owned()))?;
            inner = next;
        }
    }

    /// Whether the channel has latched down (used by the service to tear the
    /// stream surface down after a deadline expiry observed in `wait`).
    pub(crate) fn is_down(&self) -> bool {
        self.inner.lock().is_ok_and(|inner| inner.down.is_some())
    }

    /// Fails the in-flight op with `err` without touching the channel state.
    ///
    /// Returns `false` when no op is awaiting an outcome — the caller then
    /// has nothing to attribute the error to and must treat it as terminal.
    fn fail_in_flight(&self, err: FsError) -> bool {
        let Ok(mut inner) = self.inner.lock() else {
            return false;
        };
        if inner.waiting.is_none() || inner.outcome.is_some() {
            return false;
        }
        inner.outcome = Some(Err(err));
        drop(inner);
        self.signal.notify_all();
        true
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
            // Op-fatal: the line is complete (newline-framed), so the channel
            // is still in sync; an oversized or malformed line is taken as
            // the failed reply to the in-flight op. With no op in flight
            // there is nothing to attribute the garbage to — terminal.
            Err(err) => {
                if slot.fail_in_flight(err.clone()) {
                    continue;
                }
                break err;
            }
        };
        let handled = match line {
            GuestLine::Hello(_) => Err(FsError::Other(
                "app guest sent a hello after the handshake".to_owned(),
            )),
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
