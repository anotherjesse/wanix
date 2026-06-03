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
}
