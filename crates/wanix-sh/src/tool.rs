//! The `tool` builtin: a one-shot job-protocol client (`docs/toolfs.md`).
//!
//! `tool PATH [PARAMS_JSON]` drives a mounted ToolFS (or any ADR 0009 job
//! device) at `PATH` through its *visible file protocol* — it is sugar over
//! the files, never a bypass of them:
//!
//! ```text
//! id=$(cat PATH/new)
//! stdin                 > PATH/jobs/$id/in
//! PARAMS_JSON           > PATH/jobs/$id/params.json   (optional second form)
//! echo run              > PATH/jobs/$id/ctl           (synchronous: terminal on return)
//! cat PATH/jobs/$id/out                               -> stdout
//! cat PATH/jobs/$id/result.json                       -> success or one-line error
//! echo close            > PATH/jobs/$id/ctl           (best-effort cleanup)
//! ```
//!
//! Crash-resume: the job path is printed to stderr (`job: PATH/jobs/<id>`) as
//! soon as the job is allocated, so a successor of a crashed caller can find
//! the retained job directory and resume from `status`/`result.json`.
//!
//! On a failed job the taxonomy stays visible: stderr carries one line in the
//! shape `tool: <error.kind>: <error.message>` (e.g. `tool: invalid_input:
//! input is not valid UTF-8`), followed by the job's `err` diagnostics, and
//! the builtin exits with the job's recorded `exitCode` when the result has a
//! nonzero one (else 1). The trailing `close` is best-effort — a close
//! failure never masks the real outcome.

use crate::ns::NamespaceOps;
use crate::state::ShellState;

const USAGE: &[u8] = b"usage: tool PATH [PARAMS_JSON]\n";

/// The `tool` builtin body (registered in [`crate::builtins::ns_builtin`]).
pub(crate) fn tool(
    argv: &[String],
    stdin: &[u8],
    _state: &ShellState,
    ns: &mut dyn NamespaceOps,
) -> (Vec<u8>, i32) {
    let (path, params) = match (argv.get(1), argv.get(2), argv.len()) {
        (Some(path), params, 2 | 3) => (path.trim_end_matches('/'), params),
        _ => {
            let _ = ns.write_stderr(USAGE);
            return (Vec::new(), 2);
        }
    };
    match run_job(path, params.map(String::as_str), stdin, ns) {
        Ok(out) => (out, 0),
        Err(failure) => {
            let _ = ns.write_stderr(format!("tool: {}\n", failure.message).as_bytes());
            // Pass the job's diagnostics through after the taxonomy line, so
            // the runner's own explanation is not lost behind the summary.
            if !failure.diagnostics.is_empty() {
                let _ = ns.write_stderr(&failure.diagnostics);
            }
            let status = failure.exit_code.filter(|code| *code != 0).unwrap_or(1);
            (Vec::new(), status)
        }
    }
}

/// A failed invocation: the one-line summary, the job's recorded exit code
/// (when its result carried one), and its `err` diagnostics.
struct ToolFailure {
    message: String,
    exit_code: Option<i32>,
    diagnostics: Vec<u8>,
}

/// Protocol-level errors (unreachable files, malformed ids) carry no job
/// result, hence no exit code and no diagnostics.
impl From<String> for ToolFailure {
    fn from(message: String) -> Self {
        Self {
            message,
            exit_code: None,
            diagnostics: Vec::new(),
        }
    }
}

/// Allocates a job, drives it to a terminal state, and always issues the
/// best-effort `close` — whatever `drive_job` decided is the outcome.
fn run_job(
    path: &str,
    params: Option<&str>,
    stdin: &[u8],
    ns: &mut dyn NamespaceOps,
) -> Result<Vec<u8>, ToolFailure> {
    let new = format!("{path}/new");
    let id_bytes = ns.read_file(&new).map_err(|err| err.to_string())?;
    let id = String::from_utf8_lossy(&id_bytes).trim().to_owned();
    if id.is_empty() || id.contains('/') {
        return Err(format!("{new}: did not return a job id").into());
    }
    let job = format!("{path}/jobs/{id}");
    // Crash-resume breadcrumb: name the retained job directory up front, so
    // a successor of a crashed caller can pick the job back up.
    let _ = ns.write_stderr(format!("job: {job}\n").as_bytes());
    let outcome = drive_job(&job, params, stdin, ns);
    let _ = ns.write_file(&format!("{job}/ctl"), b"close\n", false);
    outcome
}

/// The in → params.json → ctl run → out → result.json dance for one job.
fn drive_job(
    job: &str,
    params: Option<&str>,
    stdin: &[u8],
    ns: &mut dyn NamespaceOps,
) -> Result<Vec<u8>, ToolFailure> {
    let write = |ns: &mut dyn NamespaceOps, file: &str, bytes: &[u8]| {
        ns.write_file(&format!("{job}/{file}"), bytes, false)
            .map_err(|err| err.to_string())
    };
    write(ns, "in", stdin)?;
    if let Some(params) = params {
        write(ns, "params.json", params.as_bytes())?;
    }
    // `ctl run` is synchronous (ADR 0009): the job is terminal on return, so
    // `out` and `result.json` reads below see final data.
    write(ns, "ctl", b"run\n")?;
    let out = ns
        .read_file(&format!("{job}/out"))
        .map_err(|err| err.to_string())?;
    let result_bytes = ns
        .read_file(&format!("{job}/result.json"))
        .map_err(|err| err.to_string())?;
    let result = String::from_utf8_lossy(&result_bytes);
    if json_str_field(&result, "state").as_deref() == Some("done") {
        return Ok(out);
    }
    // Failed/aborted: surface the taxonomy kind and message in one line,
    // carry the job's diagnostics and recorded exit code to the caller.
    let kind = json_str_field(&result, "kind").unwrap_or_else(|| "internal".to_owned());
    let message =
        json_str_field(&result, "message").unwrap_or_else(|| "job did not succeed".to_owned());
    let diagnostics = ns.read_file(&format!("{job}/err")).unwrap_or_default();
    Err(ToolFailure {
        message: format!("{kind}: {message}"),
        exit_code: json_int_field(&result, "exitCode").and_then(|code| i32::try_from(code).ok()),
        diagnostics,
    })
}

/// Extracts the first string value of `"key"` from a JSON document.
///
/// A deliberately tiny scanner: the shell carries no JSON dependency, and the
/// job protocol's `result.json` is a small flat shape whose `state`, `kind`,
/// and `message` keys are unambiguous in serialization order (ADR 0009).
fn json_str_field(json: &str, key: &str) -> Option<String> {
    parse_json_string(json_value_after(json, key)?.strip_prefix('"')?)
}

/// Extracts the first integer value of `"key"` (e.g. `exitCode`); `null` or
/// non-numeric values are `None`.
fn json_int_field(json: &str, key: &str) -> Option<i64> {
    let value = json_value_after(json, key)?;
    let digits: &str = &value[..value
        .char_indices()
        .take_while(|(index, c)| c.is_ascii_digit() || (*index == 0 && *c == '-'))
        .count()];
    digits.parse().ok()
}

/// Positions the cursor just past `"key":`, skipping `key` appearing inside a
/// *value* (where no colon follows).
fn json_value_after<'a>(json: &'a str, key: &str) -> Option<&'a str> {
    let needle = format!("\"{key}\"");
    let mut rest = json;
    loop {
        let found = rest.find(&needle)?;
        rest = &rest[found + needle.len()..];
        let after = rest.trim_start();
        if let Some(value) = after.strip_prefix(':') {
            return Some(value.trim_start());
        }
    }
}

/// Parses a JSON string body (cursor just past the opening quote).
fn parse_json_string(body: &str) -> Option<String> {
    let mut out = String::new();
    let mut chars = body.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => return Some(out),
            '\\' => match chars.next()? {
                'n' => out.push('\n'),
                't' => out.push('\t'),
                'r' => out.push('\r'),
                'u' => {
                    let code: String = chars.by_ref().take(4).collect();
                    out.push(
                        u32::from_str_radix(&code, 16)
                            .ok()
                            .and_then(char::from_u32)?,
                    );
                }
                escaped => out.push(escaped),
            },
            _ => out.push(c),
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{json_int_field, json_str_field};

    const FAILED: &str = r#"{"state":"failed","exitCode":2,"durationMs":4,"inputBytes":12,
        "outputBytes":0,"error":{"kind":"invalid_input","message":"input is not valid UTF-8"},
        "retryable":false}"#;

    #[test]
    fn extracts_state_kind_and_message() {
        assert_eq!(json_str_field(FAILED, "state").as_deref(), Some("failed"));
        assert_eq!(
            json_str_field(FAILED, "kind").as_deref(),
            Some("invalid_input")
        );
        assert_eq!(
            json_str_field(FAILED, "message").as_deref(),
            Some("input is not valid UTF-8")
        );
    }

    #[test]
    fn non_string_and_missing_values_are_none() {
        let done = r#"{"state":"done","error":null,"retryable":true}"#;
        assert_eq!(json_str_field(done, "state").as_deref(), Some("done"));
        assert_eq!(json_str_field(done, "error"), None, "null is not a string");
        assert_eq!(json_str_field(done, "kind"), None);
    }

    #[test]
    fn skips_a_key_appearing_as_a_value() {
        // "state" appearing inside a *value* must not satisfy the key search.
        let tricky = r#"{"message":"no \"state\" here","state":"done"}"#;
        assert_eq!(json_str_field(tricky, "state").as_deref(), Some("done"));
        assert_eq!(
            json_str_field(tricky, "message").as_deref(),
            Some("no \"state\" here")
        );
    }

    #[test]
    fn extracts_integer_exit_code_and_rejects_null() {
        assert_eq!(json_int_field(FAILED, "exitCode"), Some(2));
        let aborted = r#"{"state":"aborted","exitCode":null,"durationMs":null}"#;
        assert_eq!(json_int_field(aborted, "exitCode"), None);
        assert_eq!(json_int_field(r#"{"code":-7}"#, "code"), Some(-7));
    }

    #[test]
    fn decodes_escapes() {
        let json = r#"{"message":"line\nbreak A"}"#;
        assert_eq!(
            json_str_field(json, "message").as_deref(),
            Some("line\nbreak A")
        );
    }
}
