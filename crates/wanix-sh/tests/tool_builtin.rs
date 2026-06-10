//! Host-side tests for the `tool` builtin against a REAL in-process ToolFS.
//!
//! The shell's [`NamespaceOps`] surface is backed here by `ToolNs`: plain
//! files live in a map, while paths under a mount prefix (`/n/upper`, …)
//! route into mounted [`wanix_tool::ToolFs`] views — the same way the wasm
//! guest reaches a mesh-mounted tool through WASI. The builtin therefore
//! exercises the genuine job-protocol surface (allocate, in, params.json,
//! ctl run, out, result.json, close), not a fake of it.

use std::collections::HashMap;

use wanix_fs::{FileSystem, NormalizedPath, OpenOptions};
use wanix_sh::{
    NamespaceOps, ShellError, ShellResult, ShellState, SpawnHandle, SpawnSpec, run_line,
};
use wanix_tool::runners::{FailRunner, UpperRunner};
use wanix_tool::{RunContext, RunOutcome, ToolPrincipal, ToolRunner, ToolService, ToolSpec};

/// Appends the params' `"suffix"` string to the input (exercises params.json).
struct SuffixRunner;

impl ToolRunner for SuffixRunner {
    fn run(
        &self,
        input: &[u8],
        params: Option<&serde_json::Value>,
        _ctx: &RunContext,
    ) -> RunOutcome {
        let suffix = params
            .and_then(|value| value.get("suffix"))
            .and_then(|value| value.as_str())
            .unwrap_or("");
        RunOutcome::success([input, suffix.as_bytes()].concat())
    }
}

fn mounted(name: &str, runner: Box<dyn ToolRunner>) -> (String, wanix_tool::ToolFs) {
    let service =
        ToolService::new(ToolSpec::v0(name, "test tool"), runner, Box::new(|| 0)).unwrap();
    let view = service.open_view(ToolPrincipal::local("shell"));
    (format!("/n/{name}"), view)
}

/// In-memory NamespaceOps with real ToolFS views mounted under `/n/<name>`.
#[derive(Default)]
struct ToolNs {
    out: Vec<u8>,
    err: Vec<u8>,
    files: HashMap<String, Vec<u8>>,
    mounts: Vec<(String, wanix_tool::ToolFs)>,
}

impl ToolNs {
    fn mount(&mut self, name: &str, runner: Box<dyn ToolRunner>) {
        self.mounts.push(mounted(name, runner));
    }

    /// Splits a mounted path into its ToolFs and the tool-relative path.
    fn route(&self, path: &str) -> Option<(&wanix_tool::ToolFs, NormalizedPath)> {
        for (prefix, fs) in &self.mounts {
            if let Some(rel) = path.strip_prefix(prefix.as_str()) {
                let rel = rel.trim_start_matches('/');
                let rel = if rel.is_empty() { "." } else { rel };
                return Some((fs, NormalizedPath::new(rel).expect("tool path")));
            }
        }
        None
    }
}

fn io_err(path: &str, err: impl std::fmt::Display) -> ShellError {
    ShellError::Io(format!("{path}: {err}"))
}

impl NamespaceOps for ToolNs {
    fn write_stdout(&mut self, bytes: &[u8]) -> ShellResult<()> {
        self.out.extend_from_slice(bytes);
        Ok(())
    }

    fn write_stderr(&mut self, bytes: &[u8]) -> ShellResult<()> {
        self.err.extend_from_slice(bytes);
        Ok(())
    }

    fn read_stdin(&mut self, _buf: &mut [u8]) -> ShellResult<usize> {
        Ok(0)
    }

    fn exists(&self, path: &str) -> ShellResult<bool> {
        if let Some((fs, rel)) = self.route(path) {
            return Ok(fs.metadata(&rel).is_ok());
        }
        Ok(self.files.contains_key(path))
    }

    fn read_file(&mut self, path: &str) -> ShellResult<Vec<u8>> {
        if let Some((fs, rel)) = self.route(path) {
            let mut file = fs
                .open(&rel, OpenOptions::read())
                .map_err(|err| io_err(path, err))?;
            let mut bytes = Vec::new();
            let mut buf = [0u8; 256];
            loop {
                let n = file.read(&mut buf).map_err(|err| io_err(path, err))?;
                if n == 0 {
                    return Ok(bytes);
                }
                bytes.extend_from_slice(&buf[..n]);
            }
        }
        self.files
            .get(path)
            .cloned()
            .ok_or_else(|| io_err(path, "not found"))
    }

    fn write_file(&mut self, path: &str, bytes: &[u8], append: bool) -> ShellResult<()> {
        if let Some((fs, rel)) = self.route(path) {
            let options = OpenOptions {
                write: true,
                create: true,
                truncate: !append,
                ..OpenOptions::default()
            };
            let mut file = fs.open(&rel, options).map_err(|err| io_err(path, err))?;
            file.write(bytes).map_err(|err| io_err(path, err))?;
            return Ok(());
        }
        let entry = self.files.entry(path.to_owned()).or_default();
        if !append {
            entry.clear();
        }
        entry.extend_from_slice(bytes);
        Ok(())
    }

    // Every stage in these tests is a builtin, so pipelines wire through
    // shell memory: no real pipes and no external launches are reachable.
    fn pipe_new(&mut self) -> ShellResult<String> {
        Err(ShellError::Io("no pipes in this test".into()))
    }
    fn pipe_read_all(&mut self, _id: &str) -> ShellResult<Vec<u8>> {
        Err(ShellError::Io("no pipes in this test".into()))
    }
    fn pipe_open_writer(&mut self, _id: &str) -> ShellResult<()> {
        Err(ShellError::Io("no pipes in this test".into()))
    }
    fn pipe_break_reader(&mut self, _id: &str) -> ShellResult<()> {
        Err(ShellError::Io("no pipes in this test".into()))
    }
    fn pipe_write_all_and_close(&mut self, _id: &str, _bytes: &[u8]) -> ShellResult<()> {
        Err(ShellError::Io("no pipes in this test".into()))
    }
    fn spawn_start(&mut self, _spec: &SpawnSpec) -> ShellResult<SpawnHandle> {
        Err(ShellError::Io("no externals in this test".into()))
    }
    fn spawn_wait(&mut self, _handle: &SpawnHandle) -> ShellResult<i32> {
        Err(ShellError::Io("no externals in this test".into()))
    }
}

fn upper_ns() -> ToolNs {
    let mut ns = ToolNs::default();
    ns.mount("upper", Box::new(UpperRunner));
    ns.files.insert("notes.txt".into(), b"hello mesh".to_vec());
    ns
}

fn run(ns: &mut ToolNs, line: &str) -> i32 {
    run_line(line, &mut ShellState::new(), ns)
}

#[test]
fn tool_redirects_both_sides_and_closes_the_job() {
    let mut ns = upper_ns();
    let status = run(&mut ns, "tool /n/upper < notes.txt > NOTES.txt");
    assert_eq!(status, 0, "stderr: {:?}", String::from_utf8_lossy(&ns.err));
    assert_eq!(ns.files.get("NOTES.txt").unwrap(), b"HELLO MESH");
    // Success keeps stderr to the single crash-resume breadcrumb.
    let err = String::from_utf8_lossy(&ns.err);
    assert!(
        err.starts_with("job: /n/upper/jobs/") && err.trim_end().lines().count() == 1,
        "only the job path breadcrumb on success: {err:?}"
    );
    // The one-shot helper closes its job: nothing is retained under jobs/.
    let (fs, _) = ns.route("/n/upper").unwrap();
    assert!(
        fs.read_dir(&NormalizedPath::new("jobs").unwrap())
            .unwrap()
            .is_empty(),
        "close removed the retained job"
    );
}

#[test]
fn tool_is_pipeable_on_both_sides() {
    // cat (builtin) | tool (ns builtin) | cat (builtin): all in-shell stages
    // exchanging through memory, with the real ToolFS in the middle.
    let mut ns = upper_ns();
    let status = run(&mut ns, "cat < notes.txt | tool /n/upper | cat");
    assert_eq!(status, 0, "stderr: {:?}", String::from_utf8_lossy(&ns.err));
    assert_eq!(String::from_utf8_lossy(&ns.out), "HELLO MESH");
}

#[test]
fn tool_params_form_writes_params_json_before_run() {
    let mut ns = upper_ns();
    ns.mount("suffix", Box::new(SuffixRunner));
    let status = run(&mut ns, "tool /n/suffix '{\"suffix\":\"!\"}' < notes.txt");
    assert_eq!(status, 0, "stderr: {:?}", String::from_utf8_lossy(&ns.err));
    assert_eq!(String::from_utf8_lossy(&ns.out), "hello mesh!");
}

#[test]
fn failed_job_reports_taxonomy_kind_on_stderr_and_short_circuits() {
    let mut ns = upper_ns();
    ns.mount("fail", Box::new(FailRunner));
    let status = run(&mut ns, "tool /n/fail < notes.txt && echo never");
    assert_eq!(status, 2, "a failed job exits with its recorded exitCode");
    let err = String::from_utf8_lossy(&ns.err);
    let mut lines = err.lines();
    assert!(
        lines
            .next()
            .is_some_and(|line| line.starts_with("job: /n/fail/jobs/")),
        "the job path breadcrumb comes first: {err:?}"
    );
    assert_eq!(
        lines.next(),
        Some("tool: runner_failed: fail runner always fails"),
        "one human line with the taxonomy kind visible: {err:?}"
    );
    assert_eq!(
        lines.next(),
        Some("deliberate failure"),
        "the job's err diagnostics follow the taxonomy line: {err:?}"
    );
    assert!(ns.out.is_empty(), "&& short-circuits on the failure");
}

#[test]
fn failed_job_status_flows_to_dollar_question() {
    let mut ns = upper_ns();
    ns.mount("fail", Box::new(FailRunner));
    let mut state = ShellState::new();
    let status = run_line(
        "tool /n/fail < notes.txt; echo status=$?",
        &mut state,
        &mut ns,
    );
    assert_eq!(status, 0, "echo is the final command");
    assert_eq!(
        String::from_utf8_lossy(&ns.out),
        "status=2\n",
        "$? carries the job's recorded exitCode"
    );
}

#[test]
fn invalid_input_failure_uses_the_invalid_input_kind() {
    let mut ns = upper_ns();
    ns.files
        .insert("bad.bin".into(), vec![0xff, 0xfe, 0x80, 0x81]);
    let status = run(&mut ns, "tool /n/upper < bad.bin");
    assert_eq!(status, 1, "UpperRunner records exit code 1");
    let err = String::from_utf8_lossy(&ns.err);
    let tail = err
        .split_once('\n')
        .map(|(breadcrumb, tail)| {
            assert!(breadcrumb.starts_with("job: /n/upper/jobs/"), "{err:?}");
            tail
        })
        .unwrap_or_default();
    assert_eq!(
        tail,
        "tool: invalid_input: input is not valid UTF-8\ninput is not valid UTF-8\n"
    );
}

#[test]
fn missing_path_is_a_usage_error() {
    let mut ns = upper_ns();
    let status = run(&mut ns, "tool");
    assert_eq!(status, 2);
    assert_eq!(
        String::from_utf8_lossy(&ns.err),
        "usage: tool PATH [PARAMS_JSON]\n"
    );
}

#[test]
fn unmounted_tool_path_is_an_honest_io_error() {
    let mut ns = upper_ns();
    let status = run(&mut ns, "tool /n/missing < notes.txt");
    assert_eq!(status, 1);
    let err = String::from_utf8_lossy(&ns.err);
    assert!(
        err.starts_with("tool: /n/missing/new:"),
        "the failing protocol file is named: {err:?}"
    );
}
