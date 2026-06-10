//! Argument parsing for the `wanix sh` subcommand.
//!
//! `sh` is deliberately small: `-c LINE` runs one line, no `-c` is the
//! interactive REPL, and `--env`/`--cwd`/`--mount-mesh` shape the session's
//! namespace. Everything else (redirects, pipelines, command resolution) is
//! shell syntax and belongs in the line, not in CLI flags.

use std::ffi::OsString;

use wanix_fs::NormalizedPath;

use crate::CliError;
use crate::qjs_args::{MeshMountSpec, os_arg_to_string, parse_mesh_mount, validate_env_line};

/// A parsed `sh` command: one `-c` line (or interactive), plus the namespace
/// shape (env, host cwd preopen, mesh mounts).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ShCommand {
    /// `-c LINE` runs one line non-interactively; `None` starts the REPL.
    pub(crate) line: Option<String>,
    pub(crate) env: Vec<String>,
    /// Host directory preopened as the namespace root (default `.`).
    pub(crate) cwd: NormalizedPath,
    /// Native mesh imports (`--mount-mesh IROH_URL=GUEST`), dialed at run time;
    /// the runner holds the dialer keepalives for the session lifetime.
    pub(crate) mesh_mounts: Vec<MeshMountSpec>,
}

/// Parses `sh [-c LINE] [--env K=V ...] [--cwd DIR] [--mount-mesh IROH_URL=GUEST ...]`.
///
/// # Errors
///
/// Returns a usage error for unknown flags, operands, a repeated `-c`, or a
/// malformed flag value.
pub(crate) fn parse_sh_command(args: &[OsString]) -> Result<ShCommand, CliError> {
    let mut command = ShCommand {
        line: None,
        env: Vec::new(),
        cwd: NormalizedPath::new(".")?,
        mesh_mounts: Vec::new(),
    };
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        if arg == "-c" {
            if command.line.is_some() {
                return Err(CliError::usage("sh accepts only one -c LINE"));
            }
            let value = sh_option_value(args, &mut index, "-c")?;
            command.line = Some(os_arg_to_string(value, "sh -c")?);
        } else if arg == "--env" {
            let value = sh_option_value(args, &mut index, "--env")?;
            let value = os_arg_to_string(value, "sh --env")?;
            validate_env_line(&value, "sh --env")?;
            command.env.push(value);
        } else if arg == "--cwd" {
            let value = sh_option_value(args, &mut index, "--cwd")?;
            command.cwd = NormalizedPath::new(os_arg_to_string(value, "sh --cwd")?)?;
        } else if arg == "--mount-mesh" {
            let value = sh_option_value(args, &mut index, "--mount-mesh")?;
            let value = os_arg_to_string(value, "sh --mount-mesh")?;
            command
                .mesh_mounts
                .push(parse_mesh_mount(&value, "sh --mount-mesh")?);
        } else {
            return Err(CliError::usage(format!(
                "sh does not accept operand {:?}; pass the command line through -c LINE",
                arg.to_string_lossy()
            )));
        }
    }
    Ok(command)
}

fn sh_option_value<'a>(
    args: &'a [OsString],
    index: &mut usize,
    option: &str,
) -> Result<&'a OsString, CliError> {
    *index += 1;
    let value = args
        .get(*index)
        .ok_or_else(|| CliError::usage(format!("sh {option} expects a value")))?;
    *index += 1;
    Ok(value)
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use super::parse_sh_command;

    fn args(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn parses_dash_c_with_namespace_flags() {
        let command = parse_sh_command(&args(&[
            "--env",
            "MODE=test",
            "--cwd",
            "work",
            "-c",
            "echo hi > /vol/x",
            "--mount-mesh",
            "iroh://abc?addr=127.0.0.1:5599=/vol",
        ]))
        .unwrap();

        assert_eq!(command.line.as_deref(), Some("echo hi > /vol/x"));
        assert_eq!(command.env, vec!["MODE=test"]);
        assert_eq!(command.cwd.as_str(), "work");
        assert_eq!(command.mesh_mounts.len(), 1);
        assert_eq!(
            command.mesh_mounts[0].addr,
            "iroh://abc?addr=127.0.0.1:5599"
        );
        assert_eq!(command.mesh_mounts[0].guest_path.as_str(), "vol");
    }

    #[test]
    fn no_dash_c_is_the_interactive_session() {
        let command = parse_sh_command(&args(&[])).unwrap();
        assert_eq!(command.line, None);
        assert!(command.mesh_mounts.is_empty());
    }

    #[test]
    fn rejects_operands_repeated_dash_c_and_missing_values() {
        let operand = parse_sh_command(&args(&["script.sh"])).unwrap_err();
        assert_eq!(operand.exit_code(), 2);
        assert!(operand.to_string().contains("does not accept operand"));

        let repeated = parse_sh_command(&args(&["-c", "a", "-c", "b"])).unwrap_err();
        assert_eq!(repeated.exit_code(), 2);
        assert!(repeated.to_string().contains("only one -c"));

        let missing = parse_sh_command(&args(&["--mount-mesh"])).unwrap_err();
        assert_eq!(missing.exit_code(), 2);
        assert!(missing.to_string().contains("sh --mount-mesh expects"));
    }
}
