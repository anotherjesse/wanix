//! Executes a lowered [`Plan`] against the [`NamespaceOps`] surface.
//!
//! Builtins run in-process as pure functions (`argv` + stdin bytes → stdout
//! bytes + status); anything else is launched as an external child task. Stages
//! of a pipeline are wired through `#pipe` channels and — for now — run
//! sequentially: each stage completes and buffers its whole output before the
//! next drains it (see `concepts/task-exit-closes-fds` for why, and the planned
//! move to concurrent stages). A command that cannot be launched reports an
//! honest error and a 127 status.

use crate::error::ShellResult;
use crate::lower::{Pipeline, Plan, Stage};
use crate::ns::{InputSource, NamespaceOps, OutputSink, SpawnSpec};

/// The status code returned when an external command cannot be launched.
const COMMAND_NOT_FOUND_STATUS: i32 = 127;

enum Outcome {
    Status(i32),
    Exit(i32),
}

/// Runs every pipeline in `plan` in sequence and returns the final exit status.
pub fn execute(plan: &Plan, ns: &mut dyn NamespaceOps) -> i32 {
    let mut status = 0;
    for pipeline in &plan.pipelines {
        match run_pipeline(pipeline, ns) {
            Ok(Outcome::Status(code)) => status = code,
            Ok(Outcome::Exit(code)) => return code,
            Err(err) => {
                let _ = ns.write_stderr(format!("wsh: {err}\n").as_bytes());
                status = 1;
            }
        }
    }
    status
}

fn run_pipeline(pipeline: &Pipeline, ns: &mut dyn NamespaceOps) -> ShellResult<Outcome> {
    let stages = &pipeline.stages;
    if stages.len() == 1 {
        return run_single_stage(&stages[0], ns);
    }

    // One #pipe between each adjacent pair; stages run left to right.
    let mut pipes = Vec::with_capacity(stages.len() - 1);
    for _ in 1..stages.len() {
        pipes.push(ns.pipe_new()?);
    }
    let last = stages.len() - 1;
    let mut status = 0;
    for (i, stage) in stages.iter().enumerate() {
        let stdin = if i == 0 {
            InputSource::Inherit
        } else {
            InputSource::Pipe(pipes[i - 1].clone())
        };
        let stdout = if i == last {
            OutputSink::Inherit
        } else {
            OutputSink::Pipe(pipes[i].clone())
        };
        status = run_stage(stage, stdin, stdout, ns)?;
    }
    Ok(Outcome::Status(status))
}

fn run_single_stage(stage: &Stage, ns: &mut dyn NamespaceOps) -> ShellResult<Outcome> {
    if stage.argv[0] == "exit" {
        return Ok(Outcome::Exit(parse_exit_code(&stage.argv)));
    }
    run_stage(stage, InputSource::Inherit, OutputSink::Inherit, ns).map(Outcome::Status)
}

fn run_stage(
    stage: &Stage,
    stdin: InputSource,
    stdout: OutputSink,
    ns: &mut dyn NamespaceOps,
) -> ShellResult<i32> {
    if let Some(builtin) = builtin(&stage.argv[0]) {
        let input = gather_input(&stdin, ns)?;
        let (output, status) = builtin(&stage.argv, &input);
        emit_output(&stdout, &output, ns)?;
        Ok(status)
    } else {
        run_external(stage, stdin, stdout, ns)
    }
}

fn run_external(
    stage: &Stage,
    stdin: InputSource,
    stdout: OutputSink,
    ns: &mut dyn NamespaceOps,
) -> ShellResult<i32> {
    let spec = SpawnSpec {
        program: stage.argv[0].clone(),
        args: stage.argv[1..].to_vec(),
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
    }
}

fn emit_output(sink: &OutputSink, bytes: &[u8], ns: &mut dyn NamespaceOps) -> ShellResult<()> {
    match sink {
        OutputSink::Inherit => ns.write_stdout(bytes),
        OutputSink::Pipe(id) => ns.pipe_write_all_and_close(id, bytes),
    }
}

/// A builtin: `argv` + standard input bytes → standard output bytes + status.
type Builtin = fn(&[String], &[u8]) -> (Vec<u8>, i32);

fn builtin(name: &str) -> Option<Builtin> {
    match name {
        "echo" => Some(builtin_echo),
        "cat" => Some(builtin_cat),
        "true" | ":" => Some(|_, _| (Vec::new(), 0)),
        "false" => Some(|_, _| (Vec::new(), 1)),
        _ => None,
    }
}

fn builtin_echo(argv: &[String], _input: &[u8]) -> (Vec<u8>, i32) {
    let mut args = &argv[1..];
    let mut trailing_newline = true;
    if let Some(first) = args.first()
        && first == "-n"
    {
        trailing_newline = false;
        args = &args[1..];
    }
    let mut out = args.join(" ").into_bytes();
    if trailing_newline {
        out.push(b'\n');
    }
    (out, 0)
}

fn builtin_cat(_argv: &[String], input: &[u8]) -> (Vec<u8>, i32) {
    (input.to_vec(), 0)
}

fn parse_exit_code(argv: &[String]) -> i32 {
    argv.get(1).and_then(|code| code.parse().ok()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lower::lower;
    use crate::syntax::parse_program;
    use std::collections::HashMap;

    /// A fake external command: (stdin bytes, args) -> (stdout bytes, status).
    type FakeCmd = fn(&[u8], &[String]) -> (Vec<u8>, i32);

    /// An in-memory NamespaceOps that fully simulates pipes and external command
    /// launch, so whole pipelines run on the host with no real I/O.
    #[derive(Default)]
    struct FakeNs {
        out: Vec<u8>,
        err: Vec<u8>,
        commands: HashMap<String, FakeCmd>,
        pipes: HashMap<String, Vec<u8>>,
        next_pipe: u32,
    }

    impl FakeNs {
        fn register(&mut self, name: &str, cmd: FakeCmd) {
            self.commands.insert(name.to_owned(), cmd);
        }

        fn read_source(&mut self, source: &InputSource) -> Vec<u8> {
            match source {
                InputSource::Inherit => Vec::new(),
                InputSource::Pipe(id) => self.pipes.remove(id).unwrap_or_default(),
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
                }
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
        fn pipe_new(&mut self) -> ShellResult<String> {
            let id = self.next_pipe.to_string();
            self.next_pipe += 1;
            self.pipes.insert(id.clone(), Vec::new());
            Ok(id)
        }
        fn pipe_read_all(&mut self, id: &str) -> ShellResult<Vec<u8>> {
            Ok(self.pipes.remove(id).unwrap_or_default())
        }
        fn pipe_write_all_and_close(&mut self, id: &str, bytes: &[u8]) -> ShellResult<()> {
            self.pipes
                .entry(id.to_owned())
                .or_default()
                .extend_from_slice(bytes);
            Ok(())
        }
        fn spawn(&mut self, spec: &SpawnSpec) -> ShellResult<i32> {
            let input = self.read_source(&spec.stdin);
            let Some(cmd) = self.commands.get(&spec.program).copied() else {
                return Err(crate::ShellError::Io("command not found".into()));
            };
            let (output, status) = cmd(&input, &spec.args);
            self.write_sink(&spec.stdout, &output);
            Ok(status)
        }
    }

    fn run_on(input: &str, ns: &mut FakeNs) -> i32 {
        let plan = lower(&parse_program(input).expect("parses")).expect("lowers");
        execute(&plan, ns)
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
}
