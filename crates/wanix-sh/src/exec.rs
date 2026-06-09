//! Executes a lowered [`Plan`] against the [`NamespaceOps`] surface.
//!
//! Builtins run in-process as pure functions (`argv` + stdin bytes → stdout
//! bytes + status); anything else is launched as an external child task.
//! Multi-stage pipelines are wired through bounded `#pipe` channels and their
//! stages run concurrently (see [`crate::pipeline`] and ADR 0010 tier 2). A
//! command that cannot be launched reports an honest error and a 127 status.

use crate::builtins::{builtin, is_special, special_builtin};
use crate::error::{ShellError, ShellResult};
use crate::lower::{AndOrList, Connector, Pipeline, Plan, Redirect, RedirectOp, Stage};
use crate::ns::{InputSource, NamespaceOps, OutputSink, SpawnSpec};
use crate::state::ShellState;

/// The status code returned when an external command cannot be launched.
pub(crate) const COMMAND_NOT_FOUND_STATUS: i32 = 127;

enum Outcome {
    Status(i32),
    Exit(i32),
}

/// The result of executing a whole plan: the final status plus whether the
/// `exit` builtin ended the run (the REPL stops on it; `-c` just returns).
pub(crate) struct PlanOutcome {
    pub(crate) status: i32,
    pub(crate) exited: bool,
}

/// Runs every pipeline in `plan` in sequence and returns the final exit status.
///
/// `state` is threaded through so state-mutating builtins (`cd`/`export`/…) and
/// `$?` see each pipeline's effect.
pub fn execute(plan: &Plan, state: &mut ShellState, ns: &mut dyn NamespaceOps) -> i32 {
    execute_plan(plan, state, ns).status
}

/// [`execute`] with the `exit` request made observable for the REPL.
pub(crate) fn execute_plan(
    plan: &Plan,
    state: &mut ShellState,
    ns: &mut dyn NamespaceOps,
) -> PlanOutcome {
    let mut status = 0;
    for list in &plan.lists {
        match run_and_or_list(list, state, ns) {
            Ok(Outcome::Status(code)) => status = code,
            Ok(Outcome::Exit(code)) => {
                state.set_last_status(code);
                return PlanOutcome {
                    status: code,
                    exited: true,
                };
            }
            Err(err) => {
                let _ = ns.write_stderr(format!("wsh: {err}\n").as_bytes());
                status = 1;
            }
        }
        state.set_last_status(status);
    }
    PlanOutcome {
        status,
        exited: false,
    }
}

/// Runs an and-or list, short-circuiting `&&`/`||` on the running exit status.
fn run_and_or_list(
    list: &AndOrList,
    state: &mut ShellState,
    ns: &mut dyn NamespaceOps,
) -> ShellResult<Outcome> {
    let mut status = match run_pipeline(&list.first, state, ns)? {
        Outcome::Status(code) => code,
        exit @ Outcome::Exit(_) => return Ok(exit),
    };
    state.set_last_status(status);
    for (connector, pipeline) in &list.rest {
        let run = match connector {
            Connector::And => status == 0,
            Connector::Or => status != 0,
        };
        if run {
            status = match run_pipeline(pipeline, state, ns)? {
                Outcome::Status(code) => code,
                exit @ Outcome::Exit(_) => return Ok(exit),
            };
            state.set_last_status(status);
        }
    }
    Ok(Outcome::Status(status))
}

fn run_pipeline(
    pipeline: &Pipeline,
    state: &mut ShellState,
    ns: &mut dyn NamespaceOps,
) -> ShellResult<Outcome> {
    let stages = &pipeline.stages;
    if stages.len() == 1 {
        return run_single_stage(&stages[0], state, ns);
    }

    // State-mutating builtins (cd/export/…) and exit cannot run in a pipeline.
    if let Some(stage) = stages.iter().find(|stage| is_special(&stage.argv[0])) {
        return Err(ShellError::Unsupported(format!(
            "'{}' cannot be used in a pipeline",
            stage.argv[0]
        )));
    }

    crate::pipeline::run_stages(stages, state, ns).map(Outcome::Status)
}

fn run_single_stage(
    stage: &Stage,
    state: &mut ShellState,
    ns: &mut dyn NamespaceOps,
) -> ShellResult<Outcome> {
    // Expand against the *current* state so a command sees earlier same-line
    // effects (`false; echo $?`, `export X=1; echo $X`).
    let argv = expand_argv(&stage.argv, state)?;
    let name = argv[0].as_str();
    if name == "exit" {
        return Ok(Outcome::Exit(parse_exit_code(&argv)));
    }
    if let Some(special) = special_builtin(name) {
        return Ok(Outcome::Status(special(&argv, state)));
    }
    let (stdin, stdout) = apply_redirects(
        &stage.redirects,
        InputSource::Inherit,
        OutputSink::Inherit,
        state,
    )?;
    dispatch(&argv, stdin, stdout, state, ns).map(Outcome::Status)
}

/// Applies a stage's redirects over its default stdin/stdout wiring.
fn apply_redirects(
    redirects: &[Redirect],
    mut stdin: InputSource,
    mut stdout: OutputSink,
    state: &ShellState,
) -> ShellResult<(InputSource, OutputSink)> {
    for redirect in redirects {
        let target = crate::expand::expand_word(&redirect.target, state)?;
        match (redirect.fd, redirect.op) {
            (0, RedirectOp::Read) => stdin = InputSource::File(target),
            (1, RedirectOp::Write) => {
                stdout = OutputSink::File {
                    path: target,
                    append: false,
                };
            }
            (1, RedirectOp::Append) => {
                stdout = OutputSink::File {
                    path: target,
                    append: true,
                };
            }
            _ => return Err(ShellError::Unsupported("this redirection".into())),
        }
    }
    Ok((stdin, stdout))
}

fn dispatch(
    argv: &[String],
    stdin: InputSource,
    stdout: OutputSink,
    state: &ShellState,
    ns: &mut dyn NamespaceOps,
) -> ShellResult<i32> {
    if let Some(builtin) = builtin(&argv[0]) {
        let input = gather_input(&stdin, ns)?;
        let (output, status) = builtin(argv, &input, state);
        emit_output(&stdout, &output, ns)?;
        Ok(status)
    } else {
        run_external(argv, stdin, stdout, state, ns)
    }
}

/// Expands each raw word with the current state (quote removal + `$VAR`/`$?`).
pub(crate) fn expand_argv(raw: &[String], state: &ShellState) -> ShellResult<Vec<String>> {
    raw.iter()
        .map(|word| crate::expand::expand_word(word, state))
        .collect()
}

fn run_external(
    argv: &[String],
    stdin: InputSource,
    stdout: OutputSink,
    state: &ShellState,
    ns: &mut dyn NamespaceOps,
) -> ShellResult<i32> {
    // A bound file fd opens at offset 0, so append (`>>`) to an external command
    // is not expressible yet; builtins handle `>>` themselves via write_file.
    if matches!(stdout, OutputSink::File { append: true, .. }) {
        return Err(ShellError::Unsupported(
            "'>>' append to a file for an external command".into(),
        ));
    }
    let program = crate::resolve::resolve_command(&argv[0], &*ns)?;
    let spec = SpawnSpec {
        program,
        args: argv[1..].to_vec(),
        env: state
            .env_iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect(),
        stdin,
        stdout,
    };
    match ns.spawn(&spec) {
        Ok(code) => Ok(code),
        Err(err) => {
            ns.write_stderr(format!("wsh: {}: {err}\n", spec.program).as_bytes())?;
            Ok(COMMAND_NOT_FOUND_STATUS)
        }
    }
}

fn gather_input(source: &InputSource, ns: &mut dyn NamespaceOps) -> ShellResult<Vec<u8>> {
    match source {
        InputSource::Inherit => Ok(Vec::new()),
        InputSource::Pipe(id) => ns.pipe_read_all(id),
        InputSource::File(path) => ns.read_file(path),
    }
}

fn emit_output(sink: &OutputSink, bytes: &[u8], ns: &mut dyn NamespaceOps) -> ShellResult<()> {
    match sink {
        OutputSink::Inherit => ns.write_stdout(bytes),
        OutputSink::Pipe(id) => ns.pipe_write_all_and_close(id, bytes),
        OutputSink::File { path, append } => ns.write_file(path, bytes, *append),
    }
}

fn parse_exit_code(argv: &[String]) -> i32 {
    argv.get(1).and_then(|code| code.parse().ok()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lower::lower;
    use crate::ns::SpawnHandle;
    use crate::syntax::parse_program;
    use std::collections::HashMap;

    /// A fake external command: (stdin bytes, args) -> (stdout bytes, status).
    type FakeCmd = fn(&[u8], &[String]) -> (Vec<u8>, i32);

    /// An in-memory NamespaceOps that fully simulates pipes and external command
    /// launch, so whole pipelines run on the host with no real I/O.
    ///
    /// `spawn_start` is honest about the concurrent contract on one thread by
    /// being event-driven: a command whose stdin pipe has no bytes yet is
    /// deferred and runs as soon as its pipe is written (or at `spawn_wait`,
    /// with whatever arrived) — so producers feed consumers regardless of the
    /// launch-all-then-wait-all order the executor uses.
    #[derive(Default)]
    struct FakeNs {
        out: Vec<u8>,
        err: Vec<u8>,
        commands: HashMap<String, FakeCmd>,
        pipes: HashMap<String, Vec<u8>>,
        files: HashMap<String, Vec<u8>>,
        spawns: Vec<SpawnSpec>,
        broken_pipes: std::collections::HashSet<String>,
        next_pipe: u32,
        next_handle: u32,
        deferred: HashMap<String, SpawnSpec>,
        deferred_on_pipe: HashMap<String, String>,
        statuses: HashMap<String, i32>,
        wait_order: Vec<String>,
    }

    impl FakeNs {
        fn register(&mut self, name: &str, cmd: FakeCmd) {
            self.commands.insert(name.to_owned(), cmd);
        }

        /// Pretends a file exists at `path` (for command resolution tests).
        fn seed_file(&mut self, path: &str) {
            self.files.entry(path.to_owned()).or_default();
        }

        fn read_source(&mut self, source: &InputSource) -> Vec<u8> {
            match source {
                InputSource::Inherit => Vec::new(),
                InputSource::Pipe(id) => self.pipes.remove(id).unwrap_or_default(),
                InputSource::File(path) => self.files.get(path).cloned().unwrap_or_default(),
            }
        }

        fn write_sink(&mut self, sink: &OutputSink, bytes: &[u8]) {
            match sink {
                OutputSink::Inherit => self.out.extend_from_slice(bytes),
                OutputSink::Pipe(id) => {
                    self.pipes
                        .entry(id.clone())
                        .or_default()
                        .extend_from_slice(bytes);
                    self.cascade(&id.clone());
                }
                OutputSink::File { path, append } => {
                    let entry = self.files.entry(path.clone()).or_default();
                    if !append {
                        entry.clear();
                    }
                    entry.extend_from_slice(bytes);
                }
            }
        }

        /// Runs one launched command to completion, feeding any deferred
        /// consumer of its output pipe.
        fn run_fake(&mut self, handle: &str, spec: &SpawnSpec) {
            let input = self.read_source(&spec.stdin);
            let cmd = self
                .commands
                .get(&spec.program)
                .copied()
                .expect("spawn_start verified the command exists");
            let (output, status) = cmd(&input, &spec.args);
            self.statuses.insert(handle.to_owned(), status);
            self.write_sink(&spec.stdout.clone(), &output);
        }

        /// Wakes a command deferred on `pipe` now that bytes arrived.
        fn cascade(&mut self, pipe: &str) {
            if let Some(handle) = self.deferred_on_pipe.remove(pipe)
                && let Some(spec) = self.deferred.remove(&handle)
            {
                self.run_fake(&handle, &spec);
            }
        }
    }

    impl NamespaceOps for FakeNs {
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
            Ok(self.files.contains_key(path))
        }
        fn read_file(&mut self, path: &str) -> ShellResult<Vec<u8>> {
            self.files
                .get(path)
                .cloned()
                .ok_or_else(|| crate::ShellError::Io(format!("{path}: not found")))
        }
        fn write_file(&mut self, path: &str, bytes: &[u8], append: bool) -> ShellResult<()> {
            let entry = self.files.entry(path.to_owned()).or_default();
            if !append {
                entry.clear();
            }
            entry.extend_from_slice(bytes);
            Ok(())
        }
        fn pipe_new(&mut self) -> ShellResult<String> {
            let id = self.next_pipe.to_string();
            self.next_pipe += 1;
            self.pipes.insert(id.clone(), Vec::new());
            Ok(id)
        }
        fn pipe_read_all(&mut self, id: &str) -> ShellResult<Vec<u8>> {
            Ok(self.pipes.remove(id).unwrap_or_default())
        }
        fn pipe_open_writer(&mut self, _id: &str) -> ShellResult<()> {
            Ok(())
        }
        fn pipe_break_reader(&mut self, id: &str) -> ShellResult<()> {
            self.broken_pipes.insert(id.to_owned());
            Ok(())
        }
        fn pipe_write_all_and_close(&mut self, id: &str, bytes: &[u8]) -> ShellResult<()> {
            // Model the real bounded channel: a write after the reader is
            // gone is a broken pipe, never a silent buffer.
            if self.broken_pipes.contains(id) {
                return Err(crate::ShellError::Io(format!(
                    "#pipe/{id}/data: broken pipe"
                )));
            }
            self.pipes
                .entry(id.to_owned())
                .or_default()
                .extend_from_slice(bytes);
            self.cascade(id);
            Ok(())
        }
        fn spawn_start(&mut self, spec: &SpawnSpec) -> ShellResult<SpawnHandle> {
            self.spawns.push(spec.clone());
            if !self.commands.contains_key(&spec.program) {
                return Err(crate::ShellError::Io("command not found".into()));
            }
            self.next_handle += 1;
            let handle = self.next_handle.to_string();
            match &spec.stdin {
                InputSource::Pipe(id) if self.pipes.get(id).is_none_or(|p| p.is_empty()) => {
                    // Input not produced yet: run when the pipe is written.
                    self.deferred.insert(handle.clone(), spec.clone());
                    self.deferred_on_pipe.insert(id.clone(), handle.clone());
                }
                _ => self.run_fake(&handle, spec),
            }
            Ok(SpawnHandle::new(handle))
        }
        fn spawn_wait(&mut self, handle: &SpawnHandle) -> ShellResult<i32> {
            self.wait_order.push(handle.id().to_owned());
            if let Some(spec) = self.deferred.remove(handle.id()) {
                // Its input pipe was never written: run with what's there.
                self.deferred_on_pipe.retain(|_, h| h != handle.id());
                self.run_fake(handle.id(), &spec);
            }
            self.statuses
                .get(handle.id())
                .copied()
                .ok_or_else(|| crate::ShellError::Io("unknown spawn handle".into()))
        }
    }

    fn run_with(input: &str, state: &mut ShellState, ns: &mut FakeNs) -> i32 {
        let plan = lower(&parse_program(input).expect("parses")).expect("lowers");
        execute(&plan, state, ns)
    }

    fn run_on(input: &str, ns: &mut FakeNs) -> i32 {
        let mut state = ShellState::new();
        run_with(input, &mut state, ns)
    }

    fn run(input: &str) -> (i32, FakeNs) {
        let mut ns = FakeNs::default();
        let code = run_on(input, &mut ns);
        (code, ns)
    }

    // ---- single commands -------------------------------------------------

    #[test]
    fn echo_prints_args_with_newline() {
        let (code, ns) = run("echo hi there");
        assert_eq!(code, 0);
        assert_eq!(String::from_utf8(ns.out).unwrap(), "hi there\n");
    }

    #[test]
    fn echo_n_suppresses_newline() {
        assert_eq!(String::from_utf8(run("echo -n hi").1.out).unwrap(), "hi");
    }

    #[test]
    fn true_and_false_set_status() {
        assert_eq!(run("true").0, 0);
        assert_eq!(run("false").0, 1);
    }

    #[test]
    fn exit_stops_the_sequence_with_its_code() {
        let (code, ns) = run("echo a; exit 3; echo b");
        assert_eq!(code, 3);
        assert_eq!(String::from_utf8(ns.out).unwrap(), "a\n");
    }

    #[test]
    fn external_command_spawns_and_inherits_stdout() {
        let mut ns = FakeNs::default();
        ns.register("greet", |_in, args| {
            (format!("hello {}\n", args.join(" ")).into_bytes(), 0)
        });
        assert_eq!(run_on("greet wanix", &mut ns), 0);
        assert_eq!(String::from_utf8(ns.out).unwrap(), "hello wanix\n");
    }

    #[test]
    fn missing_external_command_is_127_and_honest() {
        let mut ns = FakeNs::default();
        assert_eq!(run_on("definitely-not-a-command", &mut ns), 127);
        assert!(
            String::from_utf8(ns.err)
                .unwrap()
                .contains("definitely-not-a-command")
        );
    }

    // ---- pipelines -------------------------------------------------------

    #[test]
    fn builtin_producer_to_external_consumer() {
        // echo (builtin) writes the pipe; upper (external) drains it.
        let mut ns = FakeNs::default();
        ns.register("upper", |input, _args| (input.to_ascii_uppercase(), 0));
        assert_eq!(run_on("echo hi | upper", &mut ns), 0);
        assert_eq!(String::from_utf8(ns.out).unwrap(), "HI\n");
    }

    #[test]
    fn external_producer_to_external_consumer() {
        let mut ns = FakeNs::default();
        ns.register("gen", |_in, _args| (b"x\ny\n".to_vec(), 0));
        ns.register("wc", |input, _args| {
            (format!("{}\n", input.len()).into_bytes(), 0)
        });
        assert_eq!(run_on("gen | wc", &mut ns), 0);
        assert_eq!(String::from_utf8(ns.out).unwrap(), "4\n");
    }

    #[test]
    fn external_producer_to_builtin_consumer() {
        // cat (builtin) drains the pipe filled by an external producer.
        let mut ns = FakeNs::default();
        ns.register("gen", |_in, _args| (b"piped bytes".to_vec(), 0));
        assert_eq!(run_on("gen | cat", &mut ns), 0);
        assert_eq!(String::from_utf8(ns.out).unwrap(), "piped bytes");
    }

    #[test]
    fn three_stage_pipeline_threads_data_through() {
        let mut ns = FakeNs::default();
        ns.register("upper", |input, _args| (input.to_ascii_uppercase(), 0));
        ns.register("wc", |input, _args| {
            (format!("{}\n", input.len()).into_bytes(), 0)
        });
        // echo "hi" -> "hi\n" (3) -> "HI\n" (3) -> "3\n"
        assert_eq!(run_on("echo hi | upper | wc", &mut ns), 0);
        assert_eq!(String::from_utf8(ns.out).unwrap(), "3\n");
    }

    #[test]
    fn pipeline_status_is_the_last_stage() {
        let mut ns = FakeNs::default();
        ns.register("boom", |_in, _args| (Vec::new(), 5));
        assert_eq!(run_on("echo hi | boom", &mut ns), 5);
    }

    #[test]
    fn builtin_only_pipeline_echo_into_cat() {
        let (code, ns) = run("echo hello | cat");
        assert_eq!(code, 0);
        assert_eq!(String::from_utf8(ns.out).unwrap(), "hello\n");
    }

    #[test]
    fn adjacent_builtins_use_shell_memory_not_a_pipe() {
        // A single-threaded shell cannot drain a pipe it is blocked writing, so
        // builtin->builtin gaps must never touch a real (bounded) pipe.
        let (code, ns) = run("echo hello | cat | cat");
        assert_eq!(code, 0);
        assert_eq!(String::from_utf8(ns.out).unwrap(), "hello\n");
        assert!(ns.pipes.is_empty(), "no #pipe should have been allocated");
        assert_eq!(ns.next_pipe, 0, "no #pipe should have been allocated");
    }

    #[test]
    fn all_externals_launch_before_any_wait_and_waits_run_in_stage_order() {
        let mut ns = FakeNs::default();
        ns.register("gen", |_in, _args| (b"x".to_vec(), 0));
        ns.register("mid", |input, _args| (input.to_vec(), 3));
        ns.register("last", |input, _args| (input.to_vec(), 5));
        assert_eq!(run_on("gen | mid | last", &mut ns), 5, "last stage wins");
        assert_eq!(
            ns.spawns
                .iter()
                .map(|s| s.program.clone())
                .collect::<Vec<_>>(),
            ["gen", "mid", "last"],
            "stages launch left to right before any wait"
        );
        assert_eq!(
            ns.wait_order,
            ["1", "2", "3"],
            "exit statuses are collected in stage order"
        );
    }

    #[test]
    fn pipeline_status_is_last_stage_even_when_earlier_stages_fail() {
        let mut ns = FakeNs::default();
        ns.register("boom", |_in, _args| (b"data".to_vec(), 9));
        ns.register("ok", |input, _args| (input.to_vec(), 0));
        assert_eq!(run_on("boom | ok", &mut ns), 0, "bash $? is the last stage");
    }

    #[test]
    fn failed_consumer_launch_breaks_its_input_pipe() {
        // The consumer never launches (spawn_start fails), so nothing will
        // ever drain its input pipe. The shell must break the read end: the
        // builtin producer's write then reports a broken pipe (bash's EPIPE)
        // instead of parking forever against the real bounded buffer.
        let mut ns = FakeNs::default();
        let code = run_on("echo hi | nosuchcmd", &mut ns);
        assert_eq!(
            ns.broken_pipes.len(),
            1,
            "the dead consumer's input pipe must be broken"
        );
        assert_eq!(code, 1, "the producer's broken-pipe write aborts loudly");
        let err = String::from_utf8(ns.err).unwrap();
        assert!(err.contains("nosuchcmd"), "launch failure reported: {err}");
        assert!(err.contains("broken pipe"), "EPIPE surfaced: {err}");
    }

    #[test]
    fn builtin_abort_breaks_input_pipes_of_skipped_consumers() {
        // `cat < missing.txt` aborts the builtin pass before the trailing
        // builtin runs, so that builtin never drains the external's output
        // pipe. The shell must break it so `gen` cannot block forever.
        let mut ns = FakeNs::default();
        ns.register("gen", |_in, _args| (b"data".to_vec(), 0));
        let code = run_on("cat < missing.txt | gen | cat", &mut ns);
        assert_eq!(code, 1, "the aborted builtin reports its error");
        assert!(
            !ns.broken_pipes.is_empty(),
            "the skipped consumer's input pipe must be broken"
        );
        assert!(
            String::from_utf8(ns.err).unwrap().contains("missing.txt"),
            "the original failure is reported"
        );
    }

    #[test]
    fn builtin_between_externals_threads_data_through() {
        let mut ns = FakeNs::default();
        ns.register("gen", |_in, _args| (b"abc".to_vec(), 0));
        ns.register("wc", |input, _args| {
            (format!("{}\n", input.len()).into_bytes(), 0)
        });
        // gen (external) -> cat (builtin, runs in-shell) -> wc (external)
        assert_eq!(run_on("gen | cat | wc", &mut ns), 0);
        assert_eq!(String::from_utf8(ns.out).unwrap(), "3\n");
    }

    // ---- command resolution ---------------------------------------------

    fn resolve(name: &str, ns: &FakeNs) -> String {
        crate::resolve::resolve_command(name, ns).expect("resolve")
    }

    #[test]
    fn resolve_literal_path_passes_through() {
        let ns = FakeNs::default();
        assert_eq!(resolve("usr/bin/cat", &ns), "usr/bin/cat");
    }

    #[test]
    fn resolve_finds_wasm_in_bin() {
        let mut ns = FakeNs::default();
        ns.seed_file("bin/jaq.wasm");
        assert_eq!(resolve("jaq", &ns), "bin/jaq.wasm");
    }

    #[test]
    fn resolve_prefers_wasm_over_bare() {
        let mut ns = FakeNs::default();
        ns.seed_file("bin/tool");
        ns.seed_file("bin/tool.wasm");
        assert_eq!(resolve("tool", &ns), "bin/tool.wasm");
    }

    #[test]
    fn resolve_searches_usr_bin() {
        let mut ns = FakeNs::default();
        ns.seed_file("usr/bin/thing.wasm");
        assert_eq!(resolve("thing", &ns), "usr/bin/thing.wasm");
    }

    #[test]
    fn resolve_missing_returns_bare_name() {
        let ns = FakeNs::default();
        assert_eq!(resolve("nope", &ns), "nope");
    }

    #[test]
    fn external_command_resolves_from_bin_then_runs() {
        let mut ns = FakeNs::default();
        ns.seed_file("bin/lister.wasm");
        ns.register("bin/lister.wasm", |_in, args| {
            (format!("ran {}\n", args.join(",")).into_bytes(), 0)
        });
        assert_eq!(run_on("lister a b", &mut ns), 0);
        assert_eq!(String::from_utf8(ns.out).unwrap(), "ran a,b\n");
    }

    // ---- shell state + builtins -----------------------------------------

    #[test]
    fn cd_updates_logical_cwd() {
        let mut state = ShellState::new();
        let mut ns = FakeNs::default();
        assert_eq!(run_with("cd work", &mut state, &mut ns), 0);
        assert_eq!(state.cwd(), "work");
    }

    #[test]
    fn pwd_prints_cwd() {
        let mut state = ShellState::new();
        let mut ns = FakeNs::default();
        run_with("cd work/sub", &mut state, &mut ns);
        run_with("pwd", &mut state, &mut ns);
        assert_eq!(String::from_utf8(ns.out).unwrap(), "/work/sub\n");
    }

    #[test]
    fn export_then_env_lists_sorted() {
        let mut state = ShellState::new();
        let mut ns = FakeNs::default();
        run_with("export B=2", &mut state, &mut ns);
        run_with("export A=1", &mut state, &mut ns);
        run_with("env", &mut state, &mut ns);
        assert_eq!(String::from_utf8(ns.out).unwrap(), "A=1\nB=2\n");
    }

    #[test]
    fn unset_removes_env() {
        let mut state = ShellState::new();
        let mut ns = FakeNs::default();
        run_with("export A=1", &mut state, &mut ns);
        run_with("unset A", &mut state, &mut ns);
        assert_eq!(state.env_get("A"), None);
    }

    #[test]
    fn special_builtin_in_pipeline_is_unsupported() {
        let (code, ns) = run("cd x | cat");
        assert_eq!(code, 1);
        assert!(
            String::from_utf8(ns.err)
                .unwrap()
                .contains("cannot be used in a pipeline")
        );
    }

    #[test]
    fn exported_env_propagates_to_child() {
        let mut state = ShellState::new();
        let mut ns = FakeNs::default();
        ns.register("prog", |_in, _args| (Vec::new(), 0));
        run_with("export A=1; prog", &mut state, &mut ns);
        let last = ns.spawns.last().expect("a child was spawned");
        assert_eq!(last.env, vec![("A".to_owned(), "1".to_owned())]);
    }

    // ---- expansion (at execution time) ----------------------------------

    #[test]
    fn echo_expands_var_set_earlier_on_same_line() {
        // Proves expansion happens at execution time: `echo $X` sees the export
        // that ran earlier on the same line.
        let (code, ns) = run("export X=hi; echo $X");
        assert_eq!(code, 0);
        assert_eq!(String::from_utf8(ns.out).unwrap(), "hi\n");
    }

    #[test]
    fn dollar_question_reflects_previous_status() {
        let (code, ns) = run("false; echo $?");
        assert_eq!(code, 0);
        assert_eq!(String::from_utf8(ns.out).unwrap(), "1\n");
    }

    #[test]
    fn unsupported_expansion_is_honest_at_runtime() {
        let (code, ns) = run("echo $(echo hi)");
        assert_eq!(code, 1);
        assert!(String::from_utf8(ns.err).unwrap().contains("not supported"));
    }

    // ---- && / || short-circuit ------------------------------------------

    #[test]
    fn and_runs_on_success() {
        let (code, ns) = run("true && echo yes");
        assert_eq!(code, 0);
        assert_eq!(String::from_utf8(ns.out).unwrap(), "yes\n");
    }

    #[test]
    fn and_skips_on_failure() {
        let (code, ns) = run("false && echo yes");
        assert_eq!(code, 1);
        assert!(ns.out.is_empty());
    }

    #[test]
    fn or_runs_on_failure() {
        let (code, ns) = run("false || echo recovered");
        assert_eq!(code, 0);
        assert_eq!(String::from_utf8(ns.out).unwrap(), "recovered\n");
    }

    #[test]
    fn or_skips_on_success() {
        let (code, ns) = run("true || echo no");
        assert_eq!(code, 0);
        assert!(ns.out.is_empty());
    }

    #[test]
    fn and_or_chain_short_circuits() {
        // false && echo a || echo b : `a` skipped (status 1), `b` runs.
        let (code, ns) = run("false && echo a || echo b");
        assert_eq!(code, 0);
        assert_eq!(String::from_utf8(ns.out).unwrap(), "b\n");
    }

    // ---- redirects ------------------------------------------------------

    #[test]
    fn builtin_writes_to_file_not_terminal() {
        let mut state = ShellState::new();
        let mut ns = FakeNs::default();
        run_with("echo hi > out.txt", &mut state, &mut ns);
        assert_eq!(ns.files.get("out.txt").unwrap(), b"hi\n");
        assert!(ns.out.is_empty());
    }

    #[test]
    fn builtin_appends_to_file() {
        let mut state = ShellState::new();
        let mut ns = FakeNs::default();
        run_with("echo a > out.txt", &mut state, &mut ns);
        run_with("echo b >> out.txt", &mut state, &mut ns);
        assert_eq!(ns.files.get("out.txt").unwrap(), b"a\nb\n");
    }

    #[test]
    fn builtin_reads_from_file() {
        let mut state = ShellState::new();
        let mut ns = FakeNs::default();
        ns.write_file("in.txt", b"file contents", false).unwrap();
        run_with("cat < in.txt", &mut state, &mut ns);
        assert_eq!(String::from_utf8(ns.out).unwrap(), "file contents");
    }

    #[test]
    fn redirect_overrides_pipe_sink() {
        // `echo hi | cat > out.txt`: cat's stdout goes to the file, not stdout.
        let mut state = ShellState::new();
        let mut ns = FakeNs::default();
        run_with("echo hi | cat > out.txt", &mut state, &mut ns);
        assert_eq!(ns.files.get("out.txt").unwrap(), b"hi\n");
        assert!(ns.out.is_empty());
    }

    #[test]
    fn external_append_is_unsupported() {
        let mut state = ShellState::new();
        let mut ns = FakeNs::default();
        ns.register("prog", |_in, _args| (b"x".to_vec(), 0));
        let code = run_with("prog >> out.txt", &mut state, &mut ns);
        assert_eq!(code, 1);
        assert!(String::from_utf8(ns.err).unwrap().contains("not supported"));
    }
}
