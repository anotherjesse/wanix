use std::ffi::OsString;
use std::io::{Read, Write};

mod fd;

use crate::{
    CliError, agent_exec_server, app, cpu, mesh, mount, p9_stdio, qemu, qjs_term, run_collected,
    serve, tool, volume, write_process_output,
};

/// Unix fd pair used by terminal-aware CLI entrypoints.
#[cfg(unix)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UnixTerminalFds {
    stdin_fd: libc::c_int,
    terminal_size_fd: libc::c_int,
}

#[cfg(unix)]
impl UnixTerminalFds {
    /// Builds a terminal fd pair from the input fd and the fd used for terminal
    /// size queries.
    #[must_use]
    pub fn new(stdin_fd: libc::c_int, terminal_size_fd: libc::c_int) -> Self {
        Self {
            stdin_fd,
            terminal_size_fd,
        }
    }

    pub(crate) fn stdin_fd(self) -> libc::c_int {
        self.stdin_fd
    }

    pub(crate) fn terminal_size_fd(self) -> libc::c_int {
        self.terminal_size_fd
    }
}

#[cfg(all(unix, test))]
pub(super) fn run_with_resize_queue(
    args: Vec<OsString>,
    io: &mut ProcessIo<'_>,
    stdin_fd: libc::c_int,
    resize_queue: std::sync::Arc<std::sync::Mutex<std::collections::VecDeque<(u16, u16)>>>,
) -> Result<i32, CliError> {
    fd::run_with_resize_queue(args, io, stdin_fd, resize_queue)
}

#[cfg(unix)]
pub(super) fn run_with_stdin_fd(
    args: Vec<OsString>,
    io: &mut ProcessIo<'_>,
    stdin_fd: libc::c_int,
) -> Result<i32, CliError> {
    fd::run_with_stdin_fd(args, io, stdin_fd)
}

#[cfg(unix)]
pub(super) fn run_with_terminal_fds(
    args: Vec<OsString>,
    io: &mut ProcessIo<'_>,
    stdin_fd: libc::c_int,
    terminal_size_fd: libc::c_int,
) -> Result<i32, CliError> {
    fd::run_with_terminal_fds(args, io, stdin_fd, terminal_size_fd)
}

pub(super) struct ProcessIo<'a> {
    stdin: &'a mut dyn Read,
    stdout: &'a mut dyn Write,
    stderr: &'a mut dyn Write,
}

impl<'a> ProcessIo<'a> {
    pub(super) fn new(
        stdin: &'a mut dyn Read,
        stdout: &'a mut dyn Write,
        stderr: &'a mut dyn Write,
    ) -> Self {
        Self {
            stdin,
            stdout,
            stderr,
        }
    }
}

pub(super) fn run_with_process_io(
    args: Vec<OsString>,
    process_stdin: &mut dyn Read,
    process_stdout: &mut dyn Write,
    process_stderr: &mut dyn Write,
) -> Result<i32, CliError> {
    let mut io = ProcessIo::new(process_stdin, process_stdout, process_stderr);
    run_with_process_io_inner(args, &mut io)
}

fn run_with_process_io_inner(args: Vec<OsString>, io: &mut ProcessIo<'_>) -> Result<i32, CliError> {
    if let Some(exit_code) = run_streaming_command(&args, io)? {
        return Ok(exit_code);
    }
    run_collected_with_process_output(args, io)
}

fn run_streaming_command(
    args: &[OsString],
    io: &mut ProcessIo<'_>,
) -> Result<Option<i32>, CliError> {
    let Some((command, rest)) = args.split_first() else {
        return Ok(None);
    };
    // `SUBCOMMAND --help` falls through to the collected path, which answers
    // with that subcommand's usage instead of parsing `--help` as an operand.
    if crate::help::wants_help(rest) {
        return Ok(None);
    }
    match command.to_str() {
        Some("qjs-term" | "qjs-shell") => run_qjs_streaming_command(command, rest, io),
        Some("p9-stdio") => run_9p_streaming_command(command, rest, io),
        Some("mesh-serve") => Ok(Some(mesh::run_mesh_serve_streaming(
            mesh::parse_mesh_serve_command(rest)?,
            io.stderr,
        )?)),
        Some("cpu") => Ok(Some(cpu::run_cpu_streaming(
            cpu::parse_cpu_command(rest)?,
            io.stdout,
            io.stderr,
        )?)),
        Some("agent-exec-server") => Ok(Some(agent_exec_server::run_agent_exec_server_streaming(
            agent_exec_server::parse_agent_exec_server_command(rest)?,
            io.stdin,
            io.stdout,
            io.stderr,
        )?)),
        Some("qemu") => Ok(Some(run_qemu_process_io(rest, io)?)),
        Some("serve") => Ok(Some(serve::run_serve_streaming(
            serve::parse_serve_command(rest)?,
            io.stderr,
        )?)),
        // `mount-cat --follow` streams a (possibly never-EOF) remote file to
        // the live stdout; a plain `mount-cat` falls through to collected.
        Some("mount-cat") => mount::run_mount_cat_follow_streaming(rest, io.stdout),
        // `volume serve` parks (one mesh endpoint per volume); `volume create`/`ls`
        // are filesystem-only and fall through to the collected path.
        Some("volume") => run_volume_streaming_command(rest, io),
        // `tool serve` parks too (one mesh endpoint per tool).
        Some("tool") => run_tool_streaming_command(rest, io),
        // `app serve` parks (one mesh endpoint + resident guest per app).
        Some("app") => run_app_streaming_command(rest, io),
        _ => Ok(None),
    }
}

fn run_app_streaming_command(
    rest: &[OsString],
    io: &mut ProcessIo<'_>,
) -> Result<Option<i32>, CliError> {
    match rest.split_first() {
        Some((sub, serve_rest)) if sub.to_str() == Some("serve") => Ok(Some(
            app::run_app_serve_streaming(app::parse_app_serve_command(serve_rest)?, io.stderr)?,
        )),
        _ => Ok(None),
    }
}

fn run_tool_streaming_command(
    rest: &[OsString],
    io: &mut ProcessIo<'_>,
) -> Result<Option<i32>, CliError> {
    match rest.split_first() {
        Some((sub, serve_rest)) if sub.to_str() == Some("serve") => Ok(Some(
            tool::run_tool_serve_streaming(tool::parse_tool_serve_command(serve_rest)?, io.stderr)?,
        )),
        _ => Ok(None),
    }
}

fn run_volume_streaming_command(
    rest: &[OsString],
    io: &mut ProcessIo<'_>,
) -> Result<Option<i32>, CliError> {
    match rest.split_first() {
        Some((sub, serve_rest)) if sub.to_str() == Some("serve") => {
            Ok(Some(volume::run_volume_serve_streaming(
                volume::parse_volume_serve_command(serve_rest)?,
                io.stderr,
            )?))
        }
        _ => Ok(None),
    }
}

fn run_qjs_streaming_command(
    command: &OsString,
    rest: &[OsString],
    io: &mut ProcessIo<'_>,
) -> Result<Option<i32>, CliError> {
    match command.to_str() {
        Some("qjs-term") => Ok(Some(qjs_term::run_qjs_term_streaming(
            qjs_term::parse_qjs_term_command(rest)?,
            io.stdin,
            io.stdout,
            io.stderr,
        )?)),
        Some("qjs-shell") => Ok(Some(qjs_term::run_qjs_shell_streaming(
            qjs_term::parse_qjs_shell_command(rest)?,
            io.stdin,
            io.stdout,
            io.stderr,
        )?)),
        _ => Ok(None),
    }
}

fn run_9p_streaming_command(
    command: &OsString,
    rest: &[OsString],
    io: &mut ProcessIo<'_>,
) -> Result<Option<i32>, CliError> {
    match command.to_str() {
        Some("p9-stdio") => Ok(Some(p9_stdio::run_p9_stdio_streaming(
            p9_stdio::parse_p9_stdio_command(rest)?,
            io.stdin,
            io.stdout,
            io.stderr,
        )?)),
        _ => Ok(None),
    }
}

fn run_qemu_process_io(rest: &[OsString], io: &mut ProcessIo<'_>) -> Result<i32, CliError> {
    let command = qemu::parse_qemu_command(rest)?;
    if qemu::qemu_command_exec(&command) {
        return run_qemu_exec_process_io(command, io.stderr);
    }
    run_qemu_print_process_io(command, io.stdout, io.stderr)
}

fn run_qemu_exec_process_io(
    command: qemu::QemuCommand,
    process_stderr: &mut dyn Write,
) -> Result<i32, CliError> {
    qemu::run_qemu_streaming(command, process_stderr)
}

fn run_qemu_print_process_io(
    command: qemu::QemuCommand,
    process_stdout: &mut dyn Write,
    process_stderr: &mut dyn Write,
) -> Result<i32, CliError> {
    let output = qemu::run_qemu_command(command)?;
    write_process_output(process_stdout, "stdout", output.stdout())?;
    write_process_output(process_stderr, "stderr", output.stderr())?;
    Ok(output.exit_code())
}

fn run_collected_with_process_output(
    args: Vec<OsString>,
    io: &mut ProcessIo<'_>,
) -> Result<i32, CliError> {
    let output = run_collected(args, io.stdin)?;
    write_process_output(io.stdout, "stdout", output.stdout())?;
    write_process_output(io.stderr, "stderr", output.stderr())?;
    Ok(output.exit_code())
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{
        ProcessIo, run_9p_streaming_command, run_qemu_exec_process_io, run_qemu_print_process_io,
        run_qemu_process_io,
    };

    fn args(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    fn prepared_qemu_root(name: &str) -> PathBuf {
        let root = temp_dir_path(name);
        fs::create_dir_all(root.join("boot")).unwrap();
        fs::write(root.join("boot/bzImage"), b"kernel").unwrap();
        root
    }

    fn temp_dir_path(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("{name}-{}-{nanos}", std::process::id()))
    }

    fn parse_qemu_command(root: &Path, extra: &[&str]) -> crate::qemu::QemuCommand {
        let root_arg = root.display().to_string();
        let mut values = vec!["--root", &root_arg, "--cmdline", "init=/bin/sh"];
        values.extend_from_slice(extra);
        crate::qemu::parse_qemu_command(&args(&values)).unwrap()
    }

    fn empty_process_io<'a>(
        stdin: &'a mut &'static [u8],
        stdout: &'a mut Vec<u8>,
        stderr: &'a mut Vec<u8>,
    ) -> ProcessIo<'a> {
        ProcessIo::new(stdin, stdout, stderr)
    }

    #[test]
    fn p9_streaming_command_reports_stdio_parse_errors() {
        let mut stdin = &b""[..];
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let mut io = empty_process_io(&mut stdin, &mut stdout, &mut stderr);

        let error =
            run_9p_streaming_command(&OsString::from("p9-stdio"), &[], &mut io).unwrap_err();

        assert_eq!(error.exit_code(), 2);
        assert!(error.to_string().contains("p9-stdio requires --root DIR"));
        assert!(stdout.is_empty());
        assert!(stderr.is_empty());
    }

    #[test]
    fn p9_streaming_command_routes_stdio_runtime_errors() {
        let missing_root = temp_dir_path("wanix-cli-process-io-p9-stdio-missing");
        let root_arg = missing_root.display().to_string();
        let rest = args(&["--root", &root_arg]);
        let mut stdin = &b""[..];
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let mut io = empty_process_io(&mut stdin, &mut stdout, &mut stderr);

        let error =
            run_9p_streaming_command(&OsString::from("p9-stdio"), &rest, &mut io).unwrap_err();

        assert_eq!(error.exit_code(), 1);
        assert!(error.to_string().contains("failed to open p9-stdio root"));
        assert!(stdout.is_empty());
        assert!(stderr.is_empty());
    }

    #[test]
    fn qemu_process_io_routes_print_mode_to_stdout() {
        let root = prepared_qemu_root("wanix-cli-process-io-qemu-print");
        let root_arg = root.display().to_string();
        let stdin_bytes = Vec::new();
        let mut stdin = stdin_bytes.as_slice();
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let mut io = ProcessIo::new(&mut stdin, &mut stdout, &mut stderr);

        let exit_code = run_qemu_process_io(
            &args(&["--root", &root_arg, "--cmdline", "init=/bin/sh"]),
            &mut io,
        )
        .unwrap();

        assert_eq!(exit_code, 0);
        assert!(
            String::from_utf8(stdout)
                .unwrap()
                .contains("qemu-system-i386")
        );
        assert!(stderr.is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn qemu_print_process_io_writes_captured_stdout() {
        let root = prepared_qemu_root("wanix-cli-process-io-qemu-print-helper");
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        let exit_code =
            run_qemu_print_process_io(parse_qemu_command(&root, &[]), &mut stdout, &mut stderr)
                .unwrap();

        assert_eq!(exit_code, 0);
        assert!(
            String::from_utf8(stdout)
                .unwrap()
                .contains("qemu-system-i386")
        );
        assert!(stderr.is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn qemu_exec_process_io_uses_live_stderr_and_reports_spawn_failure() {
        let root = prepared_qemu_root("wanix-cli-process-io-qemu-exec");
        let missing_qemu = root.join("missing-qemu");
        let qemu_arg = missing_qemu.display().to_string();
        let mut stderr = Vec::new();

        let error = run_qemu_exec_process_io(
            parse_qemu_command(&root, &["--exec", "--qemu-bin", &qemu_arg]),
            &mut stderr,
        )
        .unwrap_err();

        assert_eq!(error.exit_code(), 1);
        assert!(
            String::from_utf8(stderr)
                .unwrap()
                .contains("wanix-rust qemu exec:")
        );
        assert!(
            error
                .to_string()
                .contains("failed to start qemu executable")
        );
        fs::remove_dir_all(root).unwrap();
    }
}
