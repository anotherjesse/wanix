//! `wanix agent-exec-server --root DIR` — an internal command codex spawns as a
//! filesystem/exec "environment". It speaks newline JSON-RPC on stdin/stdout
//! and routes the agent's fs/process calls to a Wanix filesystem rooted at DIR,
//! confining the agent to that world.

use std::ffi::OsString;
use std::io::{BufReader, Read, Write};
use std::path::PathBuf;
use std::sync::Arc;

use wanix_agent::ExecServer;
use wanix_fs::{FileSystem, LocalFs};

use crate::{CliError, write_process_output};

/// A parsed `agent-exec-server` command.
pub(super) struct AgentExecServerCommand {
    root_path: PathBuf,
    services: bool,
}

/// Parses `agent-exec-server --root DIR`.
///
/// # Errors
///
/// Returns a usage error when `--root` is missing or duplicated.
pub(super) fn parse_agent_exec_server_command(
    args: &[OsString],
) -> Result<AgentExecServerCommand, CliError> {
    let mut root_path = None;
    let mut services = false;
    let mut index = 0;
    while index < args.len() {
        if args[index] == "--root" {
            index += 1;
            let value = args
                .get(index)
                .ok_or_else(|| CliError::usage("agent-exec-server --root expects DIR"))?;
            if root_path.is_some() {
                return Err(CliError::usage("agent-exec-server accepts only one --root"));
            }
            root_path = Some(PathBuf::from(value));
            index += 1;
        } else if args[index] == "--services" {
            services = true;
            index += 1;
        } else {
            return Err(CliError::usage(format!(
                "unexpected agent-exec-server argument: {}",
                args[index].to_string_lossy()
            )));
        }
    }
    let root_path =
        root_path.ok_or_else(|| CliError::usage("agent-exec-server requires --root DIR"))?;
    Ok(AgentExecServerCommand {
        root_path,
        services,
    })
}

/// Serves the exec-server protocol over the process stdin/stdout.
///
/// # Errors
///
/// Returns an error when the root cannot be opened or stderr cannot be written.
pub(super) fn run_agent_exec_server_streaming(
    command: AgentExecServerCommand,
    process_stdin: &mut dyn Read,
    process_stdout: &mut dyn Write,
    process_stderr: &mut dyn Write,
) -> Result<i32, CliError> {
    let world: Arc<dyn FileSystem> = if command.services {
        // The agent's filesystem is the full Wanix services namespace: the host
        // root plus #term/#pipe/#kv/#agent/#task, so the LLM operates the Wanix
        // computer (store in #kv, spawn #task programs) as files.
        crate::serve::services_namespace_for_root(&command.root_path)?
    } else {
        let root = LocalFs::new(&command.root_path).map_err(|error| {
            CliError::new(
                format!(
                    "failed to open agent-exec-server root {}: {error}",
                    command.root_path.display()
                ),
                1,
            )
        })?;
        Arc::new(root)
    };
    let mut server = ExecServer::new(world);
    match server.serve(BufReader::new(process_stdin), process_stdout) {
        Ok(()) => Ok(0),
        Err(error) => {
            write_process_output(
                process_stderr,
                "stderr",
                format!("wanix-rust agent-exec-server: {error}\n").as_bytes(),
            )?;
            Ok(1)
        }
    }
}
