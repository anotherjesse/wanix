use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use wanix_fs::{FsError, LocalFs, MemFs, NormalizedPath};
use wanix_qjs::{QuickJsTaskDriver, QuickJsTaskRuntime};
use wanix_task::{Task, TaskTable};
use wanix_term::TermDevice;
use wanix_vfs::BindOptions;

use super::{
    QJS_SHELL_IDLE_EVENT_LOOP_BUDGET_MS, QJS_SHELL_READY_IO_TURNS, QJS_SHELL_SCRIPT_SENTINEL,
    QJS_SHELL_SOURCE, TermResize, attach_task_terminal, configure_qjs_task,
    drain_terminal_output_bytes, eval_qjs_source, feed_terminal_after_eval,
    feed_terminal_resize_after_eval,
};
use crate::{CliError, parse_exit, quickjs_runner};

pub(crate) struct QjsShellSession {
    task: Task,
    terminal: Arc<TermDevice>,
    terminal_id: String,
    runtime: QuickJsTaskRuntime,
    ready_io_turns: usize,
    finished: bool,
    terminal_closed: bool,
}

impl QjsShellSession {
    #[cfg(test)]
    pub(crate) fn start(root_path: &Path) -> Result<(Self, Vec<u8>), CliError> {
        Self::start_in_cwd(root_path, &NormalizedPath::new(".")?)
    }

    pub(crate) fn start_in_cwd(
        root_path: &Path,
        cwd: &NormalizedPath,
    ) -> Result<(Self, Vec<u8>), CliError> {
        let runner = quickjs_runner()?;
        let table = TaskTable::new();
        table.register_driver("qjs", Arc::new(QuickJsTaskDriver::new(Arc::clone(&runner))))?;
        let task = table.allocate_root("qjs")?;

        let host_root = Arc::new(LocalFs::new(root_path).map_err(|error| {
            CliError::new(
                format!(
                    "failed to open terminal session root {}: {error}",
                    root_path.display()
                ),
                1,
            )
        })?);
        task.bind(host_root, ".", ".", BindOptions::default())?;

        let shell_source = Arc::new(MemFs::new());
        shell_source.write_file(QJS_SHELL_SCRIPT_SENTINEL, QJS_SHELL_SOURCE.as_bytes())?;
        task.bind(
            shell_source,
            QJS_SHELL_SCRIPT_SENTINEL,
            QJS_SHELL_SCRIPT_SENTINEL,
            BindOptions::default(),
        )?;

        let (terminal, terminal_id) = attach_task_terminal(&task, None)?;
        let runtime_cwd = NormalizedPath::new(".")?;
        configure_qjs_task(
            &task,
            QJS_SHELL_SCRIPT_SENTINEL,
            &[],
            &["WANIX_QJS_SHELL_RAW=1".to_owned()],
            &runtime_cwd,
        )?;

        let mut runtime = runner.create_task_runtime(&task)?;
        task.set_dir(cwd.as_str())?;
        eval_qjs_source(
            &mut runtime,
            QJS_SHELL_SOURCE,
            QJS_SHELL_SCRIPT_SENTINEL,
            Duration::ZERO,
            0,
        )?;
        let initial_output = drain_terminal_output_bytes(&terminal, &terminal_id)?;
        Ok((
            Self {
                task,
                terminal,
                terminal_id,
                runtime,
                ready_io_turns: QJS_SHELL_READY_IO_TURNS,
                finished: false,
                terminal_closed: false,
            },
            initial_output,
        ))
    }

    pub(crate) fn input(&mut self, bytes: &[u8]) -> Result<Vec<u8>, CliError> {
        if self.finished || bytes.is_empty() {
            return Ok(Vec::new());
        }
        feed_terminal_after_eval(&self.terminal, &self.terminal_id, bytes)?;
        self.runtime.run_ready_io_turns(self.ready_io_turns)?;
        self.finish_if_exited()?;
        drain_terminal_output_bytes(&self.terminal, &self.terminal_id)
    }

    pub(crate) fn resize(&mut self, columns: u16, rows: u16) -> Result<Vec<u8>, CliError> {
        if self.finished {
            return Ok(Vec::new());
        }
        feed_terminal_resize_after_eval(
            &self.terminal,
            &self.terminal_id,
            &TermResize { columns, rows },
        )?;
        self.runtime.run_ready_io_turns(self.ready_io_turns)?;
        self.finish_if_exited()?;
        drain_terminal_output_bytes(&self.terminal, &self.terminal_id)
    }

    pub(crate) fn pump(&mut self) -> Result<Vec<u8>, CliError> {
        if self.finished {
            return Ok(Vec::new());
        }
        self.runtime.run_event_loop_turns(
            Duration::from_millis(QJS_SHELL_IDLE_EVENT_LOOP_BUDGET_MS),
            self.ready_io_turns,
        )?;
        self.finish_if_exited()?;
        drain_terminal_output_bytes(&self.terminal, &self.terminal_id)
    }

    pub(crate) fn is_finished(&self) -> bool {
        self.finished
    }

    pub(crate) fn exit_code(&self) -> Result<Option<i32>, CliError> {
        if self.finished {
            return Ok(Some(parse_exit(&self.task.exit())));
        }
        Ok(self.runtime.exit_code()?)
    }

    #[cfg(test)]
    pub(crate) fn terminal_for_test(&self) -> Arc<TermDevice> {
        Arc::clone(&self.terminal)
    }

    #[cfg(test)]
    pub(crate) fn terminal_id_for_test(&self) -> &str {
        &self.terminal_id
    }

    pub(crate) fn close_terminal_resource(&mut self) -> Result<(), CliError> {
        if self.terminal_closed {
            return Ok(());
        }
        match self.terminal.close(&self.terminal_id) {
            Ok(()) | Err(FsError::NotFound) => {}
            Err(error) => return Err(error.into()),
        }
        self.terminal_closed = true;
        Ok(())
    }

    fn finish_if_exited(&mut self) -> Result<(), CliError> {
        if !self.finished && self.runtime.exit_code()?.is_some() {
            self.runtime.finish()?;
            self.finished = true;
        }
        Ok(())
    }
}

impl Drop for QjsShellSession {
    fn drop(&mut self) {
        let _ = self.close_terminal_resource();
    }
}
