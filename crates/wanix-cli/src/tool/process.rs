//! [`ProcRunner`]: a real host program behind one ToolFS device.
//!
//! The runner half of `docs/toolfs.md` §"Runner Boundary": the executable,
//! argv, and input/output mapping are fixed by host config (`tools.toml`),
//! never by the caller; the child runs with an empty environment in a fresh
//! private temp workdir (removed afterwards), its stdout/stderr capture is
//! bounded by the spec caps (the child is killed past them), the
//! [`RunContext`] deadline and abort flag both kill the child, stderr lines
//! stream into the job's `events` file as they arrive, and the child is
//! always reaped.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use serde_json::Value;
use wanix_job::{ErrorKind, JobError};
use wanix_tool::{RunContext, RunOutcome, ToolRunner};

use super::config::{ProcInputMode, ProcOutputMode};
use io::{Workdir, capture_capped};
use supervise::{Kill, supervise};

mod io;
mod supervise;

/// One configured host program serving one tool.
pub(crate) struct ProcRunner {
    command: PathBuf,
    args: Vec<String>,
    input: ProcInputMode,
    output: ProcOutputMode,
    max_out_bytes: u64,
    max_err_bytes: u64,
    /// Live children by job id, so the [`ToolRunner::abort`] hook can kill
    /// the exact child mid-run from another thread.
    children: Mutex<HashMap<String, Arc<Mutex<Child>>>>,
}

impl ProcRunner {
    pub(crate) fn new(
        command: PathBuf,
        args: Vec<String>,
        input: ProcInputMode,
        output: ProcOutputMode,
        max_out_bytes: u64,
        max_err_bytes: u64,
    ) -> Self {
        Self {
            command,
            args,
            input,
            output,
            max_out_bytes,
            max_err_bytes,
            children: Mutex::new(HashMap::new()),
        }
    }

    fn run_in(&self, workdir: &Workdir, input: &[u8], ctx: &RunContext) -> RunOutcome {
        let input_path = workdir.file("input");
        let output_path = workdir.file("output");
        if self.input == ProcInputMode::Tempfile
            && let Err(error) = std::fs::write(&input_path, input)
        {
            return fail(ErrorKind::Internal, format!("write input file: {error}"));
        }
        let argv = self.args.iter().map(|arg| match arg.as_str() {
            "{input}" => input_path.display().to_string(),
            "{output}" => output_path.display().to_string(),
            _ => arg.clone(),
        });
        let mut child = match Command::new(&self.command)
            .args(argv)
            .env_clear()
            .current_dir(workdir.path())
            .stdin(match self.input {
                ProcInputMode::Stdin => Stdio::piped(),
                ProcInputMode::Tempfile => Stdio::null(),
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(child) => child,
            Err(error) => {
                return fail(
                    ErrorKind::Unavailable,
                    format!("spawn {}: {error}", self.command.display()),
                );
            }
        };

        // Feed stdin from its own thread: the child may exit (or be killed)
        // without draining it, so the writer must never block the supervisor.
        let stdin_writer = child.stdin.take().map(|mut sink| {
            let bytes = input.to_vec();
            std::thread::spawn(move || {
                use std::io::Write as _;
                let _ = sink.write_all(&bytes);
            })
        });
        let out_full = Arc::new(AtomicBool::new(false));
        let err_full = Arc::new(AtomicBool::new(false));
        let stdout = child.stdout.take().expect("stdout was piped");
        let stderr = child.stderr.take().expect("stderr was piped");
        let out_thread = {
            let (cap, full) = (self.max_out_bytes, Arc::clone(&out_full));
            std::thread::spawn(move || capture_capped(stdout, cap, &full, None))
        };
        let err_thread = {
            let (cap, full, events) = (self.max_err_bytes, Arc::clone(&err_full), ctx.clone());
            std::thread::spawn(move || capture_capped(stderr, cap, &full, Some(&events)))
        };

        let child = Arc::new(Mutex::new(child));
        self.register(ctx.job_id(), Arc::clone(&child));
        let (status, kill) = supervise(&child, ctx, &out_full, &err_full);
        self.unregister(ctx.job_id());
        let out = out_thread.join().unwrap_or_default();
        let err = err_thread.join().unwrap_or_default();
        if let Some(writer) = stdin_writer {
            let _ = writer.join();
        }
        self.outcome(workdir, status, kill, out, err)
    }

    /// Maps the supervised exit to the job verdict. Kills carry the captured
    /// (capped) streams so a failed job stays inspectable.
    fn outcome(
        &self,
        workdir: &Workdir,
        status: Option<ExitStatus>,
        kill: Option<Kill>,
        out: Vec<u8>,
        err: Vec<u8>,
    ) -> RunOutcome {
        let exit_code = status.and_then(|status| status.code());
        let error = match kill {
            Some(Kill::Aborted) => Some(JobError::new(
                ErrorKind::Aborted,
                "aborted by caller; process killed",
            )),
            Some(Kill::Timeout) => Some(JobError::new(
                ErrorKind::Timeout,
                "run deadline exceeded; process killed",
            )),
            Some(Kill::OutCap) => Some(JobError::new(
                ErrorKind::RunnerFailed,
                format!(
                    "output exceeded maxOutBytes ({}); process killed",
                    self.max_out_bytes
                ),
            )),
            Some(Kill::ErrCap) => Some(JobError::new(
                ErrorKind::RunnerFailed,
                format!(
                    "diagnostics exceeded maxErrBytes ({}); process killed",
                    self.max_err_bytes
                ),
            )),
            None => match status {
                Some(status) if status.success() => None,
                Some(_) => Some(JobError::new(
                    ErrorKind::RunnerFailed,
                    match exit_code {
                        Some(code) => format!("process exited with status {code}"),
                        None => "process terminated by a signal".to_owned(),
                    },
                )),
                None => Some(JobError::new(ErrorKind::Internal, "wait on child failed")),
            },
        };
        if error.is_some() {
            return RunOutcome {
                out,
                err,
                exit_code,
                error,
            };
        }
        let out = match self.output {
            ProcOutputMode::Stdout => Ok(out),
            ProcOutputMode::Tempfile => workdir.read_output(self.max_out_bytes),
        };
        match out {
            Ok(out) => RunOutcome {
                out,
                err,
                exit_code,
                error: None,
            },
            Err(error) => RunOutcome {
                out: Vec::new(),
                err,
                exit_code,
                error: Some(error),
            },
        }
    }

    fn register(&self, job_id: &str, child: Arc<Mutex<Child>>) {
        if let Ok(mut children) = self.children.lock() {
            children.insert(job_id.to_owned(), child);
        }
    }

    fn unregister(&self, job_id: &str) {
        if let Ok(mut children) = self.children.lock() {
            children.remove(job_id);
        }
    }
}

impl ToolRunner for ProcRunner {
    /// Runs the fixed command once. `params.json` is not part of the process
    /// contract (there is no placeholder for it), so params are ignored.
    fn run(&self, input: &[u8], _params: Option<&Value>, ctx: &RunContext) -> RunOutcome {
        match Workdir::create(ctx.job_id()) {
            Ok(workdir) => self.run_in(&workdir, input, ctx),
            Err(error) => fail(
                ErrorKind::Internal,
                format!("create private workdir: {error}"),
            ),
        }
    }

    fn abort_supported(&self) -> bool {
        true
    }

    /// Kills the job's live child. The supervisor also polls the abort flag,
    /// so this hook only shortens the latency; either path reaps the child.
    fn abort(&self, job_id: &str) {
        let child = self
            .children
            .lock()
            .ok()
            .and_then(|children| children.get(job_id).cloned());
        if let Some(child) = child
            && let Ok(mut child) = child.lock()
        {
            let _ = child.kill();
        }
    }
}

fn fail(kind: ErrorKind, message: String) -> RunOutcome {
    RunOutcome::failure(JobError::new(kind, message), None, Vec::new())
}

#[cfg(test)]
#[path = "process_tests.rs"]
mod tests;
