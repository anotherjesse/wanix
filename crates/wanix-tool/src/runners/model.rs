//! The model runner: a model device is just a `ToolService` whose runner
//! completes prompts through a [`ModelEngine`].
//!
//! Budgets are mount quotas (ADR 0000): the spec's
//! `limits.maxBytesPerPrincipal` bounds how much prompt/completion data a
//! principal may hold, and exceeding it fails the job with `quota_exceeded`
//! like any other tool — no model-specific quota machinery.

use std::sync::Arc;

use serde_json::Value;
use wanix_job::{ErrorKind, JobError};

use crate::{RunContext, RunOutcome, ToolRunner};

/// The engine seam behind a model tool: complete one prompt.
pub trait ModelEngine: Send + Sync {
    /// Completes `prompt`, or fails with a taxonomy error (e.g.
    /// `unavailable` for a dead backend, `quota_exceeded` for its own caps).
    fn complete(&self, prompt: &str) -> Result<String, JobError>;
}

/// Deterministic engine for tests: completes `p` as `fake-completion: p`.
#[derive(Debug, Default, Clone, Copy)]
pub struct FakeModelEngine;

impl ModelEngine for FakeModelEngine {
    fn complete(&self, prompt: &str) -> Result<String, JobError> {
        Ok(format!("fake-completion: {}\n", prompt.trim_end()))
    }
}

/// Adapts a [`ModelEngine`] to the [`ToolRunner`] seam.
pub struct ModelRunner {
    engine: Arc<dyn ModelEngine>,
}

impl ModelRunner {
    /// A runner that completes prompts through `engine`.
    #[must_use]
    pub fn new(engine: Arc<dyn ModelEngine>) -> Self {
        Self { engine }
    }
}

impl ToolRunner for ModelRunner {
    fn run(&self, input: &[u8], _params: Option<&Value>, _ctx: &RunContext) -> RunOutcome {
        let Ok(prompt) = std::str::from_utf8(input) else {
            return RunOutcome::failure(
                JobError::new(ErrorKind::InvalidInput, "prompt is not valid UTF-8"),
                None,
                Vec::new(),
            );
        };
        match self.engine.complete(prompt) {
            Ok(completion) => RunOutcome::success(completion.into_bytes()),
            Err(error) => RunOutcome::failure(error, None, Vec::new()),
        }
    }
}
