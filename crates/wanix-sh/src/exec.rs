//! Executes a lowered [`Plan`] against the [`NamespaceOps`] surface.
//!
//! The current builtin set is deliberately small; external command launch (via
//! the `#task` device) and pipelines arrive in later phases. Unknown commands
//! report an honest "not supported yet" message and a 127 status rather than
//! pretending to run.

use crate::error::ShellResult;
use crate::lower::Plan;
use crate::ns::NamespaceOps;

/// The status code returned for a command name the shell cannot run yet.
const UNSUPPORTED_COMMAND_STATUS: i32 = 127;

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
    let name = argv[0].as_str();
    match name {
        "echo" => builtin_echo(argv, ns).map(Step::Status),
        "true" | ":" => Ok(Step::Status(0)),
        "false" => Ok(Step::Status(1)),
        "exit" => Ok(Step::Exit(parse_exit_code(argv))),
        other => {
            ns.write_stderr(
                format!("wsh: {other}: external commands not supported yet\n").as_bytes(),
            )?;
            Ok(Step::Status(UNSUPPORTED_COMMAND_STATUS))
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

    #[derive(Default)]
    struct FakeNs {
        out: Vec<u8>,
        err: Vec<u8>,
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
    }

    fn run(input: &str) -> (i32, FakeNs) {
        let plan = lower(&parse_program(input).expect("parses")).expect("lowers");
        let mut ns = FakeNs::default();
        let code = execute(&plan, &mut ns);
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
    fn unknown_command_is_honest_and_127() {
        let (code, ns) = run("ls");
        assert_eq!(code, UNSUPPORTED_COMMAND_STATUS);
        assert!(
            String::from_utf8(ns.err)
                .unwrap()
                .contains("not supported yet")
        );
    }

    #[test]
    fn exit_stops_the_sequence_with_its_code() {
        let (code, ns) = run("echo a; exit 3; echo b");
        assert_eq!(code, 3);
        assert_eq!(String::from_utf8(ns.out).unwrap(), "a\n");
    }
}
