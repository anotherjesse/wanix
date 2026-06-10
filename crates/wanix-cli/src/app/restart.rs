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

/// The live-service indirection between the endpoint and guest generations.
///
/// Clone-cheap; every clone shares one cell. The attach policy reads it per
/// connection; the supervisor empties it on guest exit and refills it once a
/// fresh guest has completed its hello handshake.
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

/// Spawns the supervisor thread: waits for the current guest to exit, then
/// re-runs it with capped backoff, swapping each fresh generation's service
/// into `slot`. Runs for the life of the serve process.
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
            let exit = guest
                .task
                .wait_exit()
                .unwrap_or_else(|error| format!("unknown ({error})"));
            // The exited generation's adapter stays alive only through views
            // already bound to connections (the honestly-dead view); new
            // attaches are refused until a fresh generation is live.
            slot.replace(None);
            if started.elapsed() >= HEALTHY_UPTIME {
                backoff = INITIAL_RESTART_BACKOFF;
            }
            eprintln!(
                "wanix-rust app serve: guest exited with status {exit}; restarting in {backoff:?} \
                 (--restart on-failure; the ticket and endpoint are unchanged)"
            );
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
