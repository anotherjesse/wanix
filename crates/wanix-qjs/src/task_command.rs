//! QuickJS task-command extraction.
//!
//! The program/argv/env/cwd extraction is shared with the wasm task driver and
//! lives in [`wanix_task`]; this module re-exports it and keeps the qjs-only
//! test helper.

pub(crate) use wanix_task::{task_command, task_program_for_check, task_wasi_argv};

#[cfg(test)]
use std::collections::BTreeMap;

#[cfg(test)]
use wanix_task::{Task, task_wasi_env};

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
