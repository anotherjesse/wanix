use std::ffi::OsString;

use super::{parse_agent_command, run_agent_command};

fn os_args(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}

#[test]
fn agent_fake_streams_event_log() {
    let command = parse_agent_command(&os_args(&["--fake", "hi", "there"])).unwrap();
    let output = run_agent_command(command).unwrap();
    let log = String::from_utf8_lossy(output.stdout());
    assert!(log.contains("you said: hi there"), "{log}");
    assert!(log.contains("turn.completed"), "{log}");
}

#[test]
fn agent_requires_a_prompt() {
    let error = parse_agent_command(&os_args(&["--fake"])).unwrap_err();
    assert_eq!(error.exit_code(), 2);
}
