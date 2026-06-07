use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value, json};

use crate::qjs_term::QjsShellSession;

use super::shell_activity::ShellMutationOperation;
use super::shell_observation::ShellCommandObservation;

pub(in crate::serve) const SHELL_HISTORY_COMMANDS_PATH: &str = "/.wanix/qjs-shell/commands.jsonl";
pub(in crate::serve) const SHELL_HISTORY_LATEST_JSON_PATH: &str = "/.wanix/qjs-shell/latest.json";
pub(in crate::serve) const SHELL_HISTORY_LATEST_MD_PATH: &str = "/.wanix/qjs-shell/latest.md";

const SHELL_HISTORY_SCHEMA: &str = "wanix.qjs-shell.command-history.v1";
const SHELL_HISTORY_DIR: &str = ".wanix/qjs-shell";
const SHELL_HISTORY_COMMANDS_FILE: &str = ".wanix/qjs-shell/commands.jsonl";
const SHELL_HISTORY_LATEST_JSON_FILE: &str = ".wanix/qjs-shell/latest.json";
const SHELL_HISTORY_LATEST_MD_FILE: &str = ".wanix/qjs-shell/latest.md";

#[derive(Debug, Clone)]
pub(in crate::serve) struct ShellHistoryContext {
    task_id: String,
    terminal_id: String,
    cwd: String,
}

impl ShellHistoryContext {
    pub(in crate::serve) fn from_shell(shell: &QjsShellSession) -> Self {
        Self {
            task_id: shell.task_id(),
            terminal_id: shell.terminal_id().to_owned(),
            cwd: shell.cwd().as_str().to_owned(),
        }
    }
}

#[derive(Debug, Clone)]
pub(in crate::serve) struct ShellHistoryWriter {
    root: PathBuf,
}

impl ShellHistoryWriter {
    pub(in crate::serve) fn new(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
        }
    }

    pub(in crate::serve) fn write_from_shell(
        &self,
        shell: &QjsShellSession,
        records: &[ShellCommandObservation],
        operations: &[ShellMutationOperation],
    ) -> io::Result<Vec<String>> {
        self.write(&ShellHistoryContext::from_shell(shell), records, operations)
    }

    pub(in crate::serve) fn write(
        &self,
        context: &ShellHistoryContext,
        records: &[ShellCommandObservation],
        operations: &[ShellMutationOperation],
    ) -> io::Result<Vec<String>> {
        let records = records
            .iter()
            .filter(|record| record.command.is_some())
            .collect::<Vec<_>>();
        if records.is_empty() {
            return Ok(Vec::new());
        }

        fs::create_dir_all(self.root.join(SHELL_HISTORY_DIR))?;
        let observed_at = unix_millis();
        let entries = records
            .iter()
            .map(|record| {
                shell_history_entry(
                    context,
                    observed_at,
                    record,
                    operations
                        .iter()
                        .find(|operation| record.command.as_deref() == Some(&operation.command)),
                )
            })
            .collect::<Vec<_>>();

        let mut jsonl = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.root.join(SHELL_HISTORY_COMMANDS_FILE))?;
        for entry in &entries {
            writeln!(jsonl, "{}", serde_json_string(entry)?)?;
        }

        let latest = json!({
            "schema": SHELL_HISTORY_SCHEMA,
            "generatedAtUnixMillis": observed_at,
            "historyPath": SHELL_HISTORY_COMMANDS_PATH,
            "latestJsonPath": SHELL_HISTORY_LATEST_JSON_PATH,
            "latestMarkdownPath": SHELL_HISTORY_LATEST_MD_PATH,
            "entries": entries,
        });
        fs::write(
            self.root.join(SHELL_HISTORY_LATEST_JSON_FILE),
            format!("{}\n", serde_json_pretty(&latest)?),
        )?;
        fs::write(
            self.root.join(SHELL_HISTORY_LATEST_MD_FILE),
            shell_history_markdown(&latest),
        )?;

        Ok(shell_history_paths())
    }
}

fn shell_history_entry(
    context: &ShellHistoryContext,
    observed_at: u64,
    record: &ShellCommandObservation,
    operation: Option<&ShellMutationOperation>,
) -> Value {
    let mut entry = Map::new();
    entry.insert("schema".to_owned(), json!(SHELL_HISTORY_SCHEMA));
    entry.insert("observedAtUnixMillis".to_owned(), json!(observed_at));
    entry.insert("taskId".to_owned(), json!(context.task_id.as_str()));
    entry.insert("terminalId".to_owned(), json!(context.terminal_id.as_str()));
    entry.insert("cwd".to_owned(), json!(context.cwd.as_str()));
    entry.insert("command".to_owned(), json!(record.command.as_deref()));
    entry.insert(
        "outcome".to_owned(),
        shell_history_outcome(record, operation),
    );
    if let Some(operation) = operation {
        entry.insert("operation".to_owned(), shell_history_operation(operation));
    }
    Value::Object(entry)
}

fn shell_history_outcome(
    record: &ShellCommandObservation,
    operation: Option<&ShellMutationOperation>,
) -> Value {
    let changed = operation.is_some_and(|operation| operation.status == "changed");
    let status = if record.diagnostic.is_some() || record.exit_code.is_some_and(|code| code != 0) {
        "error"
    } else {
        "ok"
    };
    let mut outcome = Map::new();
    outcome.insert("status".to_owned(), json!(status));
    outcome.insert("changed".to_owned(), json!(changed));
    insert_optional_string(&mut outcome, "evidence", &record.evidence);
    insert_optional_string(&mut outcome, "diagnostic", &record.diagnostic);
    if let Some(exit_code) = record.exit_code {
        outcome.insert("exitCode".to_owned(), json!(exit_code));
    }
    insert_optional_string(&mut outcome, "terminalOutput", &record.terminal_output);
    Value::Object(outcome)
}

fn shell_history_operation(operation: &ShellMutationOperation) -> Value {
    let mut value = Map::new();
    value.insert("kind".to_owned(), json!(operation.kind.as_str()));
    value.insert("status".to_owned(), json!(operation.status.as_str()));
    value.insert("command".to_owned(), json!(operation.command.as_str()));
    insert_optional_string(&mut value, "source", &operation.source);
    insert_optional_string(&mut value, "target", &operation.target);
    value.insert("paths".to_owned(), json!(&operation.paths));
    Value::Object(value)
}

fn insert_optional_string(map: &mut Map<String, Value>, key: &str, value: &Option<String>) {
    if let Some(value) = value {
        map.insert(key.to_owned(), json!(value));
    }
}

fn shell_history_markdown(latest: &Value) -> String {
    let entries = latest
        .get("entries")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    [
        "# Wanix qjs Shell Command History".to_owned(),
        String::new(),
        format!("Schema: `{SHELL_HISTORY_SCHEMA}`"),
        format!("JSONL: `{SHELL_HISTORY_COMMANDS_PATH}`"),
        format!("Latest JSON: `{SHELL_HISTORY_LATEST_JSON_PATH}`"),
        String::new(),
        "## Latest Commands".to_owned(),
        String::new(),
        "| Command | Status | Changed | Target | Evidence |".to_owned(),
        "| --- | --- | --- | --- | --- |".to_owned(),
    ]
    .into_iter()
    .chain(entries.iter().map(|entry| shell_history_markdown_row(entry)))
    .chain([
        String::new(),
        "The JSONL file is append-only for agents; this Markdown file shows the latest batch from the served qjs-shell.".to_owned(),
        String::new(),
    ])
    .collect::<Vec<_>>()
    .join("\n")
}

fn shell_history_markdown_row(entry: &Value) -> String {
    let outcome = entry.get("outcome").unwrap_or(&Value::Null);
    let operation = entry.get("operation").unwrap_or(&Value::Null);
    format!(
        "| {} | {} | {} | {} | {} |",
        markdown_cell(entry.get("command").and_then(Value::as_str)),
        markdown_cell(outcome.get("status").and_then(Value::as_str)),
        markdown_cell(
            outcome
                .get("changed")
                .and_then(Value::as_bool)
                .map(|changed| if changed { "true" } else { "false" })
        ),
        markdown_cell(
            operation
                .get("target")
                .and_then(Value::as_str)
                .or_else(|| operation.get("source").and_then(Value::as_str))
        ),
        markdown_cell(outcome.get("evidence").and_then(Value::as_str)),
    )
}

fn markdown_cell(value: Option<&str>) -> String {
    value
        .unwrap_or("")
        .replace(['\n', '\r'], " ")
        .replace('|', "\\|")
}

fn shell_history_paths() -> Vec<String> {
    vec![
        SHELL_HISTORY_COMMANDS_PATH.to_owned(),
        SHELL_HISTORY_LATEST_JSON_PATH.to_owned(),
        SHELL_HISTORY_LATEST_MD_PATH.to_owned(),
    ]
}

fn unix_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| u64::try_from(duration.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

fn serde_json_string(value: &Value) -> io::Result<String> {
    serde_json::to_string(value).map_err(io::Error::other)
}

fn serde_json_pretty(value: &Value) -> io::Result<String> {
    serde_json::to_string_pretty(value).map_err(io::Error::other)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    use serde_json::Value;

    use super::{
        SHELL_HISTORY_COMMANDS_PATH, SHELL_HISTORY_LATEST_JSON_PATH, SHELL_HISTORY_LATEST_MD_PATH,
        ShellHistoryContext, ShellHistoryWriter,
    };
    use crate::serve::terminal_ws::shell_activity::ShellMutationOperation;
    use crate::serve::terminal_ws::shell_observation::ShellCommandObservation;

    #[test]
    fn writes_append_only_jsonl_latest_json_and_markdown() {
        let root = unique_temp_dir("wanix-shell-history");
        fs::create_dir(&root).unwrap();
        let writer = ShellHistoryWriter::new(&root);
        let context = ShellHistoryContext {
            task_id: "7".to_owned(),
            terminal_id: "3".to_owned(),
            cwd: "/app".to_owned(),
        };
        let records = vec![ShellCommandObservation {
            command: Some("rm missing.txt".to_owned()),
            evidence: Some("qjs-shell-command-record".to_owned()),
            diagnostic: Some("rm: missing.txt: errno -44".to_owned()),
            exit_code: Some(0),
            terminal_output: Some("rm: missing.txt: errno -44".to_owned()),
        }];
        let operations = vec![ShellMutationOperation {
            kind: "rm".to_owned(),
            command: "rm missing.txt".to_owned(),
            status: "unchanged".to_owned(),
            evidence: Some("qjs-shell-command-record".to_owned()),
            diagnostic: Some("rm: missing.txt: errno -44".to_owned()),
            exit_code: Some(0),
            terminal_output: Some("rm: missing.txt: errno -44".to_owned()),
            source: None,
            target: Some("/app/missing.txt".to_owned()),
            paths: Vec::new(),
        }];

        let paths = writer.write(&context, &records, &operations).unwrap();

        assert_eq!(
            paths,
            vec![
                SHELL_HISTORY_COMMANDS_PATH.to_owned(),
                SHELL_HISTORY_LATEST_JSON_PATH.to_owned(),
                SHELL_HISTORY_LATEST_MD_PATH.to_owned(),
            ]
        );
        let jsonl = fs::read_to_string(root.join(".wanix/qjs-shell/commands.jsonl")).unwrap();
        let entry: Value = serde_json::from_str(jsonl.trim()).unwrap();
        assert_eq!(entry["schema"], "wanix.qjs-shell.command-history.v1");
        assert_eq!(entry["taskId"], "7");
        assert_eq!(entry["command"], "rm missing.txt");
        assert_eq!(entry["operation"]["status"], "unchanged");
        assert_eq!(entry["operation"]["target"], "/app/missing.txt");
        assert_eq!(entry["outcome"]["status"], "error");
        assert_eq!(entry["outcome"]["changed"], false);

        let latest = fs::read_to_string(root.join(".wanix/qjs-shell/latest.json")).unwrap();
        assert!(latest.contains("\"historyPath\": \"/.wanix/qjs-shell/commands.jsonl\""));
        let markdown = fs::read_to_string(root.join(".wanix/qjs-shell/latest.md")).unwrap();
        assert!(markdown.contains("# Wanix qjs Shell Command History"));
        assert!(markdown.contains(
            "| rm missing.txt | error | false | /app/missing.txt | qjs-shell-command-record |"
        ));

        fs::remove_dir_all(root).unwrap();
    }

    fn unique_temp_dir(label: &str) -> std::path::PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("{label}-{}-{nanos}", std::process::id()))
    }
}
