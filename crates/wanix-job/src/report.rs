//! The structured `status` and `result.json` report shapes (camelCase JSON).
//!
//! Field names and shapes match the sketches in `docs/toolfs.md` §"Status And
//! Result". All timestamps are caller-supplied Unix-epoch milliseconds; this
//! crate never reads a clock. Optional fields serialize as explicit `null`
//! (the sketches show `"expiresAt": null` / `"error": null`), so a report is
//! always the full shape.

use crate::{ErrorKind, JobState};
use serde::{Deserialize, Serialize};

/// A structured job failure: a workspace taxonomy kind plus a human message.
///
/// Appears as the `error` object in [`JobResult`]. Device-specific detail
/// belongs in `message`, never in a device-specific kind (ADR 0009).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobError {
    /// Which of the nine workspace error kinds this failure is.
    pub kind: ErrorKind,
    /// Human-readable diagnostics for this specific failure.
    pub message: String,
}

impl JobError {
    /// Build an error from a taxonomy kind and a message.
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

/// Live `status` snapshot of one job: where it is in the lifecycle and how
/// many bytes have crossed it so far.
///
/// `expires_at` is the retention deadline once the device has assigned one
/// (a retained terminal job), `null` before that.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobStatus {
    /// Current lifecycle state.
    pub state: JobState,
    /// When the job was allocated (Unix millis).
    pub created_at: u64,
    /// When `ctl run` was accepted (Unix millis), if it has been.
    pub started_at: Option<u64>,
    /// When the job reached a terminal state (Unix millis), if it has.
    pub finished_at: Option<u64>,
    /// When the retained job expires (Unix millis), once retention is set.
    pub expires_at: Option<u64>,
    /// Request bytes written to `in` so far.
    pub input_bytes: u64,
    /// Bytes produced on `out` so far.
    pub output_bytes: u64,
}

/// Final `result.json` summary of one job, retained after it ends.
///
/// `retryable` answers "may the caller allocate a new job and re-run this
/// call?"; on failure it usually follows
/// [`ErrorKind::default_retryable`], but the device has the last word.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobResult {
    /// Terminal lifecycle state (`done`, `failed`, or `aborted`).
    pub state: JobState,
    /// Runner exit code, when the runner has one.
    pub exit_code: Option<i32>,
    /// Wall-clock run duration in milliseconds, when the job ran.
    pub duration_ms: Option<u64>,
    /// Total request bytes the job consumed.
    pub input_bytes: u64,
    /// Total bytes the job produced on `out`.
    pub output_bytes: u64,
    /// The failure, when `state` is not `done`.
    pub error: Option<JobError>,
    /// Whether re-running this call (as a new job) is safe and sensible.
    pub retryable: bool,
}

#[cfg(test)]
mod tests {
    use super::{JobError, JobResult, JobStatus};
    use crate::{ErrorKind, JobState};
    use serde_json::json;

    #[test]
    fn status_snapshot_matches_the_toolfs_sketch() {
        // docs/toolfs.md §"Status And Result", status sketch — plus the
        // `finishedAt` field the sketch leaves undrawn (null while running).
        let status = JobStatus {
            state: JobState::Running,
            created_at: 1_710_000_000_000,
            started_at: Some(1_710_000_000_123),
            finished_at: None,
            expires_at: None,
            input_bytes: 12,
            output_bytes: 0,
        };
        assert_eq!(
            serde_json::to_value(&status).unwrap(),
            json!({
                "state": "running",
                "createdAt": 1_710_000_000_000_u64,
                "startedAt": 1_710_000_000_123_u64,
                "finishedAt": null,
                "expiresAt": null,
                "inputBytes": 12,
                "outputBytes": 0
            })
        );
    }

    #[test]
    fn done_result_matches_the_toolfs_sketch() {
        let result = JobResult {
            state: JobState::Done,
            exit_code: Some(0),
            duration_ms: Some(18),
            input_bytes: 12,
            output_bytes: 12,
            error: None,
            retryable: true,
        };
        assert_eq!(
            serde_json::to_value(&result).unwrap(),
            json!({
                "state": "done",
                "exitCode": 0,
                "durationMs": 18,
                "inputBytes": 12,
                "outputBytes": 12,
                "error": null,
                "retryable": true
            })
        );
    }

    #[test]
    fn failure_result_matches_the_toolfs_sketch() {
        // The toolfs.md failure sketch plus the always-present byte counters.
        let result = JobResult {
            state: JobState::Failed,
            exit_code: Some(2),
            duration_ms: Some(4),
            input_bytes: 12,
            output_bytes: 0,
            error: Some(JobError::new(
                ErrorKind::InvalidInput,
                "input is not valid JSON",
            )),
            retryable: ErrorKind::InvalidInput.default_retryable(),
        };
        assert_eq!(
            serde_json::to_value(&result).unwrap(),
            json!({
                "state": "failed",
                "exitCode": 2,
                "durationMs": 4,
                "inputBytes": 12,
                "outputBytes": 0,
                "error": { "kind": "invalid_input", "message": "input is not valid JSON" },
                "retryable": false
            })
        );
    }

    #[test]
    fn reports_round_trip() {
        let status = JobStatus {
            state: JobState::Aborted,
            created_at: 1,
            started_at: None,
            finished_at: Some(2),
            expires_at: Some(3),
            input_bytes: 4,
            output_bytes: 5,
        };
        let back: JobStatus =
            serde_json::from_str(&serde_json::to_string(&status).unwrap()).unwrap();
        assert_eq!(back, status);

        let result = JobResult {
            state: JobState::Aborted,
            exit_code: None,
            duration_ms: None,
            input_bytes: 4,
            output_bytes: 5,
            error: Some(JobError::new(ErrorKind::Aborted, "caller aborted")),
            retryable: false,
        };
        let back: JobResult =
            serde_json::from_str(&serde_json::to_string(&result).unwrap()).unwrap();
        assert_eq!(back, result);
    }

    #[test]
    fn optional_fields_may_be_omitted_on_input() {
        // A minimal producer may leave the nullable fields out entirely.
        let status: JobStatus = serde_json::from_value(json!({
            "state": "allocated",
            "createdAt": 7,
            "inputBytes": 0,
            "outputBytes": 0
        }))
        .unwrap();
        assert_eq!(status.state, JobState::Allocated);
        assert_eq!(status.started_at, None);
        assert_eq!(status.finished_at, None);
        assert_eq!(status.expires_at, None);
    }
}
