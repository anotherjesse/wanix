use std::ffi::OsString;
use std::io::{Read, Write};

mod fd;

use crate::{
    CliError, p9_listen, p9_stdio, p9_ws, qemu, qjs_term, run_collected, serve,
    write_process_output,
};

#[cfg(all(unix, test))]
pub(super) fn run_with_resize_queue(
    args: Vec<OsString>,
    process_stdin: &mut dyn Read,
    stdin_fd: libc::c_int,
    resize_queue: std::sync::Arc<std::sync::Mutex<std::collections::VecDeque<(u16, u16)>>>,
    process_stdout: &mut dyn Write,
    process_stderr: &mut dyn Write,
) -> Result<i32, CliError> {
    let mut io = ProcessIo::new(process_stdin, process_stdout, process_stderr);
    fd::run_with_resize_queue(args, &mut io, stdin_fd, resize_queue)
}

#[cfg(unix)]
pub(super) fn run_with_stdin_fd(
    args: Vec<OsString>,
    process_stdin: &mut dyn Read,
    stdin_fd: libc::c_int,
    process_stdout: &mut dyn Write,
    process_stderr: &mut dyn Write,
) -> Result<i32, CliError> {
    let mut io = ProcessIo::new(process_stdin, process_stdout, process_stderr);
    fd::run_with_stdin_fd(args, &mut io, stdin_fd)
}

#[cfg(unix)]
pub(super) fn run_with_terminal_fds(
    args: Vec<OsString>,
    process_stdin: &mut dyn Read,
    stdin_fd: libc::c_int,
    terminal_size_fd: libc::c_int,
    process_stdout: &mut dyn Write,
    process_stderr: &mut dyn Write,
) -> Result<i32, CliError> {
    let mut io = ProcessIo::new(process_stdin, process_stdout, process_stderr);
    fd::run_with_terminal_fds(args, &mut io, stdin_fd, terminal_size_fd)
}

struct ProcessIo<'a> {
    stdin: &'a mut dyn Read,
    stdout: &'a mut dyn Write,
    stderr: &'a mut dyn Write,
}

impl<'a> ProcessIo<'a> {
    fn new(stdin: &'a mut dyn Read, stdout: &'a mut dyn Write, stderr: &'a mut dyn Write) -> Self {
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
    match command.to_str() {
        Some("qjs-term" | "qjs-shell") => run_qjs_streaming_command(command, rest, io),
        Some("p9-stdio" | "p9-listen" | "p9-ws") => run_9p_streaming_command(command, rest, io),
        Some("qemu") => Ok(Some(run_qemu_process_io(rest, io)?)),
        Some("serve") => Ok(Some(serve::run_serve_streaming(
            serve::parse_serve_command(rest)?,
            io.stderr,
        )?)),
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
        Some("p9-listen") => Ok(Some(p9_listen::run_p9_listen_streaming(
            p9_listen::parse_p9_listen_command(rest)?,
            io.stderr,
        )?)),
        Some("p9-ws") => Ok(Some(p9_ws::run_p9_ws_streaming(
            p9_ws::parse_p9_ws_command(rest)?,
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
        ProcessIo, run_qemu_exec_process_io, run_qemu_print_process_io, run_qemu_process_io,
    };

    fn args(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    fn prepared_qemu_root(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("{name}-{}-{nanos}", std::process::id()));
        fs::create_dir_all(root.join("boot")).unwrap();
        fs::write(root.join("boot/bzImage"), b"kernel").unwrap();
        root
    }

    fn parse_qemu_command(root: &Path, extra: &[&str]) -> crate::qemu::QemuCommand {
        let root_arg = root.display().to_string();
        let mut values = vec!["--root", &root_arg, "--cmdline", "init=/bin/sh"];
        values.extend_from_slice(extra);
        crate::qemu::parse_qemu_command(&args(&values)).unwrap()
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
