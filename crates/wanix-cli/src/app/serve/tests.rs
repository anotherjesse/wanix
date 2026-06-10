//! The chatroom proof (docs/appfs.md §Chatroom Proof, ADR 0007 §Worked
//! example): a qjs guest app served over loopback QUIC through the
//! wanix-appfs adapter, with attribution and presence bound to verified
//! dialer identities, live stream fan-out across connections, durable
//! history across a guest restart, and honest guest-death behavior.

use std::ffi::OsString;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::Value;
use wanix_fs::{FileSystem, FsError, NormalizedPath, OpenOptions};
use wanix_id::NodeIdentity;
use wanix_vfs::{BindOptions, Namespace};

use super::{bind_app_endpoint, parse_app_serve_command};
use crate::app::guest::start_app_guest;
use crate::app::load_app_manifest;
use crate::app::restart::{RestartPolicy, ServiceSlot, spawn_restart_supervisor};
use crate::mesh::resource::ServedEndpoint;

/// Every blocking wait in these tests is bounded by this deadline.
const TEST_DEADLINE: Duration = Duration::from_secs(60);

fn args(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}

fn loopback() -> SocketAddr {
    SocketAddr::new(Ipv4Addr::LOCALHOST.into(), 0)
}

fn np(path: &str) -> NormalizedPath {
    NormalizedPath::new(path).unwrap()
}

fn chatroom_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/chatroom")
}

fn temp_dir(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = PathBuf::from(format!(
        "target/wanix-cli-app-{label}-{}-{nanos}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Starts the bundled chatroom guest against `state_dir` and serves it on a
/// loopback endpoint with the given identity secret.
fn serve_chatroom(
    state_dir: &Path,
    secret: u8,
) -> (
    crate::app::guest::AppGuest,
    wanix_appfs::AppFsService,
    Vec<ServedEndpoint>,
) {
    let app_dir = chatroom_dir();
    let manifest = load_app_manifest(&app_dir).unwrap();
    let (guest, service) = start_app_guest(&app_dir, state_dir, &manifest).unwrap();
    let served = bind_app_endpoint(
        "chatroom-test".to_owned(),
        NodeIdentity::from_secret_bytes([secret; 32]),
        ServiceSlot::new(service.clone()),
        Some(loopback()),
    )
    .unwrap();
    (guest, service, served)
}

/// The wire principal a dialer identity is presented to apps as (v0.2):
/// scheme-prefixed verified peer id.
fn principal_of(identity: &NodeIdentity) -> String {
    format!("iroh:{}", identity.peer_id().to_hex())
}

/// Mounts a served app ticket at `x` presenting an explicit dialer identity
/// (the test-only identity injection: each identity is a distinct verified
/// principal against the served room).
fn mount_as(identity: &NodeIdentity, ticket_url: &str) -> (Arc<Namespace>, crate::mesh::IrohMount) {
    let mount = crate::mesh::dial_iroh_remote_as(identity, ticket_url, "").unwrap();
    let mut namespace = Namespace::new();
    namespace
        .bind(mount.remote.clone(), ".", "x", BindOptions::default())
        .unwrap();
    (Arc::new(namespace), mount)
}

/// Reads a whole mesh-mounted file. ADR 0008 leaves open-file replies after
/// the first deadline-free, so the wait is bounded here: a regression that
/// wedges the guest must fail the test, not hang CI.
fn read_string(namespace: &Arc<Namespace>, path: &str) -> String {
    let namespace = Arc::clone(namespace);
    let path = path.to_owned();
    within_deadline("read", move || {
        let mut file = namespace.open(&np(&path), OpenOptions::read()).unwrap();
        let mut out = Vec::new();
        let mut buf = [0u8; 512];
        loop {
            let n = file.read(&mut buf).unwrap();
            if n == 0 {
                break;
            }
            out.extend_from_slice(&buf[..n]);
        }
        String::from_utf8(out).unwrap()
    })
}

/// Posts one message over the mesh mount, bounded like [`read_string`].
fn post(namespace: &Arc<Namespace>, body: &[u8]) {
    let namespace = Arc::clone(namespace);
    let body = body.to_vec();
    within_deadline("post", move || {
        let mut file = namespace
            .open(
                &np("x/post"),
                OpenOptions {
                    write: true,
                    create: true,
                    truncate: true,
                    ..OpenOptions::default()
                },
            )
            .unwrap();
        assert_eq!(file.write(&body).unwrap(), body.len());
    });
}

fn message_lines(text: &str) -> Vec<Value> {
    text.lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

/// Runs a blocking operation on its own thread and bounds the wait.
fn within_deadline<T: Send + 'static>(
    label: &str,
    operation: impl FnOnce() -> T + Send + 'static,
) -> T {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let _ = sender.send(operation());
    });
    receiver
        .recv_timeout(TEST_DEADLINE)
        .unwrap_or_else(|_| panic!("{label} exceeded the {TEST_DEADLINE:?} test deadline"))
}

#[test]
fn parse_requires_app_state_and_endpoint_posture() {
    let parsed = parse_app_serve_command(&args(&[
        "--app",
        "examples/chatroom",
        "--state",
        "/tmp/room",
        "--addr",
        "127.0.0.1:0",
    ]))
    .unwrap();
    assert_eq!(parsed.app_dir, PathBuf::from("examples/chatroom"));
    assert_eq!(parsed.state_dir, PathBuf::from("/tmp/room"));
    assert_eq!(parsed.local_addr, "127.0.0.1:0".parse().ok());
    assert!(!parsed.insecure_open);

    // Missing --app / --state are usage errors.
    assert!(parse_app_serve_command(&args(&["--state", "s", "--addr", "127.0.0.1:0"])).is_err());
    assert!(parse_app_serve_command(&args(&["--app", "a", "--addr", "127.0.0.1:0"])).is_err());
    // Public endpoint requires the explicit opt-in (matches tool/volume serve).
    let error = parse_app_serve_command(&args(&["--app", "a", "--state", "s"])).unwrap_err();
    assert_eq!(error.exit_code(), 2);
    assert!(error.to_string().contains("public endpoint"), "{error}");
    assert!(
        parse_app_serve_command(&args(&["--app", "a", "--state", "s", "--insecure-open"])).is_ok()
    );
    // --restart accepts exactly on-failure; the default is Never.
    assert_eq!(parsed.restart, RestartPolicy::Never);
    let parsed = parse_app_serve_command(&args(&[
        "--app",
        "a",
        "--state",
        "s",
        "--addr",
        "127.0.0.1:0",
        "--restart",
        "on-failure",
    ]))
    .unwrap();
    assert_eq!(parsed.restart, RestartPolicy::OnFailure);
    assert!(
        parse_app_serve_command(&args(&[
            "--app",
            "a",
            "--state",
            "s",
            "--addr",
            "127.0.0.1:0",
            "--restart",
            "always",
        ]))
        .is_err()
    );
    // Unknown flags are refused.
    assert!(parse_app_serve_command(&args(&["--app", "a", "--state", "s", "--bogus"])).is_err());
}

/// The attribution proof: two dialer identities post to one served room, and
/// every message carries the transport-verified principal — including when a
/// client smuggles a fake `from` inside the body. `who` reflects exactly the
/// principals holding open stream subscriptions, and `status` is readable.
#[test]
fn two_peers_chat_with_verified_attribution_and_presence() {
    let state = temp_dir("chat");
    let (_guest, _service, served) = serve_chatroom(&state, 121);

    let id_a = NodeIdentity::from_secret_bytes([122u8; 32]);
    let id_b = NodeIdentity::from_secret_bytes([123u8; 32]);
    let principal_a = principal_of(&id_a);
    let principal_b = principal_of(&id_b);
    let (ns_a, _mount_a) = mount_as(&id_a, &served[0].ticket_url);
    let (ns_b, _mount_b) = mount_as(&id_b, &served[0].ticket_url);

    // A posts a body that CLAIMS to be from "mallory": the claimed author is
    // discarded, the verified principal is stamped.
    post(&ns_a, b"{\"from\":\"mallory\",\"body\":\"hello from A\"}");
    let latest = read_string(&ns_b, "x/latest");
    assert!(
        !latest.contains("mallory"),
        "client-claimed author leaked: {latest}"
    );
    let lines = message_lines(&latest);
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0]["from"], Value::String(principal_a.clone()));
    assert_eq!(lines[0]["body"], Value::String("hello from A".to_owned()));
    assert!(lines[0]["at"].is_u64(), "missing timestamp: {latest}");

    // B's plain-text post is seen by A, stamped with B's principal.
    post(&ns_b, b"hi from B");
    let lines = message_lines(&read_string(&ns_a, "x/latest"));
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[1]["from"], Value::String(principal_b.clone()));
    assert_eq!(lines[1]["body"], Value::String("hi from B".to_owned()));

    // who = the principals currently holding open stream subscriptions
    // (sorted, one per line) — a foreign principal is just another member.
    let stream_a = ns_a.open(&np("x/stream"), OpenOptions::read()).unwrap();
    let stream_b = ns_b.open(&np("x/stream"), OpenOptions::read()).unwrap();
    let mut expected: Vec<&str> = vec![&principal_a, &principal_b];
    expected.sort_unstable();
    assert_eq!(
        read_string(&ns_b, "x/who"),
        format!("{}\n{}\n", expected[0], expected[1])
    );
    drop(stream_a);
    drop(stream_b);

    let status: Value = serde_json::from_str(&read_string(&ns_a, "x/status")).unwrap();
    assert_eq!(status["app"], Value::String("chatroom".to_owned()));
    assert_eq!(status["messages"], Value::from(2));

    drop(served);
    std::fs::remove_dir_all(state).ok();
}

/// The liveness proof (ADR 0008 unbounded open-file replies): a stream read
/// parked on one connection receives a message posted on another connection.
#[test]
fn parked_stream_read_receives_a_post_from_another_connection() {
    let state = temp_dir("stream");
    let (_guest, _service, served) = serve_chatroom(&state, 124);

    let (ns_a, _mount_a) = mount_as(
        &NodeIdentity::from_secret_bytes([125u8; 32]),
        &served[0].ticket_url,
    );
    let id_b = NodeIdentity::from_secret_bytes([126u8; 32]);
    let principal_b = principal_of(&id_b);
    let (ns_b, _mount_b) = mount_as(&id_b, &served[0].ticket_url);

    // A parks a blocking read on the never-EOF stream file.
    let mut stream = ns_a.open(&np("x/stream"), OpenOptions::read()).unwrap();
    let (sender, receiver) = mpsc::channel();
    let reader = thread::spawn(move || {
        let mut buf = [0u8; 4096];
        let n = stream.read(&mut buf).unwrap();
        sender.send(buf[..n].to_vec()).unwrap();
    });

    // B posts on its own connection; the host fans the line into A's
    // subscription buffer and the parked read wakes with it.
    post(&ns_b, b"ping across the mesh");
    let bytes = receiver
        .recv_timeout(TEST_DEADLINE)
        .expect("parked stream read never received the posted message");
    let message: Value = serde_json::from_str(String::from_utf8(bytes).unwrap().trim()).unwrap();
    assert_eq!(message["from"], Value::String(principal_b));
    assert_eq!(
        message["body"],
        Value::String("ping across the mesh".to_owned())
    );
    reader.join().unwrap();

    drop(served);
    std::fs::remove_dir_all(state).ok();
}

/// The durability proof: history lives in `--state`, not guest memory. The
/// first guest is shut down (dropping the adapter closes its stdin: EOF),
/// and a fresh `app serve` against the same state dir still serves it.
#[test]
fn history_survives_guest_restart_against_the_same_state() {
    let state = temp_dir("restart");
    let (guest, service, served) = serve_chatroom(&state, 127);
    let (ns, mount) = mount_as(
        &NodeIdentity::from_secret_bytes([128u8; 32]),
        &served[0].ticket_url,
    );
    post(&ns, b"first");
    post(&ns, b"second");
    assert_eq!(message_lines(&read_string(&ns, "x/latest")).len(), 2);

    // "Kill" the guest the v0 way: drop every adapter handle (endpoint policy
    // + local service), which closes the guest's stdin pipe; the guest sees
    // EOF and exits. Bounded wait for the recorded exit.
    drop(ns);
    drop(mount);
    drop(served);
    drop(service);
    let task = guest.task.clone();
    within_deadline("first guest exit", move || task.wait_exit().unwrap());

    // A fresh serve against the same --state reloads the log at boot.
    let (_guest2, _service2, served2) = serve_chatroom(&state, 129);
    let (ns2, _mount2) = mount_as(
        &NodeIdentity::from_secret_bytes([130u8; 32]),
        &served2[0].ticket_url,
    );
    let lines = message_lines(&read_string(&ns2, "x/latest"));
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0]["body"], Value::String("first".to_owned()));
    assert_eq!(lines[1]["body"], Value::String("second".to_owned()));

    drop(served2);
    std::fs::remove_dir_all(state).ok();
}

/// The lifecycle-honesty proof: when the guest exits, a discrete op surfaces
/// `Unreachable` (the app is down, not missing) and parked stream readers
/// observe EOF instead of hanging forever. Uses a scratch one-shot app that
/// answers exactly one request and exits.
#[test]
fn guest_exit_unreaches_discrete_ops_and_closes_streams() {
    let app_dir = temp_dir("oneshot-app");
    std::fs::write(
        app_dir.join("app.wanix.json"),
        "{\"wanix.resource\":\"v0\",\"kind\":\"app\",\"name\":\"oneshot\",\
         \"runtime\":{\"kind\":\"qjs\",\"main\":\"main.js\"},\
         \"files\":[\"ping\"],\"streams\":[\"s\"]}",
    )
    .unwrap();
    std::fs::write(
        app_dir.join("main.js"),
        "import * as std from \"qjs:std\";\n\
         std.out.puts(JSON.stringify({ hello: { proto: 1 } }) + \"\\n\");\n\
         std.out.flush();\n\
         const line = std.in.getline();\n\
         if (line !== null) {\n\
           const req = JSON.parse(line);\n\
           std.out.puts(JSON.stringify({ id: req.id, ok: {} }) + \"\\n\");\n\
           std.out.flush();\n\
         }\n",
    )
    .unwrap();
    let state = temp_dir("oneshot-state");

    let manifest = load_app_manifest(&app_dir).unwrap();
    let (guest, service) = start_app_guest(&app_dir, &state, &manifest).unwrap();
    let view = service.open_view("local-test");

    // Subscribe to the stream while the guest is alive, then spend the
    // guest's single request on a successful stat.
    let mut stream = view.open(&np("s"), OpenOptions::read()).unwrap();
    view.metadata(&np("ping")).unwrap();

    // The guest exits after its one reply; the exit watcher then tears the
    // stream surface down, so the parked reader is released with EOF.
    let task = guest.task.clone();
    within_deadline("one-shot guest exit", move || task.wait_exit().unwrap());
    let eof = within_deadline("stream EOF after guest exit", move || {
        let mut buf = [0u8; 16];
        stream.read(&mut buf).unwrap()
    });
    assert_eq!(eof, 0, "stream readers must observe EOF, not hang");

    // Discrete ops now fail through the broken channel as Unreachable: the
    // endpoint still exists, the implementation behind it is down.
    let result = within_deadline("discrete op after guest exit", move || {
        view.metadata(&np("ping"))
    });
    match result {
        Err(FsError::Unreachable(message)) => {
            assert!(message.contains("app"), "{message}");
        }
        other => panic!("expected Unreachable after guest exit, got {other:?}"),
    }

    std::fs::remove_dir_all(app_dir).ok();
    std::fs::remove_dir_all(state).ok();
}

/// The bounded-pipe liveness proof: the adapter's pump drains guest stdout
/// even when no operation is in flight, so (a) a post-reply publish burst far
/// over the 64 KiB stdout pipe reaches a parked subscriber with no further
/// discrete op, and (b) a request line far over the 64 KiB stdin pipe cannot
/// deadlock against that undrained backlog (the adapter/guest mutual stall).
#[test]
fn large_publish_backlog_and_large_write_do_not_deadlock() {
    let app_dir = temp_dir("burst-app");
    std::fs::write(
        app_dir.join("app.wanix.json"),
        "{\"wanix.resource\":\"v0\",\"kind\":\"app\",\"name\":\"burst\",\
         \"runtime\":{\"kind\":\"qjs\",\"main\":\"main.js\"},\
         \"files\":[\"ping\",\"sink\"],\"streams\":[\"s\"]}",
    )
    .unwrap();
    // A conforming guest: replies first, then (for stat) publishes a 120 KiB
    // burst — protocol-legal post-reply bytes that park the single-threaded
    // guest in its stdout write until the host drains them.
    std::fs::write(
        app_dir.join("main.js"),
        "import * as std from \"qjs:std\";\n\
         std.out.puts(JSON.stringify({ hello: { proto: 1 } }) + \"\\n\");\n\
         std.out.flush();\n\
         const big = \"x\".repeat(120 * 1024);\n\
         let line;\n\
         while ((line = std.in.getline()) !== null) {\n\
           if (!line) continue;\n\
           const req = JSON.parse(line);\n\
           std.out.puts(JSON.stringify({ id: req.id, ok: {} }) + \"\\n\");\n\
           if (req.op === \"stat\") {\n\
             std.out.puts(JSON.stringify({ publish: { stream: \"s\", data: btoa(big) } }) + \"\\n\");\n\
           }\n\
           std.out.flush();\n\
         }\n",
    )
    .unwrap();
    let state = temp_dir("burst-state");
    let manifest = load_app_manifest(&app_dir).unwrap();
    let (_guest, service) = start_app_guest(&app_dir, &state, &manifest).unwrap();

    // Subscribe first so the burst lands in a live buffer.
    let mut stream = service
        .open_view("sub")
        .open(&np("s"), OpenOptions::read())
        .unwrap();

    let view = service.open_view("writer");
    within_deadline("stat triggering the post-reply burst", move || {
        view.metadata(&np("ping")).unwrap();
    });

    // (a) The burst is delivered without another discrete op on the channel.
    let collected = within_deadline("post-reply burst delivery", move || {
        let mut out = Vec::new();
        let mut buf = vec![0u8; 64 * 1024];
        while out.len() < 120 * 1024 {
            let n = stream.read(&mut buf).unwrap();
            assert_ne!(n, 0, "stream must not EOF while the guest lives");
            out.extend_from_slice(&buf[..n]);
        }
        out
    });
    assert_eq!(collected.len(), 120 * 1024);
    assert!(collected.iter().all(|&byte| byte == b'x'));

    // (b) A ~100 KiB single write — an encoded request line beyond the stdin
    // pipe capacity — completes against the same guest.
    let view = service.open_view("writer");
    within_deadline("large write against the drained guest", move || {
        let mut file = view
            .open(
                &np("sink"),
                OpenOptions {
                    write: true,
                    ..OpenOptions::default()
                },
            )
            .unwrap();
        let body = vec![b'y'; 100 * 1024];
        assert_eq!(file.write(&body).unwrap(), body.len());
    });

    std::fs::remove_dir_all(app_dir).ok();
    std::fs::remove_dir_all(state).ok();
}

/// Reads a mesh-mounted file without panicking on a dead generation.
fn try_read_to_string(namespace: &Namespace, path: &str) -> Result<String, wanix_fs::FsError> {
    let mut file = namespace.open(&np(path), OpenOptions::read())?;
    let mut out = Vec::new();
    let mut buf = [0u8; 256];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        out.extend_from_slice(&buf[..n]);
    }
    Ok(String::from_utf8_lossy(&out).into_owned())
}

/// The restart proof (`--restart on-failure`): the supervisor re-runs an
/// exited guest and swaps the fresh service through the [`ServiceSlot`], so
/// the same ticket keeps working and `/state` history survives — while the
/// connection bound to the exited generation keeps its honestly-dead view.
/// The guest is a counter app whose hello (not its manifest) declares the
/// tree, that bumps `/state/count` at boot, and that exits after serving one
/// read — a crash stand-in.
#[test]
fn restart_on_failure_swaps_a_fresh_service_behind_the_same_ticket() {
    let app_dir = temp_dir("restart-app");
    std::fs::write(
        app_dir.join("app.wanix.json"),
        "{\"wanix.resource\":\"v0\",\"kind\":\"app\",\"name\":\"counter\",\
         \"runtime\":{\"kind\":\"qjs\",\"main\":\"main.js\"}}",
    )
    .unwrap();
    std::fs::write(
        app_dir.join("main.js"),
        "import * as std from \"qjs:std\";\n\
         std.out.puts(JSON.stringify({ hello: { proto: 1, files: [\"count\"] } }) + \"\\n\");\n\
         std.out.flush();\n\
         const mine = parseInt(std.loadFile(\"/state/count\") || \"0\", 10) + 1;\n\
         const f = std.open(\"/state/count\", \"w\");\n\
         f.puts(String(mine));\n\
         f.close();\n\
         let line;\n\
         while ((line = std.in.getline()) !== null) {\n\
           if (!line) continue;\n\
           const req = JSON.parse(line);\n\
           let ok = {};\n\
           if (req.op === \"read\") {\n\
             const bytes = String(mine) + \"\\n\";\n\
             const offset = req.offset || 0;\n\
             ok = { data: btoa(bytes.slice(offset, offset + req.len)) };\n\
           }\n\
           std.out.puts(JSON.stringify({ id: req.id, ok }) + \"\\n\");\n\
           std.out.flush();\n\
           if (req.op === \"read\") break;\n\
         }\n",
    )
    .unwrap();
    let state = temp_dir("restart-state");

    let manifest = load_app_manifest(&app_dir).unwrap();
    let (guest, service) = start_app_guest(&app_dir, &state, &manifest).unwrap();
    let slot = ServiceSlot::new(service);
    let served = bind_app_endpoint(
        "restart-test".to_owned(),
        NodeIdentity::from_secret_bytes([140u8; 32]),
        slot.clone(),
        Some(loopback()),
    )
    .unwrap();
    spawn_restart_supervisor(guest, slot, app_dir.clone(), state.clone(), manifest);

    // Generation 1 serves one read (its hello, not the empty manifest,
    // declared `count`), then exits.
    let (ns, _mount) = mount_as(
        &NodeIdentity::from_secret_bytes([141u8; 32]),
        &served[0].ticket_url,
    );
    assert_eq!(read_string(&ns, "x/count"), "1\n");

    // The connection bound to generation 1 keeps its honestly-dead view:
    // reads fail (Unreachable through the wire), they never hang.
    within_deadline("dead generation-1 view", {
        let ns = Arc::clone(&ns);
        move || {
            while try_read_to_string(&ns, "x/count").is_ok() {
                thread::sleep(Duration::from_millis(200));
            }
        }
    });

    // A fresh connection on the SAME ticket reaches the restarted guest, and
    // /state survived: the boot counter moved past generation 1.
    let ticket = served[0].ticket_url.clone();
    let count = within_deadline("fresh connection after restart", move || {
        loop {
            // Mid-restart the slot is empty: the dial or the first use of
            // the mount is refused. Keep retrying until a generation is live.
            if let Ok(mount) = crate::mesh::dial_iroh_remote_as(
                &NodeIdentity::from_secret_bytes([142u8; 32]),
                &ticket,
                "",
            ) {
                let mut namespace = Namespace::new();
                if namespace
                    .bind(mount.remote.clone(), ".", "x", BindOptions::default())
                    .is_ok()
                    && let Ok(text) = try_read_to_string(&namespace, "x/count")
                {
                    return text;
                }
            }
            thread::sleep(Duration::from_millis(500));
        }
    });
    let count: u32 = count.trim().parse().unwrap();
    assert!(count >= 2, "state did not survive the restart: {count}");

    drop(served);
    std::fs::remove_dir_all(app_dir).ok();
    std::fs::remove_dir_all(state).ok();
}
