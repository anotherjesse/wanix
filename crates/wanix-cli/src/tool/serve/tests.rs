use std::ffi::OsString;
use std::net::{Ipv4Addr, SocketAddr};

use serde_json::{Value, json};
use wanix_fs::{FileSystem, FsError, NormalizedPath, OpenOptions};
use wanix_id::NodeIdentity;
use wanix_vfs::{BindOptions, Namespace};

use super::{bind_tool_endpoints, parse_tool_serve_command};
use crate::mesh::resource::serve_record_line;

fn args(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}

fn loopback() -> SocketAddr {
    SocketAddr::new(Ipv4Addr::LOCALHOST.into(), 0)
}

fn np(path: &str) -> NormalizedPath {
    NormalizedPath::new(path).unwrap()
}

/// Mounts a served tool ticket through the production native client at `x`.
fn mount(ticket_url: &str) -> (Namespace, crate::mesh::IrohMount) {
    bind_at_x(crate::mesh::dial_iroh_remote(ticket_url, "").unwrap())
}

/// [`mount`] presenting an explicit dialer identity (a distinct principal).
fn mount_as(identity: &NodeIdentity, ticket_url: &str) -> (Namespace, crate::mesh::IrohMount) {
    bind_at_x(crate::mesh::dial_iroh_remote_as(identity, ticket_url, "").unwrap())
}

fn bind_at_x(mount: crate::mesh::IrohMount) -> (Namespace, crate::mesh::IrohMount) {
    let mut namespace = Namespace::new();
    namespace
        .bind(mount.remote.clone(), ".", "x", BindOptions::default())
        .unwrap();
    (namespace, mount)
}

fn read_string(namespace: &Namespace, path: &str) -> String {
    let mut file = namespace.open(&np(path), OpenOptions::read()).unwrap();
    let mut out = Vec::new();
    let mut buf = [0u8; 256];
    loop {
        let n = file.read(&mut buf).unwrap();
        if n == 0 {
            break;
        }
        out.extend_from_slice(&buf[..n]);
    }
    String::from_utf8(out).unwrap()
}

fn write_file(namespace: &Namespace, path: &str, bytes: &[u8]) {
    let mut file = namespace
        .open(
            &np(path),
            OpenOptions {
                write: true,
                create: true,
                truncate: true,
                ..OpenOptions::default()
            },
        )
        .unwrap();
    assert_eq!(file.write(bytes).unwrap(), bytes.len());
}

fn job_ids(namespace: &Namespace) -> Vec<String> {
    namespace
        .read_dir(&np("x/jobs"))
        .unwrap()
        .into_iter()
        .map(|entry| entry.name().to_owned())
        .collect()
}

/// Runs one complete job through a mounted tool: cat new, write in, ctl run,
/// read out + result.json. The ctl close step is the caller's, so tests can
/// inspect the job first.
fn run_job(namespace: &Namespace, input: &[u8]) -> (String, String, Value) {
    let id = read_string(namespace, "x/new").trim().to_owned();
    write_file(namespace, &format!("x/jobs/{id}/in"), input);
    write_file(namespace, &format!("x/jobs/{id}/ctl"), b"run\n");
    let out = read_string(namespace, &format!("x/jobs/{id}/out"));
    let result: Value =
        serde_json::from_str(&read_string(namespace, &format!("x/jobs/{id}/result.json"))).unwrap();
    (id, out, result)
}

#[test]
fn parse_accepts_tools_addr_and_insecure_open() {
    let parsed = parse_tool_serve_command(&args(&[
        "--tool",
        "upper",
        "--tool",
        "sha256",
        "--addr",
        "127.0.0.1:0",
    ]))
    .unwrap();
    assert_eq!(parsed.tools, vec!["upper".to_owned(), "sha256".to_owned()]);
    assert_eq!(parsed.local_addr, "127.0.0.1:0".parse().ok());
    assert!(!parsed.insecure_open);

    let public = parse_tool_serve_command(&args(&["--tool", "model", "--insecure-open"])).unwrap();
    assert!(public.insecure_open);
    assert_eq!(public.local_addr, None);
}

#[test]
fn parse_enforces_public_endpoint_posture() {
    // No --addr and no --insecure-open is a public endpoint: refused (matches
    // volume serve / mesh-serve), at parse time so every path refuses it.
    let error = parse_tool_serve_command(&args(&["--tool", "upper"])).unwrap_err();
    assert_eq!(error.exit_code(), 2);
    assert!(error.to_string().contains("public endpoint"), "{error}");
}

#[test]
fn parse_rejects_bad_combinations() {
    for bad in [
        vec![],                                     // nothing selected
        vec!["--tool", "upper", "--tool", "upper"], // duplicate
        vec!["--tool", "rm-rf"],                    // not a built-in
        vec![
            "--tool",
            "upper",
            "--tool",
            "model",
            "--addr",
            "127.0.0.1:8080",
        ], // fixed port, multi
        vec!["--bogus"],                            // unknown flag
    ] {
        assert!(
            parse_tool_serve_command(&args(&bad)).is_err(),
            "{bad:?} should be a usage error"
        );
    }
}

#[test]
fn parse_names_the_builtins_for_an_unknown_tool() {
    let error = parse_tool_serve_command(&args(&["--tool", "nope"])).unwrap_err();
    assert!(
        error.to_string().contains("model, sha256, upper"),
        "{error}"
    );
}

/// The ticket-output contract: each bound tool announces the same parse-stable
/// `NAME\tTICKET_URL\n` record the volume serve path prints (the format itself
/// is pinned in `crate::mesh::resource::tests`).
#[test]
fn announces_one_iroh_ticket_record_per_tool() {
    let tools = vec![(
        "upper".to_owned(),
        NodeIdentity::from_secret_bytes([101u8; 32]),
    )];
    let served = bind_tool_endpoints(tools, Some(loopback())).unwrap();
    assert_eq!(served.len(), 1);
    let line = serve_record_line(&served[0].name, &served[0].ticket_url);
    assert!(line.starts_with("upper\tiroh://"), "{line}");
    assert!(line.ends_with('\n'), "{line}");
    assert!(line.contains("?addr=127.0.0.1:"), "{line}");
}

/// The core slice proof: one process binds an independent endpoint per tool
/// with a distinct identity, and a complete job (cat new → write in → ctl run
/// → read out → read result.json → ctl close) runs through the production
/// native client for both `upper` and `sha256`.
#[test]
fn serves_tools_with_a_full_job_lifecycle_over_the_mesh() {
    let tools = vec![
        (
            "upper".to_owned(),
            NodeIdentity::from_secret_bytes([102u8; 32]),
        ),
        (
            "sha256".to_owned(),
            NodeIdentity::from_secret_bytes([103u8; 32]),
        ),
    ];
    let served = bind_tool_endpoints(tools, Some(loopback())).unwrap();
    assert_eq!(served.len(), 2);
    // Distinct peer ids: each tool is an independent mesh resource/ticket.
    assert_ne!(served[0].node.peer_id(), served[1].node.peer_id());

    // upper: each endpoint root IS the single tool (spec.json names it).
    let (upper_ns, _upper_mount) = mount(&served[0].ticket_url);
    let spec: Value = serde_json::from_str(&read_string(&upper_ns, "x/spec.json")).unwrap();
    assert_eq!(spec["name"], json!("upper"));

    let (id, out, result) = run_job(&upper_ns, b"hello");
    assert_eq!(out, "HELLO");
    assert_eq!(result["state"], json!("done"));
    assert_eq!(result["exitCode"], json!(0));
    write_file(&upper_ns, &format!("x/jobs/{id}/ctl"), b"close\n");
    assert_eq!(job_ids(&upper_ns), Vec::<String>::new());

    // sha256: the wanix-cli-local runner, end to end over the same wire.
    let (sha_ns, _sha_mount) = mount(&served[1].ticket_url);
    let (id, out, result) = run_job(&sha_ns, b"hello");
    assert_eq!(
        out,
        "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824\n"
    );
    assert_eq!(result["state"], json!("done"));
    assert_eq!(result["exitCode"], json!(0));
    write_file(&sha_ns, &format!("x/jobs/{id}/ctl"), b"close\n");
    assert_eq!(job_ids(&sha_ns), Vec::<String>::new());

    drop(served);
}

/// The privacy proof crossing the mesh: two dialer identities each get a view
/// bound to their own verified peer id, so neither sees the other's jobs and a
/// guessed foreign job id reads NotFound (never PermissionDenied).
#[test]
fn two_peers_see_disjoint_jobs() {
    let tools = vec![(
        "upper".to_owned(),
        NodeIdentity::from_secret_bytes([104u8; 32]),
    )];
    let served = bind_tool_endpoints(tools, Some(loopback())).unwrap();

    // Two distinct dialer identities are two distinct verified principals
    // against one served tool. (The production dial path presents ONE
    // persisted identity per user, so two invocations of one user are one
    // principal — see same_identity_redial_resumes_jobs below.)
    let (ns_a, _mount_a) = mount_as(
        &NodeIdentity::from_secret_bytes([105u8; 32]),
        &served[0].ticket_url,
    );
    let (ns_b, _mount_b) = mount_as(
        &NodeIdentity::from_secret_bytes([106u8; 32]),
        &served[0].ticket_url,
    );

    let (id_a, out, _result) = run_job(&ns_a, b"private");
    assert_eq!(out, "PRIVATE");

    // A still sees its job; B sees an empty job table.
    assert_eq!(job_ids(&ns_a), vec![id_a.clone()]);
    assert_eq!(job_ids(&ns_b), Vec::<String>::new());

    // Guessing the foreign job id is NotFound across the wire.
    match ns_b.open(&np(&format!("x/jobs/{id_a}/status")), OpenOptions::read()) {
        Err(FsError::NotFound) => {}
        Err(other) => panic!("expected NotFound for a foreign job id, got {other:?}"),
        Ok(_) => panic!("expected NotFound for a foreign job id, got an open file"),
    }

    drop(served);
}

/// The durable-principal proof (ADR 0009 "survives caller death"): one
/// persisted dialer identity re-dialing the same tool — as two successive CLI
/// invocations of one user do — resumes the SAME `jobs/` view, so a retained
/// job outlives the process that allocated it instead of being orphaned
/// behind a throwaway key until TTL expiry.
#[test]
fn same_identity_redial_resumes_jobs() {
    let tools = vec![(
        "upper".to_owned(),
        NodeIdentity::from_secret_bytes([107u8; 32]),
    )];
    let served = bind_tool_endpoints(tools, Some(loopback())).unwrap();
    let dialer = NodeIdentity::from_secret_bytes([108u8; 32]);

    let (ns_a, mount_a) = mount_as(&dialer, &served[0].ticket_url);
    let (id, out, _result) = run_job(&ns_a, b"durable");
    assert_eq!(out, "DURABLE");
    // First "invocation" exits: its mount (endpoint + runtime) is gone.
    drop(ns_a);
    drop(mount_a);

    // The next invocation presents the same persisted key and resumes.
    let (ns_b, _mount_b) = mount_as(&dialer, &served[0].ticket_url);
    assert_eq!(job_ids(&ns_b), vec![id.clone()]);
    assert_eq!(read_string(&ns_b, &format!("x/jobs/{id}/out")), "DURABLE");

    drop(served);
}
