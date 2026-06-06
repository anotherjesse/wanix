use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::json;
use wanix_fs::{FsError, FsResult};

use crate::engine::{AgentEngine, AgentSession, EventStream};

/// A deterministic agent engine for tests and demos: each prompt produces a
/// fixed `you said: <prompt>` reply streamed as normalized events, with no
/// network or subprocess. A prompt prefixed `approve:` instead parks an
/// approval request that must be resolved via `ctl` before the turn completes,
/// exercising the trust-boundary path. Keeps the `#agent` device testable
/// without a live LLM.
#[derive(Debug, Default, Clone, Copy)]
pub struct FakeEngine;

impl AgentEngine for FakeEngine {
    fn start_session(&self) -> FsResult<Arc<dyn AgentSession>> {
        Ok(Arc::new(FakeSession::new()))
    }

    fn describe(&self) -> &str {
        "fake"
    }
}

struct PendingApproval {
    id: String,
    action: String,
}

struct FakeSession {
    stream: Arc<EventStream>,
    turns: AtomicU64,
    pending: Mutex<Option<PendingApproval>>,
    last_reply: Mutex<Option<String>>,
}

impl FakeSession {
    fn new() -> Self {
        Self {
            stream: Arc::new(EventStream::new()),
            turns: AtomicU64::new(0),
            pending: Mutex::new(None),
            last_reply: Mutex::new(None),
        }
    }

    fn set_reply(&self, text: String) {
        if let Ok(mut reply) = self.last_reply.lock() {
            *reply = Some(text);
        }
    }

    fn complete_reply(&self, turn: u64, prompt: &str) {
        let reply = format!("you said: {prompt}");
        let split = reply.len().min(9);
        self.stream
            .push_line(&json!({ "t": "message.delta", "text": &reply[..split] }).to_string());
        self.stream
            .push_line(&json!({ "t": "message.delta", "text": &reply[split..] }).to_string());
        self.stream
            .push_line(&json!({ "t": "message", "text": &reply }).to_string());
        self.stream
            .push_line(&json!({ "t": "tokens", "total": 1, "input": 1, "output": 1 }).to_string());
        self.set_reply(reply);
        self.finish(turn);
    }

    fn finish(&self, turn: u64) {
        self.stream.push_line(
            &json!({ "t": "turn.completed", "turn": turn, "status": "completed" }).to_string(),
        );
    }
}

impl AgentSession for FakeSession {
    fn submit(&self, prompt: &str) -> FsResult<()> {
        let turn = self.turns.fetch_add(1, Ordering::Relaxed) + 1;
        self.stream
            .push_line(&json!({ "t": "turn.started", "turn": turn }).to_string());
        if let Some(action) = prompt.strip_prefix("approve:") {
            let action = action.trim().to_owned();
            let id = format!("req-{turn}");
            if let Ok(mut pending) = self.pending.lock() {
                *pending = Some(PendingApproval {
                    id: id.clone(),
                    action: action.clone(),
                });
            }
            // The turn now blocks on a human decision (see `pending`/`resolve`).
            self.stream.push_line(
                &json!({ "t": "approval.needed", "id": id, "action": action }).to_string(),
            );
        } else {
            self.complete_reply(turn, prompt);
        }
        Ok(())
    }

    fn read_events(&self, buf: &mut [u8]) -> FsResult<usize> {
        self.stream.read(buf)
    }

    fn events_ready(&self) -> FsResult<bool> {
        self.stream.read_ready()
    }

    fn status(&self) -> String {
        let pending = self
            .pending
            .lock()
            .ok()
            .and_then(|p| p.as_ref().map(|p| p.id.clone()));
        match pending {
            Some(id) => format!(
                "fake awaiting-approval={id} turns={}",
                self.turns.load(Ordering::Relaxed)
            ),
            None => format!("fake idle turns={}", self.turns.load(Ordering::Relaxed)),
        }
    }

    fn pending(&self) -> String {
        match self.pending.lock() {
            Ok(guard) => match guard.as_ref() {
                Some(p) => json!([{ "id": p.id, "action": p.action }]).to_string(),
                None => "[]".to_owned(),
            },
            Err(_) => "[]".to_owned(),
        }
    }

    fn resolve(&self, request_id: &str, decision: &str) -> FsResult<()> {
        let action = {
            let mut guard = self
                .pending
                .lock()
                .map_err(|_| FsError::Other("fake pending lock poisoned".to_owned()))?;
            match guard.take() {
                Some(p) if p.id == request_id => p.action,
                other => {
                    *guard = other;
                    return Err(FsError::NotFound);
                }
            }
        };
        let verb = if decision == "approve" {
            "approved"
        } else {
            "declined"
        };
        let message = format!("{verb}: {action}");
        self.stream
            .push_line(&json!({ "t": "message", "text": &message }).to_string());
        self.set_reply(message);
        self.finish(self.turns.load(Ordering::Relaxed));
        Ok(())
    }

    fn wait_reply(&self) -> FsResult<String> {
        Ok(self
            .last_reply
            .lock()
            .ok()
            .and_then(|reply| reply.clone())
            .unwrap_or_default())
    }

    fn close(&self) {
        self.stream.close();
    }
}
