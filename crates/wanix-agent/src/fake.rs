use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::json;
use wanix_fs::FsResult;

use crate::engine::{AgentEngine, AgentSession, EventStream};

/// A deterministic agent engine for tests and demos: each prompt produces a
/// fixed `you said: <prompt>` reply streamed as normalized events, with no
/// network or subprocess. This is what keeps the `#agent` device testable
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

struct FakeSession {
    stream: Arc<EventStream>,
    turns: AtomicU64,
}

impl FakeSession {
    fn new() -> Self {
        Self {
            stream: Arc::new(EventStream::new()),
            turns: AtomicU64::new(0),
        }
    }
}

impl AgentSession for FakeSession {
    fn submit(&self, prompt: &str) -> FsResult<()> {
        let turn = self.turns.fetch_add(1, Ordering::Relaxed) + 1;
        let reply = format!("you said: {prompt}");
        self.stream
            .push_line(&json!({ "t": "turn.started", "turn": turn }).to_string());
        // Stream the reply in two deltas to exercise the streaming path.
        let split = reply.len().min(9);
        self.stream
            .push_line(&json!({ "t": "message.delta", "text": &reply[..split] }).to_string());
        self.stream
            .push_line(&json!({ "t": "message.delta", "text": &reply[split..] }).to_string());
        self.stream
            .push_line(&json!({ "t": "message", "text": reply }).to_string());
        self.stream
            .push_line(&json!({ "t": "tokens", "total": 1, "input": 1, "output": 1 }).to_string());
        self.stream.push_line(
            &json!({ "t": "turn.completed", "turn": turn, "status": "completed" }).to_string(),
        );
        Ok(())
    }

    fn read_events(&self, buf: &mut [u8]) -> FsResult<usize> {
        self.stream.read(buf)
    }

    fn events_ready(&self) -> FsResult<bool> {
        self.stream.read_ready()
    }

    fn status(&self) -> String {
        format!("fake idle turns={}", self.turns.load(Ordering::Relaxed))
    }

    fn close(&self) {
        self.stream.close();
    }
}
