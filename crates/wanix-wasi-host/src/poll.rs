//! WASI Preview 1 `poll_oneoff` — a deliberate, minimal, honest subset.
//!
//! Supported: `eventtype::fd_read` subscriptions only. The call blocks (with
//! the same bounded-backoff parking as a blocking read, see [`crate::wait`])
//! until at least one subscribed fd reports read readiness or a readiness
//! error, then returns one event per decided fd. A readiness error is
//! reported in that event's `errno` field, per Preview 1.
//!
//! Everything else — clock subscriptions, `fd_write` subscriptions, unknown
//! tags — deliberately returns `ERRNO_NOSYS`: this linker stays a
//! command-style WASI subset and refuses rather than fakes what it does not
//! implement.

use wanix_wasi::{Errno, WasiCtx, WasiFd};
use wasmtime::{Caller, Linker, Result};

use super::mem::{memory, read_bytes, write_bytes, write_u32};
use super::wait::Backoff;
use super::{ERRNO_INVAL, ERRNO_NOSYS, ERRNO_SUCCESS, WasiHost};

const SUBSCRIPTION_SIZE: usize = 48;
const EVENT_SIZE: usize = 32;
const SUBSCRIPTION_TAG_OFFSET: usize = 8;
const SUBSCRIPTION_FD_OFFSET: usize = 16;
const EVENTTYPE_FD_READ: u8 = 1;

/// A parsed `fd_read` subscription (the only kind this linker accepts).
struct FdReadSubscription {
    userdata: u64,
    fd: WasiFd,
}

/// A decided poll event: the subscription's userdata plus its errno.
struct FdReadEvent {
    userdata: u64,
    errno: u16,
}

pub(super) fn register<S: WasiHost + 'static>(linker: &mut Linker<S>) -> Result<()> {
    linker.func_wrap(
        super::MODULE,
        "poll_oneoff",
        |mut caller: Caller<'_, S>,
         in_ptr: i32,
         out_ptr: i32,
         nsubscriptions: i32,
         nevents_ptr: i32|
         -> Result<i32> {
            if nsubscriptions <= 0 {
                return Ok(ERRNO_INVAL);
            }
            let mem = memory(&mut caller)?;
            let raw = read_bytes(
                &mem,
                &mut caller,
                in_ptr,
                nsubscriptions as usize * SUBSCRIPTION_SIZE,
            )?;
            let subscriptions = match parse_subscriptions(&raw) {
                Ok(subscriptions) => subscriptions,
                Err(errno) => return Ok(errno),
            };
            let events = match wait_fd_read_events(caller.data_mut().wasi(), &subscriptions) {
                PollOutcome::Decided(events) => events,
                // The kill seam: a cancelled (killed) task's poll park returns
                // EINTR for the whole call instead of waiting for readiness
                // that may never come; the wasm epoch interrupt then traps the
                // guest on its next instruction.
                PollOutcome::Cancelled => return Ok(Errno::Intr.preview1_result()),
            };
            let mut encoded = Vec::with_capacity(events.len() * EVENT_SIZE);
            for event in &events {
                encoded.extend_from_slice(&encode_event(event));
            }
            write_bytes(&mem, &mut caller, out_ptr, &encoded)?;
            write_u32(&mem, &mut caller, nevents_ptr, events.len() as u32)?;
            Ok(ERRNO_SUCCESS)
        },
    )?;
    Ok(())
}

/// Decodes raw Preview 1 subscription records, refusing anything outside the
/// supported subset with a whole-call `ERRNO_NOSYS`.
fn parse_subscriptions(raw: &[u8]) -> std::result::Result<Vec<FdReadSubscription>, i32> {
    let mut subscriptions = Vec::with_capacity(raw.len() / SUBSCRIPTION_SIZE);
    for record in raw.chunks_exact(SUBSCRIPTION_SIZE) {
        if record[SUBSCRIPTION_TAG_OFFSET] != EVENTTYPE_FD_READ {
            return Err(ERRNO_NOSYS);
        }
        let userdata = u64::from_le_bytes(record[0..8].try_into().expect("8 bytes"));
        let fd = u32::from_le_bytes(
            record[SUBSCRIPTION_FD_OFFSET..SUBSCRIPTION_FD_OFFSET + 4]
                .try_into()
                .expect("4 bytes"),
        );
        subscriptions.push(FdReadSubscription {
            userdata,
            fd: WasiFd::new(fd),
        });
    }
    Ok(subscriptions)
}

/// What the blocking poll park decided.
enum PollOutcome {
    /// At least one subscribed fd is decided: readiness as errno 0, a
    /// readiness failure as its errno.
    Decided(Vec<FdReadEvent>),
    /// The task was cancelled (killed) while parked; the call reports EINTR.
    Cancelled,
}

/// Blocks until at least one subscribed fd is decided or the task is
/// cancelled, checking the kill seam ([`WasiCtx::is_cancelled`]) on every
/// wake so a killed task parked here returns within one park interval.
fn wait_fd_read_events(ctx: &WasiCtx, subscriptions: &[FdReadSubscription]) -> PollOutcome {
    let mut backoff = Backoff::new();
    loop {
        let events = decided_events(ctx, subscriptions);
        if !events.is_empty() {
            return PollOutcome::Decided(events);
        }
        if ctx.is_cancelled() {
            return PollOutcome::Cancelled;
        }
        backoff.park();
    }
}

fn decided_events(ctx: &WasiCtx, subscriptions: &[FdReadSubscription]) -> Vec<FdReadEvent> {
    subscriptions
        .iter()
        .filter_map(|subscription| match ctx.fd_read_ready(subscription.fd) {
            Ok(true) => Some(FdReadEvent {
                userdata: subscription.userdata,
                errno: 0,
            }),
            Ok(false) => None,
            Err(errno) => Some(FdReadEvent {
                userdata: subscription.userdata,
                errno: errno.preview1_code(),
            }),
        })
        .collect()
}

fn encode_event(event: &FdReadEvent) -> [u8; EVENT_SIZE] {
    let mut out = [0u8; EVENT_SIZE];
    out[0..8].copy_from_slice(&event.userdata.to_le_bytes());
    out[8..10].copy_from_slice(&event.errno.to_le_bytes());
    out[10] = EVENTTYPE_FD_READ;
    out
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use wanix_wasi::WasiFd;

    use super::super::wait::tests::ctx_with_scripted_stdin;
    use super::{
        ERRNO_NOSYS, EVENTTYPE_FD_READ, FdReadSubscription, PollOutcome, SUBSCRIPTION_SIZE,
        parse_subscriptions, wait_fd_read_events,
    };

    fn decided(outcome: PollOutcome) -> Vec<super::FdReadEvent> {
        match outcome {
            PollOutcome::Decided(events) => events,
            PollOutcome::Cancelled => panic!("poll was cancelled, expected decided events"),
        }
    }

    fn raw_subscription(userdata: u64, tag: u8, fd: u32) -> [u8; SUBSCRIPTION_SIZE] {
        let mut record = [0u8; SUBSCRIPTION_SIZE];
        record[0..8].copy_from_slice(&userdata.to_le_bytes());
        record[8] = tag;
        record[16..20].copy_from_slice(&fd.to_le_bytes());
        record
    }

    #[test]
    fn parse_accepts_fd_read_subscriptions() {
        let raw = raw_subscription(7, EVENTTYPE_FD_READ, 0);
        let parsed = parse_subscriptions(&raw).expect("fd_read parses");
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].userdata, 7);
        assert_eq!(parsed[0].fd, WasiFd::STDIN);
    }

    #[test]
    fn parse_refuses_clock_and_fd_write_with_nosys() {
        for unsupported_tag in [0u8, 2u8, 9u8] {
            let raw = raw_subscription(1, unsupported_tag, 0);
            assert!(
                matches!(parse_subscriptions(&raw), Err(errno) if errno == ERRNO_NOSYS),
                "tag {unsupported_tag} must be refused with NOSYS"
            );
        }
    }

    #[test]
    fn poll_blocks_until_stdin_becomes_ready() {
        let (ctx, stdin) = ctx_with_scripted_stdin();
        let feeder = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            stdin.feed(b"x");
        });
        let subscriptions = [FdReadSubscription {
            userdata: 42,
            fd: WasiFd::STDIN,
        }];
        let events = decided(wait_fd_read_events(&ctx, &subscriptions));
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].userdata, 42);
        assert_eq!(events[0].errno, 0);
        feeder.join().expect("feeder thread");
    }

    #[test]
    fn poll_parked_on_a_quiet_fd_returns_cancelled_when_the_task_is_killed() {
        // The kill seam (deadline-bounded): no bytes ever arrive on the quiet
        // stdin, so only the cancel probe can end this park. The probe flips
        // after 30ms; the park must return Cancelled well within the deadline
        // instead of waiting forever for readiness.
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering};

        use wanix_wasi::{CancelToken, WasiConfig, WasiCtx};

        use super::super::wait::tests::ScriptedStdin;

        let killed = Arc::new(AtomicBool::new(false));
        let probe = Arc::clone(&killed);
        let stdin = ScriptedStdin::default(); // quiet: readiness stays false
        let ctx = WasiCtx::new(
            WasiConfig::new(Default::default())
                .with_stdin(Box::new(stdin), "stdin")
                .with_cancel_token(CancelToken::new(move || probe.load(Ordering::Relaxed))),
        );
        let killer = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            killed.store(true, Ordering::Relaxed);
        });
        let started = std::time::Instant::now();
        let outcome = wait_fd_read_events(
            &ctx,
            &[FdReadSubscription {
                userdata: 1,
                fd: WasiFd::STDIN,
            }],
        );
        assert!(
            matches!(outcome, PollOutcome::Cancelled),
            "a killed task's poll park must report cancellation"
        );
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "cancellation must land within the park's poll interval"
        );
        killer.join().expect("killer thread");
    }

    #[test]
    fn poll_reports_a_bad_fd_as_an_event_errno() {
        let (ctx, _stdin) = ctx_with_scripted_stdin();
        let subscriptions = [FdReadSubscription {
            userdata: 9,
            fd: WasiFd::new(99),
        }];
        // Decided immediately: the readiness probe fails, so the event carries
        // the errno instead of blocking forever on an unreadable fd.
        let events = decided(wait_fd_read_events(&ctx, &subscriptions));
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].userdata, 9);
        assert_ne!(events[0].errno, 0);
    }
}
