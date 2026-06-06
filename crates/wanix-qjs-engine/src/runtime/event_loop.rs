use std::thread;
use std::time::Duration;

use super::QuickJsRuntime;
use anyhow::{Result, anyhow, bail};
use wasmtime::error::Context as _;

/// Result of one QuickJS standard-library event-loop turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuickJsEventLoopStatus {
    /// No pending jobs, expired timers, or future timers remain.
    Idle,
    /// Immediate work remains and another event-loop turn should run soon.
    Pending,
    /// No immediate work remains, but a timer is scheduled for the future.
    Wait(Duration),
}

impl QuickJsRuntime {
    /// Runs one QuickJS standard-library event-loop turn.
    ///
    /// This executes pending promise jobs and at most one expired QuickJS
    /// `qjs:os` timer. It does not block waiting for future timers; when the
    /// next timer is in the future, the returned status contains that delay.
    ///
    /// # Errors
    ///
    /// Returns an error when the QuickJS WASM module does not expose
    /// `js_std_loop_once`, when the WASM call fails, or when a pending job or
    /// timer callback throws.
    pub fn execute_event_loop_once(&mut self) -> Result<QuickJsEventLoopStatus> {
        let js_std_loop_once = self
            .js_std_loop_once
            .clone()
            .ok_or_else(|| anyhow!("QuickJS WASM module does not export js_std_loop_once"))?;
        let context = self
            .qjs_get_context_ptr
            .call(&mut self.store, ())
            .context("failed to read QuickJS context pointer")?;
        let status = js_std_loop_once
            .call(&mut self.store, context)
            .context("failed to run QuickJS event loop turn")?;
        match status {
            -2 => bail!(
                "QuickJS event loop callback failed: {}",
                self.take_exception_string()?
            ),
            -1 => Ok(QuickJsEventLoopStatus::Idle),
            0 => Ok(QuickJsEventLoopStatus::Pending),
            delay_ms if delay_ms > 0 => Ok(QuickJsEventLoopStatus::Wait(Duration::from_millis(
                u64::try_from(delay_ms).context("event loop delay does not fit in u64")?,
            ))),
            other => bail!("QuickJS event loop returned unexpected status {other}"),
        }
    }

    /// Runs one nonblocking QuickJS standard-library fd readiness turn.
    ///
    /// This lets `qjs:os.setReadHandler` and `qjs:os.setWriteHandler` callbacks
    /// observe fds that are immediately ready according to the WASI
    /// `poll_oneoff` import. The call does not wait for future readiness.
    ///
    /// QuickJS's exported `js_std_poll_io` reports the same success code whether
    /// no handler was ready or one handler ran successfully, so this method does
    /// not report an idle/ready distinction.
    ///
    /// # Errors
    ///
    /// Returns an error when the QuickJS WASM module does not expose
    /// `js_std_poll_io`, when the WASM call fails, or when a readiness callback
    /// throws.
    pub fn execute_ready_io_event_loop_once(&mut self) -> Result<()> {
        let js_std_poll_io = self
            .js_std_poll_io
            .clone()
            .ok_or_else(|| anyhow!("QuickJS WASM module does not export js_std_poll_io"))?;
        let context = self
            .qjs_get_context_ptr
            .call(&mut self.store, ())
            .context("failed to read QuickJS context pointer")?;
        let status = js_std_poll_io
            .call(&mut self.store, (context, 0))
            .context("failed to run QuickJS ready-IO event loop turn")?;
        match status {
            0 => Ok(()),
            -1 | -2 => bail!(
                "QuickJS ready-IO event loop callback failed: {}",
                self.take_exception_string()?
            ),
            other => bail!("QuickJS ready-IO event loop returned unexpected status {other}"),
        }
    }

    /// Runs immediate QuickJS event-loop turns until idle or waiting on a future timer.
    ///
    /// Returns the number of turns executed. This method deliberately does not
    /// sleep for future timers; it is a bounded pump for work that is already
    /// ready, such as `sleepAsync(0)`.
    ///
    /// # Errors
    ///
    /// Returns an error if an event-loop turn fails or if immediate work remains
    /// after `max_turns` turns.
    pub fn execute_immediate_event_loop_with_limit(&mut self, max_turns: usize) -> Result<usize> {
        let mut turns = 0usize;
        loop {
            let remaining = max_turns.saturating_sub(turns);
            let jobs = self.execute_pending_jobs_with_limit(remaining)?;
            turns = turns
                .checked_add(jobs)
                .ok_or_else(|| anyhow!("QuickJS event loop turn count overflowed"))?;
            if turns >= max_turns {
                bail!("QuickJS immediate event loop limit reached after {turns} turns");
            }
            match self.execute_event_loop_once()? {
                QuickJsEventLoopStatus::Idle | QuickJsEventLoopStatus::Wait(_) => return Ok(turns),
                QuickJsEventLoopStatus::Pending => {
                    turns = turns
                        .checked_add(1)
                        .ok_or_else(|| anyhow!("QuickJS event loop turn count overflowed"))?;
                }
            }
        }
    }

    /// Runs QuickJS event-loop turns, waiting for future timers within a budget.
    ///
    /// This method sleeps the current host thread for each future timer delay
    /// that fits inside `max_wait`, advances the runtime's deterministic WASI
    /// clock by the same duration, then continues pumping pending jobs and
    /// expired timers. It returns [`QuickJsEventLoopStatus::Idle`] when the
    /// runtime has no more standard-library timer/job work, or
    /// [`QuickJsEventLoopStatus::Wait`] when a future timer remains beyond the
    /// supplied wait budget.
    ///
    /// This is a bounded runtime pump, not a Wanix scheduler. It does not drive
    /// fd readiness handlers; call [`Self::execute_ready_io_event_loop_once`]
    /// separately for nonblocking `qjs:os` fd handler work.
    ///
    /// # Errors
    ///
    /// Returns an error if an event-loop turn fails, if the runtime clock cannot
    /// be advanced, or if immediate work remains after `max_turns` turns.
    pub fn execute_event_loop_with_wait_budget(
        &mut self,
        max_turns: usize,
        max_wait: Duration,
    ) -> Result<QuickJsEventLoopStatus> {
        let mut turns = 0usize;
        let mut waited = Duration::ZERO;
        loop {
            self.drain_event_loop_jobs(&mut turns, max_turns)?;

            match self.execute_event_loop_once()? {
                QuickJsEventLoopStatus::Idle => return Ok(QuickJsEventLoopStatus::Idle),
                QuickJsEventLoopStatus::Pending => {
                    record_event_loop_turn(&mut turns)?;
                }
                QuickJsEventLoopStatus::Wait(delay) => {
                    record_wait_turn(&mut turns, max_turns)?;
                    if self.wait_for_event_loop_timer(&mut waited, delay, max_wait)? {
                        return Ok(QuickJsEventLoopStatus::Wait(delay));
                    }
                }
            }
        }
    }

    fn drain_event_loop_jobs(&mut self, turns: &mut usize, max_turns: usize) -> Result<()> {
        let remaining = max_turns.saturating_sub(*turns);
        let jobs = self.execute_pending_jobs_with_limit(remaining)?;
        add_event_loop_turns(turns, jobs)?;
        ensure_event_loop_limit_available(*turns, max_turns)
    }

    fn wait_for_event_loop_timer(
        &mut self,
        waited: &mut Duration,
        delay: Duration,
        max_wait: Duration,
    ) -> Result<bool> {
        let Some(next_waited) = waited.checked_add(delay) else {
            return Ok(true);
        };
        if next_waited > max_wait {
            return Ok(true);
        }
        if !delay.is_zero() {
            thread::sleep(delay);
            self.store.data_mut().advance_clock_time_by(delay)?;
        }
        *waited = next_waited;
        Ok(false)
    }
}

fn record_event_loop_turn(turns: &mut usize) -> Result<()> {
    add_event_loop_turns(turns, 1)
}

fn record_wait_turn(turns: &mut usize, max_turns: usize) -> Result<()> {
    record_event_loop_turn(turns)?;
    ensure_event_loop_limit_available(*turns, max_turns)
}

fn add_event_loop_turns(turns: &mut usize, count: usize) -> Result<()> {
    *turns = turns
        .checked_add(count)
        .ok_or_else(|| anyhow!("QuickJS event loop turn count overflowed"))?;
    Ok(())
}

fn ensure_event_loop_limit_available(turns: usize, max_turns: usize) -> Result<()> {
    if turns >= max_turns {
        bail!("QuickJS event loop limit reached after {turns} turns");
    }
    Ok(())
}
