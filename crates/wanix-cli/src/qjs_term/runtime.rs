use std::io::{Read, Write};
use std::sync::Arc;

use wanix_qjs::{QuickJsRunner, QuickJsTaskDriver, QuickJsTaskRuntime};
use wanix_task::{Task, TaskTable};

use super::PostEvalFeed;
use super::post_eval::{PostEvalFeedContext, run_post_eval_feeds};
use super::program_spec::{
    PreparedQjsTermProgram, QjsTermProgram, prepare_qjs_term_namespace, read_qjs_term_program,
    terminal_task_env,
};
use super::pump::{
    ProcessEventSources, TerminalPumpPolicy, TerminalPumpState, drain_terminal_output,
};
use super::terminal::{AttachedTerminal, attach_task_terminal, finish_terminal_task_output};
use crate::{
    CliError, QjsCommand, apply_qjs_task_runtime_limits, configure_qjs_task, eval_qjs_source,
    quickjs_runner, read_qjs_stdin,
};

mod request;

pub(super) use request::{QjsTermProgramIo, QjsTermProgramRequest};

pub(super) fn run_qjs_term_program_streaming(
    mut request: QjsTermProgramRequest,
    io: QjsTermProgramIo<'_>,
) -> Result<i32, CliError> {
    let input = read_qjs_term_input(&mut request.qjs_command, request.program, io.process_stdin)?;
    let runner = quickjs_runner()?;
    let execution =
        prepare_qjs_term_execution(&runner, &request.qjs_command, request.program, input)?;
    let start_result = run_prepared_terminal_program(
        PreparedTerminalRun {
            runner: &runner,
            task: &execution.task,
            qjs_command: &request.qjs_command,
            program: request.program,
            prepared: execution.prepared,
        },
        TerminalRunStreams {
            feed_after_eval: request.feed_after_eval,
            event_sources: request.event_sources,
            terminal: &execution.terminal,
            process_stdin: io.process_stdin,
            process_stdout: io.process_stdout,
        },
    );
    finish_terminal_task_output(
        start_result,
        &execution.task,
        &execution.terminal,
        io.process_stdout,
        io.process_stderr,
    )
}

struct QjsTermInput {
    script: String,
    stdin_bytes: Option<Vec<u8>>,
}

fn read_qjs_term_input(
    qjs_command: &mut QjsCommand,
    program: QjsTermProgram,
    process_stdin: &mut dyn Read,
) -> Result<QjsTermInput, CliError> {
    Ok(QjsTermInput {
        script: read_qjs_term_program(&qjs_command.script_path, program)?,
        stdin_bytes: read_qjs_stdin(qjs_command.stdin.take(), process_stdin)?,
    })
}

struct PreparedQjsTermExecution {
    task: Task,
    prepared: PreparedQjsTermProgram,
    terminal: AttachedTerminal,
}

fn prepare_qjs_term_execution(
    runner: &Arc<QuickJsRunner>,
    qjs_command: &QjsCommand,
    program: QjsTermProgram,
    input: QjsTermInput,
) -> Result<PreparedQjsTermExecution, CliError> {
    let task = allocate_qjs_term_task(runner, qjs_command)?;
    let prepared = prepare_qjs_term_namespace(&task, qjs_command, program, input.script)?;
    let terminal = attach_task_terminal(&task, input.stdin_bytes)?;
    configure_terminal_qjs_task(&task, qjs_command, program, &prepared, &terminal)?;
    Ok(PreparedQjsTermExecution {
        task,
        prepared,
        terminal,
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

struct TerminalRunStreams<'a> {
    feed_after_eval: Vec<PostEvalFeed>,
    event_sources: ProcessEventSources,
    terminal: &'a AttachedTerminal,
    process_stdin: &'a mut dyn Read,
    process_stdout: &'a mut dyn Write,
}

struct PreparedTerminalRun<'a> {
    runner: &'a QuickJsRunner,
    task: &'a Task,
    qjs_command: &'a QjsCommand,
    program: QjsTermProgram,
    prepared: PreparedQjsTermProgram,
}

fn run_prepared_terminal_program(
    run: PreparedTerminalRun<'_>,
    mut streams: TerminalRunStreams<'_>,
) -> Result<(), CliError> {
    let mut runtime =
        create_limited_task_runtime(run.runner, run.task, run.qjs_command, run.program)?;
    eval_prepared_terminal_program(
        &mut runtime,
        run.qjs_command,
        run.program,
        &run.prepared,
        &mut streams,
    )?;
    let finish_result = runtime.finish();
    drain_terminal_output(
        &streams.terminal.device,
        &streams.terminal.id,
        streams.process_stdout,
    )?;
    finish_result?;
    Ok(())
}

fn create_limited_task_runtime(
    runner: &QuickJsRunner,
    task: &Task,
    qjs_command: &QjsCommand,
    program: QjsTermProgram,
) -> Result<QuickJsTaskRuntime, CliError> {
    let mut runtime = runner.create_task_runtime(task)?;
    if program == QjsTermProgram::BundledShell {
        task.set_dir(qjs_command.cwd.as_str())?;
    }
    apply_qjs_task_runtime_limits(
        &mut runtime,
        qjs_command.interrupt_poll_budget,
        qjs_command.memory_limit_bytes,
    )?;
    Ok(runtime)
}

fn eval_prepared_terminal_program(
    runtime: &mut QuickJsTaskRuntime,
    qjs_command: &QjsCommand,
    program: QjsTermProgram,
    prepared: &PreparedQjsTermProgram,
    streams: &mut TerminalRunStreams<'_>,
) -> Result<(), CliError> {
    let eval_ready_io_turns = eval_ready_io_turns(&streams.feed_after_eval, qjs_command, program);
    let eval_result = eval_qjs_source(
        runtime,
        &prepared.script,
        &prepared.guest_script,
        qjs_command.event_loop_wait_budget,
        eval_ready_io_turns,
    );
    drain_terminal_output(
        &streams.terminal.device,
        &streams.terminal.id,
        streams.process_stdout,
    )?;
    eval_result?;
    run_terminal_post_eval_feeds(runtime, qjs_command, streams)
}

fn eval_ready_io_turns(
    feed_after_eval: &[PostEvalFeed],
    qjs_command: &QjsCommand,
    program: QjsTermProgram,
) -> usize {
    if feed_after_eval.is_empty() || program == QjsTermProgram::BundledShell {
        qjs_command.ready_io_turns
    } else {
        0
    }
}

fn run_terminal_post_eval_feeds(
    runtime: &mut QuickJsTaskRuntime,
    qjs_command: &QjsCommand,
    streams: &mut TerminalRunStreams<'_>,
) -> Result<(), CliError> {
    if streams.feed_after_eval.is_empty() {
        return Ok(());
    }
    run_post_eval_feeds(
        std::mem::take(&mut streams.feed_after_eval),
        PostEvalFeedContext {
            process_stdin: streams.process_stdin,
            terminal: &streams.terminal.device,
            terminal_id: &streams.terminal.id,
            runtime,
            pump_state: TerminalPumpState {
                policy: TerminalPumpPolicy {
                    ready_io_turns: qjs_command.ready_io_turns,
                    event_loop_wait_budget: qjs_command.event_loop_wait_budget,
                    input_mode: streams.event_sources.input_mode,
                },
                resize_source: streams.event_sources.resize_source.clone(),
            },
            process_stdout: streams.process_stdout,
        },
    )
}
