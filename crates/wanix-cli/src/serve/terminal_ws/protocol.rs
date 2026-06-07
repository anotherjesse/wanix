use crate::qjs_term::QjsShellSession;

use super::shell_activity::ShellMutationOperation;

pub(super) fn shell_session_message(shell: &QjsShellSession) -> String {
    format!(
        "{{\"type\":\"session\",\"protocol\":\"wanix-qjs-shell.v1\",\
         \"taskId\":{},\"terminalId\":{},\"cwd\":{}}}",
        json_string(&shell.task_id()),
        json_string(shell.terminal_id()),
        json_string(shell.cwd().as_str()),
    )
}

pub(super) fn shell_mutation_message(
    shell: &QjsShellSession,
    paths: &[String],
    operations: &[ShellMutationOperation],
) -> String {
    let operations = operations
        .iter()
        .map(shell_operation_json)
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "{{\"type\":\"mutation\",\"protocol\":\"wanix-qjs-shell.v1\",\
         \"taskId\":{},\"terminalId\":{},\"cwd\":{},\"paths\":[{}],\
         \"operations\":[{}]}}",
        json_string(&shell.task_id()),
        json_string(shell.terminal_id()),
        json_string(shell.cwd().as_str()),
        paths
            .iter()
            .map(|path| json_string(path))
            .collect::<Vec<_>>()
            .join(","),
        operations,
    )
}

pub(super) fn shell_exit_message(code: i32) -> String {
    format!("{{\"type\":\"exit\",\"code\":{code}}}")
}

fn shell_operation_json(operation: &ShellMutationOperation) -> String {
    let mut fields = vec![
        format!("\"kind\":{}", json_string(&operation.kind)),
        format!("\"command\":{}", json_string(&operation.command)),
        format!("\"status\":{}", json_string(&operation.status)),
    ];
    if let Some(source) = &operation.source {
        fields.push(format!("\"source\":{}", json_string(source)));
    }
    if let Some(target) = &operation.target {
        fields.push(format!("\"target\":{}", json_string(target)));
    }
    if let Some(diagnostic) = &operation.diagnostic {
        fields.push(format!("\"diagnostic\":{}", json_string(diagnostic)));
    }
    if let Some(exit_code) = operation.exit_code {
        fields.push(format!("\"exitCode\":{exit_code}"));
    }
    if let Some(terminal_output) = &operation.terminal_output {
        fields.push(format!(
            "\"terminalOutput\":{}",
            json_string(terminal_output)
        ));
    }
    fields.push(format!(
        "\"outcome\":{}",
        shell_operation_outcome_json(operation)
    ));
    fields.push(format!(
        "\"paths\":[{}]",
        operation
            .paths
            .iter()
            .map(|path| json_string(path))
            .collect::<Vec<_>>()
            .join(",")
    ));
    format!("{{{}}}", fields.join(","))
}

fn shell_operation_outcome_json(operation: &ShellMutationOperation) -> String {
    let status = shell_operation_outcome_status(operation);
    let changed = operation.status == "changed";
    let mut fields = vec![
        format!("\"status\":{}", json_string(status)),
        format!("\"changed\":{changed}"),
    ];
    if let Some(diagnostic) = &operation.diagnostic {
        fields.push(format!("\"diagnostic\":{}", json_string(diagnostic)));
    }
    if let Some(exit_code) = operation.exit_code {
        fields.push(format!("\"exitCode\":{exit_code}"));
    }
    if let Some(terminal_output) = &operation.terminal_output {
        fields.push(format!(
            "\"terminalOutput\":{}",
            json_string(terminal_output)
        ));
    }
    format!("{{{}}}", fields.join(","))
}

fn shell_operation_outcome_status(operation: &ShellMutationOperation) -> &'static str {
    if operation.diagnostic.is_some() || operation.exit_code.is_some_and(|code| code != 0) {
        "error"
    } else if operation.status == "changed" {
        "ok"
    } else {
        "unchanged"
    }
}

fn json_string(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len() + 2);
    escaped.push('"');
    for ch in value.chars() {
        match ch {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            ch if ch.is_control() => {
                use std::fmt::Write as _;
                let _ = write!(escaped, "\\u{:04x}", ch as u32);
            }
            ch => escaped.push(ch),
        }
    }
    escaped.push('"');
    escaped
}
