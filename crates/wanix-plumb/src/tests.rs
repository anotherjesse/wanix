use wanix_fs::{FileSystem, FileType, NormalizedPath, OpenOptions};

use crate::{MAX_KNOWN_TOPICS, PlumbDevice, PlumbEnvelope};

fn path(raw: &str) -> NormalizedPath {
    NormalizedPath::new(raw).expect("normalized path")
}

fn write_only() -> OpenOptions {
    OpenOptions {
        write: true,
        ..OpenOptions::default()
    }
}

#[test]
fn root_lists_topics_after_they_are_touched() {
    let device = PlumbDevice::local();
    // No topics touched yet: the root listing is empty.
    assert!(device.read_dir(&path(".")).unwrap().is_empty());
    // Opening recv on a topic makes it appear in the root listing.
    let _recv = device
        .open(&path("build/recv"), OpenOptions::read())
        .unwrap();
    let names: Vec<String> = device
        .read_dir(&path("."))
        .unwrap()
        .into_iter()
        .map(|entry| entry.name().to_owned())
        .collect();
    assert_eq!(names, vec!["build".to_owned()]);
}

#[test]
fn a_topic_directory_exposes_send_and_recv() {
    let device = PlumbDevice::local();
    let names: Vec<String> = device
        .read_dir(&path("build"))
        .unwrap()
        .into_iter()
        .map(|entry| entry.name().to_owned())
        .collect();
    assert_eq!(names, vec!["recv".to_owned(), "send".to_owned()]);
    assert_eq!(
        device.metadata(&path("build")).unwrap().file_type(),
        FileType::Directory
    );
}

#[test]
fn send_publishes_an_envelope_that_recv_observes() {
    let device = PlumbDevice::local();
    // Subscribe first, then publish: best-effort delivery requires the reader to
    // be listening before the message is sent.
    let mut recv = device
        .open(&path("build/recv"), OpenOptions::read())
        .unwrap();
    let mut send = device.open(&path("build/send"), write_only()).unwrap();
    let env = PlumbEnvelope {
        kind: "task.done".to_owned(),
        from: "agent-b".to_owned(),
        to: String::new(),
        body: serde_json::json!({ "out": "/world/result" }),
    };
    let line = env.to_line().unwrap();
    // The write unit is one envelope (the JSON without the trailing newline is
    // accepted; the device re-serializes with its own newline).
    let json = &line[..line.len() - 1];
    assert_eq!(send.write(json).unwrap(), json.len());

    let mut buf = [0_u8; 512];
    let n = recv.read(&mut buf).unwrap();
    let received = std::str::from_utf8(&buf[..n]).unwrap();
    assert!(received.ends_with('\n'));
    let parsed = PlumbEnvelope::parse(received.trim_end().as_bytes()).unwrap();
    assert_eq!(parsed, env);
}

#[test]
fn send_rejects_a_non_envelope_write() {
    let device = PlumbDevice::local();
    let mut send = device.open(&path("build/send"), write_only()).unwrap();
    // Not a JSON object: rejected, not silently published.
    assert!(send.write(b"not json").is_err());
}

#[test]
fn send_is_write_only_and_recv_is_read_only() {
    let device = PlumbDevice::local();
    // Opening send for reading is denied.
    assert!(
        device
            .open(&path("build/send"), OpenOptions::read())
            .is_err()
    );
    // Opening recv for writing is denied.
    assert!(device.open(&path("build/recv"), write_only()).is_err());
}

#[test]
fn two_subscribers_each_receive_a_broadcast() {
    let device = PlumbDevice::local();
    let mut a = device.open(&path("t/recv"), OpenOptions::read()).unwrap();
    let mut b = device.open(&path("t/recv"), OpenOptions::read()).unwrap();
    let mut send = device.open(&path("t/send"), write_only()).unwrap();
    send.write(br#"{"kind":"ping"}"#).unwrap();
    let mut buf = [0_u8; 128];
    let na = a.read(&mut buf).unwrap();
    assert!(std::str::from_utf8(&buf[..na]).unwrap().contains("ping"));
    let nb = b.read(&mut buf).unwrap();
    assert!(std::str::from_utf8(&buf[..nb]).unwrap().contains("ping"));
}

#[test]
fn the_root_listing_is_bounded_against_topic_name_flooding() {
    // A client churning through distinct topic names (the imported-9P walk path
    // a remote peer controls) must not grow the cosmetic root listing without
    // bound: it caps at MAX_KNOWN_TOPICS and refuses further first-seen names.
    let device = PlumbDevice::local();
    for i in 0..(MAX_KNOWN_TOPICS + 100) {
        // Touch a fresh topic via a send open; drop the handle immediately.
        let _send = device
            .open(&path(&format!("topic-{i}/send")), write_only())
            .unwrap();
    }
    let listed = device.read_dir(&path(".")).unwrap().len();
    assert!(
        listed <= MAX_KNOWN_TOPICS,
        "root listing grew to {listed}, above the {MAX_KNOWN_TOPICS} ceiling"
    );
}

#[test]
fn an_over_long_topic_name_is_rejected() {
    // A topic-name segment past the parse ceiling is refused at the device
    // boundary, so a remote importer cannot allocate unbounded topic strings.
    let device = PlumbDevice::local();
    let huge = "z".repeat(64 * 1024);
    assert!(
        device
            .open(&path(&format!("{huge}/recv")), OpenOptions::read())
            .is_err()
    );
    assert!(device.metadata(&path(&huge)).is_err());
}
