use serde_json::{Value, json};

/// Maps a codex app-server notification to a small, stable `#agent` event line
/// (one compact JSON object), or `None` to drop it. Unrecognized notifications
/// pass through as `{"t":"raw","method":…}` so nothing in the stream is hidden.
pub(super) fn normalize_notification(method: &str, params: Option<&Value>) -> Option<String> {
    let params = params.cloned().unwrap_or(Value::Null);
    let line = match method {
        "item/agentMessage/delta" => {
            json!({"t": "message.delta", "text": text_field(&params, "delta")})
        }
        "item/reasoning/textDelta" | "item/reasoning/summaryTextDelta" => {
            json!({"t": "reasoning.delta", "text": text_field(&params, "delta")})
        }
        "item/commandExecution/outputDelta" => {
            json!({"t": "command.output", "text": text_field(&params, "delta")})
        }
        "item/started" => return started_event(&params),
        "item/completed" => return completed_event(&params),
        "turn/started" => json!({"t": "turn.started"}),
        "turn/completed" => {
            let turn = &params["turn"];
            json!({
                "t": "turn.completed",
                "status": turn["status"].as_str().unwrap_or("completed"),
                "durationMs": turn["durationMs"].as_i64(),
            })
        }
        "thread/tokenUsage/updated" => {
            let total = &params["tokenUsage"]["total"];
            json!({
                "t": "tokens",
                "total": total["totalTokens"].as_i64(),
                "input": total["inputTokens"].as_i64(),
                "output": total["outputTokens"].as_i64(),
            })
        }
        "error" | "turn/error" => {
            json!({"t": "error", "message": params["message"].as_str().unwrap_or("error")})
        }
        // Chatty lifecycle notifications that add no signal to the event stream.
        "thread/started"
        | "thread/status/changed"
        | "account/rateLimits/updated"
        | "turn/diff/updated" => return None,
        other => json!({"t": "raw", "method": other}),
    };
    Some(line.to_string())
}

fn text_field(params: &Value, key: &str) -> String {
    params[key].as_str().unwrap_or_default().to_owned()
}

fn started_event(params: &Value) -> Option<String> {
    let item = &params["item"];
    if item["type"].as_str() == Some("commandExecution") {
        let command = item["command"].as_str().unwrap_or_default();
        return Some(json!({"t": "command.started", "cmd": command}).to_string());
    }
    None
}

fn completed_event(params: &Value) -> Option<String> {
    let item = &params["item"];
    match item["type"].as_str() {
        Some("agentMessage") => Some(
            json!({"t": "message", "text": item["text"].as_str().unwrap_or_default()}).to_string(),
        ),
        Some("commandExecution") => {
            Some(json!({"t": "command.completed", "exit": item["exitCode"].as_i64()}).to_string())
        }
        Some("fileChange") => Some(
            json!({"t": "file.change", "path": item["path"].as_str().unwrap_or_default()})
                .to_string(),
        ),
        _ => None,
    }
}
