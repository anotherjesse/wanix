use super::{ProcInputMode, ProcOutputMode, ToolConfigFile, load_tool_config};

fn parse(text: &str) -> Result<ToolConfigFile, crate::CliError> {
    let dir = std::env::temp_dir().join(format!(
        "wanix-tool-config-test-{}-{:p}",
        std::process::id(),
        &text
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("tools.toml");
    std::fs::write(&path, text).unwrap();
    let result = load_tool_config(&path);
    let _ = std::fs::remove_dir_all(&dir);
    result
}

#[test]
fn parses_the_docs_sketch_and_maps_it_onto_the_spec() {
    let file = parse(
        r#"
[tools.upper]
description = "Uppercase UTF-8 text"
command = "/usr/bin/tr"
args = ["[:lower:]", "[:upper:]"]
input = "stdin"
output = "stdout"
visibility = "private"

[tools.upper.limits]
max_input_bytes = 1048576
run_timeout_ms = 5000
max_concurrent_per_principal = 2

[tools.upper.lifecycle]
allocated_ttl_ms = 300000
retain_done_ms = 600000
retain_failed_ms = 3600000
"#,
    )
    .unwrap();
    assert_eq!(file.names(), vec!["upper".to_owned()]);
    let tool = file.get("upper").unwrap();
    assert_eq!(tool.input, ProcInputMode::Stdin);
    assert_eq!(tool.output, ProcOutputMode::Stdout);
    let spec = tool.tool_spec("upper");
    assert_eq!(spec.envelope.name, "upper");
    assert_eq!(spec.envelope.description, "Uppercase UTF-8 text");
    assert_eq!(spec.input.max_bytes, 1_048_576);
    assert_eq!(spec.limits.run_timeout_ms, 5_000);
    assert_eq!(spec.limits.max_concurrent_per_principal, 2);
    assert_eq!(spec.lifecycle.allocated_ttl_ms, 300_000);
    assert_eq!(spec.lifecycle.retain_done_ms, 600_000);
    assert_eq!(spec.lifecycle.retain_failed_ms, 3_600_000);
    // Unset knobs keep the shipped defaults.
    assert_eq!(spec.limits.max_out_bytes, 16_777_216);
}

#[test]
fn defaults_are_stdin_stdout_private_with_shipped_limits() {
    let file = parse("[tools.cat]\ndescription = \"copy\"\ncommand = \"/bin/cat\"\n").unwrap();
    let tool = file.get("cat").unwrap();
    assert_eq!(tool.input, ProcInputMode::Stdin);
    assert_eq!(tool.output, ProcOutputMode::Stdout);
    let spec = tool.tool_spec("cat");
    assert_eq!(spec.limits, wanix_jobfs::JobLimits::default());
    assert_eq!(spec.lifecycle, wanix_jobfs::JobLifecycle::default());
    assert_eq!(spec.visibility, wanix_tool::ToolVisibility::Private);
}

#[test]
fn tempfile_modes_accept_their_placeholders() {
    let file = parse(
        r#"
[tools.copy]
description = "copy a file"
command = "/bin/cp"
args = ["{input}", "{output}"]
input = "tempfile"
output = "tempfile"
"#,
    )
    .unwrap();
    let tool = file.get("copy").unwrap();
    assert_eq!(tool.input, ProcInputMode::Tempfile);
    assert_eq!(tool.output, ProcOutputMode::Tempfile);
}

#[test]
fn rejects_a_relative_command() {
    let error = parse("[tools.t]\ndescription = \"x\"\ncommand = \"tr\"\n").unwrap_err();
    assert!(error.to_string().contains("absolute"), "{error}");
}

#[test]
fn rejects_a_shell_looking_command() {
    for command in ["/bin/sh -c 'cat'", "/bin/cat|/bin/cat", "/bin/echo $HOME"] {
        let error = parse(&format!(
            "[tools.t]\ndescription = \"x\"\ncommand = {command:?}\n"
        ))
        .unwrap_err();
        assert!(error.to_string().contains("shell"), "{command:?}: {error}");
    }
}

#[test]
fn rejects_placeholders_outside_their_tempfile_mode() {
    // {input} with input = "stdin" (the default).
    let error =
        parse("[tools.t]\ndescription = \"x\"\ncommand = \"/bin/cat\"\nargs = [\"{input}\"]\n")
            .unwrap_err();
    assert!(error.to_string().contains("{input}"), "{error}");

    // {output} with output = "stdout" (the default).
    let error =
        parse("[tools.t]\ndescription = \"x\"\ncommand = \"/bin/cat\"\nargs = [\"{output}\"]\n")
            .unwrap_err();
    assert!(error.to_string().contains("{output}"), "{error}");
}

#[test]
fn rejects_a_missing_or_duplicated_required_placeholder() {
    // input = "tempfile" with no {input} arg.
    let error =
        parse("[tools.t]\ndescription = \"x\"\ncommand = \"/bin/cat\"\ninput = \"tempfile\"\n")
            .unwrap_err();
    assert!(error.to_string().contains("exactly one"), "{error}");

    let error = parse(
        "[tools.t]\ndescription = \"x\"\ncommand = \"/bin/cat\"\ninput = \"tempfile\"\n\
         args = [\"{input}\", \"{input}\"]\n",
    )
    .unwrap_err();
    assert!(error.to_string().contains("exactly one"), "{error}");
}

#[test]
fn rejects_embedded_and_unknown_placeholders() {
    let error = parse(
        "[tools.t]\ndescription = \"x\"\ncommand = \"/bin/cat\"\ninput = \"tempfile\"\n\
         args = [\"--file={input}\"]\n",
    )
    .unwrap_err();
    assert!(error.to_string().contains("whole argument"), "{error}");

    let error =
        parse("[tools.t]\ndescription = \"x\"\ncommand = \"/bin/cat\"\nargs = [\"{cmd}\"]\n")
            .unwrap_err();
    assert!(error.to_string().contains("unknown placeholder"), "{error}");
}

#[test]
fn rejects_non_private_visibility_and_bad_names() {
    let error =
        parse("[tools.t]\ndescription = \"x\"\ncommand = \"/bin/cat\"\nvisibility = \"public\"\n")
            .unwrap_err();
    assert!(error.to_string().contains("private"), "{error}");

    for name in [".hidden", "-flag", "a/b", "no spaces"] {
        let error = parse(&format!(
            "[tools.{name:?}]\ndescription = \"x\"\ncommand = \"/bin/cat\"\n"
        ))
        .unwrap_err();
        assert!(error.to_string().contains("tool name"), "{name:?}: {error}");
    }
}

#[test]
fn rejects_an_empty_config_unknown_keys_and_a_missing_file() {
    assert!(parse("").unwrap_err().to_string().contains("no [tools"));
    let error = parse("[tools.t]\ndescription = \"x\"\ncommand = \"/bin/cat\"\nshell = true\n")
        .unwrap_err();
    assert!(error.to_string().contains("shell"), "{error}");
    assert!(load_tool_config(std::path::Path::new("/nonexistent/tools.toml")).is_err());
}
