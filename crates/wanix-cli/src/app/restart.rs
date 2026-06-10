//! `--restart on-failure`: the guest supervisor and the live-service slot.
//!
//! The endpoint and its ticket outlive any one guest generation: the attach
//! policy reads the *current* [`wanix_appfs::AppFsService`] out of a
//! [`ServiceSlot`] per [`wanix_id::AttachPolicy::evaluate`], so a restart
//! swaps a fresh adapter behind the same ticket. New connections bind the
//! live service; connections opened against an exited generation keep their
//! honestly-dead view (discrete ops `Unreachable`, streams EOF — see
//! [`super::guest`]). While no generation is live the slot is empty and new
//! attaches are refused, which surfaces to dialers as an authorization
//! error, not a hang. Restarts run with capped exponential backoff so a
//! crash-looping guest cannot spin the host.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use wanix_appfs::AppFsService;
use wanix_task::Task;

use super::AppManifest;
use super::guest::{AppGuest, start_app_guest};

/// When a failed guest is started again (parsed from `--restart`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum RestartPolicy {
    /// A dead guest stays dead (the pre-v0.2 behavior); the default.
    #[default]
    Never,
    /// The supervisor re-runs the guest with capped backoff after every exit.
    OnFailure,
}

/// Delay before the first restart attempt of a generation.
const INITIAL_RESTART_BACKOFF: Duration = Duration::from_secs(1);
/// Ceiling on the delay between restart attempts.
const MAX_RESTART_BACKOFF: Duration = Duration::from_secs(30);
/// A guest that lived at least this long resets the backoff (it was healthy,
/// not crash-looping).
const HEALTHY_UPTIME: Duration = Duration::from_secs(30);

/// How often the supervisor re-probes a live guest for exit or latch-down.
const SUPERVISE_POLL_INTERVAL: Duration = Duration::from_millis(250);

/// The live-service indirection between the endpoint and guest generations.
///
/// Clone-cheap; every clone shares one cell. The attach policy reads it per
/// connection; the supervisor empties it on guest death (exit or channel
/// latch-down) and refills it once a fresh guest has completed its hello
/// handshake.
#[derive(Clone)]
pub(crate) struct ServiceSlot {
    inner: Arc<Mutex<Option<AppFsService>>>,
}

impl ServiceSlot {
    pub(crate) fn new(service: AppFsService) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Some(service))),
        }
    }

    /// The currently live service, if any generation is up.
    pub(crate) fn current(&self) -> Option<AppFsService> {
        self.inner.lock().ok().and_then(|slot| slot.clone())
    }

    fn replace(&self, service: Option<AppFsService>) {
        if let Ok(mut slot) = self.inner.lock() {
            *slot = service;
        }
    }
}

/// Spawns the supervisor thread: waits for the current guest to die — exit
/// *or* channel latch-down (an op deadline expired: the guest is wedged and,
/// with no qjs interrupt seam, may never exit) — then re-runs it with capped
/// backoff, swapping each fresh generation's service into `slot`. A wedged
/// generation is abandoned, not waited on: its engine thread leaks (bounded,
/// one per wedge, logged) so the endpoint can recover instead of staying
/// Unreachable for the life of the serve process. Runs for the life of the
/// serve process.
pub(crate) fn spawn_restart_supervisor(
    guest: AppGuest,
    slot: ServiceSlot,
    app_dir: PathBuf,
    state_dir: PathBuf,
    manifest: AppManifest,
) {
    std::thread::spawn(move || {
        let mut guest = guest;
        let mut backoff = INITIAL_RESTART_BACKOFF;
        loop {
            let started = std::time::Instant::now();
            let ended = wait_exit_or_down(&guest.task, || {
                slot.current().is_some_and(|service| service.is_down())
            });
            // The dead generation's adapter stays alive only through views
            // already bound to connections (the honestly-dead view); new
            // attaches are refused until a fresh generation is live.
            slot.replace(None);
            if started.elapsed() >= HEALTHY_UPTIME {
                backoff = INITIAL_RESTART_BACKOFF;
            }
            eprintln!(
                "wanix-rust app serve: guest {ended}; restarting in {backoff:?} \
                 (--restart on-failure; the ticket and endpoint are unchanged)"
            );
            // The wedged case leaves the old task running; this loop holds no
            // reference to it (only `guest` is replaced below), so nothing
            // here ever blocks on a task that will never exit.
            loop {
                std::thread::sleep(backoff);
                backoff = (backoff * 2).min(MAX_RESTART_BACKOFF);
                match start_app_guest(&app_dir, &state_dir, &manifest) {
                    Ok((next_guest, service)) => {
                        slot.replace(Some(service));
                        guest = next_guest;
                        break;
                    }
                    Err(error) => {
                        eprintln!(
                            "wanix-rust app serve: guest restart failed ({error}); \
                             next attempt in {backoff:?}"
                        );
                    }
                }
            }
        }
    });
}

/// Blocks until the guest task records an exit or `is_down` reports the
/// adapter channel latched down, returning the human-readable cause.
///
/// Latch-down covers the guest the exit wait can never see: one wedged in an
/// infinite loop or a parked handler. Its own docs declare an unresponsive
/// guest a dead guest (`wanix-appfs` op deadline); this is where the restart
/// policy honors that. When both race (a crashing guest EOFs its channel just
/// before its exit status lands), the exit status wins after one extra probe.
fn wait_exit_or_down(task: &Task, is_down: impl Fn() -> bool) -> String {
    loop {
        let exit = task.exit();
        if !exit.is_empty() {
            return format!("exited with status {exit}");
        }
        if is_down() {
            // Give a crashed guest one beat to record its real exit status.
            std::thread::sleep(SUPERVISE_POLL_INTERVAL);
            let exit = task.exit();
            if !exit.is_empty() {
                return format!("exited with status {exit}");
            }
            return "is unresponsive (its channel latched down past the op deadline; \
                    abandoning this generation)"
                .to_owned();
        }
        std::thread::sleep(SUPERVISE_POLL_INTERVAL);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::{Duration, Instant};

    use wanix_task::TaskTable;
    use wanix_vfs::Namespace;

    use super::wait_exit_or_down;

    #[test]
    fn supervisor_wait_ends_on_latch_down_even_when_the_task_never_exits() {
        let table = TaskTable::new();
        let task = table
            .allocate_root_with_namespace("auto", Namespace::new())
            .unwrap();
        let down = Arc::new(AtomicBool::new(false));
        let armed = Arc::clone(&down);
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            armed.store(true, Ordering::SeqCst);
        });
        let started = Instant::now();
        let ended = wait_exit_or_down(&task, || down.load(Ordering::SeqCst));
        assert!(ended.contains("unresponsive"), "{ended}");
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "a wedged (never-exiting) guest must not park the supervisor"
        );
    }

    #[test]
    fn supervisor_wait_prefers_the_exit_status_when_the_task_exits() {
        let table = TaskTable::new();
        let task = table
            .allocate_root_with_namespace("auto", Namespace::new())
            .unwrap();
        task.set_exit("7").unwrap();
        // Even with the channel already down (the exit/latch race), the
        // recorded exit wins.
        let ended = wait_exit_or_down(&task, || true);
        assert_eq!(ended, "exited with status 7");
    }
}
