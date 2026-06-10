//! Child supervision: the poll loop that turns the abort flag, the run
//! deadline, and the capture caps into one kill, and always reaps the child.

use std::process::{Child, ExitStatus};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use wanix_tool::RunContext;

/// How often the supervisor polls the child and the abort/deadline state.
const POLL_INTERVAL: Duration = Duration::from_millis(10);

/// Why the supervisor killed the child.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Kill {
    Aborted,
    Timeout,
    OutCap,
    ErrCap,
}

/// Waits for the child to exit, killing it once on the first abort, deadline,
/// or capture-cap trigger, and keeps waiting after a kill so the child is
/// always reaped (no zombies) and the capture threads always reach EOF.
pub(super) fn supervise(
    child: &Mutex<Child>,
    ctx: &RunContext,
    out_full: &AtomicBool,
    err_full: &AtomicBool,
) -> (Option<ExitStatus>, Option<Kill>) {
    let mut kill: Option<Kill> = None;
    loop {
        let Ok(mut locked) = child.lock() else {
            return (None, kill);
        };
        match locked.try_wait() {
            Ok(Some(status)) => return (Some(status), kill),
            Ok(None) => {}
            Err(_) => return (None, kill),
        }
        if kill.is_none() {
            let trigger = if ctx.aborted() {
                Some(Kill::Aborted)
            } else if ctx.deadline().is_some_and(|deadline| now_ms() > deadline) {
                Some(Kill::Timeout)
            } else if out_full.load(Ordering::SeqCst) {
                Some(Kill::OutCap)
            } else if err_full.load(Ordering::SeqCst) {
                Some(Kill::ErrCap)
            } else {
                None
            };
            if let Some(trigger) = trigger {
                let _ = locked.kill();
                kill = Some(trigger);
            }
        }
        drop(locked);
        std::thread::sleep(POLL_INTERVAL);
    }
}

/// The wall clock in Unix-epoch milliseconds — the same clock the service's
/// deadline is minted from.
pub(super) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| u64::try_from(elapsed.as_millis()).unwrap_or(0))
}
