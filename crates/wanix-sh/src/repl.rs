//! Interactive REPL: guest-owned cooked-line editing over a raw byte stream.
//!
//! Per ADR 0003 the *shell* owns echo, simple editing, Ctrl-C line
//! cancellation, Ctrl-D exit, and command dispatch; the host and terminal
//! clients only forward bytes. [`LineEditor`] is the pure byte → (echo, event)
//! state machine, so every editing behavior is host-testable; [`run_repl`] is
//! the thin effectful loop that reads stdin, echoes, and dispatches completed
//! lines through the ordinary parse → lower → execute path.
//!
//! Deliberately not here yet: tab completion, history, cursor movement, and
//! escape-sequence (arrow-key) handling — editing is byte-wise ASCII, and
//! control bytes outside the set below are ignored. The prompt is `$PS1`
//! (default [`DEFAULT_PS1`]) and a non-zero `$?` is shown as a `[code]`
//! prefix so command failure is visible between prompts.

use crate::error::ShellError;
use crate::exec::execute_plan;
use crate::lower::lower;
use crate::ns::NamespaceOps;
use crate::prompt::{DEFAULT_PS1, render as render_prompt};
use crate::state::ShellState;
use crate::syntax::parse_program;

const CTRL_C: u8 = 0x03;
const CTRL_D: u8 = 0x04;
const BACKSPACE: u8 = 0x08;
const DELETE: u8 = 0x7f;

/// A line-level event produced by feeding bytes to the editor.
enum LineEvent {
    /// Enter completed a line; dispatch it.
    Line(String),
    /// Ctrl-C cancelled the pending line; reprint the prompt.
    Cancelled,
    /// Ctrl-D on an empty line; end the session.
    Eof,
}

/// What one input byte produced: bytes to echo, and at most one line event.
struct Feed {
    echo: Vec<u8>,
    event: Option<LineEvent>,
}

impl Feed {
    fn echo(echo: &[u8]) -> Self {
        Self {
            echo: echo.to_vec(),
            event: None,
        }
    }

    fn event(echo: &[u8], event: LineEvent) -> Self {
        Self {
            echo: echo.to_vec(),
            event: Some(event),
        }
    }
}

/// The pure cooked-line editor: a byte buffer plus the editing rules.
#[derive(Default)]
struct LineEditor {
    buf: Vec<u8>,
}

impl LineEditor {
    fn feed(&mut self, byte: u8) -> Feed {
        match byte {
            b'\r' | b'\n' => {
                let line = String::from_utf8_lossy(&self.buf).into_owned();
                self.buf.clear();
                Feed::event(b"\n", LineEvent::Line(line))
            }
            CTRL_C => {
                self.buf.clear();
                Feed::event(b"^C\n", LineEvent::Cancelled)
            }
            CTRL_D if self.buf.is_empty() => Feed::event(b"\n", LineEvent::Eof),
            BACKSPACE | DELETE => {
                if self.buf.pop().is_some() {
                    // Move back, blank the cell, move back again.
                    Feed::echo(b"\x08 \x08")
                } else {
                    Feed::echo(b"")
                }
            }
            // Other control bytes (incl. mid-line Ctrl-D and escape sequences)
            // are ignored rather than inserted into the line.
            byte if byte < 0x20 => Feed::echo(b""),
            byte => {
                self.buf.push(byte);
                Feed::echo(&[byte])
            }
        }
    }
}

/// Runs the interactive shell until end-of-input, Ctrl-D, or `exit`.
///
/// Returns the session's exit code: the `exit` builtin's code when invoked,
/// otherwise the last command's status.
#[must_use]
pub fn run_repl(state: &mut ShellState, ns: &mut dyn NamespaceOps) -> i32 {
    state.set_interactive(true);
    let mut editor = LineEditor::default();
    write_prompt(state, ns);
    let mut byte = [0u8; 1];
    loop {
        match ns.read_stdin(&mut byte) {
            Ok(0) => return state.last_status(),
            Ok(_) => {}
            Err(err) => {
                let _ = ns.write_stderr(format!("wsh: stdin: {err}\n").as_bytes());
                return state.last_status();
            }
        }
        let fed = editor.feed(byte[0]);
        if !fed.echo.is_empty() {
            let _ = ns.write_stdout(&fed.echo);
        }
        match fed.event {
            None => {}
            Some(LineEvent::Eof) => return state.last_status(),
            Some(LineEvent::Cancelled) => write_prompt(state, ns),
            Some(LineEvent::Line(line)) => {
                if let Some(code) = dispatch_line(&line, state, ns) {
                    return code;
                }
                write_prompt(state, ns);
            }
        }
    }
}

/// Parses, lowers, and executes one completed line. Returns `Some(code)` only
/// when the `exit` builtin ended the session.
fn dispatch_line(line: &str, state: &mut ShellState, ns: &mut dyn NamespaceOps) -> Option<i32> {
    if line.trim().is_empty() {
        return None;
    }
    let program = match parse_program(line) {
        Ok(program) => program,
        Err(err) => return report(state, ns, &err),
    };
    let plan = match lower(&program) {
        Ok(plan) => plan,
        Err(err) => return report(state, ns, &err),
    };
    let outcome = execute_plan(&plan, state, ns);
    state.set_last_status(outcome.status);
    outcome.exited.then_some(outcome.status)
}

fn report(state: &mut ShellState, ns: &mut dyn NamespaceOps, err: &ShellError) -> Option<i32> {
    let _ = ns.write_stderr(format!("wsh: {err}\n").as_bytes());
    state.set_last_status(2);
    None
}

fn write_prompt(state: &ShellState, ns: &mut dyn NamespaceOps) {
    let ps1 = state.env_get("PS1").unwrap_or(DEFAULT_PS1);
    let rendered = render_prompt(ps1, state);
    let prompt = if state.last_status() == 0 {
        rendered
    } else {
        format!("[{}] {rendered}", state.last_status())
    };
    let _ = ns.write_stdout(prompt.as_bytes());
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use super::run_repl;
    use crate::error::ShellResult;
    use crate::ns::{NamespaceOps, SpawnHandle, SpawnSpec};
    use crate::state::ShellState;

    /// A scripted terminal: `read_stdin` drains a typed byte script (then
    /// reports end-of-stream) and stdout/stderr are captured for assertions.
    #[derive(Default)]
    struct TypedNs {
        input: VecDeque<u8>,
        out: Vec<u8>,
        err: Vec<u8>,
    }

    impl TypedNs {
        fn typing(script: &str) -> Self {
            Self {
                input: script.bytes().collect(),
                ..Self::default()
            }
        }

        fn out(&self) -> String {
            String::from_utf8_lossy(&self.out).into_owned()
        }
    }

    impl NamespaceOps for TypedNs {
        fn write_stdout(&mut self, bytes: &[u8]) -> ShellResult<()> {
            self.out.extend_from_slice(bytes);
            Ok(())
        }
        fn write_stderr(&mut self, bytes: &[u8]) -> ShellResult<()> {
            self.err.extend_from_slice(bytes);
            Ok(())
        }
        fn read_stdin(&mut self, buf: &mut [u8]) -> ShellResult<usize> {
            match self.input.pop_front() {
                Some(byte) => {
                    buf[0] = byte;
                    Ok(1)
                }
                None => Ok(0),
            }
        }
        fn exists(&self, _path: &str) -> ShellResult<bool> {
            Ok(false)
        }
        fn read_file(&mut self, path: &str) -> ShellResult<Vec<u8>> {
            Err(crate::ShellError::Io(format!("{path}: not found")))
        }
        fn write_file(&mut self, _path: &str, _bytes: &[u8], _append: bool) -> ShellResult<()> {
            Ok(())
        }
        fn pipe_new(&mut self) -> ShellResult<String> {
            Err(crate::ShellError::Io("no pipes in this fake".into()))
        }
        fn pipe_read_all(&mut self, _id: &str) -> ShellResult<Vec<u8>> {
            Ok(Vec::new())
        }
        fn pipe_open_writer(&mut self, _id: &str) -> ShellResult<()> {
            Ok(())
        }
        fn pipe_break_reader(&mut self, _id: &str) -> ShellResult<()> {
            Ok(())
        }
        fn pipe_write_all_and_close(&mut self, _id: &str, _bytes: &[u8]) -> ShellResult<()> {
            Ok(())
        }
        fn spawn_start(&mut self, _spec: &SpawnSpec) -> ShellResult<SpawnHandle> {
            Err(crate::ShellError::Io("no externals in this fake".into()))
        }
        fn spawn_wait(&mut self, _handle: &SpawnHandle) -> ShellResult<i32> {
            Err(crate::ShellError::Io("no externals in this fake".into()))
        }
    }

    fn repl(script: &str) -> (i32, TypedNs) {
        let mut ns = TypedNs::typing(script);
        let mut state = ShellState::new();
        let code = run_repl(&mut state, &mut ns);
        (code, ns)
    }

    #[test]
    fn types_a_command_sees_echo_output_and_next_prompt() {
        let (code, ns) = repl("echo hi\r");
        // Prompt, byte-wise echo of the typed line, newline, output, prompt.
        assert_eq!(ns.out(), "/ $ echo hi\nhi\n/ $ ");
        // Script exhausted -> end-of-stream exit with the last status.
        assert_eq!(code, 0);
    }

    #[test]
    fn newline_byte_also_completes_a_line() {
        let (_code, ns) = repl("echo hi\n");
        assert!(ns.out().contains("\nhi\n"), "{:?}", ns.out());
    }

    #[test]
    fn backspace_edits_the_pending_line() {
        // Type `echox`, erase the `x`, finish ` hi`.
        let (code, ns) = repl("echox\x7f hi\r");
        assert_eq!(code, 0);
        assert!(ns.out().contains("\x08 \x08"), "{:?}", ns.out());
        assert!(ns.out().contains("\nhi\n"), "{:?}", ns.out());
    }

    #[test]
    fn backspace_on_empty_line_is_silent() {
        let (_code, ns) = repl("\x7fecho hi\r");
        assert!(!ns.out().contains('\x08'), "{:?}", ns.out());
        assert!(ns.out().contains("\nhi\n"), "{:?}", ns.out());
    }

    #[test]
    fn ctrl_c_cancels_the_line_and_reprompts() {
        let (code, ns) = repl("echo bad\x03echo ok\r");
        assert_eq!(code, 0);
        assert!(ns.out().contains("^C\n/ $ "), "{:?}", ns.out());
        assert!(ns.out().contains("\nok\n"), "{:?}", ns.out());
        assert!(
            !ns.out().contains("bad\n"),
            "cancelled line ran: {:?}",
            ns.out()
        );
    }

    #[test]
    fn ctrl_d_on_empty_line_exits_with_last_status() {
        let (code, ns) = repl("false\r\x04unreached\r");
        assert_eq!(code, 1, "Ctrl-D exits with $?");
        // The post-`false` prompt shows the non-zero status.
        assert!(ns.out().contains("[1] / $ "), "{:?}", ns.out());
        assert!(!ns.out().contains("unreached"), "{:?}", ns.out());
    }

    #[test]
    fn ctrl_d_mid_line_is_ignored() {
        let (code, ns) = repl("echo hi\x04\r");
        assert_eq!(code, 0);
        assert!(ns.out().contains("\nhi\n"), "{:?}", ns.out());
    }

    #[test]
    fn exit_builtin_ends_the_session_with_its_code() {
        let (code, ns) = repl("exit 3\recho after\r");
        assert_eq!(code, 3);
        assert!(!ns.out().contains("after"), "{:?}", ns.out());
    }

    #[test]
    fn empty_line_just_reprompts() {
        let (code, ns) = repl("\r\r");
        assert_eq!(code, 0);
        assert_eq!(ns.out(), "/ $ \n/ $ \n/ $ ");
    }

    #[test]
    fn parse_error_reports_and_sets_status_2() {
        let (code, ns) = repl("echo 'unterminated\r");
        assert_eq!(code, 2, "end-of-stream exit carries the parse status");
        assert!(!ns.err.is_empty(), "parse error should be reported");
        assert!(ns.out().contains("[2] / $ "), "{:?}", ns.out());
    }

    #[test]
    fn state_persists_across_lines() {
        let (_code, ns) = repl("export GREETING=hello\recho $GREETING\r");
        assert!(ns.out().contains("\nhello\n"), "{:?}", ns.out());
    }

    #[test]
    fn ps1_env_overrides_the_prompt() {
        let (_code, ns) = repl("export PS1='> '\recho hi\r");
        assert!(ns.out().contains("> echo hi"), "{:?}", ns.out());
    }
}
