//! The workspace-wide error taxonomy (ADR 0009 §"The shared error taxonomy").

use serde::{Deserialize, Serialize};

/// One of the nine workspace-wide error kinds.
///
/// Devices may add device-specific detail in the error `message` (or a
/// `detail` field of their own), never device-specific top-level kinds.
/// Serializes as the snake_case wire string used in `result.json`/`status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    /// `params.json` failed validation (schema or semantic).
    InvalidParams,
    /// The `in` bytes are unacceptable to the runner (e.g. malformed input).
    InvalidInput,
    /// Input exceeded the device's declared `maxBytes` limit.
    InputTooLarge,
    /// A per-principal quota (jobs, bytes, concurrency) was exhausted.
    QuotaExceeded,
    /// The run exceeded its time budget.
    Timeout,
    /// Cancellation (`ctl abort`) interrupted the job.
    Aborted,
    /// The runner itself crashed or misbehaved.
    RunnerFailed,
    /// A dependency the device needs is unreachable right now.
    Unavailable,
    /// The device's own invariant broke; a bug, not a caller error.
    Internal,
}

impl ErrorKind {
    /// Every kind, in ADR order — for pinning tests and iteration.
    pub const ALL: [ErrorKind; 9] = [
        ErrorKind::InvalidParams,
        ErrorKind::InvalidInput,
        ErrorKind::InputTooLarge,
        ErrorKind::QuotaExceeded,
        ErrorKind::Timeout,
        ErrorKind::Aborted,
        ErrorKind::RunnerFailed,
        ErrorKind::Unavailable,
        ErrorKind::Internal,
    ];

    /// The snake_case wire string, identical to the serde encoding.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InvalidParams => "invalid_params",
            Self::InvalidInput => "invalid_input",
            Self::InputTooLarge => "input_too_large",
            Self::QuotaExceeded => "quota_exceeded",
            Self::Timeout => "timeout",
            Self::Aborted => "aborted",
            Self::RunnerFailed => "runner_failed",
            Self::Unavailable => "unavailable",
            Self::Internal => "internal",
        }
    }

    /// The default `retryable` verdict for this kind.
    ///
    /// Transient conditions (`quota_exceeded`, `timeout`, `unavailable`) are
    /// retryable by default; everything else is not. A device may override
    /// the verdict in a specific [`crate::JobResult`] when it knows better.
    pub fn default_retryable(self) -> bool {
        matches!(
            self,
            Self::QuotaExceeded | Self::Timeout | Self::Unavailable
        )
    }
}

#[cfg(test)]
mod tests {
    use super::ErrorKind;

    #[test]
    fn wire_strings_are_pinned_to_the_adr() {
        let expected = [
            "invalid_params",
            "invalid_input",
            "input_too_large",
            "quota_exceeded",
            "timeout",
            "aborted",
            "runner_failed",
            "unavailable",
            "internal",
        ];
        assert_eq!(ErrorKind::ALL.len(), expected.len());
        for (kind, wire) in ErrorKind::ALL.into_iter().zip(expected) {
            assert_eq!(kind.as_str(), wire);
            assert_eq!(serde_json::to_string(&kind).unwrap(), format!("\"{wire}\""));
            let back: ErrorKind = serde_json::from_str(&format!("\"{wire}\"")).unwrap();
            assert_eq!(back, kind);
        }
    }

    #[test]
    fn default_retryable_per_kind() {
        for kind in ErrorKind::ALL {
            let expected = matches!(
                kind,
                ErrorKind::QuotaExceeded | ErrorKind::Timeout | ErrorKind::Unavailable
            );
            assert_eq!(kind.default_retryable(), expected, "{}", kind.as_str());
        }
    }

    #[test]
    fn device_specific_kinds_are_rejected() {
        assert!(serde_json::from_str::<ErrorKind>("\"upper_case_failed\"").is_err());
    }
}
