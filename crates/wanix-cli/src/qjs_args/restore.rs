use std::ffi::OsString;
use std::path::PathBuf;

use wanix_fs::NormalizedPath;

use super::{QjsRestoreCommand, os_arg_to_string, parse_host_mount, validate_env_line};
use crate::CliError;

pub(crate) fn parse_qjs_restore_command(args: &[OsString]) -> Result<QjsRestoreCommand, CliError> {
    let mut cwd = NormalizedPath::new(".")?;
    let mut mounts = Vec::new();
    let mut before_args = Vec::new();
    let mut after_args = Vec::new();
    let mut before_env = Vec::new();
    let mut after_env = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--cwd" {
            i += 1;
            let value = args
                .get(i)
                .ok_or_else(|| CliError::usage("qjs-restore --cwd expects a Wanix path"))?;
            cwd = NormalizedPath::new(os_arg_to_string(value, "qjs-restore --cwd")?)?;
            i += 1;
        } else if args[i] == "--before-env" {
            i += 1;
            let value = args
                .get(i)
                .ok_or_else(|| CliError::usage("qjs-restore --before-env expects KEY=VALUE"))?;
            let value = os_arg_to_string(value, "qjs-restore --before-env")?;
            validate_env_line(&value, "qjs-restore --before-env")?;
            before_env.push(value);
            i += 1;
        } else if args[i] == "--after-env" {
            i += 1;
            let value = args
                .get(i)
                .ok_or_else(|| CliError::usage("qjs-restore --after-env expects KEY=VALUE"))?;
            let value = os_arg_to_string(value, "qjs-restore --after-env")?;
            validate_env_line(&value, "qjs-restore --after-env")?;
            after_env.push(value);
            i += 1;
        } else if args[i] == "--before-arg" {
            i += 1;
            let value = args
                .get(i)
                .ok_or_else(|| CliError::usage("qjs-restore --before-arg expects VALUE"))?;
            before_args.push(os_arg_to_string(value, "qjs-restore --before-arg")?);
            i += 1;
        } else if args[i] == "--after-arg" {
            i += 1;
            let value = args
                .get(i)
                .ok_or_else(|| CliError::usage("qjs-restore --after-arg expects VALUE"))?;
            after_args.push(os_arg_to_string(value, "qjs-restore --after-arg")?);
            i += 1;
        } else if args[i] == "--mount" {
            i += 1;
            let value = args
                .get(i)
                .ok_or_else(|| CliError::usage("qjs-restore --mount expects HOST=GUEST"))?;
            mounts.push(parse_host_mount(
                &os_arg_to_string(value, "qjs-restore --mount")?,
                "qjs-restore --mount",
            )?);
            i += 1;
        } else if args[i] == "--" {
            i += 1;
            break;
        } else {
            break;
        }
    }

    let before_script = args
        .get(i)
        .ok_or_else(|| CliError::usage("qjs-restore expects before.js and after.js"))?;
    let before_script_path = PathBuf::from(before_script);
    i += 1;

    let after_script = args
        .get(i)
        .ok_or_else(|| CliError::usage("qjs-restore expects before.js and after.js"))?;
    let after_script_path = PathBuf::from(after_script);
    i += 1;

    if let Some(extra) = args.get(i) {
        return Err(CliError::usage(format!(
            "unexpected qjs-restore argument: {}",
            extra.to_string_lossy()
        )));
    }

    Ok(QjsRestoreCommand {
        before_script_path,
        after_script_path,
        before_args,
        after_args,
        before_env,
        after_env,
        cwd,
        mounts,
    })
}
