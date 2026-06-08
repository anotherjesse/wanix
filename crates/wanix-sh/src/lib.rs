//! `wanix-sh`: a Wanix-native shell.
//!
//! The shell uses [`brush-parser`](brush_parser) for bash-compatible *syntax*
//! and supplies its own Wanix-native executor. It is compiled to
//! `wasm32-wasip1` and run as an ordinary Wanix task (via the `wanix-wasm`
//! driver), touching the outside world only through the [`NamespaceOps`] trait.
//!
//! The pipeline is: [`parse_program`](syntax::parse_program) (text → AST) →
//! [`lower`](lower::lower) (AST → [`Plan`](lower::Plan), rejecting unsupported
//! constructs honestly) → [`execute`](exec::execute) (run against
//! [`NamespaceOps`]). Parsing and lowering are pure and host-testable; only
//! execution needs the OS surface.
//!
//! ## Supported subset (today)
//!
//! Simple commands and arguments with quote removal, `;` / newline sequences,
//! `|` pipelines (sequential, `#pipe`-backed), the `echo`, `cat`, `pwd`, `env`,
//! `true`, `false`, `:`, `exit`, `cd`, `export`, and `unset` builtins, and
//! external command launch (child tasks via the `#task` device, resolved from a
//! `bin` directory, inheriting the shell's exported env). Redirections, `&&`/`||`,
//! control flow, and expansion are recognized by the parser but reported as
//! [`ShellError::Unsupported`] until their executor support lands.

mod builtins;
mod error;
mod exec;
mod expand;
mod lower;
mod ns;
mod prompt;
mod resolve;
mod state;
mod syntax;

pub use error::{ShellError, ShellResult};
pub use ns::{InputSource, NamespaceOps, OutputSink, SpawnSpec};
pub use prompt::{DEFAULT_PS1, render as render_prompt};
pub use state::ShellState;

use exec::execute;
use lower::lower;
use syntax::parse_program;

/// Runs the shell for one invocation and returns its process exit code.
///
/// `args` is the full argument vector (`args[0]` is the program name). The only
/// supported invocation today is `-c <command-line>`; anything else prints a
/// usage note to standard error.
#[must_use]
pub fn run_shell(args: &[String], ns: &mut dyn NamespaceOps) -> i32 {
    match command_string(args) {
        Some(line) => {
            let mut state = ShellState::new();
            run_line(&line, &mut state, ns)
        }
        None => {
            let _ = ns.write_stderr(
                b"wsh: interactive mode is not supported yet; use -c \"<command>\"\n",
            );
            2
        }
    }
}

/// Parses, lowers, and executes a single command-line string against `state`.
///
/// Parse and lowering failures are reported to standard error with status 2;
/// execution failures carry their own status.
#[must_use]
pub fn run_line(line: &str, state: &mut ShellState, ns: &mut dyn NamespaceOps) -> i32 {
    let program = match parse_program(line) {
        Ok(program) => program,
        Err(err) => return report(ns, &err),
    };
    let plan = match lower(&program) {
        Ok(plan) => plan,
        Err(err) => return report(ns, &err),
    };
    execute(&plan, state, ns)
}

fn command_string(args: &[String]) -> Option<String> {
    let mut iter = args.iter().skip(1);
    while let Some(arg) = iter.next() {
        if arg == "-c" {
            return iter.next().cloned();
        }
    }
    None
}

fn report(ns: &mut dyn NamespaceOps, err: &ShellError) -> i32 {
    let _ = ns.write_stderr(format!("wsh: {err}\n").as_bytes());
    2
}
