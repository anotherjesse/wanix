//! Job lifecycle states and the pure legal-transition relation (ADR 0009).

use serde::{Deserialize, Serialize};

/// Lifecycle state of one reified call (job).
///
/// The legal flow is `allocated → receiving → running → done | failed |
/// aborted`, where `receiving` may be skipped (a job with no input bytes runs
/// straight from `allocated`) and `aborted` is reachable from every
/// non-terminal state. `done`, `failed`, and `aborted` are terminal: retention
/// (`retained → expired`) is a device lifecycle concern layered on top of a
/// terminal state, not a `JobState`.
///
/// Serializes as the snake_case wire string (`"allocated"`, `"receiving"`,
/// ...), the same word that appears in `status` and `result.json`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    /// Job exists (`new` returned its id); nothing written or run yet.
    Allocated,
    /// Input bytes/params are being written; `ctl run` has not sealed them.
    Receiving,
    /// `ctl run` accepted the job and the runner is executing it.
    Running,
    /// Terminal: the runner finished successfully.
    Done,
    /// Terminal: the runner finished unsuccessfully (see [`crate::JobError`]).
    Failed,
    /// Terminal: cancellation (`ctl abort`) won before completion.
    Aborted,
}

impl JobState {
    /// Every state, in lifecycle order — for truth-table tests and iteration.
    pub const ALL: [JobState; 6] = [
        JobState::Allocated,
        JobState::Receiving,
        JobState::Running,
        JobState::Done,
        JobState::Failed,
        JobState::Aborted,
    ];

    /// True for the three end states (`done`, `failed`, `aborted`).
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Done | Self::Failed | Self::Aborted)
    }

    /// The pure legal-transition relation: may a job in `self` move to `next`?
    ///
    /// Encodes exactly `allocated → receiving → running → {done | failed |
    /// aborted}`, the receiving-skip `allocated → running`, and abort from
    /// either pre-run state. Terminal states admit no transition; self-loops
    /// are not transitions.
    pub fn can_transition_to(self, next: JobState) -> bool {
        use JobState::*;
        matches!(
            (self, next),
            (Allocated, Receiving)
                | (Allocated, Running)
                | (Allocated, Aborted)
                | (Receiving, Running)
                | (Receiving, Aborted)
                | (Running, Done)
                | (Running, Failed)
                | (Running, Aborted)
        )
    }

    /// The snake_case wire string, identical to the serde encoding.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Allocated => "allocated",
            Self::Receiving => "receiving",
            Self::Running => "running",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Aborted => "aborted",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::JobState;
    use JobState::*;

    #[test]
    fn transition_truth_table() {
        let legal = [
            (Allocated, Receiving),
            (Allocated, Running),
            (Allocated, Aborted),
            (Receiving, Running),
            (Receiving, Aborted),
            (Running, Done),
            (Running, Failed),
            (Running, Aborted),
        ];
        for from in JobState::ALL {
            for to in JobState::ALL {
                let expected = legal.contains(&(from, to));
                assert_eq!(
                    from.can_transition_to(to),
                    expected,
                    "{} -> {} should be {}",
                    from.as_str(),
                    to.as_str(),
                    expected
                );
            }
        }
    }

    #[test]
    fn terminal_states_admit_no_transition() {
        for from in JobState::ALL.into_iter().filter(|s| s.is_terminal()) {
            for to in JobState::ALL {
                assert!(!from.can_transition_to(to));
            }
        }
        assert!(!Allocated.is_terminal());
        assert!(!Receiving.is_terminal());
        assert!(!Running.is_terminal());
    }

    #[test]
    fn wire_strings_are_pinned() {
        let expected = [
            (Allocated, "allocated"),
            (Receiving, "receiving"),
            (Running, "running"),
            (Done, "done"),
            (Failed, "failed"),
            (Aborted, "aborted"),
        ];
        for (state, wire) in expected {
            assert_eq!(state.as_str(), wire);
            assert_eq!(
                serde_json::to_string(&state).unwrap(),
                format!("\"{wire}\"")
            );
            let back: JobState = serde_json::from_str(&format!("\"{wire}\"")).unwrap();
            assert_eq!(back, state);
        }
    }

    #[test]
    fn unknown_state_string_is_rejected() {
        assert!(serde_json::from_str::<JobState>("\"retained\"").is_err());
    }
}
