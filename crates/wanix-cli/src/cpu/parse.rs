//! Parsing `wanix cpu` into a [`CpuCommand`].
//!
//! Grammar: `cpu --node TICKET [--cwd DIR] [--write] [--env KEY=VALUE ...] --
//! KIND PROGRAM [ARG ...]`. Options precede the `--` separator; everything after
//! `--` is the job command (`KIND PROGRAM [ARG ...]`), so a program's own flags
//! are never confused for cpu options.

use std::ffi::OsString;
use std::path::PathBuf;

use crate::CliError;

/// A parsed `wanix cpu` invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CpuCommand {
    /// The `iroh://` ticket of the data node serving the cpu exec plane.
    pub(crate) node: String,
    /// The local directory reverse-exported as the job's world (default `.`).
    pub(crate) cwd: PathBuf,
    /// Whether the exported job subtree is read-write (default read-only).
    pub(crate) writable: bool,
    /// Environment lines in `KEY=value` form passed to the remote task.
    pub(crate) env: Vec<String>,
    /// The task driver kind to run under on the remote node (e.g. `qjs`, `wasm`).
    pub(crate) kind: String,
    /// The program path within the exported world, the task's argv[0].
    pub(crate) program: String,
    /// Arguments passed after the program.
    pub(crate) args: Vec<String>,
}

/// Parses `cpu --node TICKET [--cwd DIR] [--write] [--env KEY=VALUE ...] -- KIND
/// PROGRAM [ARG ...]`.
///
/// # Errors
///
/// Returns a usage error when `--node` is missing, an option lacks its value, an
/// unknown option appears before `--`, the `--` separator is missing, or the job
/// command after `--` lacks a kind and program.
pub(crate) fn parse_cpu_command(args: &[OsString]) -> Result<CpuCommand, CliError> {
    let (options, job) = split_at_separator(args)?;
    let parsed_options = parse_options(options)?;
    let (kind, program, job_args) = parse_job(job)?;
    Ok(CpuCommand {
        node: parsed_options.node,
        cwd: parsed_options.cwd,
        writable: parsed_options.writable,
        env: parsed_options.env,
        kind,
        program,
        args: job_args,
    })
}

/// Options parsed from the segment before the `--` separator.
struct CpuOptions {
    node: String,
    cwd: PathBuf,
    writable: bool,
    env: Vec<String>,
}

/// Splits the argument list into the options segment and the job segment at the
/// first `--`.
fn split_at_separator(args: &[OsString]) -> Result<(&[OsString], &[OsString]), CliError> {
    let separator = args.iter().position(|arg| arg == "--").ok_or_else(|| {
        CliError::usage("cpu requires `-- KIND PROGRAM [ARG ...]` after its options")
    })?;
    Ok((&args[..separator], &args[separator + 1..]))
}

/// Parses the options segment (everything before `--`).
fn parse_options(args: &[OsString]) -> Result<CpuOptions, CliError> {
    let mut node = None;
    let mut cwd = PathBuf::from(".");
    let mut writable = false;
    let mut env = Vec::new();
    let mut index = 0;
    while index < args.len() {
        let flag = args[index].to_string_lossy().into_owned();
        let value = || {
            args.get(index + 1)
                .map(|value| value.to_string_lossy().into_owned())
                .ok_or_else(|| CliError::usage(format!("cpu {flag} expects a value")))
        };
        match flag.as_str() {
            "--write" => {
                writable = true;
                index += 1;
                continue;
            }
            "--node" => node = Some(value()?),
            "--cwd" => cwd = PathBuf::from(value()?),
            "--env" => env.push(value()?),
            other => {
                return Err(CliError::usage(format!("unexpected cpu argument: {other}")));
            }
        }
        index += 2;
    }
    let node = node.ok_or_else(|| {
        CliError::usage("cpu requires --node iroh://PEER[?addr=IP:PORT] naming the data node")
    })?;
    Ok(CpuOptions {
        node,
        cwd,
        writable,
        env,
    })
}

/// Parses the job segment (everything after `--`) into kind, program, and args.
fn parse_job(job: &[OsString]) -> Result<(String, String, Vec<String>), CliError> {
    let mut job = job.iter();
    let kind = job
        .next()
        .ok_or_else(|| CliError::usage("cpu job command after `--` requires a KIND (e.g. qjs)"))?
        .to_string_lossy()
        .into_owned();
    let program = job
        .next()
        .ok_or_else(|| CliError::usage("cpu job command after `--` requires a PROGRAM path"))?
        .to_string_lossy()
        .into_owned();
    let args = job.map(|arg| arg.to_string_lossy().into_owned()).collect();
    Ok((kind, program, args))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn parses_a_minimal_job() {
        let command =
            parse_cpu_command(&args(&["--node", "iroh://peer", "--", "qjs", "build.js"])).unwrap();
        assert_eq!(command.node, "iroh://peer");
        assert_eq!(command.kind, "qjs");
        assert_eq!(command.program, "build.js");
        assert!(command.args.is_empty());
        assert_eq!(command.cwd, PathBuf::from("."));
        assert!(!command.writable);
        assert!(command.env.is_empty());
    }

    #[test]
    fn parses_cwd_write_env_and_job_args() {
        let command = parse_cpu_command(&args(&[
            "--node",
            "iroh://peer?addr=127.0.0.1:5000",
            "--cwd",
            "work",
            "--write",
            "--env",
            "K=v",
            "--env",
            "J=w",
            "--",
            "wasm",
            "tool.wasm",
            "--flag",
            "arg",
        ]))
        .unwrap();
        assert_eq!(command.cwd, PathBuf::from("work"));
        assert!(command.writable);
        assert_eq!(command.env, vec!["K=v".to_owned(), "J=w".to_owned()]);
        assert_eq!(command.kind, "wasm");
        assert_eq!(command.program, "tool.wasm");
        assert_eq!(command.args, vec!["--flag".to_owned(), "arg".to_owned()]);
    }

    #[test]
    fn job_flags_after_separator_are_not_cpu_options() {
        // A program's own `--cwd`/`--node` after `--` belongs to the job, not cpu.
        let command = parse_cpu_command(&args(&[
            "--node",
            "iroh://peer",
            "--",
            "qjs",
            "build.js",
            "--node",
            "x",
            "--cwd",
            "y",
        ]))
        .unwrap();
        assert_eq!(
            command.args,
            vec![
                "--node".to_owned(),
                "x".to_owned(),
                "--cwd".to_owned(),
                "y".to_owned(),
            ]
        );
    }

    #[test]
    fn requires_node() {
        let error = parse_cpu_command(&args(&["--", "qjs", "build.js"])).unwrap_err();
        assert!(error.to_string().contains("cpu requires --node"));
    }

    #[test]
    fn requires_the_separator() {
        let error = parse_cpu_command(&args(&["--node", "iroh://peer", "qjs"])).unwrap_err();
        assert!(error.to_string().contains("after its options"));
    }

    #[test]
    fn requires_a_kind_and_program_after_the_separator() {
        let only_kind =
            parse_cpu_command(&args(&["--node", "iroh://peer", "--", "qjs"])).unwrap_err();
        assert!(only_kind.to_string().contains("requires a PROGRAM"));
        let empty_job = parse_cpu_command(&args(&["--node", "iroh://peer", "--"])).unwrap_err();
        assert!(empty_job.to_string().contains("requires a KIND"));
    }

    #[test]
    fn reports_a_missing_option_value() {
        // `--node` is the last option before the `--` separator, so its value is
        // missing (the separator is not consumed as the value).
        let error = parse_cpu_command(&args(&["--node", "--", "qjs", "x"])).unwrap_err();
        assert!(error.to_string().contains("cpu --node expects a value"));
    }

    #[test]
    fn rejects_an_unknown_option() {
        let error = parse_cpu_command(&args(&[
            "--node",
            "iroh://peer",
            "--bogus",
            "--",
            "qjs",
            "x",
        ]))
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("unexpected cpu argument: --bogus")
        );
    }
}
