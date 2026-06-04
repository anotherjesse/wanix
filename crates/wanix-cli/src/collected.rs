use std::ffi::OsString;
use std::io::Read;

use crate::{
    CliError, CliOutput, p9_listen, p9_stdio, p9_ws, parse_qjs_command,
    parse_qjs_snapshot_file_command, qemu, qjs, qjs_restore, qjs_term, rootfs, serve,
};

type CollectedHandler = fn(&[OsString], &mut dyn Read) -> Result<CliOutput, CliError>;

const COLLECTED_COMMANDS: &[(&str, CollectedHandler)] = &[
    ("qjs", run_qjs_collected),
    ("qjs-term", run_qjs_term_collected),
    ("qjs-shell", run_qjs_shell_collected),
    ("qjs-snapshot", run_qjs_snapshot_collected),
    ("qjs-resume", run_qjs_resume_collected),
    ("qjs-restore", run_qjs_restore_collected),
    ("p9-stdio", run_p9_stdio_collected),
    ("rootfs", run_rootfs_collected),
    ("p9-listen", run_p9_listen_collected),
    ("p9-ws", run_p9_ws_collected),
    ("qemu", run_qemu_collected),
    ("serve", run_serve_collected),
];

pub(super) fn run_collected_command(
    command: &OsString,
    rest: &[OsString],
    process_stdin: &mut dyn Read,
) -> Result<CliOutput, CliError> {
    let Some(handler) = collected_handler(command) else {
        return Err(CliError::usage(format!(
            "unknown wanix-rust command: {}",
            command.to_string_lossy()
        )));
    };
    handler(rest, process_stdin)
}

fn collected_handler(command: &OsString) -> Option<CollectedHandler> {
    let command = command.to_str()?;
    COLLECTED_COMMANDS
        .iter()
        .find_map(|(name, handler)| (*name == command).then_some(*handler))
}

fn run_qjs_collected(
    rest: &[OsString],
    process_stdin: &mut dyn Read,
) -> Result<CliOutput, CliError> {
    qjs::run_qjs(parse_qjs_command(rest)?, process_stdin)
}

fn run_qjs_term_collected(
    rest: &[OsString],
    process_stdin: &mut dyn Read,
) -> Result<CliOutput, CliError> {
    qjs_term::run_qjs_term(qjs_term::parse_qjs_term_command(rest)?, process_stdin)
}

fn run_qjs_shell_collected(
    rest: &[OsString],
    process_stdin: &mut dyn Read,
) -> Result<CliOutput, CliError> {
    qjs_term::run_qjs_shell(qjs_term::parse_qjs_shell_command(rest)?, process_stdin)
}

fn run_qjs_snapshot_collected(
    rest: &[OsString],
    process_stdin: &mut dyn Read,
) -> Result<CliOutput, CliError> {
    qjs::run_qjs_snapshot(
        parse_qjs_snapshot_file_command(rest, "qjs-snapshot")?,
        process_stdin,
    )
}

fn run_qjs_resume_collected(
    rest: &[OsString],
    process_stdin: &mut dyn Read,
) -> Result<CliOutput, CliError> {
    qjs::run_qjs_resume(
        parse_qjs_snapshot_file_command(rest, "qjs-resume")?,
        process_stdin,
    )
}

fn run_qjs_restore_collected(
    rest: &[OsString],
    _process_stdin: &mut dyn Read,
) -> Result<CliOutput, CliError> {
    qjs_restore::parse_and_run_qjs_restore(rest)
}

fn run_p9_stdio_collected(
    rest: &[OsString],
    process_stdin: &mut dyn Read,
) -> Result<CliOutput, CliError> {
    p9_stdio::run_p9_stdio(p9_stdio::parse_p9_stdio_command(rest)?, process_stdin)
}

fn run_rootfs_collected(
    rest: &[OsString],
    _process_stdin: &mut dyn Read,
) -> Result<CliOutput, CliError> {
    rootfs::run_rootfs_command(rootfs::parse_rootfs_command(rest)?)
}

fn run_p9_listen_collected(
    rest: &[OsString],
    _process_stdin: &mut dyn Read,
) -> Result<CliOutput, CliError> {
    require_live_process_io(p9_listen::parse_p9_listen_command(rest), "p9-listen")
}

fn run_p9_ws_collected(
    rest: &[OsString],
    _process_stdin: &mut dyn Read,
) -> Result<CliOutput, CliError> {
    require_live_process_io(p9_ws::parse_p9_ws_command(rest), "p9-ws")
}

fn run_qemu_collected(
    rest: &[OsString],
    _process_stdin: &mut dyn Read,
) -> Result<CliOutput, CliError> {
    qemu::run_qemu_command(qemu::parse_qemu_command(rest)?)
}

fn run_serve_collected(
    rest: &[OsString],
    _process_stdin: &mut dyn Read,
) -> Result<CliOutput, CliError> {
    require_live_process_io(serve::parse_serve_command(rest), "serve")
}

fn require_live_process_io<T>(
    parsed: Result<T, CliError>,
    command_name: &str,
) -> Result<CliOutput, CliError> {
    let _ = parsed?;
    Err(CliError::usage(format!(
        "{command_name} requires live process IO; use the wanix-rust binary"
    )))
}
