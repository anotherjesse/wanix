use std::ffi::OsString;
use std::io::Read;

use crate::wasm_args::parse_wasm_command;
use crate::{
    CliError, CliOutput, agent, agent_exec_server, app, capsule, catalog, cpu, mesh, mount, new,
    p9_stdio, parse_qjs_command, parse_qjs_snapshot_file_command, qemu, qjs, qjs_restore, qjs_term,
    recipe, rootfs, serve, sh, tool, volume, wasm,
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
        Some("p9-stdio") => {
            p9_stdio::run_p9_stdio(p9_stdio::parse_p9_stdio_command(rest)?, process_stdin)
        }
        Some(verb @ ("mount-ls" | "mount-cat" | "mount-write")) => {
            mount::run_mount_command(mount::parse_mount_command(verb, rest)?)
        }
        Some("wasm") => wasm::run_wasm(parse_wasm_command(rest)?, process_stdin),
        // `sh -c LINE` runs collected; interactive `sh` needs the fd-aware
        // terminal path and is refused here with a usage pointer.
        Some("sh") => sh::run_sh(sh::parse_sh_command(rest)?, process_stdin),
        Some("agent") => agent::run_agent_command(agent::parse_agent_command(rest)?),
        Some("capsule") => capsule::run_capsule_command(capsule::parse_capsule_command(rest)?),
        Some("agent-exec-server") => require_live_process_io(
            agent_exec_server::parse_agent_exec_server_command(rest),
            "agent-exec-server",
        ),
        Some("new") => new::run_new_command(new::parse_new_command(rest)?),
        Some("catalog") => catalog::run_catalog_command(catalog::parse_catalog_command(rest)?),
        // `recipe run` of a run-less recipe is an interactive shell session and
        // is refused inside run_recipe_command; everything else runs collected.
        Some("recipe") => {
            recipe::run_recipe_command(recipe::parse_recipe_command(rest)?, process_stdin)
        }
        Some("volume") => run_volume_collected_command(rest),
        Some("tool") => run_tool_collected_command(rest),
        Some("app") => run_app_collected_command(rest),
        Some("rootfs") => rootfs::run_rootfs_command(rootfs::parse_rootfs_command(rest)?),
        Some("qemu") => qemu::run_qemu_command(qemu::parse_qemu_command(rest)?),
        Some("serve") => require_live_process_io(serve::parse_serve_command(rest), "serve"),
        Some("mesh-serve") => {
            require_live_process_io(mesh::parse_mesh_serve_command(rest), "mesh-serve")
        }
        Some("cpu") => require_live_process_io(cpu::parse_cpu_command(rest), "cpu"),
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

/// `volume create`/`ls` run in the collected path; `volume serve` is streaming
/// (it parks binding mesh endpoints), so it parses here but defers to the live
/// process-IO path, mirroring `mesh-serve`.
fn run_volume_collected_command(rest: &[OsString]) -> Result<CliOutput, CliError> {
    if rest.first().and_then(|arg| arg.to_str()) == Some("serve") {
        return require_live_process_io(
            volume::parse_volume_serve_command(&rest[1..]),
            "volume serve",
        );
    }
    volume::run_volume_command(volume::parse_volume_command(rest)?)
}

/// `tool serve` is streaming (it parks binding one mesh endpoint per tool), so
/// it parses here but defers to the live process-IO path, mirroring
/// `volume serve`.
fn run_tool_collected_command(rest: &[OsString]) -> Result<CliOutput, CliError> {
    if rest.first().and_then(|arg| arg.to_str()) == Some("serve") {
        return require_live_process_io(tool::parse_tool_serve_command(&rest[1..]), "tool serve");
    }
    Err(CliError::usage(
        "tool: expected a subcommand (serve --tool NAME ...)",
    ))
}

/// `app serve` is streaming (it parks serving one mesh endpoint plus the
/// resident guest task), so it parses here but defers to the live process-IO
/// path, mirroring `tool serve`.
fn run_app_collected_command(rest: &[OsString]) -> Result<CliOutput, CliError> {
    if rest.first().and_then(|arg| arg.to_str()) == Some("serve") {
        return require_live_process_io(app::parse_app_serve_command(&rest[1..]), "app serve");
    }
    Err(CliError::usage(
        "app: expected a subcommand (serve --app DIR --state DIR ...)",
    ))
}

fn require_live_process_io<T>(
    parsed: Result<T, CliError>,
    command_name: &str,
) -> Result<CliOutput, CliError> {
    let _ = parsed?;
    Err(CliError::usage(format!(
        "{command_name} requires live process IO; use the wanix binary"
    )))
}

fn unknown_collected_command(command: &OsString) -> Result<CliOutput, CliError> {
    unknown_collected_command_name(&command.to_string_lossy())
}

fn unknown_collected_command_name(command: &str) -> Result<CliOutput, CliError> {
    Err(CliError::usage(format!("unknown wanix command: {command}")))
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::io;

    use super::run_collected_command;

    #[test]
    fn collected_streaming_commands_require_live_process_io_after_parsing() {
        let error = run_collected("serve", &[]).unwrap_err();

        assert_eq!(error.exit_code(), 2);
        assert!(
            error.to_string().contains("serve requires live process IO"),
            "serve produced {error}"
        );
    }

    #[test]
    fn collected_streaming_commands_preserve_parser_errors() {
        // serve --grant without --peer is a parser error surfaced before the
        // live-process-IO refusal.
        let error =
            run_collected("serve", &["--p9", "127.0.0.1:0", "--grant", "a:b:rw"]).unwrap_err();

        assert_eq!(error.exit_code(), 2);
        assert!(
            error.to_string().contains("--grant requires --peer"),
            "{error}"
        );
    }

    fn run_collected(command: &str, args: &[&str]) -> Result<crate::CliOutput, crate::CliError> {
        let command = OsString::from(command);
        let args: Vec<_> = args.iter().map(OsString::from).collect();
        run_collected_command(&command, &args, &mut io::empty())
    }
}
