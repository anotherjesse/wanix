//! Builtin commands.
//!
//! Two kinds:
//! - **Pipeable** builtins are pure of effects on the shell: `(argv, stdin,
//!   &ShellState) -> (stdout, status)`. They may *read* state (`pwd`, `env`) but
//!   not mutate it, so they compose inside pipelines.
//! - **Special** builtins (`cd`, `export`, `unset`) mutate `&mut ShellState` and
//!   are valid only as a single, unpiped command — using one in a pipeline is an
//!   honest [`ShellError::Unsupported`](crate::ShellError::Unsupported).

use crate::state::{ShellState, join_cwd};

/// A pipeable builtin: argv + stdin bytes + read-only state -> stdout + status.
pub type Builtin = fn(&[String], &[u8], &ShellState) -> (Vec<u8>, i32);

/// A special builtin: argv + mutable state -> status. Single-stage only.
pub type SpecialBuiltin = fn(&[String], &mut ShellState) -> i32;

/// Looks up a pipeable builtin by name.
pub fn builtin(name: &str) -> Option<Builtin> {
    match name {
        "echo" => Some(echo),
        "cat" => Some(cat),
        "pwd" => Some(pwd),
        "env" => Some(env),
        "true" | ":" => Some(|_, _, _| (Vec::new(), 0)),
        "false" => Some(|_, _, _| (Vec::new(), 1)),
        _ => None,
    }
}

/// Looks up a special (state-mutating) builtin by name.
pub fn special_builtin(name: &str) -> Option<SpecialBuiltin> {
    match name {
        "cd" => Some(cd),
        "export" => Some(export),
        "unset" => Some(unset),
        _ => None,
    }
}

/// Whether a name mutates shell state or exits — not valid inside a pipeline.
#[must_use]
pub fn is_special(name: &str) -> bool {
    matches!(name, "cd" | "export" | "unset" | "exit")
}

fn echo(argv: &[String], _stdin: &[u8], _state: &ShellState) -> (Vec<u8>, i32) {
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

fn cat(_argv: &[String], stdin: &[u8], _state: &ShellState) -> (Vec<u8>, i32) {
    (stdin.to_vec(), 0)
}

fn pwd(_argv: &[String], _stdin: &[u8], state: &ShellState) -> (Vec<u8>, i32) {
    (format!("{}\n", state.cwd_display()).into_bytes(), 0)
}

fn env(_argv: &[String], _stdin: &[u8], state: &ShellState) -> (Vec<u8>, i32) {
    let mut out = String::new();
    for (key, value) in state.env_iter() {
        out.push_str(key);
        out.push('=');
        out.push_str(value);
        out.push('\n');
    }
    (out.into_bytes(), 0)
}

fn cd(argv: &[String], state: &mut ShellState) -> i32 {
    let target = argv.get(1).map_or(".", String::as_str);
    state.set_cwd(join_cwd(state.cwd(), target));
    0
}

fn export(argv: &[String], state: &mut ShellState) -> i32 {
    for pair in &argv[1..] {
        if let Some((key, value)) = pair.split_once('=') {
            state.env_set(key.to_owned(), value.to_owned());
        }
    }
    0
}

fn unset(argv: &[String], state: &mut ShellState) -> i32 {
    for key in &argv[1..] {
        state.env_unset(key);
    }
    0
}
