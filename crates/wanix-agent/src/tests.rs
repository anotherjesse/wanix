use std::sync::Arc;

use wanix_fs::{File, FileSystem, FsError, NormalizedPath, OpenOptions};

use super::{AgentDevice, CRATE_PURPOSE, FakeEngine};

fn np(path: &str) -> NormalizedPath {
    NormalizedPath::new(path).unwrap()
}

fn write_options() -> OpenOptions {
    OpenOptions {
        write: true,
        ..OpenOptions::default()
    }
}

fn device() -> AgentDevice {
    AgentDevice::new(Arc::new(FakeEngine))
}

fn read_all(file: &mut Box<dyn File>) -> String {
    let mut out = Vec::new();
    let mut buf = [0u8; 64];
    loop {
        let n = file.read(&mut buf).unwrap();
        if n == 0 {
            break;
        }
        out.extend_from_slice(&buf[..n]);
    }
    String::from_utf8(out).unwrap()
}

fn read_until(file: &mut Box<dyn File>, marker: &str) -> String {
    let mut out = Vec::new();
    let mut buf = [0u8; 128];
    loop {
        let n = file.read(&mut buf).unwrap();
        if n == 0 {
            break;
        }
        out.extend_from_slice(&buf[..n]);
        if String::from_utf8_lossy(&out).contains(marker) {
            break;
        }
    }
    String::from_utf8(out).unwrap()
}

fn alloc_session(device: &AgentDevice) -> String {
    read_all(&mut device.open(&np("new"), OpenOptions::read()).unwrap())
        .trim()
        .to_owned()
}

#[test]
fn purpose_is_declared() {
    assert!(!CRATE_PURPOSE.is_empty());
}

#[test]
fn new_allocates_a_session_directory() {
    let device = device();
    let id = alloc_session(&device);
    assert_eq!(id, "1");

    let entries: Vec<String> = device
        .read_dir(&np("1"))
        .unwrap()
        .iter()
        .map(|entry| entry.name().to_owned())
        .collect();
    assert_eq!(
        entries,
        [
            "ctl", "events", "id", "pending", "prompt", "reply", "status"
        ]
    );

    let mut id_file = device.open(&np("1/id"), OpenOptions::read()).unwrap();
    assert_eq!(read_all(&mut id_file), "1\n");
}

#[test]
fn prompt_submit_streams_a_normalized_reply() {
    let device = device();
    let id = alloc_session(&device);

    let mut prompt = device
        .open(&np(&format!("{id}/prompt")), write_options())
        .unwrap();
    prompt.write(b"hi there").unwrap();

    let mut events = device
        .open(&np(&format!("{id}/events")), OpenOptions::read())
        .unwrap();
    let stream = read_until(&mut events, "turn.completed");

    assert!(stream.contains("\"t\":\"turn.started\""), "{stream}");
    assert!(stream.contains("\"t\":\"message.delta\""), "{stream}");
    assert!(stream.contains("you said: hi there"), "{stream}");
    assert!(stream.contains("\"t\":\"turn.completed\""), "{stream}");
}

#[test]
fn reply_gives_a_single_read_eof_terminated_answer() {
    // The reply file is how one agent delegates to another: a single read that
    // blocks for the answer and ends, unlike the never-EOF events stream.
    let device = device();
    let id = alloc_session(&device);
    device
        .open(&np(&format!("{id}/prompt")), write_options())
        .unwrap()
        .write(b"do the thing")
        .unwrap();

    let mut reply = device
        .open(&np(&format!("{id}/reply")), OpenOptions::read())
        .unwrap();
    assert_eq!(read_all(&mut reply), "you said: do the thing\n");
}

#[test]
fn status_reports_turn_count() {
    let device = device();
    let id = alloc_session(&device);
    let mut prompt = device
        .open(&np(&format!("{id}/prompt")), write_options())
        .unwrap();
    prompt.write(b"one").unwrap();

    let mut status = device
        .open(&np(&format!("{id}/status")), OpenOptions::read())
        .unwrap();
    assert!(read_all(&mut status).contains("turns=1"));
}

#[test]
fn powerful_action_is_gated_by_ctl_approve() {
    let device = device();
    let id = alloc_session(&device);

    // A prompt that parks an approval instead of completing.
    let mut prompt = device
        .open(&np(&format!("{id}/prompt")), write_options())
        .unwrap();
    prompt.write(b"approve: delete everything").unwrap();

    // The stream pauses at approval.needed — no turn.completed yet.
    let mut events = device
        .open(&np(&format!("{id}/events")), OpenOptions::read())
        .unwrap();
    let paused = read_until(&mut events, "approval.needed");
    assert!(paused.contains("approval.needed"), "{paused}");
    assert!(!paused.contains("turn.completed"), "{paused}");

    // The request is visible as a file.
    let mut pending = device
        .open(&np(&format!("{id}/pending")), OpenOptions::read())
        .unwrap();
    let pending = read_all(&mut pending);
    assert!(pending.contains("delete everything"), "{pending}");
    assert!(pending.contains("req-1"), "{pending}");

    // Writing the approval to ctl unblocks the turn.
    let mut ctl = device
        .open(&np(&format!("{id}/ctl")), write_options())
        .unwrap();
    ctl.write(b"approve req-1\n").unwrap();

    let resumed = read_until(&mut events, "turn.completed");
    assert!(resumed.contains("approved: delete everything"), "{resumed}");
    assert!(resumed.contains("turn.completed"), "{resumed}");
}

#[test]
#[ignore = "requires a live, authenticated codex app-server (CODEX_HOME/~/.codex)"]
fn codex_live_agent_streams_reply() {
    use std::path::PathBuf;

    use super::CodexEngine;

    let engine = CodexEngine::new(PathBuf::from("codex"), None, "/tmp".to_owned());
    let device = AgentDevice::new(Arc::new(engine));
    let id = alloc_session(&device);

    let mut prompt = device
        .open(&np(&format!("{id}/prompt")), write_options())
        .unwrap();
    prompt.write(b"Reply with exactly: hello").unwrap();

    let mut events = device
        .open(&np(&format!("{id}/events")), OpenOptions::read())
        .unwrap();
    let stream = read_until(&mut events, "turn.completed");
    eprintln!("--- #agent/{id}/events ---\n{stream}");
    assert!(stream.to_lowercase().contains("hello"), "{stream}");
}

#[test]
fn ctl_close_removes_session_and_eofs_open_readers() {
    let device = device();
    let id = alloc_session(&device);
    let mut events = device
        .open(&np(&format!("{id}/events")), OpenOptions::read())
        .unwrap();

    let mut ctl = device
        .open(&np(&format!("{id}/ctl")), write_options())
        .unwrap();
    ctl.write(b"close\n").unwrap();

    // An already-open events reader observes end-of-stream.
    let mut buf = [0u8; 16];
    assert_eq!(events.read(&mut buf).unwrap(), 0);
    // The session is gone from the device.
    assert!(matches!(
        device.open(&np(&format!("{id}/events")), OpenOptions::read()),
        Err(FsError::NotFound)
    ));
}
