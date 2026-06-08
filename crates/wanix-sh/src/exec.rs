//! Executes a lowered [`Plan`] against the [`NamespaceOps`] surface.
//!
//! Builtins run in-process; anything else is launched as an external command
//! (a child task) via [`NamespaceOps::spawn`]. A command that cannot be launched
//! reports an honest error and a 127 status. Pipelines and redirections arrive
//! in later phases (they are still rejected during lowering).

use crate::error::ShellResult;
use crate::lower::Plan;
use crate::ns::{NamespaceOps, SpawnSpec};

/// The status code returned when an external command cannot be launched.
const COMMAND_NOT_FOUND_STATUS: i32 = 127;

enum Step {
    Status(i32),
    Exit(i32),
}

/// Runs every command in `plan` in sequence and returns the final exit status.
///
/// An `exit` builtin stops the sequence immediately with its code. I/O errors
/// are reported to standard error and treated as status 1.
pub fn execute(plan: &Plan, ns: &mut dyn NamespaceOps) -> i32 {
    let mut status = 0;
    for command in &plan.commands {
        match run_command(&command.argv, ns) {
            Ok(Step::Status(code)) => status = code,
            Ok(Step::Exit(code)) => return code,
            Err(err) => {
                let _ = ns.write_stderr(format!("wsh: {err}\n").as_bytes());
                status = 1;
            }
        }
    }
    status
}

fn run_command(argv: &[String], ns: &mut dyn NamespaceOps) -> ShellResult<Step> {
    match argv[0].as_str() {
        "echo" => builtin_echo(argv, ns).map(Step::Status),
        "true" | ":" => Ok(Step::Status(0)),
        "false" => Ok(Step::Status(1)),
        "exit" => Ok(Step::Exit(parse_exit_code(argv))),
        _ => run_external(argv, ns).map(Step::Status),
    }
}

fn run_external(argv: &[String], ns: &mut dyn NamespaceOps) -> ShellResult<i32> {
    let spec = SpawnSpec::from_argv(argv);
    match ns.spawn(&spec) {
        Ok(code) => Ok(code),
        Err(err) => {
            ns.write_stderr(format!("wsh: {}: {err}\n", spec.program).as_bytes())?;
            Ok(COMMAND_NOT_FOUND_STATUS)
        }
    }
}

fn builtin_echo(argv: &[String], ns: &mut dyn NamespaceOps) -> ShellResult<i32> {
    let mut args = &argv[1..];
    let mut trailing_newline = true;
    if let Some(first) = args.first()
        && first == "-n"
    {
        trailing_newline = false;
        args = &args[1..];
    }
    ns.write_stdout(args.join(" ").as_bytes())?;
    if trailing_newline {
        ns.write_stdout(b"\n")?;
    }
    Ok(0)
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

    /// A fake command: maps args to (stdout bytes, exit code).
    type FakeCmd = fn(&[String]) -> (Vec<u8>, i32);

    #[derive(Default)]
    struct FakeNs {
        out: Vec<u8>,
        err: Vec<u8>,
        commands: HashMap<String, FakeCmd>,
        spawned: Vec<SpawnSpec>,
    }

    impl FakeNs {
        fn register(&mut self, name: &str, cmd: FakeCmd) {
            self.commands.insert(name.to_owned(), cmd);
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
        fn spawn(&mut self, spec: &SpawnSpec) -> ShellResult<i32> {
            self.spawned.push(spec.clone());
            match self.commands.get(&spec.program) {
                Some(cmd) => {
                    let (stdout, code) = cmd(&spec.args);
                    self.out.extend_from_slice(&stdout); // inherited stdout
                    Ok(code)
                }
                None => Err(crate::ShellError::Io("command not found".into())),
            }
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

    #[test]
    fn echo_prints_args_with_newline() {
        let (code, ns) = run("echo hi there");
        assert_eq!(code, 0);
        assert_eq!(String::from_utf8(ns.out).unwrap(), "hi there\n");
    }

    #[test]
    fn echo_n_suppresses_newline() {
        let (code, ns) = run("echo -n hi");
        assert_eq!(code, 0);
        assert_eq!(String::from_utf8(ns.out).unwrap(), "hi");
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
        ns.register("greet", |args| {
            (format!("hello {}\n", args.join(" ")).into_bytes(), 0)
        });
        let code = run_on("greet wanix world", &mut ns);
        assert_eq!(code, 0);
        assert_eq!(String::from_utf8(ns.out).unwrap(), "hello wanix world\n");
        assert_eq!(ns.spawned.len(), 1);
        assert_eq!(ns.spawned[0].program, "greet");
        assert_eq!(ns.spawned[0].args, vec!["wanix", "world"]);
    }

    #[test]
    fn external_command_exit_code_propagates() {
        let mut ns = FakeNs::default();
        ns.register("flaky", |_| (Vec::new(), 2));
        assert_eq!(run_on("flaky", &mut ns), 2);
    }

    #[test]
    fn missing_external_command_is_127_and_honest() {
        let mut ns = FakeNs::default();
        let code = run_on("definitely-not-a-command", &mut ns);
        assert_eq!(code, COMMAND_NOT_FOUND_STATUS);
        assert!(
            String::from_utf8(ns.err)
                .unwrap()
                .contains("definitely-not-a-command")
        );
    }
}
