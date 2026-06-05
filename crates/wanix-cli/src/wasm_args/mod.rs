//! Argument parsing for the `wanix-rust wasm` subcommand.
//!
//! `wasm` is the compiled `wasm32-wasi` sibling of `qjs`, so it shares the same
//! invocation surface for the four flags that make sense for a command-style
//! task: `--env`, `--cwd`, `--stdin`, and `--stdin-file`. It deliberately does
//! **not** reuse `QjsRunOptions`, which also carries qjs-only flags
//! (`--event-loop-ms`, `--memory-limit-bytes`, `--mount`, …) that are
//! meaningless for a bare WASI command. Instead it composes the lower-level
//! helpers the qjs setters already call so the two stay consistent.

use std::ffi::OsString;
use std::path::PathBuf;

use wanix_fs::NormalizedPath;

use crate::CliError;
use crate::qjs_args::{QjsStdin, os_arg_to_string, set_qjs_stdin, validate_env_line};

/// A parsed `wasm` command: the module path, guest args, and the shared
/// invocation flags (env, cwd, stdin).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WasmCommand {
    pub(crate) path: PathBuf,
    pub(crate) args: Vec<String>,
    pub(crate) env: Vec<String>,
    pub(crate) cwd: NormalizedPath,
    pub(crate) stdin: Option<QjsStdin>,
}

/// The wasm-specific invocation options: only the four flags that apply to a
/// command-style WASI guest.
struct WasmRunOptions {
    env: Vec<String>,
    cwd: NormalizedPath,
    stdin: Option<QjsStdin>,
}

impl WasmRunOptions {
    fn new() -> Result<Self, CliError> {
        Ok(Self {
            env: Vec::new(),
            cwd: NormalizedPath::new(".")?,
            stdin: None,
        })
    }

    fn into_command(self, path: PathBuf, args: Vec<String>) -> WasmCommand {
        WasmCommand {
            path,
            args,
            env: self.env,
            cwd: self.cwd,
            stdin: self.stdin,
        }
    }

    fn add_env(&mut self, value: &OsString, label: &str) -> Result<(), CliError> {
        let value = os_arg_to_string(value, label)?;
        validate_env_line(&value, label)?;
        self.env.push(value);
        Ok(())
    }

    fn set_cwd(&mut self, value: &OsString, label: &str) -> Result<(), CliError> {
        self.cwd = NormalizedPath::new(os_arg_to_string(value, label)?)?;
        Ok(())
    }

    fn set_stdin_bytes(&mut self, value: &OsString, label: &str) -> Result<(), CliError> {
        set_qjs_stdin(
            &mut self.stdin,
            QjsStdin::Bytes(os_arg_to_string(value, label)?.into_bytes()),
            "wasm",
        )
    }

    fn set_stdin_file(&mut self, value: &OsString, _label: &str) -> Result<(), CliError> {
        let source = if value == "-" {
            QjsStdin::Process
        } else {
            QjsStdin::File(PathBuf::from(value))
        };
        set_qjs_stdin(&mut self.stdin, source, "wasm")
    }
}

/// Parses `wasm [--env K=V] [--cwd DIR] [--stdin TEXT | --stdin-file PATH|-] FILE.wasm [args...]`.
///
/// # Errors
///
/// Returns a usage error if no module path is given or a flag is malformed.
pub(crate) fn parse_wasm_command(args: &[OsString]) -> Result<WasmCommand, CliError> {
    let mut options = WasmRunOptions::new()?;
    let i = parse_wasm_options(args, &mut options)?;
    let (path, i) = wasm_module_path(args, i)?;
    let wasm_args = wasm_guest_args(args, i)?;

    Ok(options.into_command(path, wasm_args))
}

type WasmOptionApplier = fn(&mut WasmRunOptions, &OsString, &str) -> Result<(), CliError>;

struct WasmOptionSpec {
    name: &'static str,
    apply: WasmOptionApplier,
}

const WASM_OPTIONS: &[WasmOptionSpec] = &[
    WasmOptionSpec {
        name: "--env",
        apply: WasmRunOptions::add_env,
    },
    WasmOptionSpec {
        name: "--cwd",
        apply: WasmRunOptions::set_cwd,
    },
    WasmOptionSpec {
        name: "--stdin",
        apply: WasmRunOptions::set_stdin_bytes,
    },
    WasmOptionSpec {
        name: "--stdin-file",
        apply: WasmRunOptions::set_stdin_file,
    },
];

fn parse_wasm_options(args: &[OsString], options: &mut WasmRunOptions) -> Result<usize, CliError> {
    let mut index = 0;
    while index < args.len() {
        if args[index] == "--" {
            return Ok(index + 1);
        }
        let Some(spec) = wasm_option_spec(&args[index]) else {
            return Ok(index);
        };
        apply_wasm_option(args, &mut index, options, spec)?;
    }
    Ok(index)
}

fn wasm_option_spec(arg: &OsString) -> Option<&'static WasmOptionSpec> {
    let arg = arg.to_str()?;
    WASM_OPTIONS.iter().find(|spec| spec.name == arg)
}

fn apply_wasm_option(
    args: &[OsString],
    index: &mut usize,
    options: &mut WasmRunOptions,
    spec: &WasmOptionSpec,
) -> Result<(), CliError> {
    let value = wasm_option_value(args, index, spec.name)?;
    let label = format!("wasm {}", spec.name);
    (spec.apply)(options, value, &label)
}

fn wasm_option_value<'a>(
    args: &'a [OsString],
    index: &mut usize,
    option: &str,
) -> Result<&'a OsString, CliError> {
    *index += 1;
    let value = args
        .get(*index)
        .ok_or_else(|| CliError::usage(format!("wasm {option} expects a value")))?;
    *index += 1;
    Ok(value)
}

fn wasm_module_path(args: &[OsString], index: usize) -> Result<(PathBuf, usize), CliError> {
    let path = args
        .get(index)
        .ok_or_else(|| CliError::usage("wasm expects a FILE.wasm path"))?;
    Ok((PathBuf::from(path), index + 1))
}

fn wasm_guest_args(args: &[OsString], mut index: usize) -> Result<Vec<String>, CliError> {
    if args.get(index).is_some_and(|arg| arg == "--") {
        index += 1;
    }
    args[index..]
        .iter()
        .map(|arg| os_arg_to_string(arg, "wasm guest arg"))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::path::PathBuf;

    use super::parse_wasm_command;
    use crate::qjs_args::QjsStdin;

    fn os_args<const N: usize>(args: [&str; N]) -> Vec<OsString> {
        args.into_iter().map(OsString::from).collect()
    }

    #[test]
    fn parse_wasm_command_applies_flags_before_module_path() {
        let command = parse_wasm_command(&os_args([
            "--env",
            "MODE=test",
            "--cwd",
            "work",
            "guest.wasm",
            "--",
            "--guest-flag",
            "value",
        ]))
        .unwrap();

        assert_eq!(command.path, PathBuf::from("guest.wasm"));
        assert_eq!(command.args, vec!["--guest-flag", "value"]);
        assert_eq!(command.env, vec!["MODE=test"]);
        assert_eq!(command.cwd.as_str(), "work");
        assert_eq!(command.stdin, None);
    }

    #[test]
    fn parse_wasm_command_stops_flags_at_module_path() {
        let command =
            parse_wasm_command(&os_args(["guest.wasm", "--env", "GUEST_ARG=not-an-option"]))
                .unwrap();

        assert_eq!(command.path, PathBuf::from("guest.wasm"));
        assert_eq!(command.args, vec!["--env", "GUEST_ARG=not-an-option"]);
        assert!(command.env.is_empty());
    }

    #[test]
    fn parse_wasm_command_selects_stdin_source_variants() {
        let bytes = parse_wasm_command(&os_args(["--stdin", "hello", "guest.wasm"])).unwrap();
        assert_eq!(bytes.stdin, Some(QjsStdin::Bytes(b"hello".to_vec())));

        let file =
            parse_wasm_command(&os_args(["--stdin-file", "input.txt", "guest.wasm"])).unwrap();
        assert_eq!(file.stdin, Some(QjsStdin::File(PathBuf::from("input.txt"))));

        let process = parse_wasm_command(&os_args(["--stdin-file", "-", "guest.wasm"])).unwrap();
        assert_eq!(process.stdin, Some(QjsStdin::Process));
    }

    #[test]
    fn parse_wasm_command_rejects_double_stdin() {
        let error = parse_wasm_command(&os_args([
            "--stdin",
            "a",
            "--stdin-file",
            "b",
            "guest.wasm",
        ]))
        .unwrap_err();
        assert_eq!(error.exit_code(), 2);
        assert!(
            error
                .to_string()
                .contains("wasm accepts only one of --stdin or --stdin-file")
        );
    }

    #[test]
    fn parse_wasm_command_requires_a_module_path() {
        let error = parse_wasm_command(&os_args(["--env", "MODE=test"])).unwrap_err();
        assert_eq!(error.exit_code(), 2);
        assert!(error.to_string().contains("wasm expects a FILE.wasm path"));
    }

    #[test]
    fn parse_wasm_command_reports_missing_flag_value_and_bad_env() {
        let error = parse_wasm_command(&os_args(["--cwd"])).unwrap_err();
        assert_eq!(error.exit_code(), 2);
        assert!(error.to_string().contains("wasm --cwd expects a value"));

        let error =
            parse_wasm_command(&os_args(["--env", "MISSING_VALUE", "guest.wasm"])).unwrap_err();
        assert_eq!(error.exit_code(), 2);
        assert!(error.to_string().contains("wasm --env expects KEY=VALUE"));
    }
}
