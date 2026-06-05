use super::QuickJsRuntime;
use anyhow::{Result, bail};
use wasmtime::error::Context as _;

impl QuickJsRuntime {
    /// Executes pending QuickJS jobs until the job queue is empty.
    ///
    /// This method is unbounded. Use [`Self::execute_pending_jobs_with_limit`]
    /// when JavaScript may schedule more jobs while the queue is being drained.
    ///
    /// # Errors
    ///
    /// Returns an error if QuickJS reports a failing job or the job queue cannot
    /// be inspected/executed.
    pub fn execute_pending_jobs(&mut self) -> Result<usize> {
        self.execute_pending_jobs_inner(None)
    }

    /// Executes up to `max_jobs` pending QuickJS jobs.
    ///
    /// Returns an error if the queue is still non-empty after `max_jobs` jobs
    /// have been executed. A limit of zero checks the queue but does not execute
    /// any jobs.
    ///
    /// # Errors
    ///
    /// Returns an error if QuickJS reports a failing job, the job queue cannot
    /// be inspected/executed, or the configured job limit is reached before the
    /// queue becomes empty.
    pub fn execute_pending_jobs_with_limit(&mut self, max_jobs: usize) -> Result<usize> {
        self.execute_pending_jobs_inner(Some(max_jobs))
    }

    fn execute_pending_jobs_inner(&mut self, max_jobs: Option<usize>) -> Result<usize> {
        let mut executed = 0;
        while self
            .qjs_is_job_pending
            .call(&mut self.store, ())
            .context("failed to check QuickJS job queue")?
            != 0
        {
            if let Some(max_jobs) = max_jobs
                && executed >= max_jobs
            {
                bail!("QuickJS pending job limit reached after {executed} jobs");
            }
            let result = self
                .qjs_execute_pending_job
                .call(&mut self.store, ())
                .context("failed to execute QuickJS pending job")?;
            if result < 0 {
                bail!(
                    "QuickJS pending job failed: {}",
                    self.take_exception_string()?
                );
            }
            executed = executed
                .checked_add(1)
                .ok_or_else(|| anyhow::anyhow!("QuickJS pending job count overflowed"))?;
        }
        Ok(executed)
    }
}
