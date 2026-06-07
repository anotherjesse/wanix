use super::command_record::{QJS_SHELL_COMMAND_RECORD_EVIDENCE, ShellCommandRecord};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(in crate::serve) struct ShellCommandObservation {
    pub(in crate::serve) command: Option<String>,
    pub(in crate::serve) evidence: Option<String>,
    pub(in crate::serve) diagnostic: Option<String>,
    pub(in crate::serve) exit_code: Option<i32>,
    pub(in crate::serve) terminal_output: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(in crate::serve) struct ShellCommandObservations {
    records: Vec<ShellCommandObservation>,
    fallback: ShellCommandObservation,
}

impl ShellCommandObservations {
    pub(in crate::serve) fn new(
        records: Vec<ShellCommandRecord>,
        fallback: ShellCommandObservation,
    ) -> Self {
        Self {
            records: records
                .into_iter()
                .map(ShellCommandObservation::from_record)
                .collect(),
            fallback,
        }
    }

    pub(in crate::serve) fn for_command(&self, command: &str) -> ShellCommandObservation {
        self.record_for_command(command).unwrap_or_else(|| {
            if self.records.is_empty() {
                self.fallback.clone()
            } else {
                ShellCommandObservation::default()
            }
        })
    }

    pub(in crate::serve) fn record_for_command(
        &self,
        command: &str,
    ) -> Option<ShellCommandObservation> {
        self.records
            .iter()
            .find(|record| record.command.as_deref() == Some(command))
            .cloned()
    }

    pub(in crate::serve) fn recorded_commands(&self) -> &[ShellCommandObservation] {
        &self.records
    }
}

impl ShellCommandObservation {
    fn from_record(record: ShellCommandRecord) -> Self {
        Self {
            command: Some(record.command),
            evidence: Some(QJS_SHELL_COMMAND_RECORD_EVIDENCE.to_owned()),
            diagnostic: record.diagnostic,
            exit_code: record.exit_code,
            terminal_output: record.terminal_output,
        }
    }

    pub(in crate::serve) fn indicates_error(&self) -> bool {
        self.diagnostic.is_some() || self.exit_code.is_some_and(|code| code != 0)
    }
}
