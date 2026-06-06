//! A process runner that lets the agent run real Wanix programs: a
//! `qjs FILE`/`wasm FILE` command from the agent's shell is executed through the
//! actual task runtime against the agent's namespace, so the LLM can write a
//! program and run it. Anything else returns `None` and falls back to the
//! exec-server's built-in shell.

use std::sync::Arc;

use wanix_agent::ProcessRunner;
use wanix_fs::{FileSystem, NormalizedPath};
use wanix_qjs::{QuickJsRunner, QuickJsTaskDriver};
use wanix_task::TaskTable;
use wanix_vfs::BindOptions;
use wanix_wasm::WasmTaskDriver;

use crate::{
    CliError, attach_task_stdio, configure_qjs_task, parse_exit, quickjs_runner, read_file,
};

/// Builds a runner that executes `qjs`/`wasm` programs against `world`.
pub(super) fn wanix_process_runner(world: Arc<dyn FileSystem>) -> ProcessRunner {
    let runner = quickjs_runner().ok();
    Box::new(move |source: &str, _cwd: &str| run_program(&world, runner.as_ref(), source))
}

fn run_program(
    world: &Arc<dyn FileSystem>,
    runner: Option<&Arc<QuickJsRunner>>,
    source: &str,
) -> Option<(Vec<u8>, Vec<u8>, i64)> {
    // Only claim a plain `qjs FILE [args]` / `wasm FILE [args]` (no shell
    // operators); leave everything else to the built-in shell.
    if source.contains("&&") || source.contains('|') || source.contains('>') {
        return None;
    }
    let tokens: Vec<&str> = source.split_whitespace().collect();
    let (kind, rest) = match tokens.split_first() {
        Some((&"qjs", rest)) => ("qjs", rest),
        Some((&"wasm", rest)) => ("wasm", rest),
        _ => return None,
    };
    let runner = runner?;
    let program = rest.first()?.trim_start_matches('/').to_owned();
    let args: Vec<String> = rest.iter().skip(1).map(|arg| (*arg).to_owned()).collect();
    match execute(world, runner, kind, &program, &args) {
        Ok(result) => Some(result),
        Err(error) => Some((Vec::new(), format!("{error}\n").into_bytes(), 1)),
    }
}

fn execute(
    world: &Arc<dyn FileSystem>,
    runner: &Arc<QuickJsRunner>,
    kind: &str,
    program: &str,
    args: &[String],
) -> Result<(Vec<u8>, Vec<u8>, i64), CliError> {
    let table = TaskTable::new();
    register_drivers(&table, runner)?;
    let task = table
        .allocate_root(kind)
        .map_err(|error| CliError::new(format!("agent run: allocate {kind}: {error:?}"), 1))?;
    task.bind(Arc::clone(world), ".", ".", BindOptions::default())
        .map_err(|error| CliError::new(format!("agent run: bind world: {error:?}"), 1))?;
    let (stdout, stderr) = attach_task_stdio(&task, None)?;
    let cwd = NormalizedPath::new(".")
        .map_err(|error| CliError::new(format!("agent run: cwd: {error:?}"), 1))?;
    configure_qjs_task(&task, program, args, &[], &cwd)?;
    let _ = table.start(task.id());
    let exit = parse_exit(&task.exit());
    let out = read_file(&*stdout, "stdout")?;
    let err = read_file(&*stderr, "stderr")?;
    Ok((out, err, i64::from(exit)))
}

fn register_drivers(table: &TaskTable, runner: &Arc<QuickJsRunner>) -> Result<(), CliError> {
    table
        .register_noop_driver("noop")
        .map_err(|error| CliError::new(format!("agent run: register noop: {error:?}"), 1))?;
    table
        .register_driver("qjs", Arc::new(QuickJsTaskDriver::new(Arc::clone(runner))))
        .map_err(|error| CliError::new(format!("agent run: register qjs: {error:?}"), 1))?;
    table
        .register_driver("wasm", Arc::new(WasmTaskDriver::new()))
        .map_err(|error| CliError::new(format!("agent run: register wasm: {error:?}"), 1))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use wanix_fs::{FileSystem, MemFs};

    use super::wanix_process_runner;

    #[test]
    fn agent_runs_a_qjs_program_in_its_world() {
        let world = Arc::new(MemFs::new());
        world
            .write_file("hello.js", b"print('hello from a wanix program');")
            .unwrap();
        let mut runner = wanix_process_runner(world as Arc<dyn FileSystem>);

        let (stdout, _stderr, exit) = runner("qjs hello.js", "/").expect("qjs is handled");
        assert_eq!(exit, 0, "stderr/exit: {exit}");
        assert!(
            String::from_utf8_lossy(&stdout).contains("hello from a wanix program"),
            "{}",
            String::from_utf8_lossy(&stdout)
        );
    }

    #[test]
    fn non_program_commands_fall_through() {
        let world = Arc::new(MemFs::new()) as Arc<dyn FileSystem>;
        let mut runner = wanix_process_runner(world);
        assert!(runner("echo hi", "/").is_none());
        assert!(runner("qjs a.js && echo done", "/").is_none());
    }
}
