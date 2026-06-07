use std::ffi::OsString;
use std::io::Read;

use crate::wasm_args::parse_wasm_command;
use crate::{
    CliError, CliOutput, agent, agent_exec_server, capsule, mesh, mount, new, p9_listen, p9_stdio,
    p9_ws, parse_qjs_command, parse_qjs_snapshot_file_command, qemu, qjs, qjs_restore, qjs_term,
    rootfs, serve, wasm,
};

pub(super) fn run_collected_command(
    command: &OsString,
    rest: &[OsString],
    process_stdin: &mut dyn Read,
) -> Result<CliOutput, CliError> {
    match command.to_str() {
        Some(
            command_name @ ("qjs" | "qjs-term" | "qjs-shell" | "qjs-snapshot" | "qjs-resume"
            | "qjs-restore"),
        ) => run_qjs_collected_command(command_name, rest, process_stdin),
        Some(command_name @ ("p9-stdio" | "p9-listen" | "p9-ws")) => {
            run_9p_collected_command(command_name, rest, process_stdin)
        }
        Some(verb @ ("mount-ls" | "mount-cat" | "mount-write")) => {
            mount::run_mount_command(mount::parse_mount_command(verb, rest)?)
        }
        Some("wasm") => wasm::run_wasm(parse_wasm_command(rest)?, process_stdin),
        Some("agent") => agent::run_agent_command(agent::parse_agent_command(rest)?),
        Some("capsule") => capsule::run_capsule_command(capsule::parse_capsule_command(rest)?),
        Some("agent-exec-server") => require_live_process_io(
            agent_exec_server::parse_agent_exec_server_command(rest),
            "agent-exec-server",
        ),
        Some("new") => new::run_new_command(new::parse_new_command(rest)?),
        Some("rootfs") => rootfs::run_rootfs_command(rootfs::parse_rootfs_command(rest)?),
        Some("qemu") => qemu::run_qemu_command(qemu::parse_qemu_command(rest)?),
        Some("serve") => require_live_process_io(serve::parse_serve_command(rest), "serve"),
        Some("mesh-serve") => {
            require_live_process_io(mesh::parse_mesh_serve_command(rest), "mesh-serve")
        }
        _ => unknown_collected_command(command),
    }
}

fn run_qjs_collected_command(
    command: &str,
    rest: &[OsString],
    process_stdin: &mut dyn Read,
) -> Result<CliOutput, CliError> {
    match command {
        "qjs" => qjs::run_qjs(parse_qjs_command(rest)?, process_stdin),
        "qjs-term" => {
            qjs_term::run_qjs_term(qjs_term::parse_qjs_term_command(rest)?, process_stdin)
        }
        "qjs-shell" => {
            qjs_term::run_qjs_shell(qjs_term::parse_qjs_shell_command(rest)?, process_stdin)
        }
        "qjs-snapshot" => qjs::run_qjs_snapshot(
            parse_qjs_snapshot_file_command(rest, "qjs-snapshot")?,
            process_stdin,
        ),
        "qjs-resume" => qjs::run_qjs_resume(
            parse_qjs_snapshot_file_command(rest, "qjs-resume")?,
            process_stdin,
        ),
        "qjs-restore" => qjs_restore::parse_and_run_qjs_restore(rest),
        _ => unknown_collected_command_name(command),
    }
}

fn run_9p_collected_command(
    command: &str,
    rest: &[OsString],
    process_stdin: &mut dyn Read,
) -> Result<CliOutput, CliError> {
    match command {
        "p9-stdio" => {
            p9_stdio::run_p9_stdio(p9_stdio::parse_p9_stdio_command(rest)?, process_stdin)
        }
        "p9-listen" => {
            require_live_process_io(p9_listen::parse_p9_listen_command(rest), "p9-listen")
        }
        "p9-ws" => require_live_process_io(p9_ws::parse_p9_ws_command(rest), "p9-ws"),
        _ => unknown_collected_command_name(command),
    }
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

fn unknown_collected_command(command: &OsString) -> Result<CliOutput, CliError> {
    unknown_collected_command_name(&command.to_string_lossy())
}

fn unknown_collected_command_name(command: &str) -> Result<CliOutput, CliError> {
    Err(CliError::usage(format!(
        "unknown wanix-rust command: {command}"
    )))
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::io;

    use super::run_collected_command;

    #[test]
    fn collected_streaming_commands_require_live_process_io_after_parsing() {
        for (command, args) in [
            ("p9-listen", vec!["--root", ".", "--addr", "127.0.0.1:0"]),
            ("p9-ws", vec!["--root", ".", "--addr", "127.0.0.1:0"]),
            ("serve", Vec::new()),
        ] {
            let error = run_collected(command, &args).unwrap_err();

            assert_eq!(error.exit_code(), 2);
            assert!(
                error
                    .to_string()
                    .contains(&format!("{command} requires live process IO")),
                "{command} produced {error}"
            );
        }
    }

    #[test]
    fn collected_streaming_commands_preserve_parser_errors() {
        let error = run_collected("p9-listen", &[]).unwrap_err();

        assert_eq!(error.exit_code(), 2);
        assert!(
            error.to_string().contains("p9-listen requires --root DIR"),
            "{error}"
        );
    }

    fn run_collected(command: &str, args: &[&str]) -> Result<crate::CliOutput, crate::CliError> {
        let command = OsString::from(command);
        let args: Vec<_> = args.iter().map(OsString::from).collect();
        run_collected_command(&command, &args, &mut io::empty())
    }
}
