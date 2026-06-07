//! `POST /agent` — an HTTP front end to the served `#agent` device: the request
//! body is a prompt; it allocates a session, submits the turn, and returns the
//! normalized JSONL event log. Loopback-only (the agent is a local-trust
//! surface) and requires `--wanix-services`. The agentic x webby convergence.

use std::net::SocketAddr;
use std::sync::Arc;

use wanix_fs::{FileSystem, NormalizedPath, OpenOptions};

use super::super::ServeRoots;
use super::{HttpStatus, StaticResponse};

pub(super) fn agent_endpoint(
    roots: &ServeRoots,
    body: &[u8],
    peer_addr: SocketAddr,
) -> StaticResponse {
    if !peer_addr.ip().is_loopback() {
        return StaticResponse::plain(HttpStatus::Forbidden, "agent endpoint is loopback-only");
    }
    if !roots.wanix_services {
        return StaticResponse::plain(
            HttpStatus::BadRequest,
            "agent endpoint requires serve --wanix-services",
        );
    }
    let prompt = String::from_utf8_lossy(body).trim().to_owned();
    if prompt.is_empty() {
        return StaticResponse::plain(
            HttpStatus::BadRequest,
            "agent endpoint requires a prompt body",
        );
    }
    match run_agent_turn(&roots.p9_root, &prompt) {
        Ok(log) => StaticResponse {
            status: HttpStatus::Ok,
            content_type: "application/x-ndjson; charset=utf-8",
            headers: Vec::new(),
            body: log,
        },
        Err(error) => StaticResponse::plain(HttpStatus::Conflict, &error),
    }
}

fn run_agent_turn(fs: &Arc<dyn FileSystem>, prompt: &str) -> Result<Vec<u8>, String> {
    let id_bytes = read_path(fs, "#agent/new")?;
    let id = String::from_utf8_lossy(&id_bytes).trim().to_owned();
    if id.is_empty() {
        return Err("agent: session allocation failed".to_owned());
    }
    write_path(fs, &format!("#agent/{id}/prompt"), prompt.as_bytes())?;
    stream_events(fs, &id)
}

fn path(raw: &str) -> Result<NormalizedPath, String> {
    NormalizedPath::new(raw).map_err(|error| format!("agent: bad path {raw}: {error:?}"))
}

fn read_path(fs: &Arc<dyn FileSystem>, raw: &str) -> Result<Vec<u8>, String> {
    let mut file = fs
        .open(&path(raw)?, OpenOptions::read())
        .map_err(|error| format!("agent: open {raw}: {error:?}"))?;
    let mut out = Vec::new();
    let mut buffer = [0u8; 256];
    loop {
        let n = file
            .read(&mut buffer)
            .map_err(|error| format!("agent: read {raw}: {error:?}"))?;
        if n == 0 {
            break;
        }
        out.extend_from_slice(&buffer[..n]);
    }
    Ok(out)
}

fn write_path(fs: &Arc<dyn FileSystem>, raw: &str, bytes: &[u8]) -> Result<(), String> {
    let mut file = fs
        .open(&path(raw)?, OpenOptions::read_write())
        .map_err(|error| format!("agent: open {raw}: {error:?}"))?;
    file.write(bytes)
        .map(|_| ())
        .map_err(|error| format!("agent: write {raw}: {error:?}"))
}

fn stream_events(fs: &Arc<dyn FileSystem>, id: &str) -> Result<Vec<u8>, String> {
    let raw = format!("#agent/{id}/events");
    let mut file = fs
        .open(&path(&raw)?, OpenOptions::read())
        .map_err(|error| format!("agent: open {raw}: {error:?}"))?;
    let mut out = Vec::new();
    let mut buffer = [0u8; 512];
    loop {
        let n = file
            .read(&mut buffer)
            .map_err(|error| format!("agent: read {raw}: {error:?}"))?;
        if n == 0 {
            break;
        }
        out.extend_from_slice(&buffer[..n]);
        if String::from_utf8_lossy(&out).contains("\"turn.completed\"") {
            break;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};

    use crate::serve::services_namespace_for_root;

    use super::run_agent_turn;

    static COUNTER: AtomicU32 = AtomicU32::new(0);

    #[test]
    fn agent_turn_streams_event_log_through_the_served_device() {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("wanix-agent-http-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let fs = services_namespace_for_root(&dir).unwrap();

        let log = run_agent_turn(&fs, "hi there").unwrap();
        let text = String::from_utf8_lossy(&log);
        assert!(text.contains("you said: hi there"), "{text}");
        assert!(text.contains("turn.completed"), "{text}");

        std::fs::remove_dir_all(&dir).ok();
    }
}
