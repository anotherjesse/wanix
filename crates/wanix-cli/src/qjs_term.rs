#[cfg(all(test, unix))]
use std::collections::VecDeque;
use std::io::{Read, Write};
#[cfg(all(test, unix))]
use std::sync::Arc;
#[cfg(all(test, unix))]
use std::sync::Mutex;

use super::{CliError, CliOutput};

const QJS_SHELL_SOURCE: &str = include_str!("../../../examples/qjs-term-shell-demo.js");
const QJS_SHELL_SCRIPT_SENTINEL: &str = "__wanix_qjs_shell.js";
const QJS_SHELL_READY_IO_TURNS: usize = 2;
const QJS_SHELL_IDLE_EVENT_LOOP_BUDGET_MS: u64 = 20;

mod command;
mod post_eval;
mod process;
mod program_spec;
mod pump;
mod runtime;
mod session;
mod terminal;

use command::{PostEvalFeed, qjs_shell_command};
pub(super) use command::{
    QjsShellCommand, QjsTermCommand, parse_qjs_shell_command, parse_qjs_term_command,
};
use program_spec::QjsTermProgram;
use pump::ProcessEventSources;
use runtime::{QjsTermProgramIo, QjsTermProgramRequest, run_qjs_term_program_streaming};
pub(crate) use session::QjsShellSession;

// Shared terminal-host plumbing reused by the `sh` session (the wasm-shell
// sibling of qjs-shell): device attach, byte feed/drain, resize sources, and
// the polled-stdin fd helpers. The qjs-specific pump (event-loop turns) stays
// private to this module.
#[cfg(unix)]
pub(crate) use process::{
    NonBlockingFd, ProcessStdinPoll, ProcessStdinRead, poll_process_stdin,
    read_process_stdin_after_poll,
};
#[cfg(unix)]
pub(crate) use pump::terminal_size_source;
pub(crate) use pump::{
    ProcessResizeSource, drain_terminal_output_bytes, feed_terminal_after_eval,
    feed_terminal_resize_after_eval,
};
pub(crate) use terminal::{AttachedTerminal, attach_task_terminal};

pub(super) struct QjsShellStreamingIo<'a> {
    pub(super) process_stdin: &'a mut dyn Read,
    pub(super) process_stdout: &'a mut dyn Write,
    pub(super) process_stderr: &'a mut dyn Write,
}

pub(super) fn run_qjs_term(
    command: QjsTermCommand,
    process_stdin: &mut dyn Read,
) -> Result<CliOutput, CliError> {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let exit_code = run_qjs_term_streaming(command, process_stdin, &mut stdout, &mut stderr)?;
    Ok(CliOutput::new(stdout, stderr, exit_code))
}

pub(super) fn run_qjs_term_streaming(
    command: QjsTermCommand,
    process_stdin: &mut dyn Read,
    process_stdout: &mut dyn Write,
    process_stderr: &mut dyn Write,
) -> Result<i32, CliError> {
    run_qjs_term_program_streaming(
        QjsTermProgramRequest::blocking(
            command.qjs,
            command.feed_after_eval,
            QjsTermProgram::HostScript,
        ),
        QjsTermProgramIo {
            process_stdin,
            process_stdout,
            process_stderr,
        },
    )
}

fn qjs_shell_feed_after_eval(raw: bool) -> Vec<PostEvalFeed> {
    vec![if raw {
        PostEvalFeed::RawBytesProcess
    } else {
        PostEvalFeed::LinesProcess
    }]
}

fn qjs_shell_program_request(
    command: QjsShellCommand,
    event_sources: ProcessEventSources,
) -> QjsTermProgramRequest {
    QjsTermProgramRequest {
        qjs_command: qjs_shell_command(command.qjs, command.raw),
        feed_after_eval: qjs_shell_feed_after_eval(command.raw),
        program: QjsTermProgram::BundledShell,
        event_sources,
    }
}

fn qjs_term_program_io<'a>(
    process_stdin: &'a mut dyn Read,
    process_stdout: &'a mut dyn Write,
    process_stderr: &'a mut dyn Write,
) -> QjsTermProgramIo<'a> {
    QjsTermProgramIo {
        process_stdin,
        process_stdout,
        process_stderr,
    }
}

pub(super) fn run_qjs_shell(
    command: QjsShellCommand,
    process_stdin: &mut dyn Read,
) -> Result<CliOutput, CliError> {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let exit_code = run_qjs_shell_streaming(command, process_stdin, &mut stdout, &mut stderr)?;
    Ok(CliOutput::new(stdout, stderr, exit_code))
}

pub(super) fn run_qjs_shell_streaming(
    command: QjsShellCommand,
    process_stdin: &mut dyn Read,
    process_stdout: &mut dyn Write,
    process_stderr: &mut dyn Write,
) -> Result<i32, CliError> {
    run_qjs_term_program_streaming(
        qjs_shell_program_request(command, ProcessEventSources::blocking()),
        qjs_term_program_io(process_stdin, process_stdout, process_stderr),
    )
}

#[cfg(unix)]
pub(super) fn run_qjs_shell_streaming_with_input_fd(
    command: QjsShellCommand,
    input_fd: libc::c_int,
    io: QjsShellStreamingIo<'_>,
) -> Result<i32, CliError> {
    run_qjs_term_program_streaming(
        qjs_shell_program_request(command, ProcessEventSources::input_fd(input_fd)),
        QjsTermProgramIo {
            process_stdin: io.process_stdin,
            process_stdout: io.process_stdout,
            process_stderr: io.process_stderr,
        },
    )
}

#[cfg(unix)]
pub(super) fn run_qjs_shell_streaming_with_terminal_fds(
    command: QjsShellCommand,
    input_fd: libc::c_int,
    terminal_size_fd: libc::c_int,
    io: QjsShellStreamingIo<'_>,
) -> Result<i32, CliError> {
    run_qjs_term_program_streaming(
        qjs_shell_program_request(
            command,
            ProcessEventSources::terminal_fds(input_fd, terminal_size_fd),
        ),
        QjsTermProgramIo {
            process_stdin: io.process_stdin,
            process_stdout: io.process_stdout,
            process_stderr: io.process_stderr,
        },
    )
}

#[cfg(all(test, unix))]
pub(super) fn run_qjs_shell_streaming_with_resize_queue(
    command: QjsShellCommand,
    input_fd: libc::c_int,
    resize_queue: Arc<Mutex<VecDeque<(u16, u16)>>>,
    io: QjsShellStreamingIo<'_>,
) -> Result<i32, CliError> {
    run_qjs_term_program_streaming(
        qjs_shell_program_request(
            command,
            ProcessEventSources::resize_queue(input_fd, resize_queue),
        ),
        QjsTermProgramIo {
            process_stdin: io.process_stdin,
            process_stdout: io.process_stdout,
            process_stderr: io.process_stderr,
        },
    )
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::pump::TermResize;
    use super::{
        PostEvalFeed, QJS_SHELL_SCRIPT_SENTINEL, QjsShellSession, parse_qjs_shell_command,
        parse_qjs_term_command,
    };
    use wanix_fs::{FileSystem, FsError, NormalizedPath, OpenOptions};

    #[test]
    fn parse_qjs_term_collects_pre_script_post_eval_feeds() {
        let command = parse_qjs_term_command(&[
            "--ready-io-turns".into(),
            "2".into(),
            "--feed-after-eval".into(),
            "first".into(),
            "--feed-after-eval-file".into(),
            "second.txt".into(),
            "demo.js".into(),
            "--".into(),
            "arg".into(),
        ])
        .unwrap();

        assert_eq!(
            command.feed_after_eval,
            [
                PostEvalFeed::Bytes(b"first".to_vec()),
                PostEvalFeed::File(PathBuf::from("second.txt"))
            ]
        );
        assert_eq!(command.qjs.script_path, PathBuf::from("demo.js"));
        assert_eq!(command.qjs.args, vec!["arg".to_owned()]);
        assert_eq!(command.qjs.ready_io_turns, 2);
    }

    #[test]
    fn parse_qjs_term_continues_after_qjs_value_options_before_script() {
        let command = parse_qjs_term_command(&[
            "--cwd".into(),
            "app".into(),
            "--feed-after-eval".into(),
            "input".into(),
            "--ready-io-turns".into(),
            "3".into(),
            "--resize-after-eval".into(),
            "120x50".into(),
            "demo.js".into(),
        ])
        .unwrap();

        assert_eq!(command.qjs.cwd.as_str(), "app");
        assert_eq!(command.qjs.ready_io_turns, 3);
        assert_eq!(command.qjs.script_path, PathBuf::from("demo.js"));
        assert_eq!(
            command.feed_after_eval,
            [
                PostEvalFeed::Bytes(b"input".to_vec()),
                PostEvalFeed::Resize(TermResize {
                    columns: 120,
                    rows: 50
                })
            ]
        );
    }

    #[test]
    fn parse_qjs_term_preserves_feed_option_after_script_as_argument() {
        let command = parse_qjs_term_command(&[
            "demo.js".into(),
            "--".into(),
            "--feed-after-eval".into(),
            "script-arg".into(),
        ])
        .unwrap();

        assert!(command.feed_after_eval.is_empty());
        assert_eq!(
            command.qjs.args,
            vec!["--feed-after-eval".to_owned(), "script-arg".to_owned()]
        );
    }

    #[test]
    fn parse_qjs_term_collects_process_post_eval_feed() {
        let command = parse_qjs_term_command(&[
            "--feed-after-eval-file".into(),
            "-".into(),
            "demo.js".into(),
        ])
        .unwrap();

        assert_eq!(command.feed_after_eval, [PostEvalFeed::Process]);
        assert_eq!(command.qjs.script_path, PathBuf::from("demo.js"));
    }

    #[test]
    fn parse_qjs_term_collects_line_segmented_post_eval_feed() {
        let command = parse_qjs_term_command(&[
            "--feed-after-eval-lines".into(),
            "session.txt".into(),
            "--feed-after-eval-lines".into(),
            "-".into(),
            "demo.js".into(),
        ])
        .unwrap();

        assert_eq!(
            command.feed_after_eval,
            [
                PostEvalFeed::LinesFile(PathBuf::from("session.txt")),
                PostEvalFeed::LinesProcess
            ]
        );
        assert_eq!(command.qjs.script_path, PathBuf::from("demo.js"));
    }

    #[test]
    fn parse_qjs_term_collects_post_eval_resize_feed() {
        let command = parse_qjs_term_command(&[
            "--resize-after-eval".into(),
            "100x40".into(),
            "demo.js".into(),
        ])
        .unwrap();

        assert_eq!(
            command.feed_after_eval,
            [PostEvalFeed::Resize(TermResize {
                columns: 100,
                rows: 40
            })]
        );
        assert_eq!(command.qjs.script_path, PathBuf::from("demo.js"));
    }

    #[test]
    fn parse_qjs_term_rejects_invalid_post_eval_resize_feed() {
        let error = parse_qjs_term_command(&[
            "--resize-after-eval".into(),
            "100by40".into(),
            "demo.js".into(),
        ])
        .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("qjs-term --resize-after-eval expects COLSxROWS")
        );
    }

    #[test]
    fn parse_qjs_shell_uses_bundled_script_sentinel_without_script_args() {
        let command = parse_qjs_shell_command(&[
            "--raw".into(),
            "--cwd".into(),
            "app".into(),
            "--ready-io-turns".into(),
            "2".into(),
        ])
        .unwrap();

        assert_eq!(
            command.qjs.script_path,
            PathBuf::from(QJS_SHELL_SCRIPT_SENTINEL)
        );
        assert_eq!(command.qjs.cwd.as_str(), "app");
        assert_eq!(command.qjs.ready_io_turns, 2);
        assert!(command.qjs.args.is_empty());
        assert!(command.qjs.stdin.is_none());
        assert!(command.raw);
    }

    #[test]
    fn parse_qjs_shell_rejects_script_path_and_preloaded_stdin() {
        let script_error = parse_qjs_shell_command(&["demo.js".into()]).unwrap_err();
        assert!(
            script_error
                .to_string()
                .contains("qjs-shell does not accept a script path")
        );

        let stdin_error =
            parse_qjs_shell_command(&["--stdin".into(), "preloaded".into()]).unwrap_err();
        assert!(
            stdin_error
                .to_string()
                .contains("qjs-shell reads native stdin as terminal input")
        );
    }

    #[test]
    fn qjs_shell_session_runs_bundled_shell_from_host_root() {
        let root = temp_dir("wanix-qjs-shell-session");
        fs::write(root.join("visible.txt"), "served root").unwrap();
        let (mut session, initial_output) = QjsShellSession::start(&root).unwrap();

        assert_eq!(initial_output, b"shell task: 1\r\n$ ");
        let output = session.input(b"echo hello ws\nexit\n").unwrap();

        assert_eq!(output, b"echo hello ws\r\nhello ws\r\n$ exit\r\nbye\r\n");
        assert!(session.is_finished());
    }

    #[test]
    fn qjs_shell_session_pumps_delayed_output_without_input() {
        let root = temp_dir("wanix-qjs-shell-session-pump");
        let (mut session, initial_output) = QjsShellSession::start(&root).unwrap();

        assert_eq!(initial_output, b"shell task: 1\r\n$ ");
        let output = session.input(b"later tick\n").unwrap();
        assert_eq!(output, b"later tick\r\nscheduled\r\n");

        let output = session.pump().unwrap();
        assert_eq!(output, b"later: tick\r\n$ ");
        assert!(!session.is_finished());

        let output = session.input(b"exit\n").unwrap();
        assert_eq!(output, b"exit\r\nbye\r\n");
        assert!(session.is_finished());
    }

    #[test]
    fn qjs_shell_session_closes_owned_terminal_on_drop() {
        let root = temp_dir("wanix-qjs-shell-session-drop");
        let (session, _initial_output) = QjsShellSession::start(&root).unwrap();
        let terminal = session.terminal_for_test();
        let terminal_id = session.terminal_id_for_test().to_owned();

        drop(session);

        let result = terminal.open(
            &NormalizedPath::new(format!("{terminal_id}/data")).unwrap(),
            OpenOptions {
                read: true,
                ..OpenOptions::default()
            },
        );
        assert!(matches!(result, Err(FsError::NotFound)));
    }

    #[test]
    fn qjs_shell_session_close_releases_terminal_resource() {
        let root = temp_dir("wanix-qjs-shell-session-close");
        let (mut session, _initial_output) = QjsShellSession::start(&root).unwrap();
        let terminal = session.terminal_for_test();
        let terminal_id = session.terminal_id_for_test().to_owned();

        assert!(
            terminal
                .metadata(&NormalizedPath::new(format!("{terminal_id}/id")).unwrap())
                .is_ok()
        );

        session.close_terminal_resource().unwrap();
        session.close_terminal_resource().unwrap();

        assert!(matches!(
            terminal.metadata(&NormalizedPath::new(format!("{terminal_id}/id")).unwrap()),
            Err(FsError::NotFound)
        ));
    }

    #[test]
    fn qjs_shell_session_close_tolerates_external_terminal_close() {
        let root = temp_dir("wanix-qjs-shell-session-external-close");
        let (mut session, _initial_output) = QjsShellSession::start(&root).unwrap();
        let terminal = session.terminal_for_test();
        let terminal_id = session.terminal_id_for_test().to_owned();

        terminal.close(&terminal_id).unwrap();

        session.close_terminal_resource().unwrap();
        session.close_terminal_resource().unwrap();
    }

    #[test]
    fn qjs_shell_session_can_start_in_served_cwd() {
        let root = temp_dir("wanix-qjs-shell-session-cwd");
        fs::create_dir(root.join("app")).unwrap();
        let cwd = NormalizedPath::new("app").unwrap();
        let (mut session, initial_output) = QjsShellSession::start_in_cwd(&root, &cwd).unwrap();

        assert_eq!(initial_output, b"shell task: 1\r\n$ ");
        assert!(session.resize(100, 40).unwrap().is_empty());
        let output = session.input(b"pwd\nsize\nexit\n").unwrap();

        assert_eq!(
            output,
            b"pwd\r\napp\r\n$ size\r\nsize 100 40\r\n$ exit\r\nbye\r\n"
        );
        assert!(session.is_finished());
    }

    #[test]
    fn qjs_shell_session_starts_child_qjs_task() {
        let root = temp_dir("wanix-qjs-shell-session-child");
        fs::write(root.join("input.txt"), "redirected stdin\n").unwrap();
        fs::write(
            root.join("foreground-child.js"),
            r##"import * as std from "qjs:std";
import * as os from "qjs:os";

const bytes = new Uint8Array(64);
const count = os.read(0, bytes.buffer, 0, bytes.length);
if (count < 0) {
  throw new Error("stdin read failed: " + count);
}
const text = Array.from(bytes.slice(0, count)).map((byte) => String.fromCharCode(byte)).join("");
std.out.puts("foreground child stdin " + text.trimEnd() + "\n");
std.exit(0);
"##,
        )
        .unwrap();
        fs::write(
            root.join("terminal-child.js"),
            r#"import * as std from "qjs:std";

std.out.puts("terminal child stdout\n");
std.err.puts("terminal child stderr\n");
std.exit(0);
"#,
        )
        .unwrap();
        fs::write(
            root.join("child.js"),
            r##"import * as std from "qjs:std";
import * as os from "qjs:os";

function readStdin() {
  const bytes = new Uint8Array(64);
  const count = os.read(0, bytes.buffer, 0, bytes.length);
  if (count < 0) {
    throw new Error("stdin read failed: " + count);
  }
  return Array.from(bytes.slice(0, count)).map((byte) => String.fromCharCode(byte)).join("");
}

std.out.puts("child task " + std.loadFile("#task/self/id").trim() + "\n");
std.out.puts("child cwd " + std.loadFile("#task/self/dir").trim() + "\n");
std.out.puts("child argv " + scriptArgs.join("|") + "\n");
std.out.puts("child stdin " + readStdin().trimEnd() + "\n");
std.out.puts("child raw " + std.getenv("WANIX_QJS_SHELL_RAW") + "\n");
std.out.puts("child mode " + std.getenv("MODE") + "\n");
std.out.flush();
std.err.puts("child stderr " + scriptArgs[1] + "\n");
std.err.flush();
std.exit(7);
"##,
        )
        .unwrap();
        let (mut session, initial_output) = QjsShellSession::start(&root).unwrap();

        assert_eq!(initial_output, b"shell task: 1\r\n$ ");
        let output = session
            .input(b"qjs foreground-child.js\nforeground stdin\n")
            .unwrap();
        assert_eq!(
            output,
            b"qjs foreground-child.js\r\nforeground child stdin foreground stdin\r\n$ "
        );
        let output = session
            .input(
                b"qjs terminal-child.js\nenv\nsetenv MODE throwaway\nenv MODE\nunsetenv MODE\nenv MODE\nsetenv MODE shell-mode\nenv MODE\nqjs child.js alpha 'two words' < input.txt > child-out.txt 2> child-err.txt\nstatus\nps\ncat child-out.txt\ncat child-err.txt\nexit\n",
            )
            .unwrap();

        assert_eq!(
            output,
            b"qjs terminal-child.js\r\nterminal child stdout\r\nterminal child stderr\r\n$ env\r\nWANIX_QJS_SHELL_RAW=1\r\n$ setenv MODE throwaway\r\n$ env MODE\r\nMODE=throwaway\r\n$ unsetenv MODE\r\n$ env MODE\r\n$ setenv MODE shell-mode\r\n$ env MODE\r\nMODE=shell-mode\r\n$ qjs child.js alpha 'two words' < input.txt > child-out.txt 2> child-err.txt\r\nqjs exit 7\r\n$ status\r\nstatus 7\r\n$ ps\r\nid kind exit dir cmd\r\n1 qjs - . __wanix_qjs_shell.js\r\n2 qjs 0 . foreground-child.js\r\n3 qjs 0 . terminal-child.js\r\n4 qjs 7 . child.js alpha 'two words'\r\n$ cat child-out.txt\r\nchild task 4\r\nchild cwd .\r\nchild argv child.js|alpha|two words\r\nchild stdin redirected stdin\r\nchild raw 1\r\nchild mode shell-mode\r\n$ cat child-err.txt\r\nchild stderr alpha\r\n$ exit\r\nbye\r\n"
        );
        assert!(session.is_finished());
    }

    #[test]
    fn qjs_shell_session_starts_child_wasm_task() {
        // BothShells (ADR 0002): the qjs-shell's synchronous launcher allocates
        // `#task/new/auto`, so a compiled `.wasm` program is claimed by the
        // wasm driver registered on the session table — launch + wait + exit
        // status, no terminal handoff (stdio is redirected or unused).
        const RUST_GUEST: &[u8] = include_bytes!("../../wanix-wasm/fixtures/rust-guest.wasm");
        let root = temp_dir("wanix-qjs-shell-session-wasm-child");
        fs::write(root.join("guest.wasm"), RUST_GUEST).unwrap();
        fs::write(root.join("in.txt"), "wasm child input").unwrap();
        let (mut session, initial_output) = QjsShellSession::start(&root).unwrap();

        assert_eq!(initial_output, b"shell task: 1\r\n$ ");
        let output = session
            .input(b"qjs guest.wasm /in.txt /out.txt\nstatus\ncat out.txt\nps\nexit\n")
            .unwrap();
        let output = String::from_utf8(output).unwrap();

        // The child's stdout (inherited fd 1) reaches the terminal…
        assert!(output.contains("rust-wasm: read 16 bytes"), "{output}");
        // …its exit is observable through the synchronous wait…
        assert!(output.contains("status 0"), "{output}");
        // …its file write landed in the shared root…
        assert!(
            output.contains("rust-wasm saw: wasm child input"),
            "{output}"
        );
        // …and the auto task resolved to the wasm driver.
        assert!(output.contains("2 wasm 0 . guest.wasm"), "{output}");
        assert!(session.is_finished());
        assert_eq!(
            fs::read_to_string(root.join("out.txt")).unwrap(),
            "rust-wasm saw: wasm child input"
        );
    }

    #[test]
    fn qjs_shell_session_navigates_and_edits_served_files() {
        let root = temp_dir("wanix-qjs-shell-session-files");
        fs::write(root.join("visible.txt"), "served root\n").unwrap();
        fs::create_dir(root.join("app")).unwrap();
        fs::write(root.join("app").join("note.txt"), "from app\n").unwrap();
        let (mut session, initial_output) = QjsShellSession::start(&root).unwrap();

        assert_eq!(initial_output, b"shell task: 1\r\n$ ");
        let output = session
            .input(
                b"ls\ncat visible.txt\ncd app\npwd\nls\ncat note.txt\nwrite made.txt made by shell\ncat made.txt\nmkdir docs\nwrite docs/readme.txt copied note\ncp docs/readme.txt copy.txt\ncat copy.txt\nmv copy.txt moved.txt\ncat moved.txt\nrm moved.txt\ncat moved.txt\nrmdir docs\nrm docs/readme.txt\nrmdir docs\nls\ncat missing.txt\ncd ..\npwd\nexit\n",
            )
            .unwrap();

        assert_eq!(
            output,
            b"ls\r\napp visible.txt\r\n$ cat visible.txt\r\nserved root\r\n$ cd app\r\n$ pwd\r\napp\r\n$ ls\r\nnote.txt\r\n$ cat note.txt\r\nfrom app\r\n$ write made.txt made by shell\r\nwrote made.txt\r\n$ cat made.txt\r\nmade by shell\r\n$ mkdir docs\r\n$ write docs/readme.txt copied note\r\nwrote docs/readme.txt\r\n$ cp docs/readme.txt copy.txt\r\n$ cat copy.txt\r\ncopied note\r\n$ mv copy.txt moved.txt\r\n$ cat moved.txt\r\ncopied note\r\n$ rm moved.txt\r\n$ cat moved.txt\r\ncat: moved.txt: not found\r\n$ rmdir docs\r\nrmdir: docs: directory not empty\r\n$ rm docs/readme.txt\r\n$ rmdir docs\r\n$ ls\r\nmade.txt note.txt\r\n$ cat missing.txt\r\ncat: missing.txt: not found\r\n$ cd ..\r\n$ pwd\r\n.\r\n$ exit\r\nbye\r\n"
        );
        assert_eq!(
            fs::read_to_string(root.join("app").join("made.txt")).unwrap(),
            "made by shell\n"
        );
        assert!(!root.join("app").join("docs").exists());
        assert!(!root.join("app").join("moved.txt").exists());
        assert!(session.is_finished());
    }

    #[cfg(unix)]
    #[test]
    fn qjs_shell_session_reports_stat_metadata() {
        use std::os::unix::fs::symlink;

        let root = temp_dir("wanix-qjs-shell-session-stat");
        fs::write(root.join("visible.txt"), "served root\n").unwrap();
        fs::create_dir(root.join("app")).unwrap();
        symlink("visible.txt", root.join("link.txt")).unwrap();
        let (mut session, initial_output) = QjsShellSession::start(&root).unwrap();

        assert_eq!(initial_output, b"shell task: 1\r\n$ ");
        let output = session
            .input(
                b"stat visible.txt\nstat app\nlstat link.txt\nstat link.txt\nstat missing.txt\nexit\n",
            )
            .unwrap();
        let output = String::from_utf8(output).unwrap();

        assert!(
            output.contains("visible.txt type file mode 100"),
            "{output}"
        );
        assert!(
            output.contains("visible.txt type file") && output.contains("size 12"),
            "{output}"
        );
        assert!(output.contains("app type dir mode 40"), "{output}");
        assert!(
            output.contains("link.txt type symlink mode 120"),
            "{output}"
        );
        assert!(output.contains("link.txt type file mode 100"), "{output}");
        assert!(output.contains("stat: missing.txt: errno "), "{output}");
        assert!(output.ends_with("$ exit\r\nbye\r\n"), "{output}");
        assert!(session.is_finished());
    }

    #[test]
    fn qjs_shell_session_reports_served_resize() {
        let root = temp_dir("wanix-qjs-shell-session-resize");
        let (mut session, initial_output) = QjsShellSession::start(&root).unwrap();

        assert_eq!(initial_output, b"shell task: 1\r\n$ ");
        assert!(session.resize(100, 40).unwrap().is_empty());
        let output = session.input(b"size\nexit\n").unwrap();

        assert_eq!(output, b"size\r\nsize 100 40\r\n$ exit\r\nbye\r\n");
        assert!(session.is_finished());
    }

    fn temp_dir(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("{name}-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&path).unwrap();
        path
    }
}
