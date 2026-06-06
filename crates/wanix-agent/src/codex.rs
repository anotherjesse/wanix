use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use serde_json::{Value, json};
use wanix_fs::{FsError, FsResult};

use crate::engine::{AgentEngine, AgentSession, EventStream};

mod normalize;
mod overlay;
use normalize::normalize_notification;
use overlay::build_codex_overlay;

/// How long to wait for a JSON-RPC response before giving up.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(180);

fn other(message: impl Into<String>) -> FsError {
    FsError::Other(message.into())
}

/// A codex "environment" selecting the wanix exec-server for a thread/turn.
#[derive(Debug, Clone)]
struct EnvSpec {
    id: String,
    cwd: String,
}

fn environments_value(environment: &EnvSpec) -> Value {
    json!([{"environmentId": environment.id, "cwd": environment.cwd}])
}

/// Tells the agent its filesystem is a Wanix services namespace.
const WANIX_WORLD_INSTRUCTIONS: &str = "Your filesystem is a Wanix namespace rooted at `/`. \
Besides ordinary files, these service paths exist (they are not listed by `ls /`): \
`/#kv/<key>` is a key/value store — write a file to set a key, read it to get the value, \
list `/#kv` for keys. `/#task`, `/#term`, and `/#pipe` are the Wanix task, terminal, and \
pipe devices. Operate on all of them with ordinary file reads and writes.";

/// An [`AgentEngine`] that bridges a real `codex app-server` subprocess.
///
/// Each session spawns `codex app-server --listen stdio://`, performs the
/// JSON-RPC handshake, and starts a thread; prompts become `turn/start`
/// requests and the streamed notifications become normalized `#agent` events.
/// Requires a working codex auth (`CODEX_HOME`/`~/.codex`); gate it behind
/// explicit local trust.
#[derive(Debug, Clone)]
pub struct CodexEngine {
    codex_bin: PathBuf,
    codex_home: Option<PathBuf>,
    cwd: String,
    environment: Option<EnvSpec>,
}

impl CodexEngine {
    /// Creates an engine that runs `codex_bin` (resolved from `PATH` when a bare
    /// name) with the agent working directory `cwd` and an optional
    /// `CODEX_HOME` override (defaults to codex's own `~/.codex`). The agent
    /// reads the host filesystem under a read-only sandbox.
    #[must_use]
    pub fn new(codex_bin: PathBuf, codex_home: Option<PathBuf>, cwd: String) -> Self {
        Self {
            codex_bin,
            codex_home,
            cwd,
            environment: None,
        }
    }

    /// Creates an engine whose agent edits a confined Wanix world: the host
    /// directory `world_root` is exposed as the agent's ENTIRE filesystem via
    /// the wanix exec-server, so the agent's reads/writes/commands land there
    /// and nowhere else (even under a full-access sandbox). Builds a private
    /// `CODEX_HOME` overlay registering the exec-server.
    ///
    /// # Errors
    ///
    /// Returns an error when the overlay cannot be built.
    pub fn with_wanix_world(codex_bin: PathBuf, world_root: PathBuf) -> FsResult<Self> {
        // The exec-server backs the agent's filesystem with the host directory
        // `world_root`, but the agent SEES that as its root `/` — so the agent's
        // cwd is "/", not the host path (otherwise it builds doubly-rooted paths).
        let world = world_root.to_string_lossy().into_owned();
        let overlay = build_codex_overlay(&world)?;
        Ok(Self {
            codex_bin,
            codex_home: Some(overlay),
            cwd: "/".to_owned(),
            environment: Some(EnvSpec {
                id: "wanix".to_owned(),
                cwd: "/".to_owned(),
            }),
        })
    }
}

impl AgentEngine for CodexEngine {
    fn start_session(&self) -> FsResult<Arc<dyn AgentSession>> {
        let session = CodexSession::start(
            &self.codex_bin,
            self.codex_home.as_deref(),
            &self.cwd,
            self.environment.clone(),
        )?;
        Ok(Arc::new(session))
    }

    fn describe(&self) -> &str {
        "codex"
    }
}

struct CodexSession {
    client: Arc<AppServerClient>,
    thread_id: String,
    stream: Arc<EventStream>,
    turns: AtomicU64,
    environment: Option<EnvSpec>,
}

impl CodexSession {
    fn start(
        codex_bin: &Path,
        codex_home: Option<&Path>,
        cwd: &str,
        environment: Option<EnvSpec>,
    ) -> FsResult<Self> {
        let stream = Arc::new(EventStream::new());
        let client = AppServerClient::spawn(codex_bin, codex_home, Arc::clone(&stream))?;
        let thread_id = client.start_thread(cwd, environment.as_ref())?;
        Ok(Self {
            client,
            thread_id,
            stream,
            turns: AtomicU64::new(0),
            environment,
        })
    }
}

impl AgentSession for CodexSession {
    fn submit(&self, prompt: &str) -> FsResult<()> {
        self.client
            .turn_start(&self.thread_id, prompt, self.environment.as_ref())?;
        self.turns.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    fn read_events(&self, buf: &mut [u8]) -> FsResult<usize> {
        self.stream.read(buf)
    }

    fn events_ready(&self) -> FsResult<bool> {
        self.stream.read_ready()
    }

    fn status(&self) -> String {
        format!(
            "codex thread={} turns={}",
            self.thread_id,
            self.turns.load(Ordering::Relaxed)
        )
    }

    fn close(&self) {
        self.client.shutdown();
        self.stream.close();
    }
}

/// JSON-RPC client over a `codex app-server` child: a reader thread classifies
/// each newline-delimited message as a response (routed to the waiting
/// request), a server-request (replied to so the turn does not hang), or a
/// notification (normalized into the event stream).
struct AppServerClient {
    child: Mutex<Child>,
    stdin: Mutex<ChildStdin>,
    next_id: AtomicI64,
    pending: Mutex<HashMap<i64, Sender<Value>>>,
    stream: Arc<EventStream>,
}

impl AppServerClient {
    fn spawn(
        codex_bin: &Path,
        codex_home: Option<&Path>,
        stream: Arc<EventStream>,
    ) -> FsResult<Arc<Self>> {
        let mut command = Command::new(codex_bin);
        command.args(["app-server", "--listen", "stdio://"]);
        if let Some(home) = codex_home {
            command.env("CODEX_HOME", home);
        }
        command.stdin(Stdio::piped());
        command.stdout(Stdio::piped());
        command.stderr(Stdio::null());
        let mut child = command
            .spawn()
            .map_err(|error| other(format!("failed to spawn codex app-server: {error}")))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| other("codex app-server stdin unavailable"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| other("codex app-server stdout unavailable"))?;

        let client = Arc::new(Self {
            child: Mutex::new(child),
            stdin: Mutex::new(stdin),
            next_id: AtomicI64::new(0),
            pending: Mutex::new(HashMap::new()),
            stream,
        });

        let reader = Arc::clone(&client);
        thread::spawn(move || reader.read_loop(stdout));
        client.initialize()?;
        Ok(client)
    }

    fn write_message(&self, message: &Value) -> FsResult<()> {
        let mut stdin = self
            .stdin
            .lock()
            .map_err(|_| other("codex client stdin lock poisoned"))?;
        let line = serde_json::to_string(message)
            .map_err(|error| other(format!("serialize codex request: {error}")))?;
        stdin
            .write_all(line.as_bytes())
            .and_then(|()| stdin.write_all(b"\n"))
            .and_then(|()| stdin.flush())
            .map_err(|error| other(format!("write codex request: {error}")))
    }

    fn request(&self, method: &str, params: Value) -> FsResult<Value> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst) + 1;
        let (tx, rx) = channel();
        self.pending
            .lock()
            .map_err(|_| other("codex pending lock poisoned"))?
            .insert(id, tx);
        self.write_message(&json!({"id": id, "method": method, "params": params}))?;
        let response = rx
            .recv_timeout(REQUEST_TIMEOUT)
            .map_err(|_| other(format!("codex request `{method}` timed out")))?;
        if let Some(error) = response.get("error") {
            return Err(other(format!("codex `{method}` error: {error}")));
        }
        Ok(response.get("result").cloned().unwrap_or(Value::Null))
    }

    fn notify(&self, method: &str, params: Value) -> FsResult<()> {
        self.write_message(&json!({"method": method, "params": params}))
    }

    fn initialize(&self) -> FsResult<()> {
        self.request(
            "initialize",
            json!({
                "clientInfo": {"name": "wanix", "title": "Wanix", "version": env!("CARGO_PKG_VERSION")},
                "capabilities": {"experimentalApi": true}
            }),
        )?;
        self.notify("initialized", json!({}))
    }

    fn start_thread(&self, cwd: &str, environment: Option<&EnvSpec>) -> FsResult<String> {
        let sandbox = if environment.is_some() {
            "danger-full-access"
        } else {
            "read-only"
        };
        let mut params = json!({"cwd": cwd, "approvalPolicy": "never", "sandbox": sandbox});
        if let Some(environment) = environment {
            params["environments"] = environments_value(environment);
            params["developerInstructions"] = json!(WANIX_WORLD_INSTRUCTIONS);
        }
        let result = self.request("thread/start", params)?;
        result["thread"]["id"]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| other("codex thread/start returned no thread id"))
    }

    fn turn_start(
        &self,
        thread_id: &str,
        prompt: &str,
        environment: Option<&EnvSpec>,
    ) -> FsResult<()> {
        let mut params = json!({
            "threadId": thread_id,
            "input": [{"type": "text", "text": prompt, "text_elements": []}]
        });
        if let Some(environment) = environment {
            params["environments"] = environments_value(environment);
        }
        self.request("turn/start", params)?;
        Ok(())
    }

    fn read_loop(&self, stdout: ChildStdout) {
        let reader = BufReader::new(stdout);
        for line in reader.lines() {
            let Ok(line) = line else { break };
            if line.trim().is_empty() {
                continue;
            }
            let Ok(message) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            self.dispatch(message);
        }
        // codex exited: end the event stream so readers observe EOF.
        self.stream.close();
    }

    fn dispatch(&self, message: Value) {
        let id = message.get("id").and_then(Value::as_i64);
        let method = message.get("method").and_then(Value::as_str);
        match (id, method) {
            (Some(id), Some(method)) => self.reply_server_request(id, method),
            (Some(id), None) => self.route_response(id, message),
            (None, Some(method)) => {
                if let Some(line) = normalize_notification(method, message.get("params")) {
                    self.stream.push_line(&line);
                }
            }
            (None, None) => {}
        }
    }

    fn route_response(&self, id: i64, message: Value) {
        if let Ok(mut pending) = self.pending.lock()
            && let Some(tx) = pending.remove(&id)
        {
            let _ = tx.send(message);
        }
    }

    fn reply_server_request(&self, id: i64, method: &str) {
        // Under approvalPolicy=never the agent cannot exec/patch, so approval
        // requests should not fire; reply generically to anything that does so
        // the turn never blocks, and surface it as an event for visibility.
        self.stream
            .push_line(&json!({"t": "server.request", "method": method}).to_string());
        let result = if method.to_ascii_lowercase().contains("approval") {
            json!({"decision": "decline"})
        } else {
            json!({})
        };
        let _ = self.write_message(&json!({"id": id, "result": result}));
    }

    fn shutdown(&self) {
        if let Ok(mut child) = self.child.lock() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for AppServerClient {
    fn drop(&mut self) {
        self.shutdown();
    }
}
