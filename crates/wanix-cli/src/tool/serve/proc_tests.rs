//! End-to-end proofs for `tool serve --config`: real host programs behind
//! ToolFS, exercised through the production native mesh client over loopback.
//! Each test that needs a host binary skips honestly when it is missing
//! (`/usr/bin/tr`, `/bin/sleep`, `/usr/bin/yes`, `/bin/cp` are POSIX-ubiquitous
//! on CI hosts); the runner-level proofs live in `../process_tests.rs`.

use std::ffi::OsString;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::Path;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use wanix_id::NodeIdentity;

use super::tests::{job_ids, mount, read_string, run_job, write_file};
use super::{bind_tool_endpoints, parse_tool_serve_command};
use crate::mesh::resource::ServedEndpoint;

fn args(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}

fn have(command: &str) -> bool {
    let present = Path::new(command).exists();
    if !present {
        eprintln!("skipping: {command} not present on this host");
    }
    present
}

/// Writes `text` as a tools.toml, parses the full `tool serve` command line
/// for it, and binds every selected tool on loopback with test identities.
fn serve_config(text: &str, key: u8) -> (Vec<ServedEndpoint>, tempdir::Guard) {
    let dir = tempdir::create(key);
    let path = dir.path.join("tools.toml");
    std::fs::write(&path, text).unwrap();
    let command = parse_tool_serve_command(&args(&[
        "--config",
        path.to_str().unwrap(),
        "--listen",
        "127.0.0.1:0",
    ]))
    .unwrap();
    let tools = command
        .tools
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let mut secret = [key; 32];
            secret[0] = secret[0].wrapping_add(i as u8 + 1);
            (name.clone(), NodeIdentity::from_secret_bytes(secret))
        })
        .collect();
    let served = bind_tool_endpoints(
        tools,
        command.config.as_ref(),
        Some(SocketAddr::new(Ipv4Addr::LOCALHOST.into(), 0)),
    )
    .unwrap();
    (served, dir)
}

/// Minimal owned temp dir (std-only; no tempfile dep in this crate).
mod tempdir {
    use std::path::PathBuf;

    pub(super) struct Guard {
        pub(super) path: PathBuf,
    }

    impl Drop for Guard {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    pub(super) fn create(key: u8) -> Guard {
        let path = std::env::temp_dir().join(format!(
            "wanix-tool-serve-test-{}-{key}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).unwrap();
        Guard { path }
    }
}

fn result_json(namespace: &wanix_vfs::Namespace, id: &str) -> Value {
    serde_json::from_str(&read_string(namespace, &format!("x/jobs/{id}/result.json"))).unwrap()
}

/// The docs-sketch upper tool, running the real `/usr/bin/tr` over the mesh:
/// `--config` defines the tool (shadowing the built-in `upper`), `spec.json`
/// honestly reflects the configured description and limits, and a complete
/// job round-trips through the subprocess.
#[test]
fn config_upper_runs_a_real_process_over_the_mesh() {
    if !have("/usr/bin/tr") {
        return;
    }
    let (served, _dir) = serve_config(
        r#"
[tools.upper]
description = "Uppercase UTF-8 text (host /usr/bin/tr)"
command = "/usr/bin/tr"
args = ["[:lower:]", "[:upper:]"]
input = "stdin"
output = "stdout"

[tools.upper.limits]
max_input_bytes = 1048576
run_timeout_ms = 5000
max_concurrent_per_principal = 2
"#,
        120,
    );
    assert_eq!(served.len(), 1);
    let (namespace, _mount) = mount(&served[0].ticket_url);

    let spec: Value = serde_json::from_str(&read_string(&namespace, "x/spec.json")).unwrap();
    assert_eq!(spec["name"], json!("upper"));
    assert_eq!(
        spec["description"],
        json!("Uppercase UTF-8 text (host /usr/bin/tr)")
    );
    assert_eq!(spec["limits"]["runTimeoutMs"], json!(5000));
    assert_eq!(spec["limits"]["maxConcurrentPerPrincipal"], json!(2));
    assert_eq!(spec["input"]["maxBytes"], json!(1048576));

    let (id, out, result) = run_job(&namespace, b"hello mesh process");
    assert_eq!(out, "HELLO MESH PROCESS");
    assert_eq!(result["state"], json!("done"));
    assert_eq!(result["exitCode"], json!(0));
    write_file(&namespace, &format!("x/jobs/{id}/ctl"), b"close\n");
    assert_eq!(job_ids(&namespace), Vec::<String>::new());
    drop(served);
}

/// The timeout proof: a child that would run for 60s is killed at the 400ms
/// configured deadline and the job records the taxonomy `timeout`.
#[test]
fn timeout_kills_a_long_running_child() {
    if !have("/bin/sleep") {
        return;
    }
    let (served, _dir) = serve_config(
        r#"
[tools.zzz]
description = "sleeps far past its budget"
command = "/bin/sleep"
args = ["60"]

[tools.zzz.limits]
run_timeout_ms = 400
"#,
        130,
    );
    let (namespace, _mount) = mount(&served[0].ticket_url);
    let started = Instant::now();
    let (_id, out, result) = run_job(&namespace, b"");
    assert!(
        started.elapsed() < Duration::from_secs(30),
        "child must be killed at the deadline, not waited out"
    );
    assert_eq!(out, "");
    assert_eq!(result["state"], json!("failed"));
    assert_eq!(result["error"]["kind"], json!("timeout"));
    drop(served);
}

/// The abort proof: `ctl abort` from a second handle kills the child mid-run
/// (no time budget at all — only the abort ends it) and records `aborted`.
#[test]
fn ctl_abort_kills_the_child_mid_run() {
    if !have("/bin/sleep") {
        return;
    }
    let (served, _dir) = serve_config(
        r#"
[tools.zzz]
description = "sleeps until aborted"
command = "/bin/sleep"
args = ["60"]

[tools.zzz.limits]
run_timeout_ms = 0
"#,
        140,
    );
    let (namespace, _mount) = mount(&served[0].ticket_url);
    let id = read_string(&namespace, "x/new").trim().to_owned();
    write_file(&namespace, &format!("x/jobs/{id}/in"), b"");

    let started = Instant::now();
    std::thread::scope(|scope| {
        // `ctl run` blocks its writer for the whole run; drive it from a
        // scoped thread and abort from the main thread over the same mount.
        let runner = scope.spawn(|| {
            write_file(&namespace, &format!("x/jobs/{id}/ctl"), b"run\n");
        });
        let status_path = format!("x/jobs/{id}/status");
        while !read_string(&namespace, &status_path).contains("\"running\"") {
            assert!(
                started.elapsed() < Duration::from_secs(30),
                "job never reached running"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        write_file(&namespace, &format!("x/jobs/{id}/ctl"), b"abort\n");
        runner.join().unwrap();
    });
    assert!(
        started.elapsed() < Duration::from_secs(30),
        "abort must kill the child, not wait out sleep 60"
    );
    let result = result_json(&namespace, &id);
    assert_eq!(result["state"], json!("aborted"));
    assert_eq!(result["error"]["kind"], json!("aborted"));
    drop(served);
}

/// The output-cap proof: an unbounded writer (`yes`) is killed once its
/// captured stdout crosses `max_out_bytes` and the job records the clear
/// `runner_failed` cap message, long before its generous time budget.
#[test]
fn output_cap_kills_an_unbounded_child() {
    if !have("/usr/bin/yes") {
        return;
    }
    let (served, _dir) = serve_config(
        r#"
[tools.spew]
description = "writes stdout forever"
command = "/usr/bin/yes"

[tools.spew.limits]
run_timeout_ms = 30000
max_out_bytes = 4096
"#,
        150,
    );
    let (namespace, _mount) = mount(&served[0].ticket_url);
    let started = Instant::now();
    let (_id, _out, result) = run_job(&namespace, b"");
    assert!(started.elapsed() < Duration::from_secs(20));
    assert_eq!(result["state"], json!("failed"));
    assert_eq!(result["error"]["kind"], json!("runner_failed"));
    let message = result["error"]["message"].as_str().unwrap();
    assert!(message.contains("maxOutBytes"), "{message}");
    drop(served);
}

/// The tempfile-mapping proof: input lands in a private workdir file, the
/// child (`cp {input} {output}`) writes the output file, and the job's `out`
/// is read back from it.
#[test]
fn tempfile_mapping_round_trips_through_cp() {
    if !have("/bin/cp") {
        return;
    }
    let (served, _dir) = serve_config(
        r#"
[tools.copy]
description = "cp {input} {output} in a private workdir"
command = "/bin/cp"
args = ["{input}", "{output}"]
input = "tempfile"
output = "tempfile"
"#,
        160,
    );
    let (namespace, _mount) = mount(&served[0].ticket_url);
    let (_id, out, result) = run_job(&namespace, b"tempfile bytes\n");
    assert_eq!(out, "tempfile bytes\n");
    assert_eq!(result["state"], json!("done"));
    drop(served);
}

#[test]
fn parse_merges_config_and_builtin_registries() {
    let dir = tempdir::create(170);
    let path = dir.path.join("tools.toml");
    std::fs::write(
        &path,
        "[tools.copy]\ndescription = \"copy\"\ncommand = \"/bin/cat\"\n\n\
         [tools.zzz]\ndescription = \"sleep\"\ncommand = \"/bin/sleep\"\nargs = [\"1\"]\n",
    )
    .unwrap();
    let config = path.to_str().unwrap();

    // No --tool: every configured tool is served.
    let all =
        parse_tool_serve_command(&args(&["--config", config, "--listen", "127.0.0.1:0"])).unwrap();
    assert_eq!(all.tools, vec!["copy".to_owned(), "zzz".to_owned()]);

    // --tool selects from the merged registry: configured AND built-in names
    // resolve, wherever --config appears on the line.
    let mixed = parse_tool_serve_command(&args(&[
        "--tool",
        "copy",
        "--tool",
        "sha256",
        "--config",
        config,
        "--addr",
        "127.0.0.1:0",
    ]))
    .unwrap();
    assert_eq!(mixed.tools, vec!["copy".to_owned(), "sha256".to_owned()]);

    // Unknown names list both registries.
    let error = parse_tool_serve_command(&args(&[
        "--config",
        config,
        "--tool",
        "nope",
        "--listen",
        "127.0.0.1:0",
    ]))
    .unwrap_err();
    let message = error.to_string();
    assert!(message.contains("copy, zzz"), "{message}");
    assert!(message.contains("model, sha256, upper"), "{message}");

    // A bad config is a parse-time usage error.
    let bad = dir.path.join("bad.toml");
    std::fs::write(&bad, "[tools.t]\ndescription = \"x\"\ncommand = \"tr\"\n").unwrap();
    let error = parse_tool_serve_command(&args(&[
        "--config",
        bad.to_str().unwrap(),
        "--listen",
        "127.0.0.1:0",
    ]))
    .unwrap_err();
    assert!(error.to_string().contains("absolute"), "{error}");
}
