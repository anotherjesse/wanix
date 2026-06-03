use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use anyhow::anyhow;
use wanix_fs::{FsError, FsResult, NormalizedPath};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WanixTaskContext {
    cmd: String,
    args: Vec<String>,
    env: BTreeMap<String, String>,
    cwd: NormalizedPath,
}

impl WanixTaskContext {
    pub(crate) fn new(
        cmd: impl Into<String>,
        args: Vec<String>,
        env: BTreeMap<String, String>,
        cwd: NormalizedPath,
    ) -> Self {
        Self {
            cmd: cmd.into(),
            args,
            env,
            cwd,
        }
    }

    pub(crate) fn cmd(&self) -> &str {
        &self.cmd
    }

    pub(crate) fn args(&self) -> &[String] {
        &self.args
    }

    pub(crate) fn env(&self) -> &BTreeMap<String, String> {
        &self.env
    }

    pub(crate) fn cwd(&self) -> &NormalizedPath {
        &self.cwd
    }
}

impl Default for WanixTaskContext {
    fn default() -> Self {
        Self {
            cmd: String::new(),
            args: Vec::new(),
            env: BTreeMap::new(),
            cwd: NormalizedPath::new(".").expect("root path is valid"),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct WanixExitState {
    code: Arc<Mutex<Option<i32>>>,
}

impl WanixExitState {
    pub(crate) fn request_exit(&self, code: i32) -> anyhow::Result<()> {
        let mut current = self
            .code
            .lock()
            .map_err(|_| anyhow!("exit state lock poisoned"))?;
        if current.is_none() {
            *current = Some(code);
        }
        Ok(())
    }

    pub(crate) fn code(&self) -> FsResult<Option<i32>> {
        self.code
            .lock()
            .map(|code| *code)
            .map_err(|_| FsError::Other("exit state lock poisoned".to_owned()))
    }

    pub(crate) fn is_requested(&self) -> anyhow::Result<bool> {
        self.code
            .lock()
            .map(|code| code.is_some())
            .map_err(|_| anyhow!("exit state lock poisoned"))
    }
}
