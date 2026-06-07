use serde_json::Value;

pub(in crate::serve) const QJS_SHELL_COMMAND_RECORD_EVIDENCE: &str = "qjs-shell-command-record";

const COMMAND_RECORD_PREFIX: &[u8] = b"\x1b]777;wanix-qjs-shell-command;";
const COMMAND_RECORD_END: u8 = 0x07;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::serve) struct ShellCommandRecord {
    pub(in crate::serve) command: String,
    pub(in crate::serve) diagnostic: Option<String>,
    pub(in crate::serve) exit_code: Option<i32>,
    pub(in crate::serve) terminal_output: Option<String>,
}

pub(in crate::serve) fn strip_shell_command_records(
    output: Vec<u8>,
) -> (Vec<u8>, Vec<ShellCommandRecord>) {
    let mut visible = Vec::with_capacity(output.len());
    let mut records = Vec::new();
    let mut index = 0;
    while index < output.len() {
        if output[index..].starts_with(COMMAND_RECORD_PREFIX) {
            let payload_start = index + COMMAND_RECORD_PREFIX.len();
            if let Some(payload_end) = output[payload_start..]
                .iter()
                .position(|byte| *byte == COMMAND_RECORD_END)
                .map(|offset| payload_start + offset)
            {
                if let Some(record) =
                    parse_shell_command_record(&output[payload_start..payload_end])
                {
                    records.push(record);
                }
                index = payload_end + 1;
                continue;
            }
        }
        visible.push(output[index]);
        index += 1;
    }
    (visible, records)
}

fn parse_shell_command_record(payload: &[u8]) -> Option<ShellCommandRecord> {
    let value: Value = serde_json::from_slice(payload).ok()?;
    if value.get("schema")?.as_str()? != "wanix.qjs-shell.command.v1" {
        return None;
    }
    let command = value.get("command")?.as_str()?.to_owned();
    if command.is_empty() {
        return None;
    }
    Some(ShellCommandRecord {
        command,
        diagnostic: optional_string(&value, "diagnostic"),
        exit_code: optional_i32(&value, "exitCode"),
        terminal_output: optional_string(&value, "terminalOutput"),
    })
}

fn optional_string(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

fn optional_i32(value: &Value, key: &str) -> Option<i32> {
    value
        .get(key)
        .and_then(Value::as_i64)
        .and_then(|number| i32::try_from(number).ok())
}

#[cfg(test)]
mod tests {
    use super::{
        QJS_SHELL_COMMAND_RECORD_EVIDENCE, ShellCommandRecord, strip_shell_command_records,
    };

    #[test]
    fn strips_and_decodes_shell_command_records() {
        assert_eq!(
            QJS_SHELL_COMMAND_RECORD_EVIDENCE,
            "qjs-shell-command-record"
        );
        let (visible, records) = strip_shell_command_records(
            b"write a\r\nwrote a\r\n\x1b]777;wanix-qjs-shell-command;{\"schema\":\"wanix.qjs-shell.command.v1\",\"command\":\"write a\",\"terminalOutput\":\"wrote a\",\"exitCode\":0}\x07$ ".to_vec(),
        );

        assert_eq!(visible, b"write a\r\nwrote a\r\n$ ");
        assert_eq!(
            records,
            vec![ShellCommandRecord {
                command: "write a".to_owned(),
                diagnostic: None,
                exit_code: Some(0),
                terminal_output: Some("wrote a".to_owned()),
            }]
        );
    }

    #[test]
    fn leaves_incomplete_records_visible() {
        let output = b"\x1b]777;wanix-qjs-shell-command;{\"command\":\"oops\"".to_vec();

        let (visible, records) = strip_shell_command_records(output.clone());

        assert_eq!(visible, output);
        assert!(records.is_empty());
    }
}
