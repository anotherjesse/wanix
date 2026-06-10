//! The BinVerbs proof over a live loopback room (ADR 0007 §Confinement
//! contract): a served app ships executable verbs in `bin/` beside its tree;
//! a shell that mounts the room gains its vocabulary (`room:post`), and every
//! verb runs confined — its namespace is exactly the resource at `/res`, plus
//! stdio and argv/env.
//!
//! These tests use a scratch room app shipping the *wasm* verb fixtures
//! (`wanix_wasm::VERB_POST_WASM`/`VERB_PROBE_WASM`); the CLI `sh` path
//! registers both task drivers, and the `.js` half of the verb contract is
//! proven in `crate::sh::both_kinds_tests` (a `.js` post verb runs confined
//! with argv and piped stdin).

use std::path::{Path, PathBuf};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use wanix_fs::{FileSystem, FsError, NormalizedPath, OpenOptions};
use wanix_id::NodeIdentity;
use wanix_vfs::{BindOptions, Namespace};

use super::bind_app_endpoint;
use crate::app::guest::start_app_guest;
use crate::app::load_app_manifest;
use crate::app::restart::ServiceSlot;
use crate::mesh::resource::ServedEndpoint;
use crate::qjs_args::MeshMountSpec;
use crate::sh::ShCommand;

/// Every blocking wait in these tests is bounded by this deadline.
const TEST_DEADLINE: Duration = Duration::from_secs(60);

fn np(path: &str) -> NormalizedPath {
    NormalizedPath::new(path).unwrap()
}

fn temp_dir(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = PathBuf::from(format!(
        "target/wanix-cli-verb-{label}-{}-{nanos}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
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

/// Writes a scratch room app: `post` appends a message, `latest` reads them
/// back one per line, and `bin/` ships the wasm post/probe verbs.
fn write_room_app(app_dir: &Path) {
    std::fs::write(
        app_dir.join("app.wanix.json"),
        "{\"wanix.resource\":\"v0\",\"kind\":\"app\",\"name\":\"verbroom\",\
         \"runtime\":{\"kind\":\"qjs\",\"main\":\"main.js\"},\
         \"files\":[\"post\",\"latest\"]}",
    )
    .unwrap();
    std::fs::write(
        app_dir.join("main.js"),
        "import * as std from \"qjs:std\";\n\
         std.out.puts(JSON.stringify({ hello: { proto: 1 } }) + \"\\n\");\n\
         std.out.flush();\n\
         const textBytes = (t) => unescape(encodeURIComponent(t));\n\
         const messages = [];\n\
         const latest = () => messages.join(\"\\n\") + (messages.length ? \"\\n\" : \"\");\n\
         let line;\n\
         while ((line = std.in.getline()) !== null) {\n\
           if (!line) continue;\n\
           const req = JSON.parse(line);\n\
           let ok = {};\n\
           if (req.op === \"write\" && req.path === \"post\") {\n\
             messages.push(decodeURIComponent(escape(atob(req.data || \"\"))));\n\
           } else if (req.op === \"read\" && req.path === \"latest\") {\n\
             const bytes = textBytes(latest());\n\
             const offset = req.offset || 0;\n\
             const len = req.len === undefined ? bytes.length : req.len;\n\
             ok = { data: btoa(bytes.slice(offset, offset + len)) };\n\
           } else if (req.op === \"stat\") {\n\
             if (req.path === \"latest\") ok = { size: textBytes(latest()).length };\n\
           }\n\
           std.out.puts(JSON.stringify({ id: req.id, ok }) + \"\\n\");\n\
           std.out.flush();\n\
         }\n",
    )
    .unwrap();
    let bin = app_dir.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::write(bin.join("post.wasm"), wanix_wasm::VERB_POST_WASM).unwrap();
    std::fs::write(bin.join("probe.wasm"), wanix_wasm::VERB_PROBE_WASM).unwrap();
}

/// Serves the scratch room (guest + verb bin) on a loopback endpoint.
fn serve_room(
    app_dir: &Path,
    state_dir: &Path,
    secret: u8,
) -> (crate::app::guest::AppGuest, Vec<ServedEndpoint>) {
    let manifest = load_app_manifest(app_dir).unwrap();
    let (guest, service) = start_app_guest(app_dir, state_dir, &manifest).unwrap();
    let bin = crate::verb_bin::load_verb_bin(&app_dir.join("bin"), "app serve").unwrap();
    assert!(bin.is_some(), "the scratch room ships a bin/");
    let served = bind_app_endpoint(
        "verbroom-test".to_owned(),
        NodeIdentity::from_secret_bytes([secret; 32]),
        ServiceSlot::new(service),
        bin,
        Some(std::net::SocketAddr::new(
            std::net::Ipv4Addr::LOCALHOST.into(),
            0,
        )),
    )
    .unwrap();
    (guest, served)
}

/// Runs one `sh -c` line with the room mounted at `/n/room` and a secret file
/// in the shell's own cwd, returning (exit, stdout, stderr).
fn run_room_line(ticket: &str, line: &str) -> (i32, String, String) {
    let cwd = temp_dir("cwd");
    std::fs::write(cwd.join("secret.txt"), b"outside the resource").unwrap();
    let command = ShCommand {
        line: Some(line.to_owned()),
        env: Vec::new(),
        cwd: np(cwd.to_str().unwrap()),
        mesh_mounts: vec![MeshMountSpec {
            addr: ticket.to_owned(),
            guest_path: np("n/room"),
        }],
    };
    let output = within_deadline("sh line against the live room", move || {
        crate::sh::run_sh(command, &mut std::io::empty())
    })
    .unwrap();
    let result = (
        output.exit_code(),
        String::from_utf8_lossy(output.stdout()).into_owned(),
        String::from_utf8_lossy(output.stderr()).into_owned(),
    );
    std::fs::remove_dir_all(cwd).ok();
    result
}

/// The centerpiece: mounting the room means gaining its vocabulary. The verb
/// arrives from the resource over the mesh, runs confined, posts back into
/// the same resource, and composes in a pipeline; the confinement is proven
/// by the probe failing on a file the shell itself reads freely.
#[test]
fn mounted_room_verbs_post_compose_and_stay_confined() {
    let app_dir = temp_dir("room-app");
    write_room_app(&app_dir);
    let state = temp_dir("room-state");
    let (_guest, served) = serve_room(&app_dir, &state, 171);
    let ticket = served[0].ticket_url.clone();

    // Post via the verb (argv form), then read the room back over the mount.
    let (exit, out, err) = run_room_line(
        &ticket,
        "room:post hello mesh; echo post=$?; cat /n/room/latest",
    );
    assert_eq!(exit, 0, "stderr: {err}");
    assert!(
        out.contains("post=0"),
        "the verb exited 0: out={out:?} err={err:?}"
    );
    assert!(
        out.contains("hello mesh"),
        "the verb posted: out={out:?} err={err:?}"
    );

    // Pipeline composition (stdin form) and the confinement proof in one
    // session: the shell reads its own secret freely, the verb cannot.
    let (exit, out, err) = run_room_line(
        &ticket,
        "echo piped | room:post; cat secret.txt; room:probe /secret.txt; echo probe=$?; \
         cat /n/room/latest",
    );
    assert_eq!(exit, 0, "stderr: {err}");
    assert!(out.contains("outside the resource"), "shell view: {out:?}");
    assert!(out.contains("probe=1"), "the confined probe fails: {out:?}");
    assert!(err.contains("/secret.txt"), "honest probe error: {err:?}");
    assert!(out.contains("piped"), "the piped post landed: {out:?}");
    // And the verb CAN see its own resource.
    let (exit, out, _err) = run_room_line(&ticket, "room:probe /res/bin/post.wasm");
    assert_eq!(exit, 0);
    assert!(out.starts_with("ok "), "probe reads inside /res: {out:?}");

    drop(served);
    std::fs::remove_dir_all(app_dir).ok();
    std::fs::remove_dir_all(state).ok();
}

/// The served `bin/` surface itself: listed beside the app tree, read-only,
/// byte-identical, and size-capped with an honest error.
#[test]
fn served_bin_is_listed_read_only_and_size_capped() {
    let app_dir = temp_dir("bin-app");
    write_room_app(&app_dir);
    // An over-cap "verb" must be refused at open, not streamed.
    std::fs::write(
        app_dir.join("bin/big.wasm"),
        vec![0u8; (wanix_fs::MAX_VERB_FILE_BYTES + 1) as usize],
    )
    .unwrap();
    let state = temp_dir("bin-state");
    let (_guest, served) = serve_room(&app_dir, &state, 172);

    let mount = crate::mesh::dial_iroh_remote_as(
        &NodeIdentity::from_secret_bytes([173u8; 32]),
        &served[0].ticket_url,
        "",
    )
    .unwrap();
    let mut namespace = Namespace::new();
    namespace
        .bind(mount.remote.clone(), ".", "x", BindOptions::default())
        .unwrap();
    let namespace = Arc::new(namespace);

    within_deadline("bin surface over the wire", {
        let namespace = Arc::clone(&namespace);
        move || {
            // The resource's own listing shows bin/ beside its declared tree.
            let names: Vec<String> = namespace
                .read_dir(&np("x"))
                .unwrap()
                .into_iter()
                .map(|entry| entry.name().to_owned())
                .collect();
            assert!(names.contains(&"bin".to_owned()), "{names:?}");
            assert!(names.contains(&"post".to_owned()), "{names:?}");

            // Verb bytes are byte-identical and read-only.
            let mut file = namespace
                .open(&np("x/bin/post.wasm"), OpenOptions::read())
                .unwrap();
            let mut bytes = Vec::new();
            let mut buf = [0u8; 64 * 1024];
            loop {
                let n = file.read(&mut buf).unwrap();
                if n == 0 {
                    break;
                }
                bytes.extend_from_slice(&buf[..n]);
            }
            assert_eq!(bytes, wanix_wasm::VERB_POST_WASM);
            assert!(
                namespace
                    .open(&np("x/bin/post.wasm"), OpenOptions::read_write())
                    .is_err(),
                "verbs are read-only"
            );

            // The size cap travels as an honest error.
            match namespace.open(&np("x/bin/big.wasm"), OpenOptions::read()) {
                Err(FsError::Other(message)) => {
                    assert!(message.contains("verb size cap"), "{message}");
                }
                Err(other) => panic!("expected the size-cap error, got {other:?}"),
                Ok(_) => panic!("an over-cap verb must not open"),
            }
        }
    });

    drop(served);
    std::fs::remove_dir_all(app_dir).ok();
    std::fs::remove_dir_all(state).ok();
}
