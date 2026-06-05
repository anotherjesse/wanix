use std::collections::BTreeMap;

use wanix_fs::NormalizedPath;
use wanix_task::{Fd, Task, TaskSpec, quote_cmd_argv};

use crate::CliError;

pub(crate) fn configure_qjs_task(
    task: &Task,
    program: &str,
    args: &[String],
    env: &[String],
    cwd: &NormalizedPath,
) -> Result<(), CliError> {
    configure_qjs_task_spec(task, program, args, env, cwd)?;
    configure_qjs_task_observability(task, program, args, env, cwd)
}

fn configure_qjs_task_spec(
    task: &Task,
    program: &str,
    args: &[String],
    env: &[String],
    cwd: &NormalizedPath,
) -> Result<(), CliError> {
    task.set_spec(qjs_task_spec(program, args, env, cwd)?)?;
    Ok(())
}

fn qjs_task_spec(
    program: &str,
    args: &[String],
    env: &[String],
    cwd: &NormalizedPath,
) -> Result<TaskSpec, CliError> {
    let mut spec = TaskSpec::new(program)?;
    spec.args = args.to_vec();
    spec.env = env_map(env);
    spec.cwd = cwd.clone();
    Ok(spec)
}

fn configure_qjs_task_observability(
    task: &Task,
    program: &str,
    args: &[String],
    env: &[String],
    cwd: &NormalizedPath,
) -> Result<(), CliError> {
    task.set_cmd(task_cmd(program, args))?;
    task.set_env_lines(env.join("\n"))?;
    task.set_dir(cwd.to_string())?;
    Ok(())
}

fn task_cmd(program: &str, args: &[String]) -> String {
    quote_cmd_argv(std::iter::once(program).chain(args.iter().map(String::as_str)))
}

fn env_map(lines: &[String]) -> BTreeMap<String, String> {
    lines
        .iter()
        .filter_map(|line| {
            let (key, value) = line.split_once('=')?;
            Some((key.to_owned(), value.to_owned()))
        })
        .collect()
}

pub(crate) fn bind_child_output_to_parent(child: &Task, parent: &Task) -> Result<(), CliError> {
    let parent_id = parent.id().get();
    child.bind_fd_from_namespace(format!("#task/{parent_id}/fd/1"), Fd::STDOUT)?;
    child.bind_fd_from_namespace(format!("#task/{parent_id}/fd/2"), Fd::STDERR)?;
    Ok(())
}
