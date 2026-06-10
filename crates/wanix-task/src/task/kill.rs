//! Task kill: the ADR 0010 "kill is a task operation" seam.
//!
//! `#task/<id>/ctl` accepts `kill`. Kill promises **fd release and an
//! observable exit only** — no transactional cleanup of half-written device
//! state. The mechanism is driver-supplied: a running driver arms an
//! [`InterruptHook`] (the wasm driver's Wasmtime epoch interrupter) that
//! [`Task::kill`] invokes; the driver then records the distinct
//! [`KILLED_EXIT`] status and runs the same fd-release path as a normal exit
//! (`task-exit-closes-fds`, which gives pipeline EOF on death for free).
//!
//! The hook trips guest *code*: a guest parked inside a blocking host read
//! dies on its next return to guest execution, not mid-park. A driver without
//! an interrupt seam (today the qjs driver, whose tasks share one engine)
//! ignores the request; the flag stays observable via
//! [`Task::kill_requested`].

use std::sync::Arc;

use wanix_fs::FsResult;

use super::Task;

/// The distinct exit status text recorded for a killed task, observably
/// different from any numeric guest exit code.
pub const KILLED_EXIT: &str = "killed";

/// A driver-armed callback that interrupts the task's running guest.
pub type InterruptHook = Arc<dyn Fn() + Send + Sync>;

/// What [`Task::kill`] decided under the state lock; acted on outside it.
enum KillAction {
    /// Already exited (or running with no armed hook): nothing to invoke.
    Nothing,
    /// Never started: record the killed exit and release fds directly.
    RecordExit,
    /// Running with an armed hook: interrupt the guest.
    Interrupt(InterruptHook),
}

impl Task {
    /// Requests that this task die.
    ///
    /// - Already exited: `Ok` no-op.
    /// - Not yet started: claims the task's one start so it can never run,
    ///   releases its fds, and records [`KILLED_EXIT`].
    /// - Running: marks the kill requested and invokes the driver's armed
    ///   [`InterruptHook`]; the driver records [`KILLED_EXIT`] and releases
    ///   fds when the interrupted run unwinds. A driver that armed no hook
    ///   only gets the observable [`Self::kill_requested`] flag.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when the task state lock is poisoned.
    pub fn kill(&self) -> FsResult<()> {
        let action = self.write_state(|state| {
            if !state.exit.is_empty() {
                return Ok(KillAction::Nothing);
            }
            state.kill_requested = true;
            if !state.started {
                state.started = true;
                return Ok(KillAction::RecordExit);
            }
            Ok(match &state.interrupt_hook {
                Some(hook) => KillAction::Interrupt(Arc::clone(hook)),
                None => KillAction::Nothing,
            })
        })?;
        match action {
            KillAction::Nothing => Ok(()),
            KillAction::RecordExit => {
                self.close_all_fds();
                self.set_exit(KILLED_EXIT)
            }
            // Invoked outside the state lock: the hook may do engine work.
            KillAction::Interrupt(hook) => {
                hook();
                Ok(())
            }
        }
    }

    /// Returns whether a kill has been requested for this task.
    #[must_use]
    pub fn kill_requested(&self) -> bool {
        self.read_state(|state| state.kill_requested)
            .unwrap_or(false)
    }

    /// Arms the driver's interrupt hook for the duration of a run.
    ///
    /// If a kill was already requested (it raced the driver's startup), the
    /// hook is invoked immediately — the driver must still check
    /// [`Self::kill_requested`] before entering the guest, since an interrupt
    /// delivered before the guest's execution context exists is otherwise
    /// lost.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when the task state lock is poisoned.
    pub fn arm_interrupt(&self, hook: InterruptHook) -> FsResult<()> {
        let already_requested = self.write_state(|state| {
            state.interrupt_hook = Some(Arc::clone(&hook));
            Ok(state.kill_requested)
        })?;
        if already_requested {
            hook();
        }
        Ok(())
    }

    /// Disarms the interrupt hook after a run finishes (idempotent).
    pub fn disarm_interrupt(&self) {
        let _ = self.write_state(|state| {
            state.interrupt_hook = None;
            Ok(())
        });
    }
}
