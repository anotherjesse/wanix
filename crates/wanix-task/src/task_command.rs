//! Shared extraction of a task's program, argv, env, and cwd.
//!
//! Task runtimes (`qjs`, `wasm`, future WASI drivers) all need the same view of
//! "what command does this task run": the resolved program path, the argv and
//! env handed to the guest, and the working directory. This module is the single
//! source of that extraction so each driver does not re-parse the task command.
//!
//! The underlying shell-quote `cmd` parsing already lives in
//! [`crate::quote_cmd_argv`] / `task.cmd_argv()`; these helpers only choose
//! between an explicit [`TaskSpec`] and the legacy `cmd`/`dir`/`env` fields.

use wanix_fs::{FsError, FsResult, NormalizedPath};

use crate::cmd::quote_cmd_argv;
use crate::{Task, TaskSpec};

/// The resolved program, argv, and cwd for a task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskCommand {
    /// Raw command text for `#task` display.
    pub raw: String,
    /// Program path resolved against the task cwd.
    pub program: NormalizedPath,
    /// Program arguments, excluding the program itself.
    pub args: Vec<String>,
    /// Working directory inside the task namespace.
    pub cwd: NormalizedPath,
}

/// Resolves the task program, argv, and cwd.
///
/// # Errors
///
/// Returns a filesystem error when the task command is empty or a path cannot
/// be normalized.
pub fn task_command(task: &Task) -> FsResult<TaskCommand> {
    let raw = task.cmd();
    let spec = task.spec();
    if task_spec_is_set(&spec) {
        let program = resolve_from_cwd(&spec.cwd, &spec.program)?;
        let raw = if raw.is_empty() {
            raw_command(&spec.program, &spec.args)
        } else {
            raw
        };
        return Ok(TaskCommand {
            raw,
            program,
            args: spec.args,
            cwd: spec.cwd,
        });
    }

    let cwd = task.dir();
    let argv = task
        .cmd_argv()
        .ok_or_else(|| FsError::Other("task cmd is empty".to_owned()))?;
    let (script, args) = argv
        .split_first()
        .ok_or_else(|| FsError::Other("task cmd is empty".to_owned()))?;
    let program = resolve_from_cwd(&cwd, &NormalizedPath::new(script)?)?;
    Ok(TaskCommand {
        raw,
        program,
        args: args.to_vec(),
        cwd,
    })
}

/// Returns the argv (program first) handed to the guest.
#[must_use]
pub fn task_wasi_argv(task: &Task) -> Vec<String> {
    let spec = task.spec();
    if task_spec_is_set(&spec) {
        return std::iter::once(spec.program.to_string())
            .chain(spec.args)
            .collect();
    }
    task.cmd_argv().unwrap_or_default()
}

/// Returns the `KEY=value` environment lines handed to the guest.
#[must_use]
pub fn task_wasi_env(task: &Task) -> Vec<String> {
    let spec = task.spec();
    if task_spec_is_set(&spec) {
        return spec
            .env
            .into_iter()
            .map(|(key, value)| format!("{key}={value}"))
            .collect();
    }
    task.env()
}

/// Returns the working directory used as the guest root preopen source.
#[must_use]
pub fn task_wasi_cwd(task: &Task) -> NormalizedPath {
    let spec = task.spec();
    if task_spec_is_set(&spec) {
        return spec.cwd;
    }
    task.dir()
}

/// Returns the program name used to select a task driver.
#[must_use]
pub fn task_program_for_check(task: &Task) -> Option<String> {
    let spec = task.spec();
    if task_spec_is_set(&spec) {
        return Some(spec.program.to_string());
    }
    task.cmd_argv()
        .and_then(|argv| argv.first().map(ToOwned::to_owned))
}

fn task_spec_is_set(spec: &TaskSpec) -> bool {
    spec.program.as_str() != "."
}

fn raw_command(program: &NormalizedPath, args: &[String]) -> String {
    quote_cmd_argv(std::iter::once(program.as_str()).chain(args.iter().map(String::as_str)))
}

fn resolve_from_cwd(cwd: &NormalizedPath, path: &NormalizedPath) -> FsResult<NormalizedPath> {
    if cwd.as_str() == "." {
        return Ok(path.clone());
    }
    if path.as_str() == "." {
        return Ok(cwd.clone());
    }
    NormalizedPath::new(format!("{cwd}/{path}"))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use wanix_vfs::Namespace;

    use super::*;
    use crate::TaskId;

    fn task_with_cmd(cmd: &str, dir: &str) -> Task {
        let task = Task::new(TaskId::new(1), TaskSpec::unset(), Namespace::new());
        task.set_dir(dir).expect("set dir");
        task.set_cmd(cmd).expect("set cmd");
        task
    }

    #[test]
    fn resolves_program_against_cwd() {
        let task = task_with_cmd("app.wasm a b", "work");
        let command = task_command(&task).expect("command");
        assert_eq!(command.program.as_str(), "work/app.wasm");
        assert_eq!(command.args, vec!["a".to_owned(), "b".to_owned()]);
        assert_eq!(command.cwd.as_str(), "work");
    }

    #[test]
    fn argv_includes_program_from_cmd() {
        let task = task_with_cmd("app.wasm one", ".");
        assert_eq!(
            task_wasi_argv(&task),
            vec!["app.wasm".to_owned(), "one".to_owned()]
        );
    }

    #[test]
    fn spec_overrides_cmd_fields() {
        let mut spec = TaskSpec::new("bin/run.wasm").expect("spec");
        spec.args = vec!["--flag".to_owned()];
        spec.env = BTreeMap::from([("A".to_owned(), "1".to_owned())]);
        spec.cwd = NormalizedPath::new("here").expect("cwd");
        let task = Task::new(TaskId::new(2), spec, Namespace::new());

        let command = task_command(&task).expect("command");
        assert_eq!(command.program.as_str(), "here/bin/run.wasm");
        assert_eq!(command.args, vec!["--flag".to_owned()]);
        assert_eq!(task_wasi_argv(&task)[0], "bin/run.wasm");
        assert_eq!(task_wasi_env(&task), vec!["A=1".to_owned()]);
        assert_eq!(task_wasi_cwd(&task).as_str(), "here");
        assert_eq!(
            task_program_for_check(&task).as_deref(),
            Some("bin/run.wasm")
        );
    }

    #[test]
    fn empty_cmd_is_an_error() {
        let task = Task::new(TaskId::new(3), TaskSpec::unset(), Namespace::new());
        assert!(task_command(&task).is_err());
        assert_eq!(task_program_for_check(&task), None);
    }
}
