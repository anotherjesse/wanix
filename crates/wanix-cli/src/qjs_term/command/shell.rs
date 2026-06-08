use std::ffi::OsString;
use std::path::Path;

use super::super::{QJS_SHELL_READY_IO_TURNS, QJS_SHELL_SCRIPT_SENTINEL};
use super::CliError;
use crate::{QjsCommand, os_arg_to_string, parse_mesh_mount, parse_qjs_command_for};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct QjsShellCommand {
    pub(in crate::qjs_term) qjs: QjsCommand,
    pub(in crate::qjs_term) raw: bool,
}

pub(crate) fn parse_qjs_shell_command(args: &[OsString]) -> Result<QjsShellCommand, CliError> {
    // `--mount-mesh IROH_URL=GUEST` is intercepted here as the Slice 1 entry point.
    // This is NOT a parsing limitation: the shared common-option loop could take it
    // with a dedicated arm using `rsplit_once('=')` (the iroh URL's own `=` in
    // `?addr=` is handled by splitting on the LAST `=`, exactly as `parse_mesh_mount`
    // does). It lives in qjs-shell only because `bind_mesh_mounts` returns IrohMount
    // keepalives that must be held for the task lifetime, and the qjs-term/qjs-shell
    // runtime path is the one that currently has a place to hold them
    // (`PreparedQjsTermExecution`). The MeshMountSpec/bind_mesh_mounts path is
    // namespace-generic; qjs/qjs-term/wasm should grow the flag once each runtime has
    // a keepalive home.
    let mut qjs_args = Vec::new();
    let mut raw = false;
    let mut mesh_mounts = Vec::new();
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        if arg == "--raw" {
            raw = true;
            index += 1;
        } else if arg == "--mount-mesh" {
            let value = args
                .get(index + 1)
                .ok_or_else(|| CliError::usage("qjs-shell --mount-mesh expects IROH_URL=GUEST"))?;
            let value = os_arg_to_string(value, "qjs-shell --mount-mesh")?;
            mesh_mounts.push(parse_mesh_mount(&value, "qjs-shell --mount-mesh")?);
            index += 2;
        } else {
            qjs_args.push(arg.clone());
            index += 1;
        }
    }
    qjs_args.push(OsString::from(QJS_SHELL_SCRIPT_SENTINEL));
    let mut qjs = parse_qjs_command_for(&qjs_args, "qjs-shell")?;
    if qjs.script_path != Path::new(QJS_SHELL_SCRIPT_SENTINEL) || !qjs.args.is_empty() {
        return Err(CliError::usage(
            "qjs-shell does not accept a script path or script arguments",
        ));
    }
    if qjs.stdin.is_some() {
        return Err(CliError::usage(
            "qjs-shell reads native stdin as terminal input; use qjs-term for explicit stdin fixtures",
        ));
    }
    qjs.mesh_mounts = mesh_mounts;
    Ok(QjsShellCommand { qjs, raw })
}

pub(in crate::qjs_term) fn qjs_shell_command(mut command: QjsCommand, raw: bool) -> QjsCommand {
    command.ready_io_turns = command.ready_io_turns.max(QJS_SHELL_READY_IO_TURNS);
    if raw {
        command.env.push("WANIX_QJS_SHELL_RAW=1".to_owned());
    }
    command
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use super::parse_qjs_shell_command;

    fn args(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn qjs_shell_captures_mount_mesh_specs() {
        // The iroh URL's own `?addr=...=` must survive: the spec splits on the LAST
        // `=`, so the address keeps its query and the guest path is `/vol`.
        let command = parse_qjs_shell_command(&args(&[
            "--mount-mesh",
            "iroh://abc?addr=127.0.0.1:5599=/vol",
        ]))
        .unwrap();
        assert_eq!(command.qjs.mesh_mounts.len(), 1);
        assert_eq!(
            command.qjs.mesh_mounts[0].addr,
            "iroh://abc?addr=127.0.0.1:5599"
        );
        assert_eq!(command.qjs.mesh_mounts[0].guest_path.as_str(), "vol");
        assert!(!command.raw);
    }

    #[test]
    fn qjs_shell_mount_mesh_requires_a_value() {
        let error = parse_qjs_shell_command(&args(&["--mount-mesh"])).unwrap_err();
        assert_eq!(error.exit_code(), 2);
        assert!(error.to_string().contains("--mount-mesh expects"));
    }
}
