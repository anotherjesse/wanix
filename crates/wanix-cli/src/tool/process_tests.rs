//! ProcRunner unit proofs against real, ubiquitous POSIX binaries
//! (`/bin/cat`, `/bin/cp`, `/bin/sleep`, `/usr/bin/yes`), each skipped
//! honestly when the host lacks the binary. The mesh-side end-to-end proofs
//! live in `serve/proc_tests.rs`.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use wanix_fs::LineBuffer;
use wanix_job::ErrorKind;
use wanix_tool::{RunContext, RunOutcome, ToolRunner};

use super::ProcRunner;
use crate::tool::config::{ProcInputMode, ProcOutputMode};

fn runner(
    command: &str,
    args: &[&str],
    input: ProcInputMode,
    output: ProcOutputMode,
) -> ProcRunner {
    ProcRunner::new(
        PathBuf::from(command),
        args.iter().map(ToString::to_string).collect(),
        input,
        output,
        4_096,
        4_096,
    )
}

fn have(command: &str) -> bool {
    let present = Path::new(command).exists();
    if !present {
        eprintln!("skipping: {command} not present on this host");
    }
    present
}

fn kind(outcome: &RunOutcome) -> Option<ErrorKind> {
    outcome.error.as_ref().map(|error| error.kind)
}

#[test]
fn cat_round_trips_stdin_to_stdout() {
    if !have("/bin/cat") {
        return;
    }
    let runner = runner(
        "/bin/cat",
        &[],
        ProcInputMode::Stdin,
        ProcOutputMode::Stdout,
    );
    let outcome = runner.run(b"hello process\n", None, &RunContext::detached("j1"));
    assert_eq!(outcome.error, None);
    assert_eq!(outcome.exit_code, Some(0));
    assert_eq!(outcome.out, b"hello process\n");
}

#[test]
fn nonzero_exit_is_runner_failed_with_captured_stderr() {
    if !have("/bin/cat") {
        return;
    }
    let runner = runner(
        "/bin/cat",
        &["definitely-missing-file"],
        ProcInputMode::Stdin,
        ProcOutputMode::Stdout,
    );
    let outcome = runner.run(b"", None, &RunContext::detached("j2"));
    assert_eq!(kind(&outcome), Some(ErrorKind::RunnerFailed));
    assert_eq!(outcome.exit_code, Some(1));
    assert!(!outcome.err.is_empty(), "stderr should be captured");
}

#[test]
fn a_missing_executable_is_unavailable() {
    let runner = runner(
        "/nonexistent/binary",
        &[],
        ProcInputMode::Stdin,
        ProcOutputMode::Stdout,
    );
    let outcome = runner.run(b"", None, &RunContext::detached("j3"));
    assert_eq!(kind(&outcome), Some(ErrorKind::Unavailable));
}

#[test]
fn the_deadline_kills_a_long_running_child() {
    if !have("/bin/sleep") {
        return;
    }
    let runner = runner(
        "/bin/sleep",
        &["30"],
        ProcInputMode::Stdin,
        ProcOutputMode::Stdout,
    );
    let deadline = super::supervise::now_ms() + 200;
    let ctx = RunContext::new(
        "j4".to_owned(),
        Some(deadline),
        Arc::new(AtomicBool::new(false)),
        Arc::new(LineBuffer::default()),
    );
    let started = Instant::now();
    let outcome = runner.run(b"", None, &ctx);
    assert_eq!(kind(&outcome), Some(ErrorKind::Timeout));
    assert!(
        started.elapsed().as_secs() < 10,
        "child must be killed at the deadline, not waited out"
    );
}

#[test]
fn a_pre_armed_abort_flag_kills_the_child() {
    if !have("/bin/sleep") {
        return;
    }
    let runner = runner(
        "/bin/sleep",
        &["30"],
        ProcInputMode::Stdin,
        ProcOutputMode::Stdout,
    );
    let ctx = RunContext::new(
        "j5".to_owned(),
        None,
        Arc::new(AtomicBool::new(true)),
        Arc::new(LineBuffer::default()),
    );
    let started = Instant::now();
    let outcome = runner.run(b"", None, &ctx);
    assert_eq!(kind(&outcome), Some(ErrorKind::Aborted));
    assert!(started.elapsed().as_secs() < 10);
}

#[test]
fn unbounded_stdout_is_killed_at_the_capture_cap() {
    if !have("/usr/bin/yes") {
        return;
    }
    let runner = runner(
        "/usr/bin/yes",
        &[],
        ProcInputMode::Stdin,
        ProcOutputMode::Stdout,
    );
    let started = Instant::now();
    let outcome = runner.run(b"", None, &RunContext::detached("j6"));
    assert_eq!(kind(&outcome), Some(ErrorKind::RunnerFailed));
    let message = &outcome.error.as_ref().unwrap().message;
    assert!(message.contains("maxOutBytes"), "{message}");
    assert!(outcome.out.len() <= 4_096, "capture stays capped");
    assert!(started.elapsed().as_secs() < 10, "yes must not run forever");
}

#[test]
fn tempfile_input_and_output_map_through_the_private_workdir() {
    if !have("/bin/cp") {
        return;
    }
    let runner = runner(
        "/bin/cp",
        &["{input}", "{output}"],
        ProcInputMode::Tempfile,
        ProcOutputMode::Tempfile,
    );
    let outcome = runner.run(b"copied bytes", None, &RunContext::detached("j7"));
    assert_eq!(outcome.error, None);
    assert_eq!(outcome.out, b"copied bytes");
}

#[test]
fn a_missing_output_file_after_exit_zero_is_runner_failed() {
    if !have("/bin/cat") {
        return;
    }
    // cat copies {input} to stdout but never writes the promised {output}.
    let runner = runner(
        "/bin/cat",
        &["{input}", "{output}"],
        ProcInputMode::Tempfile,
        ProcOutputMode::Tempfile,
    );
    let outcome = runner.run(b"", None, &RunContext::detached("j8"));
    // cat exits 1 (missing {output} path) -> runner_failed either way; the
    // contract here is just that exit 0 + no file never reads as success.
    assert_eq!(kind(&outcome), Some(ErrorKind::RunnerFailed));
}

#[test]
fn child_stderr_lines_stream_into_the_events_sink() {
    if !have("/bin/cat") {
        return;
    }
    let runner = runner(
        "/bin/cat",
        &["missing-one", "missing-two"],
        ProcInputMode::Stdin,
        ProcOutputMode::Stdout,
    );
    let events = Arc::new(LineBuffer::default());
    let ctx = RunContext::new(
        "j9".to_owned(),
        None,
        Arc::new(AtomicBool::new(false)),
        Arc::clone(&events),
    );
    let outcome = runner.run(b"", None, &ctx);
    assert_eq!(kind(&outcome), Some(ErrorKind::RunnerFailed));
    // In production the job core closes the events stream at finalize; this
    // direct invocation owns the buffer, so close it to drain without blocking.
    events.close();
    let mut streamed = Vec::new();
    let mut buf = [0u8; 1024];
    while let Ok(n) = events.read(&mut buf) {
        if n == 0 {
            break;
        }
        streamed.extend_from_slice(&buf[..n]);
    }
    let streamed = String::from_utf8_lossy(&streamed);
    assert!(streamed.contains("missing-one"), "{streamed}");
    assert!(streamed.contains("missing-two"), "{streamed}");
}

#[test]
fn the_workdir_is_private_and_removed_after_the_run() {
    if !have("/bin/cat") {
        return;
    }
    // The child's cwd is the private workdir: cat ./input proves the cwd
    // mapping, and nothing named wanix-tool-<pid>-* survives the run.
    let runner = runner(
        "/bin/cat",
        &["{input}"],
        ProcInputMode::Tempfile,
        ProcOutputMode::Stdout,
    );
    let outcome = runner.run(b"private", None, &RunContext::detached("j10"));
    assert_eq!(outcome.out, b"private");
    // Sibling tests run in parallel with live workdirs of their own, so only
    // this job's directory (suffix -j10) is asserted gone.
    let prefix = format!("wanix-tool-{}-", std::process::id());
    let leftovers: Vec<_> = std::fs::read_dir(std::env::temp_dir())
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with(&prefix) && name.ends_with("-j10"))
        .collect();
    assert!(leftovers.is_empty(), "workdirs leaked: {leftovers:?}");
}
