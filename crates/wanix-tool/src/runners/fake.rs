//! Deterministic fake runners for pinning the filesystem contract.

use serde_json::Value;
use wanix_job::{ErrorKind, JobError};

use crate::runner::{RunOutcome, ToolRunner};

/// Uppercases UTF-8 input. Invalid UTF-8 is `invalid_input`.
#[derive(Debug, Default, Clone, Copy)]
pub struct UpperRunner;

impl ToolRunner for UpperRunner {
    fn run(&self, input: &[u8], _params: Option<&Value>) -> RunOutcome {
        match std::str::from_utf8(input) {
            Ok(text) => RunOutcome::success(text.to_uppercase().into_bytes()),
            Err(_) => RunOutcome::failure(
                JobError::new(ErrorKind::InvalidInput, "input is not valid UTF-8"),
                Some(1),
                b"input is not valid UTF-8\n".to_vec(),
            ),
        }
    }
}

/// Always fails: `runner_failed`, exit code 2, diagnostics on `err`.
#[derive(Debug, Default, Clone, Copy)]
pub struct FailRunner;

impl ToolRunner for FailRunner {
    fn run(&self, _input: &[u8], _params: Option<&Value>) -> RunOutcome {
        RunOutcome::failure(
            JobError::new(ErrorKind::RunnerFailed, "fail runner always fails"),
            Some(2),
            b"deliberate failure\n".to_vec(),
        )
    }
}

/// Copies input to output unchanged.
#[derive(Debug, Default, Clone, Copy)]
pub struct EchoRunner;

impl ToolRunner for EchoRunner {
    fn run(&self, input: &[u8], _params: Option<&Value>) -> RunOutcome {
        RunOutcome::success(input.to_vec())
    }
}
