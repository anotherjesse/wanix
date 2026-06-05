use wanix_fs::{FsError, FsResult, NormalizedPath};
use wanix_task::{Task, TaskSpec, quote_cmd_argv};

#[cfg(test)]
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TaskCommand {
    pub(crate) raw: String,
    pub(crate) program: NormalizedPath,
    pub(crate) args: Vec<String>,
    pub(crate) cwd: NormalizedPath,
}

pub(crate) fn task_command(task: &Task) -> FsResult<TaskCommand> {
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
        .ok_or_else(|| FsError::Other("qjs task cmd is empty".to_owned()))?;
    let (script, args) = argv
        .split_first()
        .ok_or_else(|| FsError::Other("qjs task cmd is empty".to_owned()))?;
    let program = resolve_from_cwd(&cwd, &NormalizedPath::new(script)?)?;
    Ok(TaskCommand {
        raw,
        program,
        args: args.to_vec(),
        cwd,
    })
}

pub(crate) fn task_wasi_argv(task: &Task) -> Vec<String> {
    let spec = task.spec();
    if task_spec_is_set(&spec) {
        return std::iter::once(spec.program.to_string())
            .chain(spec.args)
            .collect();
    }
    task.cmd_argv().unwrap_or_default()
}

pub(crate) fn task_wasi_env(task: &Task) -> Vec<String> {
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

pub(crate) fn task_wasi_cwd(task: &Task) -> NormalizedPath {
    let spec = task.spec();
    if task_spec_is_set(&spec) {
        return spec.cwd;
    }
    task.dir()
}

pub(crate) fn task_program_for_check(task: &Task) -> Option<String> {
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
pub(crate) fn task_env_map(task: &Task) -> BTreeMap<String, String> {
    task_wasi_env(task)
        .into_iter()
        .filter_map(|line| {
            let (key, value) = line.split_once('=')?;
            if key.is_empty() {
                return None;
            }
            Some((key.to_owned(), value.to_owned()))
        })
        .collect()
}
