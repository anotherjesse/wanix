//! ToolFS contract tests: the `docs/toolfs.md` validation matrix rows that
//! apply to the v0 core crate, exercised through the filesystem surface.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{Value, json};
use wanix_fs::{File, FileSystem, FsError, NormalizedPath, OpenOptions};

use crate::runners::{
    EchoRunner, FailRunner, FakeModelEngine, ManualRunner, ModelRunner, UpperRunner,
};
use crate::{ToolClock, ToolFs, ToolPrincipal, ToolRunner, ToolService, ToolSpec};

const T0: u64 = 1_710_000_000_000;

fn np(path: &str) -> NormalizedPath {
    NormalizedPath::new(path).unwrap()
}

fn write_options() -> OpenOptions {
    OpenOptions {
        write: true,
        create: true,
        truncate: true,
        ..OpenOptions::default()
    }
}

fn clock(time: &Arc<AtomicU64>) -> ToolClock {
    let time = Arc::clone(time);
    Box::new(move || time.load(Ordering::SeqCst))
}

fn service_with(spec: ToolSpec, runner: Box<dyn ToolRunner>) -> (ToolService, Arc<AtomicU64>) {
    let time = Arc::new(AtomicU64::new(T0));
    (ToolService::new(spec, runner, clock(&time)), time)
}

fn upper_view() -> (ToolFs, Arc<AtomicU64>) {
    let (service, time) = service_with(
        ToolSpec::v0("upper", "Uppercase UTF-8 text."),
        Box::new(UpperRunner),
    );
    (service.open_view(ToolPrincipal::local("alice")), time)
}

fn read_all(file: &mut Box<dyn File>) -> Vec<u8> {
    let mut out = Vec::new();
    let mut buf = [0u8; 64];
    loop {
        let n = file.read(&mut buf).unwrap();
        if n == 0 {
            break;
        }
        out.extend_from_slice(&buf[..n]);
    }
    out
}

fn read_string(fs: &ToolFs, path: &str) -> String {
    let mut file = fs.open(&np(path), OpenOptions::read()).unwrap();
    String::from_utf8(read_all(&mut file)).unwrap()
}

fn read_json(fs: &ToolFs, path: &str) -> Value {
    serde_json::from_str(&read_string(fs, path)).unwrap()
}

fn alloc_job(fs: &ToolFs) -> String {
    read_string(fs, "new").trim().to_owned()
}

fn write_file(fs: &ToolFs, path: &str, bytes: &[u8]) {
    let mut file = fs.open(&np(path), write_options()).unwrap();
    assert_eq!(file.write(bytes).unwrap(), bytes.len());
}

fn ctl(fs: &ToolFs, id: &str, verb: &str) -> Result<(), FsError> {
    let mut file = fs.open(&np(&format!("jobs/{id}/ctl")), write_options())?;
    file.write(format!("{verb}\n").as_bytes()).map(|_| ())
}

fn dir_names(fs: &ToolFs, path: &str) -> Vec<String> {
    fs.read_dir(&np(path))
        .unwrap()
        .into_iter()
        .map(|entry| entry.name().to_owned())
        .collect()
}

#[test]
fn contract_shape_spec_health_and_schema() {
    let time = Arc::new(AtomicU64::new(T0));
    let service = ToolService::with_schema(
        ToolSpec::v0("upper", "Uppercase UTF-8 text."),
        Some(json!({ "type": "object" })),
        Box::new(UpperRunner),
        clock(&time),
    );
    let fs = service.open_view(ToolPrincipal::local("alice"));

    assert_eq!(
        dir_names(&fs, "."),
        [
            "spec.json",
            "params.schema.json",
            "health",
            "usage",
            "new",
            "jobs"
        ]
    );
    assert_eq!(dir_names(&fs, "jobs"), Vec::<String>::new());

    let spec = read_json(&fs, "spec.json");
    assert_eq!(spec["wanix.resource"], json!("v0"));
    assert_eq!(spec["kind"], json!("tool"));
    assert_eq!(spec["name"], json!("upper"));
    assert_eq!(spec["input"]["mode"], json!("bytes"));
    assert_eq!(spec["input"]["maxBytes"], json!(1_048_576));
    assert_eq!(spec["params"]["schemaPath"], json!("params.schema.json"));
    assert_eq!(spec["outputs"]["primary"]["path"], json!("out"));
    assert_eq!(spec["limits"]["runTimeoutMs"], json!(5_000));
    assert_eq!(spec["limits"]["maxJobsPerPrincipal"], json!(32));
    assert_eq!(spec["lifecycle"]["allocatedTtlMs"], json!(300_000));
    assert_eq!(spec["visibility"], json!("private"));
    assert_eq!(spec["sideEffects"], json!("none"));
    assert_eq!(spec["retryable"], json!(true));

    assert_eq!(
        read_json(&fs, "params.schema.json"),
        json!({ "type": "object" })
    );
    assert_eq!(read_json(&fs, "health"), json!({ "ok": true }));

    let id = alloc_job(&fs);
    assert_eq!(
        dir_names(&fs, &format!("jobs/{id}")),
        [
            "in",
            "params.json",
            "ctl",
            "out",
            "err",
            "status",
            "result.json"
        ]
    );
}

#[test]
fn schema_file_absent_when_not_configured() {
    let (fs, _) = upper_view();
    assert!(matches!(
        fs.open(&np("params.schema.json"), OpenOptions::read()),
        Err(FsError::NotFound)
    ));
    assert!(!dir_names(&fs, ".").contains(&"params.schema.json".to_owned()));
}

#[test]
fn upper_success_end_to_end_with_pinned_shapes() {
    let (fs, time) = upper_view();
    let id = alloc_job(&fs);

    assert_eq!(
        read_json(&fs, &format!("jobs/{id}/status")),
        json!({
            "state": "allocated",
            "createdAt": T0,
            "startedAt": null,
            "finishedAt": null,
            "expiresAt": null,
            "inputBytes": 0,
            "outputBytes": 0
        })
    );

    write_file(&fs, &format!("jobs/{id}/in"), b"hello\n");
    time.fetch_add(5, Ordering::SeqCst);
    ctl(&fs, &id, "run").unwrap();

    assert_eq!(read_string(&fs, &format!("jobs/{id}/out")), "HELLO\n");
    assert_eq!(read_string(&fs, &format!("jobs/{id}/err")), "");
    assert_eq!(
        read_json(&fs, &format!("jobs/{id}/status")),
        json!({
            "state": "done",
            "createdAt": T0,
            "startedAt": T0 + 5,
            "finishedAt": T0 + 5,
            "expiresAt": T0 + 5 + 600_000,
            "inputBytes": 6,
            "outputBytes": 6
        })
    );
    assert_eq!(
        read_json(&fs, &format!("jobs/{id}/result.json")),
        json!({
            "state": "done",
            "exitCode": 0,
            "durationMs": 0,
            "inputBytes": 6,
            "outputBytes": 6,
            "error": null,
            "retryable": true
        })
    );
    assert_eq!(read_json(&fs, "usage"), json!({ "jobs": 1, "bytes": 12 }));
}

#[test]
fn invalid_params_rejected_at_write_time() {
    let (fs, _) = upper_view();
    let id = alloc_job(&fs);
    let mut file = fs
        .open(&np(&format!("jobs/{id}/params.json")), write_options())
        .unwrap();
    assert!(matches!(file.write(b"not json"), Err(FsError::Other(_))));
}

#[test]
fn incomplete_params_fail_the_run_before_the_runner() {
    let (fs, _) = upper_view();
    let id = alloc_job(&fs);
    write_file(&fs, &format!("jobs/{id}/params.json"), b"{\"mode\":");
    write_file(&fs, &format!("jobs/{id}/in"), b"hi");
    ctl(&fs, &id, "run").unwrap();

    let result = read_json(&fs, &format!("jobs/{id}/result.json"));
    assert_eq!(result["state"], json!("failed"));
    assert_eq!(result["error"]["kind"], json!("invalid_params"));
    assert_eq!(result["retryable"], json!(false));
    // The runner never ran: no output was produced.
    assert_eq!(read_string(&fs, &format!("jobs/{id}/out")), "");
}

#[test]
fn valid_params_commit_across_partial_writes() {
    let (fs, _) = upper_view();
    let id = alloc_job(&fs);
    {
        let mut file = fs
            .open(&np(&format!("jobs/{id}/params.json")), write_options())
            .unwrap();
        file.write(b"{\"mode\":").unwrap();
        file.write(b"\"ascii\"}").unwrap();
    }
    assert_eq!(
        read_json(&fs, &format!("jobs/{id}/params.json")),
        json!({ "mode": "ascii" })
    );
    write_file(&fs, &format!("jobs/{id}/in"), b"ok");
    ctl(&fs, &id, "run").unwrap();
    assert_eq!(read_string(&fs, &format!("jobs/{id}/out")), "OK");
}

#[test]
fn failure_result_shape_via_fail_runner() {
    let (service, _) = service_with(ToolSpec::v0("fail", "Always fails."), Box::new(FailRunner));
    let fs = service.open_view(ToolPrincipal::local("alice"));
    let id = alloc_job(&fs);
    write_file(&fs, &format!("jobs/{id}/in"), b"x");
    ctl(&fs, &id, "run").unwrap();

    assert_eq!(
        read_json(&fs, &format!("jobs/{id}/result.json")),
        json!({
            "state": "failed",
            "exitCode": 2,
            "durationMs": 0,
            "inputBytes": 1,
            "outputBytes": 0,
            "error": { "kind": "runner_failed", "message": "fail runner always fails" },
            "retryable": false
        })
    );
    assert_eq!(
        read_string(&fs, &format!("jobs/{id}/err")),
        "deliberate failure\n"
    );
}

#[test]
fn invalid_input_taxonomy_via_upper_runner() {
    let (fs, _) = upper_view();
    let id = alloc_job(&fs);
    write_file(&fs, &format!("jobs/{id}/in"), &[0xff, 0xfe]);
    ctl(&fs, &id, "run").unwrap();
    let result = read_json(&fs, &format!("jobs/{id}/result.json"));
    assert_eq!(result["error"]["kind"], json!("invalid_input"));
    assert_eq!(result["exitCode"], json!(1));
}

#[test]
fn abort_interrupts_a_running_job() {
    let (runner, handle) = ManualRunner::new();
    let (service, _) = service_with(ToolSpec::v0("slow", "Hand-driven."), Box::new(runner));
    let fs = service.open_view(ToolPrincipal::local("alice"));
    let id = alloc_job(&fs);
    write_file(&fs, &format!("jobs/{id}/in"), b"x");

    let run_fs = fs.clone();
    let run_id = id.clone();
    let runner_thread = std::thread::spawn(move || ctl(&run_fs, &run_id, "run"));
    handle.wait_running();

    // Running: status is readable, out/result are not finished yet.
    let status = read_json(&fs, &format!("jobs/{id}/status"));
    assert_eq!(status["state"], json!("running"));
    match fs.open(&np(&format!("jobs/{id}/out")), OpenOptions::read()) {
        Err(FsError::Other(message)) => assert!(message.contains("not finished")),
        Err(other) => panic!("expected not-finished error, got {other:?}"),
        Ok(_) => panic!("expected not-finished error, got an open file"),
    }

    ctl(&fs, &id, "abort").unwrap();
    runner_thread.join().unwrap().unwrap();

    let result = read_json(&fs, &format!("jobs/{id}/result.json"));
    assert_eq!(result["state"], json!("aborted"));
    assert_eq!(result["error"]["kind"], json!("aborted"));
    assert_eq!(result["retryable"], json!(false));
}

#[test]
fn abort_before_run_is_terminal_and_run_after_is_a_no_op() {
    let (fs, _) = upper_view();
    let id = alloc_job(&fs);
    write_file(&fs, &format!("jobs/{id}/in"), b"hello");
    ctl(&fs, &id, "abort").unwrap();

    let result = read_json(&fs, &format!("jobs/{id}/result.json"));
    assert_eq!(result["state"], json!("aborted"));
    assert_eq!(result["exitCode"], json!(null));
    assert_eq!(result["durationMs"], json!(null));

    // Idempotent accept: run/abort on a terminal job change nothing.
    ctl(&fs, &id, "run").unwrap();
    ctl(&fs, &id, "abort").unwrap();
    assert_eq!(read_json(&fs, &format!("jobs/{id}/result.json")), result);
}

#[test]
fn privacy_two_views_see_separate_jobs() {
    let (service, _) = service_with(ToolSpec::v0("upper", "Upper."), Box::new(UpperRunner));
    let alice = service.open_view(ToolPrincipal::local("alice"));
    let bob = service.open_view(ToolPrincipal::node("b0b"));

    let alice_job = alloc_job(&alice);
    let bob_job = alloc_job(&bob);
    assert_eq!(dir_names(&alice, "jobs"), std::slice::from_ref(&alice_job));
    assert_eq!(dir_names(&bob, "jobs"), [bob_job]);

    // Guessing a foreign job id is NotFound, never PermissionDenied.
    assert!(matches!(
        bob.open(
            &np(&format!("jobs/{alice_job}/status")),
            OpenOptions::read()
        ),
        Err(FsError::NotFound)
    ));
    assert!(matches!(
        bob.metadata(&np(&format!("jobs/{alice_job}"))),
        Err(FsError::NotFound)
    ));
}

#[test]
fn allocated_jobs_expire_after_the_ttl() {
    let (fs, time) = upper_view();
    let id = alloc_job(&fs);
    time.fetch_add(300_001, Ordering::SeqCst);
    assert_eq!(dir_names(&fs, "jobs"), Vec::<String>::new());
    assert!(matches!(
        fs.open(&np(&format!("jobs/{id}/status")), OpenOptions::read()),
        Err(FsError::NotFound)
    ));
}

#[test]
fn done_jobs_retain_then_expire_and_close_deletes() {
    let (fs, time) = upper_view();
    let id = alloc_job(&fs);
    write_file(&fs, &format!("jobs/{id}/in"), b"hi");
    ctl(&fs, &id, "run").unwrap();
    time.fetch_add(599_999, Ordering::SeqCst);
    assert_eq!(dir_names(&fs, "jobs"), std::slice::from_ref(&id));
    time.fetch_add(2, Ordering::SeqCst);
    assert_eq!(dir_names(&fs, "jobs"), Vec::<String>::new());

    // close deletes a retained terminal job immediately...
    let second = alloc_job(&fs);
    write_file(&fs, &format!("jobs/{second}/in"), b"hi");
    ctl(&fs, &second, "run").unwrap();
    ctl(&fs, &second, "close").unwrap();
    assert_eq!(dir_names(&fs, "jobs"), Vec::<String>::new());

    // ...and refuses a job that is not finished.
    let third = alloc_job(&fs);
    match ctl(&fs, &third, "close") {
        Err(FsError::Other(message)) => assert!(message.contains("not finished")),
        other => panic!("expected not-finished error, got {other:?}"),
    }
}

#[test]
fn job_count_quota_rejects_allocation() {
    let mut spec = ToolSpec::v0("upper", "Upper.");
    spec.limits.max_jobs_per_principal = 1;
    let (service, _) = service_with(spec, Box::new(UpperRunner));
    let fs = service.open_view(ToolPrincipal::local("alice"));
    let _first = alloc_job(&fs);

    let mut file = fs.open(&np("new"), OpenOptions::read()).unwrap();
    let mut buf = [0u8; 8];
    match file.read(&mut buf) {
        Err(FsError::Other(message)) => assert!(message.contains("quota_exceeded")),
        other => panic!("expected quota error, got {other:?}"),
    }
}

#[test]
fn oversize_input_fails_with_input_too_large() {
    let mut spec = ToolSpec::v0("upper", "Upper.");
    spec.input.max_bytes = 4;
    let (service, _) = service_with(spec, Box::new(UpperRunner));
    let fs = service.open_view(ToolPrincipal::local("alice"));
    let id = alloc_job(&fs);
    write_file(&fs, &format!("jobs/{id}/in"), b"hello");
    ctl(&fs, &id, "run").unwrap();
    let result = read_json(&fs, &format!("jobs/{id}/result.json"));
    assert_eq!(result["error"]["kind"], json!("input_too_large"));
    assert_eq!(result["retryable"], json!(false));
}

#[test]
fn concurrency_quota_fails_the_second_run() {
    let (runner, handle) = ManualRunner::new();
    let mut spec = ToolSpec::v0("slow", "Hand-driven.");
    spec.limits.max_concurrent_per_principal = 1;
    let (service, _) = service_with(spec, Box::new(runner));
    let fs = service.open_view(ToolPrincipal::local("alice"));

    let first = alloc_job(&fs);
    write_file(&fs, &format!("jobs/{first}/in"), b"x");
    let run_fs = fs.clone();
    let run_id = first.clone();
    let runner_thread = std::thread::spawn(move || ctl(&run_fs, &run_id, "run"));
    handle.wait_running();

    let second = alloc_job(&fs);
    ctl(&fs, &second, "run").unwrap();
    let result = read_json(&fs, &format!("jobs/{second}/result.json"));
    assert_eq!(result["error"]["kind"], json!("quota_exceeded"));
    assert_eq!(result["retryable"], json!(true));

    handle.finish(crate::RunOutcome::success(b"done".to_vec()));
    runner_thread.join().unwrap().unwrap();
    assert_eq!(read_string(&fs, &format!("jobs/{first}/out")), "done");
}

#[test]
fn run_is_idempotent_and_input_is_sealed_after_run() {
    let (service, _) = service_with(ToolSpec::v0("echo", "Echo."), Box::new(EchoRunner));
    let fs = service.open_view(ToolPrincipal::local("alice"));
    let id = alloc_job(&fs);
    write_file(&fs, &format!("jobs/{id}/in"), b"once");
    ctl(&fs, &id, "run").unwrap();
    let result = read_json(&fs, &format!("jobs/{id}/result.json"));

    ctl(&fs, &id, "run").unwrap();
    assert_eq!(read_json(&fs, &format!("jobs/{id}/result.json")), result);
    assert_eq!(read_string(&fs, &format!("jobs/{id}/out")), "once");

    let mut file = fs
        .open(
            &np(&format!("jobs/{id}/in")),
            OpenOptions {
                write: true,
                ..OpenOptions::default()
            },
        )
        .unwrap();
    match file.write(b"more") {
        Err(FsError::Other(message)) => assert!(message.contains("sealed")),
        other => panic!("expected sealed error, got {other:?}"),
    }
}

#[test]
fn truncate_resets_unsealed_input() {
    let (service, _) = service_with(ToolSpec::v0("echo", "Echo."), Box::new(EchoRunner));
    let fs = service.open_view(ToolPrincipal::local("alice"));
    let id = alloc_job(&fs);
    write_file(&fs, &format!("jobs/{id}/in"), b"first");
    write_file(&fs, &format!("jobs/{id}/in"), b"second");
    ctl(&fs, &id, "run").unwrap();
    assert_eq!(read_string(&fs, &format!("jobs/{id}/out")), "second");
}

#[test]
fn unknown_ctl_verb_is_an_error() {
    let (fs, _) = upper_view();
    let id = alloc_job(&fs);
    assert!(matches!(ctl(&fs, &id, "explode"), Err(FsError::Other(_))));
}

#[test]
fn model_service_enforces_byte_budget_as_mount_quota() {
    let mut spec = ToolSpec::v0("model", "Prompt completion.");
    spec.limits.max_bytes_per_principal = 40;
    let (service, _) = service_with(spec, Box::new(ModelRunner::new(Arc::new(FakeModelEngine))));
    let fs = service.open_view(ToolPrincipal::local("agent"));

    let first = alloc_job(&fs);
    write_file(&fs, &format!("jobs/{first}/in"), b"hi");
    ctl(&fs, &first, "run").unwrap();
    assert_eq!(
        read_string(&fs, &format!("jobs/{first}/out")),
        "fake-completion: hi\n"
    );

    // Stored bytes so far: 2 in + 20 out = 22; 20 more breach the 40 cap.
    let second = alloc_job(&fs);
    write_file(&fs, &format!("jobs/{second}/in"), &[b'p'; 20]);
    ctl(&fs, &second, "run").unwrap();
    let result = read_json(&fs, &format!("jobs/{second}/result.json"));
    assert_eq!(result["state"], json!("failed"));
    assert_eq!(result["error"]["kind"], json!("quota_exceeded"));
    assert_eq!(result["retryable"], json!(true));
}

#[test]
fn job_ids_are_opaque_not_sequential() {
    let (fs, _) = upper_view();
    let first = alloc_job(&fs);
    let second = alloc_job(&fs);
    assert_ne!(first, second);
    for id in [&first, &second] {
        assert!(id.starts_with('j') && id.len() == 17, "opaque id: {id}");
    }
}

#[test]
fn crate_purpose_is_stable() {
    assert_eq!(
        crate::CRATE_PURPOSE,
        "wanix tool device filesystem (job protocol)"
    );
}
