use std::ffi::OsString;
use std::io::{Read, Write};

use crate::{
    CliError, p9_listen, p9_stdio, p9_ws, qemu, qjs_term, run_collected, serve,
    write_process_output,
};

type StreamingHandler =
    fn(&[OsString], &mut dyn Read, &mut dyn Write, &mut dyn Write) -> Result<i32, CliError>;

const STREAMING_COMMANDS: &[(&str, StreamingHandler)] = &[
    ("qjs-term", run_qjs_term_process_io),
    ("qjs-shell", run_qjs_shell_process_io),
    ("p9-stdio", run_p9_stdio_process_io),
    ("p9-listen", run_p9_listen_process_io),
    ("p9-ws", run_p9_ws_process_io),
    ("qemu", run_qemu_process_io),
    ("serve", run_serve_process_io),
];

pub(super) fn run_with_process_io(
    args: Vec<OsString>,
    process_stdin: &mut dyn Read,
    process_stdout: &mut dyn Write,
    process_stderr: &mut dyn Write,
) -> Result<i32, CliError> {
    if let Some(exit_code) =
        run_streaming_command(&args, process_stdin, process_stdout, process_stderr)?
    {
        return Ok(exit_code);
    }
    run_collected_with_process_output(args, process_stdin, process_stdout, process_stderr)
}

fn run_streaming_command(
    args: &[OsString],
    process_stdin: &mut dyn Read,
    process_stdout: &mut dyn Write,
    process_stderr: &mut dyn Write,
) -> Result<Option<i32>, CliError> {
    let Some((command, rest)) = args.split_first() else {
        return Ok(None);
    };
    let Some(handler) = streaming_handler(command) else {
        return Ok(None);
    };
    Ok(Some(handler(
        rest,
        process_stdin,
        process_stdout,
        process_stderr,
    )?))
}

fn streaming_handler(command: &OsString) -> Option<StreamingHandler> {
    let command = command.to_str()?;
    STREAMING_COMMANDS
        .iter()
        .find_map(|(name, handler)| (*name == command).then_some(*handler))
}

fn run_qjs_term_process_io(
    rest: &[OsString],
    process_stdin: &mut dyn Read,
    process_stdout: &mut dyn Write,
    process_stderr: &mut dyn Write,
) -> Result<i32, CliError> {
    qjs_term::run_qjs_term_streaming(
        qjs_term::parse_qjs_term_command(rest)?,
        process_stdin,
        process_stdout,
        process_stderr,
    )
}

fn run_qjs_shell_process_io(
    rest: &[OsString],
    process_stdin: &mut dyn Read,
    process_stdout: &mut dyn Write,
    process_stderr: &mut dyn Write,
) -> Result<i32, CliError> {
    qjs_term::run_qjs_shell_streaming(
        qjs_term::parse_qjs_shell_command(rest)?,
        process_stdin,
        process_stdout,
        process_stderr,
    )
}

fn run_p9_stdio_process_io(
    rest: &[OsString],
    process_stdin: &mut dyn Read,
    process_stdout: &mut dyn Write,
    process_stderr: &mut dyn Write,
) -> Result<i32, CliError> {
    p9_stdio::run_p9_stdio_streaming(
        p9_stdio::parse_p9_stdio_command(rest)?,
        process_stdin,
        process_stdout,
        process_stderr,
    )
}

fn run_p9_listen_process_io(
    rest: &[OsString],
    _process_stdin: &mut dyn Read,
    _process_stdout: &mut dyn Write,
    process_stderr: &mut dyn Write,
) -> Result<i32, CliError> {
    p9_listen::run_p9_listen_streaming(p9_listen::parse_p9_listen_command(rest)?, process_stderr)
}

fn run_p9_ws_process_io(
    rest: &[OsString],
    _process_stdin: &mut dyn Read,
    _process_stdout: &mut dyn Write,
    process_stderr: &mut dyn Write,
) -> Result<i32, CliError> {
    p9_ws::run_p9_ws_streaming(p9_ws::parse_p9_ws_command(rest)?, process_stderr)
}

fn run_qemu_process_io(
    rest: &[OsString],
    _process_stdin: &mut dyn Read,
    process_stdout: &mut dyn Write,
    process_stderr: &mut dyn Write,
) -> Result<i32, CliError> {
    let command = qemu::parse_qemu_command(rest)?;
    if qemu::qemu_command_exec(&command) {
        return run_qemu_exec_process_io(command, process_stderr);
    }
    run_qemu_print_process_io(command, process_stdout, process_stderr)
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

fn run_serve_process_io(
    rest: &[OsString],
    _process_stdin: &mut dyn Read,
    _process_stdout: &mut dyn Write,
    process_stderr: &mut dyn Write,
) -> Result<i32, CliError> {
    serve::run_serve_streaming(serve::parse_serve_command(rest)?, process_stderr)
}

fn run_collected_with_process_output(
    args: Vec<OsString>,
    process_stdin: &mut dyn Read,
    process_stdout: &mut dyn Write,
    process_stderr: &mut dyn Write,
) -> Result<i32, CliError> {
    let output = run_collected(args, process_stdin)?;
    write_process_output(process_stdout, "stdout", output.stdout())?;
    write_process_output(process_stderr, "stderr", output.stderr())?;
    Ok(output.exit_code())
}
