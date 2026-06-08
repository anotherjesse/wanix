use std::io::Read;
use std::sync::Arc;

use wanix_qjs::{QuickJsRunner, QuickJsTaskDriver};
use wanix_task::{Task, TaskTable};

use super::super::program_spec::{
    PreparedQjsTermProgram, QjsTermProgram, prepare_qjs_term_namespace, read_qjs_term_program,
    terminal_task_env,
};
use super::super::terminal::{AttachedTerminal, attach_task_terminal};
use crate::mesh::IrohMount;
use crate::{CliError, QjsCommand, configure_qjs_task, read_qjs_stdin};

pub(super) struct PreparedQjsTermExecution {
    pub(super) task: Task,
    pub(super) prepared: PreparedQjsTermProgram,
    pub(super) terminal: AttachedTerminal,
    /// Native mesh-mount (`--mount-mesh`) keepalives. Each owns the tokio runtime
    /// its imported `FileSystem` drives QUIC ops on, so this must outlive `task`'s
    /// namespace ops. Declared LAST so it drops AFTER `task` (Rust drops fields in
    /// declaration order), keeping the runtime alive while `task`'s bindings drop.
    _mesh_mounts: Vec<IrohMount>,
}

pub(super) struct QjsTermInput {
    script: String,
    stdin_bytes: Option<Vec<u8>>,
}

pub(super) fn prepare_qjs_term_execution(
    runner: &Arc<QuickJsRunner>,
    qjs_command: &QjsCommand,
    program: QjsTermProgram,
    input: QjsTermInput,
) -> Result<PreparedQjsTermExecution, CliError> {
    let task = allocate_qjs_term_task(runner, qjs_command)?;
    let (prepared, mesh_mounts) =
        prepare_qjs_term_namespace(&task, qjs_command, program, input.script)?;
    let terminal = attach_task_terminal(&task, input.stdin_bytes)?;
    configure_terminal_qjs_task(&task, qjs_command, program, &prepared, &terminal)?;
    Ok(PreparedQjsTermExecution {
        task,
        prepared,
        terminal,
        _mesh_mounts: mesh_mounts,
    })
}

pub(super) fn read_qjs_term_input(
    qjs_command: &mut QjsCommand,
    program: QjsTermProgram,
    process_stdin: &mut dyn Read,
) -> Result<QjsTermInput, CliError> {
    Ok(QjsTermInput {
        script: read_qjs_term_program(&qjs_command.script_path, program)?,
        stdin_bytes: read_qjs_stdin(qjs_command.stdin.take(), process_stdin)?,
    })
}

fn allocate_qjs_term_task(
    runner: &Arc<QuickJsRunner>,
    qjs_command: &QjsCommand,
) -> Result<Task, CliError> {
    let table = TaskTable::new();
    let mut driver = QuickJsTaskDriver::new(Arc::clone(runner))
        .with_event_loop_wait_budget(qjs_command.event_loop_wait_budget)
        .with_ready_io_turns(qjs_command.ready_io_turns);
    if let Some(budget) = qjs_command.interrupt_poll_budget {
        driver = driver.with_interrupt_poll_budget(budget);
    }
    if let Some(bytes) = qjs_command.memory_limit_bytes {
        driver = driver.with_memory_limit_bytes(bytes);
    }
    table.register_driver("qjs", Arc::new(driver))?;
    Ok(table.allocate_root("qjs")?)
}

fn configure_terminal_qjs_task(
    task: &Task,
    qjs_command: &QjsCommand,
    program: QjsTermProgram,
    prepared: &PreparedQjsTermProgram,
    terminal: &AttachedTerminal,
) -> Result<(), CliError> {
    let task_env = terminal_task_env(qjs_command, program, terminal);
    configure_qjs_task(
        task,
        prepared.program_path,
        &qjs_command.args,
        &task_env,
        &prepared.runtime_cwd,
    )
}
