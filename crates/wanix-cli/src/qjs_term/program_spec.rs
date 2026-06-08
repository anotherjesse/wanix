use std::path::Path;
use std::sync::Arc;

use wanix_fs::{MemFs, NormalizedPath};
use wanix_task::Task;
use wanix_vfs::BindOptions;

use super::terminal::AttachedTerminal;
use super::{QJS_SHELL_SCRIPT_SENTINEL, QJS_SHELL_SOURCE};
use crate::mesh::IrohMount;
use crate::{
    CliError, QJS_GUEST_SCRIPT, QjsCommand, bind_host_mounts, bind_mesh_mounts,
    copy_script_directory, guest_path_in_cwd, read_utf8_script,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum QjsTermProgram {
    HostScript,
    BundledShell,
}

pub(super) struct PreparedQjsTermProgram {
    pub(super) script: String,
    pub(super) guest_script: String,
    pub(super) program_path: &'static str,
    pub(super) runtime_cwd: NormalizedPath,
}

pub(super) fn read_qjs_term_program(
    script_path: &Path,
    program: QjsTermProgram,
) -> Result<String, CliError> {
    match program {
        QjsTermProgram::HostScript => read_utf8_script(script_path),
        QjsTermProgram::BundledShell => Ok(QJS_SHELL_SOURCE.to_owned()),
    }
}

/// Prepares the qjs-term namespace and returns the prepared program alongside
/// the native-mesh-mount keepalives (`--mount-mesh`). The keepalives MUST be
/// held for the task lifetime — they own the runtimes the imported mesh
/// filesystems drive QUIC ops on — so the caller stores them on the
/// prepared-execution object.
pub(super) fn prepare_qjs_term_namespace(
    task: &Task,
    qjs_command: &QjsCommand,
    program: QjsTermProgram,
    script: String,
) -> Result<(PreparedQjsTermProgram, Vec<IrohMount>), CliError> {
    let runtime_cwd = qjs_term_runtime_cwd(&qjs_command.cwd, program)?;
    let program_path = qjs_term_program_path(program);
    let guest_script = qjs_term_guest_script(&qjs_command.cwd, program)?;
    let root = qjs_term_root(qjs_command, program, &guest_script, &script)?;
    let mesh_mounts = bind_qjs_term_namespace(task, root, qjs_command)?;
    Ok((
        PreparedQjsTermProgram {
            script,
            guest_script,
            program_path,
            runtime_cwd,
        },
        mesh_mounts,
    ))
}

pub(super) fn terminal_task_env(
    qjs_command: &QjsCommand,
    program: QjsTermProgram,
    terminal: &AttachedTerminal,
) -> Vec<String> {
    let mut task_env = qjs_command.env.clone();
    if program == QjsTermProgram::BundledShell {
        task_env.push(format!("WANIX_TERM_ID={}", terminal.id));
    }
    task_env
}

fn qjs_term_root(
    qjs_command: &QjsCommand,
    program: QjsTermProgram,
    guest_script: &str,
    script: &str,
) -> Result<Arc<MemFs>, CliError> {
    let root = Arc::new(MemFs::new());
    copy_host_script_directory_if_needed(&root, qjs_command, program)?;
    root.write_file(guest_script, script.as_bytes())?;
    Ok(root)
}

fn copy_host_script_directory_if_needed(
    root: &Arc<MemFs>,
    qjs_command: &QjsCommand,
    program: QjsTermProgram,
) -> Result<(), CliError> {
    if program == QjsTermProgram::HostScript {
        copy_script_directory(&qjs_command.script_path, root, &qjs_command.cwd)?;
    }
    Ok(())
}

fn bind_qjs_term_namespace(
    task: &Task,
    root: Arc<MemFs>,
    qjs_command: &QjsCommand,
) -> Result<Vec<IrohMount>, CliError> {
    task.bind(root, ".", ".", BindOptions::default())?;
    bind_host_mounts(task, &qjs_command.mounts)?;
    bind_mesh_mounts(task, &qjs_command.mesh_mounts)
}

fn qjs_term_runtime_cwd(
    command_cwd: &NormalizedPath,
    program: QjsTermProgram,
) -> Result<NormalizedPath, CliError> {
    match program {
        QjsTermProgram::BundledShell => {
            // The bundled shell implements cwd in guest code; keep the WASI root
            // at namespace root so shell navigation is not trapped below --cwd.
            Ok(NormalizedPath::new(".")?)
        }
        QjsTermProgram::HostScript => Ok(command_cwd.clone()),
    }
}

fn qjs_term_program_path(program: QjsTermProgram) -> &'static str {
    match program {
        QjsTermProgram::BundledShell => QJS_SHELL_SCRIPT_SENTINEL,
        QjsTermProgram::HostScript => QJS_GUEST_SCRIPT,
    }
}

fn qjs_term_guest_script(
    command_cwd: &NormalizedPath,
    program: QjsTermProgram,
) -> Result<String, CliError> {
    match program {
        QjsTermProgram::BundledShell => Ok(QJS_SHELL_SCRIPT_SENTINEL.to_owned()),
        QjsTermProgram::HostScript => guest_path_in_cwd(command_cwd, QJS_GUEST_SCRIPT),
    }
}
