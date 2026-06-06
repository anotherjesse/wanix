//! A codex "environment" exec-server backed by a Wanix filesystem.
//!
//! `codex app-server` launches this as a child process and routes the agent's
//! `fs/*` and `process/start` tool calls to it over newline-delimited JSON-RPC.
//! Because every operation is serviced here against a Wanix [`FileSystem`], the
//! agent edits ONLY that confined Wanix world — it never touches the host, even
//! under `sandbox: danger-full-access`. The exec-server is the boundary.

use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde_json::{Value, json};
use wanix_fs::{FileSystem, FileType, FsResult, MetadataLookup, NormalizedPath, OpenOptions};

mod shell;

/// Maps an agent-supplied absolute path (`/a/b`) to a Wanix namespace path.
fn vpath(absolute: &str) -> FsResult<NormalizedPath> {
    let relative = absolute.trim_start_matches('/');
    NormalizedPath::new(if relative.is_empty() { "." } else { relative })
}

struct CompletedProcess {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    exit_code: i64,
}

/// A pluggable runner tried before the built-in shell for `process/start`.
/// Given the unwrapped shell source and cwd, it returns `(stdout, stderr,
/// exit_code)` when it handles the command, or `None` to fall back to the shell.
/// A host wires real Wanix program execution (`qjs`/`wasm`) in here.
pub type ProcessRunner = Box<dyn FnMut(&str, &str) -> Option<(Vec<u8>, Vec<u8>, i64)> + Send>;

/// A Wanix-filesystem-backed codex environment exec-server.
pub struct ExecServer {
    fs: Arc<dyn FileSystem>,
    processes: HashMap<String, CompletedProcess>,
    process_runner: Option<ProcessRunner>,
}

impl ExecServer {
    /// Creates an exec-server that services the agent against `fs`.
    #[must_use]
    pub fn new(fs: Arc<dyn FileSystem>) -> Self {
        Self {
            fs,
            processes: HashMap::new(),
            process_runner: None,
        }
    }

    /// Installs a runner tried before the built-in shell for `process/start`,
    /// so a host can execute real Wanix programs for the agent.
    pub fn set_process_runner(&mut self, runner: ProcessRunner) {
        self.process_runner = Some(runner);
    }

    /// Serves the JSON-RPC protocol over newline-delimited `input`/`output`
    /// until end of input.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when a line cannot be read or a reply written.
    pub fn serve(&mut self, input: impl BufRead, mut output: impl Write) -> std::io::Result<()> {
        for line in input.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let Ok(message) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            for reply in self.handle(&message) {
                serde_json::to_writer(&mut output, &reply)?;
                output.write_all(b"\n")?;
            }
            output.flush()?;
        }
        Ok(())
    }

    /// Handles one parsed message, returning the JSON lines to write (a response
    /// plus any notifications). Notifications (no `id`) produce no reply.
    #[must_use]
    pub fn handle(&mut self, message: &Value) -> Vec<Value> {
        let method = message
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let params = message.get("params").cloned().unwrap_or(Value::Null);
        let Some(id) = message.get("id").cloned() else {
            return Vec::new();
        };
        match self.dispatch(method, &params) {
            Ok(result) => vec![json!({ "id": id, "result": result })],
            Err(message) => {
                vec![json!({ "id": id, "error": { "code": -32000, "message": message } })]
            }
        }
    }

    fn dispatch(&mut self, method: &str, params: &Value) -> Result<Value, String> {
        match method {
            "initialize" => Ok(json!({ "sessionId": "wanix" })),
            "fs/readFile" => self.fs_read_file(params),
            "fs/writeFile" => self.fs_write_file(params),
            "fs/getMetadata" => self.fs_metadata(params),
            "fs/readDirectory" => self.fs_read_directory(params),
            "fs/createDirectory" => self.fs_create_directory(params),
            "fs/remove" => self.fs_remove(params),
            "fs/canonicalize" => Ok(json!({ "path": param_path(params) })),
            "fs/join" => Ok(
                json!({ "path": join_paths(params["basePath"].as_str().unwrap_or("/"), params["path"].as_str().unwrap_or("")) }),
            ),
            "fs/parent" => Ok(json!({ "path": parent_path(param_path(params)) })),
            "process/start" => self.process_start(params),
            "process/read" => self.process_read(params),
            "process/write" => Ok(json!({ "status": "stdinClosed" })),
            "process/terminate" => Ok(json!({ "running": false })),
            other => Err(format!("method not found: {other}")),
        }
    }

    fn fs_read_file(&self, params: &Value) -> Result<Value, String> {
        let bytes = self.read_bytes(param_path(params)).map_err(fs_err)?;
        Ok(json!({ "dataBase64": BASE64.encode(bytes) }))
    }

    fn fs_write_file(&self, params: &Value) -> Result<Value, String> {
        let data = BASE64
            .decode(params["dataBase64"].as_str().unwrap_or_default())
            .map_err(|error| format!("invalid dataBase64: {error}"))?;
        self.write_bytes(param_path(params), &data)
            .map_err(fs_err)?;
        Ok(json!({}))
    }

    fn fs_metadata(&self, params: &Value) -> Result<Value, String> {
        let metadata = self
            .fs
            .metadata_with_lookup(
                &vpath(param_path(params)).map_err(fs_err)?,
                MetadataLookup::NoFollow,
            )
            .map_err(fs_err)?;
        let file_type = metadata.file_type();
        Ok(json!({
            "isDirectory": matches!(file_type, FileType::Directory),
            "isFile": matches!(file_type, FileType::File),
            "isSymlink": matches!(file_type, FileType::Symlink),
            "createdAtMs": 0,
            "modifiedAtMs": 0,
        }))
    }

    fn fs_read_directory(&self, params: &Value) -> Result<Value, String> {
        let entries = self
            .fs
            .read_dir(&vpath(param_path(params)).map_err(fs_err)?)
            .map_err(fs_err)?;
        let entries: Vec<Value> = entries
            .iter()
            .map(|entry| {
                json!({
                    "fileName": entry.name(),
                    "isDirectory": matches!(entry.metadata().file_type(), FileType::Directory),
                    "isFile": matches!(entry.metadata().file_type(), FileType::File),
                })
            })
            .collect();
        Ok(json!({ "entries": entries }))
    }

    fn fs_create_directory(&self, params: &Value) -> Result<Value, String> {
        self.fs
            .create_dir(&vpath(param_path(params)).map_err(fs_err)?)
            .map_err(fs_err)?;
        Ok(json!({}))
    }

    fn fs_remove(&self, params: &Value) -> Result<Value, String> {
        let path = vpath(param_path(params)).map_err(fs_err)?;
        self.fs
            .remove_file(&path)
            .or_else(|_| self.fs.remove_dir(&path))
            .map_err(fs_err)?;
        Ok(json!({}))
    }

    fn process_start(&mut self, params: &Value) -> Result<Value, String> {
        let process_id = params["processId"].as_str().unwrap_or("p0").to_owned();
        let argv: Vec<String> = serde_json::from_value(params["argv"].clone()).unwrap_or_default();
        let cwd = params["cwd"].as_str().unwrap_or("/").to_owned();
        let source = shell::unwrap_source(&argv);
        let completed = self.run_process(&source, &cwd);
        self.processes.insert(process_id.clone(), completed);
        Ok(json!({ "processId": process_id }))
    }

    fn run_process(&mut self, source: &str, cwd: &str) -> CompletedProcess {
        let mut runner = self.process_runner.take();
        let hooked = runner.as_mut().and_then(|run| run(source, cwd));
        self.process_runner = runner;
        match hooked {
            Some((stdout, stderr, exit_code)) => CompletedProcess {
                stdout,
                stderr,
                exit_code,
            },
            None => shell::run_source(self, source, cwd),
        }
    }

    fn process_read(&mut self, params: &Value) -> Result<Value, String> {
        let process_id = params["processId"].as_str().unwrap_or_default();
        let after_seq = params["afterSeq"].as_i64().unwrap_or(0);
        let Some(process) = self.processes.get(process_id) else {
            return Ok(
                json!({ "chunks": [], "nextSeq": 0, "exited": true, "exitCode": 127, "closed": true, "failure": null }),
            );
        };
        let mut chunks = Vec::new();
        if after_seq < 1 && !process.stdout.is_empty() {
            chunks.push(
                json!({ "seq": 1, "stream": "stdout", "chunk": BASE64.encode(&process.stdout) }),
            );
        }
        if after_seq < 2 && !process.stderr.is_empty() {
            chunks.push(
                json!({ "seq": 2, "stream": "stderr", "chunk": BASE64.encode(&process.stderr) }),
            );
        }
        Ok(json!({
            "chunks": chunks,
            "nextSeq": 3,
            "exited": true,
            "exitCode": process.exit_code,
            "closed": true,
            "failure": null,
        }))
    }

    pub(crate) fn read_bytes(&self, path: &str) -> FsResult<Vec<u8>> {
        let mut file = self.fs.open(&vpath(path)?, OpenOptions::read())?;
        let mut out = Vec::new();
        let mut buf = [0u8; 4096];
        loop {
            let n = file.read(&mut buf)?;
            if n == 0 {
                break;
            }
            out.extend_from_slice(&buf[..n]);
        }
        Ok(out)
    }

    pub(crate) fn write_bytes(&self, path: &str, data: &[u8]) -> FsResult<()> {
        self.ensure_parents(path);
        let options = OpenOptions {
            read: false,
            write: true,
            create: true,
            truncate: true,
        };
        let mut file = self.fs.open(&vpath(path)?, options)?;
        let mut written = 0;
        while written < data.len() {
            let n = file.write(&data[written..])?;
            if n == 0 {
                break;
            }
            written += n;
        }
        Ok(())
    }

    pub(crate) fn make_dir(&self, path: &str) -> FsResult<()> {
        self.fs.create_dir(&vpath(path)?)
    }

    pub(crate) fn chmod(&self, path: &str, mode: u32) -> FsResult<()> {
        self.fs.set_permissions(&vpath(path)?, mode)
    }

    pub(crate) fn remove(&self, path: &str) -> FsResult<()> {
        let path = vpath(path)?;
        self.fs
            .remove_file(&path)
            .or_else(|_| self.fs.remove_dir(&path))
    }

    pub(crate) fn list(&self, path: &str) -> FsResult<Vec<String>> {
        Ok(self
            .fs
            .read_dir(&vpath(path)?)?
            .iter()
            .map(|entry| entry.name().to_owned())
            .collect())
    }

    fn ensure_parents(&self, path: &str) {
        let trimmed = path.trim_start_matches('/');
        let segments: Vec<&str> = trimmed.split('/').filter(|s| !s.is_empty()).collect();
        let mut prefix = String::new();
        for segment in segments.iter().take(segments.len().saturating_sub(1)) {
            prefix.push_str(segment);
            if let Ok(dir) = NormalizedPath::new(&prefix) {
                let _ = self.fs.create_dir(&dir);
            }
            prefix.push('/');
        }
    }
}

fn param_path(params: &Value) -> &str {
    params["path"].as_str().unwrap_or("/")
}

fn fs_err(error: wanix_fs::FsError) -> String {
    format!("{error:?}")
}

fn join_paths(base: &str, path: &str) -> String {
    if path.starts_with('/') {
        return path.to_owned();
    }
    format!("{}/{}", base.trim_end_matches('/'), path)
}

fn parent_path(path: &str) -> Value {
    match path.trim_end_matches('/').rsplit_once('/') {
        Some(("", _)) | None => Value::Null,
        Some((parent, _)) => Value::String(parent.to_owned()),
    }
}

#[cfg(test)]
mod tests;
