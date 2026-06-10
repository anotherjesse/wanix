//! Concurrent multi-stage pipeline execution (ADR 0010 tier 2).
//!
//! All external stages are launched first (`spawn_start`; on the host each
//! runs on its own thread), then builtin stages run in the shell left to
//! right, then every external is waited (`spawn_wait`) in stage order; the
//! pipeline status is the last stage's status (bash semantics). The bounded
//! `#pipe` back-pressures fast producers, so this ordering is deadlock-free:
//! every blocking pipe operation the shell performs has a live counterparty —
//! a running external on its own thread, or bytes a builtin already produced.
//! When a consumer stage never materializes (its launch failed, or a builtin
//! abort skipped it), its input pipe is broken on the consumer's behalf so the
//! producer observes a broken-pipe error (the `EPIPE` analog) instead of
//! blocking forever against a buffer nothing will ever drain.
//!
//! Two wiring rules keep the single-threaded shell honest with itself:
//! adjacent builtins exchange bytes through shell memory, never a real pipe
//! (the shell cannot drain a pipe it is blocked writing); and a builtin
//! producer's write end is held open *before* consumers launch, so a
//! concurrent consumer cannot read a premature EOF.

use crate::builtins::{builtin, is_pipeable_builtin, ns_builtin};
use crate::error::ShellResult;
use crate::exec::{COMMAND_NOT_FOUND_STATUS, expand_argv};
use crate::lower::{RedirectOp, Stage};
use crate::ns::{InputSource, NamespaceOps, OutputSink, SpawnHandle, SpawnSpec};
use crate::state::ShellState;

/// Where one stage reads from.
enum StageIn {
    /// The shell's stdin (stage 0) — builtins treat it as empty.
    Inherit,
    /// A real `#pipe` channel.
    Pipe(String),
    /// A `<` redirect target.
    File(String),
    /// Shell-memory bytes from the adjacent builtin (gap index).
    Memory(usize),
    /// Nothing: the producer's output was redirected away (builtins only).
    Empty,
}

/// Where one stage writes to.
enum StageOut {
    /// The shell's stdout (last stage).
    Inherit,
    /// A real `#pipe` channel.
    Pipe(String),
    /// A `>`/`>>` redirect target.
    File { path: String, append: bool },
    /// Shell-memory bytes for the adjacent builtin (gap index).
    Memory(usize),
    /// Nowhere: the consumer's input was redirected away (builtins only).
    Discard,
}

struct StagePlan {
    argv: Vec<String>,
    is_builtin: bool,
    stdin: StageIn,
    stdout: StageOut,
}

/// Runs a multi-stage pipeline and returns the last stage's exit status.
pub(crate) fn run_stages(
    stages: &[Stage],
    state: &ShellState,
    ns: &mut dyn NamespaceOps,
) -> ShellResult<i32> {
    let (plans, orphans) = wire(stages, state, ns)?;
    // Hold builtin-producer write ends before any consumer can observe EOF.
    for plan in &plans {
        if plan.is_builtin
            && let StageOut::Pipe(id) = &plan.stdout
        {
            ns.pipe_open_writer(id)?;
        }
    }

    let mut statuses = vec![0i32; plans.len()];
    let mut handles: Vec<Option<SpawnHandle>> = Vec::with_capacity(plans.len());
    for (i, plan) in plans.iter().enumerate() {
        handles.push(if plan.is_builtin {
            None
        } else {
            let handle = launch(plan, state, ns, &mut statuses[i]);
            if handle.is_none() {
                // The consumer never launched, so nothing will ever drain
                // its input pipe: break it before its producer can park.
                break_unconsumed_input(plan, ns);
            }
            handle
        });
    }

    let builtins_result = run_builtins(&plans, state, ns, &mut statuses);
    if builtins_result.is_err() {
        // An aborted builtin leaves held write ends open; close them so
        // downstream externals observe EOF before we wait on them.
        close_unwritten_pipes(&plans, ns);
        // Builtins the abort skipped will never drain their input pipes;
        // break them so upstream externals fail their writes instead of
        // blocking. Breaking an already-drained pipe is harmless.
        for plan in &plans {
            if plan.is_builtin {
                break_unconsumed_input(plan, ns);
            }
        }
    }
    // Drain pipes no stage consumes so their producers can finish.
    for id in &orphans {
        let _ = ns.pipe_read_all(id);
    }
    for (i, handle) in handles.iter().enumerate() {
        if let Some(handle) = handle {
            statuses[i] = match ns.spawn_wait(handle) {
                Ok(code) => code,
                Err(err) => {
                    let _ = ns.write_stderr(format!("wsh: {err}\n").as_bytes());
                    COMMAND_NOT_FOUND_STATUS
                }
            };
        }
    }
    builtins_result?;
    Ok(*statuses.last().unwrap_or(&0))
}

/// Plans stage wiring. Returns the per-stage plans plus "orphan" pipes — pipes
/// an external writes but no stage reads (a `<` redirect displaced the read
/// end) — which the shell must drain on the consumer's behalf.
fn wire(
    stages: &[Stage],
    state: &ShellState,
    ns: &mut dyn NamespaceOps,
) -> ShellResult<(Vec<StagePlan>, Vec<String>)> {
    let mut plans = Vec::with_capacity(stages.len());
    for stage in stages {
        let argv = expand_argv(&stage.argv, state)?;
        plans.push(StagePlan {
            is_builtin: is_pipeable_builtin(&argv[0]),
            argv,
            stdin: StageIn::Inherit,
            stdout: StageOut::Inherit,
        });
    }

    let mut orphans = Vec::new();
    for gap in 0..plans.len() - 1 {
        // A stage's own redirect displaces its pipe end (bash precedence).
        let feeds = !has_redirect(&stages[gap], 1);
        let drains = !has_redirect(&stages[gap + 1], 0);
        let producer_builtin = plans[gap].is_builtin;
        let consumer_builtin = plans[gap + 1].is_builtin;
        match (feeds, drains) {
            (true, true) if producer_builtin && consumer_builtin => {
                plans[gap].stdout = StageOut::Memory(gap);
                plans[gap + 1].stdin = StageIn::Memory(gap);
            }
            (true, true) => {
                let id = ns.pipe_new()?;
                plans[gap].stdout = StageOut::Pipe(id.clone());
                plans[gap + 1].stdin = StageIn::Pipe(id);
            }
            (true, false) if producer_builtin => plans[gap].stdout = StageOut::Discard,
            (true, false) => {
                let id = ns.pipe_new()?;
                plans[gap].stdout = StageOut::Pipe(id.clone());
                orphans.push(id);
            }
            (false, true) if consumer_builtin => plans[gap + 1].stdin = StageIn::Empty,
            (false, true) => {
                // A pipe nothing ever writes: the external consumer reads an
                // immediate EOF, like bash when the producer's stdout leaves.
                plans[gap + 1].stdin = StageIn::Pipe(ns.pipe_new()?);
            }
            (false, false) => {}
        }
    }

    for (plan, stage) in plans.iter_mut().zip(stages) {
        apply_redirects(plan, stage, state)?;
    }
    Ok((plans, orphans))
}

fn has_redirect(stage: &Stage, fd: u32) -> bool {
    stage.redirects.iter().any(|redirect| redirect.fd == fd)
}

fn apply_redirects(plan: &mut StagePlan, stage: &Stage, state: &ShellState) -> ShellResult<()> {
    for redirect in &stage.redirects {
        let target = crate::expand::expand_word(&redirect.target, state)?;
        match (redirect.fd, redirect.op) {
            (0, RedirectOp::Read) => plan.stdin = StageIn::File(target),
            (1, RedirectOp::Write) => {
                plan.stdout = StageOut::File {
                    path: target,
                    append: false,
                };
            }
            (1, RedirectOp::Append) => {
                plan.stdout = StageOut::File {
                    path: target,
                    append: true,
                };
            }
            _ => {
                return Err(crate::error::ShellError::Unsupported(
                    "this redirection".into(),
                ));
            }
        }
    }
    Ok(())
}

/// Launches one external stage; a launch failure reports 127 in `status`.
fn launch(
    plan: &StagePlan,
    state: &ShellState,
    ns: &mut dyn NamespaceOps,
    status: &mut i32,
) -> Option<SpawnHandle> {
    let spec = match external_spec(plan, state, ns) {
        Ok(spec) => spec,
        Err(err) => {
            let _ = ns.write_stderr(format!("wsh: {}: {err}\n", plan.argv[0]).as_bytes());
            *status = COMMAND_NOT_FOUND_STATUS;
            return None;
        }
    };
    match ns.spawn_start(&spec) {
        Ok(handle) => Some(handle),
        Err(err) => {
            let _ = ns.write_stderr(format!("wsh: {}: {err}\n", spec.program).as_bytes());
            *status = COMMAND_NOT_FOUND_STATUS;
            None
        }
    }
}

fn external_spec(
    plan: &StagePlan,
    state: &ShellState,
    ns: &dyn NamespaceOps,
) -> ShellResult<SpawnSpec> {
    let stdout = match &plan.stdout {
        StageOut::Inherit => OutputSink::Inherit,
        StageOut::Pipe(id) => OutputSink::Pipe(id.clone()),
        StageOut::File { append: true, .. } => {
            // A bound file fd opens at offset 0 (see exec::run_external).
            return Err(crate::error::ShellError::Unsupported(
                "'>>' append to a file for an external command".into(),
            ));
        }
        StageOut::File { path, append } => OutputSink::File {
            path: path.clone(),
            append: *append,
        },
        StageOut::Memory(_) | StageOut::Discard => {
            unreachable!("memory/discard wiring is builtin-only")
        }
    };
    let stdin = match &plan.stdin {
        StageIn::Inherit => InputSource::Inherit,
        StageIn::Pipe(id) => InputSource::Pipe(id.clone()),
        StageIn::File(path) => InputSource::File(path.clone()),
        StageIn::Memory(_) | StageIn::Empty => {
            unreachable!("memory/empty wiring is builtin-only")
        }
    };
    let target = crate::verbs::resolve_spawn_target(&plan.argv[0], ns)?;
    Ok(SpawnSpec {
        program: target.program,
        args: plan.argv[1..].to_vec(),
        env: state
            .env_iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect(),
        stdin,
        stdout,
        confine: target.confine,
    })
}

/// Runs the builtin stages left to right, recording their statuses.
fn run_builtins(
    plans: &[StagePlan],
    state: &ShellState,
    ns: &mut dyn NamespaceOps,
    statuses: &mut [i32],
) -> ShellResult<()> {
    let mut memory: Vec<Vec<u8>> = (0..plans.len()).map(|_| Vec::new()).collect();
    for (i, plan) in plans.iter().enumerate() {
        if !plan.is_builtin {
            continue;
        }
        let input = match &plan.stdin {
            StageIn::Inherit | StageIn::Empty => Vec::new(),
            StageIn::Pipe(id) => ns.pipe_read_all(id)?,
            StageIn::File(path) => ns.read_file(path)?,
            StageIn::Memory(gap) => std::mem::take(&mut memory[*gap]),
        };
        let (output, code) = if let Some(run) = builtin(&plan.argv[0]) {
            run(&plan.argv, &input, state)
        } else {
            let run = ns_builtin(&plan.argv[0]).expect("plan marked builtin");
            run(&plan.argv, &input, state, ns)
        };
        statuses[i] = code;
        match &plan.stdout {
            StageOut::Inherit => ns.write_stdout(&output)?,
            StageOut::Pipe(id) => ns.pipe_write_all_and_close(id, &output)?,
            StageOut::File { path, append } => ns.write_file(path, &output, *append)?,
            StageOut::Memory(gap) => memory[*gap] = output,
            StageOut::Discard => {}
        }
    }
    Ok(())
}

/// Closes every builtin-held pipe write end (an empty write-and-close is a
/// pure close). Used on the abort path so consumers observe EOF; closing an
/// already-closed end is harmless.
fn close_unwritten_pipes(plans: &[StagePlan], ns: &mut dyn NamespaceOps) {
    for plan in plans {
        if plan.is_builtin
            && let StageOut::Pipe(id) = &plan.stdout
        {
            let _ = ns.pipe_write_all_and_close(id, &[]);
        }
    }
}

/// Gives a pipe whose consumer will never read it a counterparty: breaking
/// the read end makes the producer's blocked or future writes fail with a
/// broken-pipe error (bash's `EPIPE`/`SIGPIPE`) rather than park forever
/// against the bounded buffer — the module-level deadlock-freedom invariant.
fn break_unconsumed_input(plan: &StagePlan, ns: &mut dyn NamespaceOps) {
    if let StageIn::Pipe(id) = &plan.stdin {
        let _ = ns.pipe_break_reader(id);
    }
}
