//! BothShells proofs for the CLI `sh` path (ADR 0002): `.js` and `.wasm`
//! externals are both first-class from inside the wasm shell — same
//! resolution, same `#task` auto dispatch, same argv/stdio/exit contract.

use std::sync::Arc;

use wanix_fs::MemFs;
use wanix_pipe::PipeDevice;
use wanix_vfs::{BindOptions, Namespace};

use super::{allocate_sh_task, configure_sh_task};
use crate::{CliError, CliOutput, attach_task_stdio, finish_cli_task_output};

/// `rust-guest.wasm` provides `--cat` (stdin -> stdout), the `.wasm` half of
/// the mixed pipeline.
const RUST_GUEST: &[u8] = include_bytes!("../../../wanix-wasm/fixtures/rust-guest.wasm");

/// A `.js` producer: argv joined onto stdout, exiting with the named status.
const GEN_JS: &str = r#"import * as std from "qjs:std";
std.out.puts("from-js " + scriptArgs.slice(1).join(",") + "\n");
std.out.flush();
"#;

const FAIL_JS: &str = r#"import * as std from "qjs:std";
std.exit(7);
"#;

/// A `.js` mirror of the wasm `post` verb (same input convention as
/// `fixtures/verbs-src/src/post.rs`): argv joined is the body, no argv reads
/// stdin; the body lands on `/res/post` — the confined resource.
const POST_JS: &str = r#"import * as std from "qjs:std";
import * as os from "qjs:os";

function bytesFromString(text) {
  const bytes = new Uint8Array(text.length);
  for (let i = 0; i < text.length; i++) {
    bytes[i] = text.charCodeAt(i) & 0xff;
  }
  return bytes;
}

let body;
if (scriptArgs.length > 1) {
  body = bytesFromString(scriptArgs.slice(1).join(" "));
} else {
  const chunks = [];
  const buffer = new Uint8Array(4096);
  for (;;) {
    const count = os.read(0, buffer.buffer, 0, buffer.length);
    if (count < 0) {
      std.err.puts("post: stdin: errno " + count + "\n");
      std.exit(1);
    }
    if (count === 0) {
      break;
    }
    chunks.push(buffer.slice(0, count));
  }
  let total = 0;
  for (const chunk of chunks) {
    total += chunk.length;
  }
  body = new Uint8Array(total);
  let offset = 0;
  for (const chunk of chunks) {
    body.set(chunk, offset);
    offset += chunk.length;
  }
}
// Open write-only without create/truncate, like the wasm post verb.
const fd = os.open("/res/post", os.O_WRONLY);
if (fd < 0) {
  std.err.puts("post: /res/post: errno " + fd + "\n");
  std.exit(1);
}
os.write(fd, body.buffer, 0, body.length);
os.close(fd);
"#;

/// Runs one `sh -c LINE` against `namespace` through the real CLI task setup
/// (both drivers registered) and returns the captured output.
fn run_line(namespace: Namespace, line: &str) -> CliOutput {
    let (table, task) = allocate_sh_task(namespace).expect("allocate sh task");
    configure_sh_task(&task, Some(line), &[]).expect("configure sh task");
    let (stdout, stderr) = attach_task_stdio(&task, Some(Vec::new())).expect("attach stdio");
    let result = table.start(task.id()).map_err(CliError::from);
    finish_cli_task_output("sh", result, &task, &stdout, &stderr).expect("collect output")
}

/// A shell-session namespace shaped like `sh_namespace`: `root` at `.`,
/// `#pipe`, and a `bin` carrying the bundled shell plus the given commands.
fn shell_namespace(root: Arc<MemFs>, commands: &[(&str, &[u8])]) -> Namespace {
    let bin = MemFs::new();
    bin.write_file("sh.wasm", wanix_wasm::SHELL_WASM)
        .expect("seed sh.wasm");
    for (name, bytes) in commands {
        bin.write_file(name, bytes).expect("seed bin command");
    }
    let mut namespace = Namespace::new();
    namespace
        .bind(root, ".", ".", BindOptions::default())
        .expect("bind root");
    namespace
        .bind(
            Arc::new(PipeDevice::new()),
            ".",
            "#pipe",
            BindOptions::default(),
        )
        .expect("bind #pipe");
    namespace
        .bind(Arc::new(bin), ".", "bin", BindOptions::default())
        .expect("bind bin");
    namespace
}

#[test]
fn sh_runs_js_from_bin_in_a_mixed_pipeline_with_a_wasm_stage() {
    // `gen` resolves to bin/gen.js (the qjs driver claims it), `slurp` to
    // bin/slurp.wasm (the wasm driver claims it); the pipe between them is the
    // mixed-kind pipeline ADR 0002 promises.
    let namespace = shell_namespace(
        Arc::new(MemFs::new()),
        &[("gen.js", GEN_JS.as_bytes()), ("slurp.wasm", RUST_GUEST)],
    );
    let output = run_line(namespace, "gen one two | slurp --cat");

    assert_eq!(
        output.exit_code(),
        0,
        "stderr: {}",
        String::from_utf8_lossy(output.stderr())
    );
    assert_eq!(
        output.stdout(),
        b"from-js one,two\n",
        "js argv crossed the pipe into the wasm consumer"
    );
}

#[test]
fn sh_observes_a_js_command_exit_status() {
    let namespace = shell_namespace(Arc::new(MemFs::new()), &[("fail.js", FAIL_JS.as_bytes())]);
    let output = run_line(namespace, "fail; echo status=$?");

    assert_eq!(
        output.exit_code(),
        0,
        "stderr: {}",
        String::from_utf8_lossy(output.stderr())
    );
    assert_eq!(
        output.stdout(),
        b"status=7\n",
        "the js child's exit reaches $? through #task wait"
    );
}

/// Runs one line against a fresh `room` resource shipping the `.js` post verb
/// at `n/room/bin/post.js`; returns (resource, output).
fn run_js_verb_line(line: &str) -> (Arc<MemFs>, CliOutput) {
    let resource = Arc::new(MemFs::new());
    resource.create_dir_all("bin").expect("make bin");
    resource
        .write_file("bin/post.js", POST_JS.as_bytes())
        .expect("seed post.js");
    resource.write_file("post", b"").expect("seed post file");
    let mut namespace = shell_namespace(Arc::new(MemFs::new()), &[]);
    namespace
        .bind(resource.clone(), ".", "n/room", BindOptions::default())
        .expect("mount resource at n/room");
    let output = run_line(namespace, line);
    (resource, output)
}

#[test]
fn js_bin_verb_posts_argv_into_its_resource() {
    // The BinVerbs seam is driver-agnostic: a resource shipping bin/post.js
    // runs confined exactly like the wasm verb, and the write lands back on
    // the resource it came from.
    let (resource, output) = run_js_verb_line("room:post hello world; echo status=$?");

    assert_eq!(
        output.exit_code(),
        0,
        "stderr: {}",
        String::from_utf8_lossy(output.stderr())
    );
    assert_eq!(output.stdout(), b"status=0\n");
    assert_eq!(
        resource.read_file("post").expect("read post"),
        b"hello world"
    );
}

#[test]
fn over_cap_verb_refusal_reaches_the_shell_stderr() {
    // ADR 0007 §Confinement contract: an over-cap verb is refused at open with
    // an honest error — and that error must reach the operator. The shell
    // launches externals detached (`start &`), where the table reduces the
    // driver's Err to an exit code, so the diagnostic's only honest surface is
    // the child's inherited stderr (Task::report_run_failure).
    let resource = Arc::new(MemFs::new());
    resource
        .write_file("data.txt", b"resource bytes")
        .expect("seed resource data");
    let bin_backing = Arc::new(MemFs::new());
    bin_backing
        .write_file("huge.js", [b'x'; 64])
        .expect("seed over-cap verb");
    let verb_bin = Arc::new(wanix_fs::VerbBinFs::with_max_bytes(bin_backing, 16));
    let mut namespace = shell_namespace(Arc::new(MemFs::new()), &[]);
    namespace
        .bind(resource, ".", "n/room", BindOptions::default())
        .expect("mount resource at n/room");
    namespace
        .bind(verb_bin, ".", "n/room/bin", BindOptions::default())
        .expect("mount capped verb bin");

    let output = run_line(namespace, "room:huge; echo status=$?");

    assert_eq!(
        output.exit_code(),
        0,
        "stderr: {}",
        String::from_utf8_lossy(output.stderr())
    );
    assert_eq!(
        output.stdout(),
        b"status=1\n",
        "the refused verb never runs and the child exits 1"
    );
    let stderr = String::from_utf8_lossy(output.stderr());
    assert!(
        stderr.contains("exceeds the 16-byte verb size cap"),
        "the honest over-cap refusal must reach the operator: {stderr:?}"
    );
    assert!(
        stderr.contains("wanix:"),
        "the diagnostic names its host-level origin: {stderr:?}"
    );
}

#[test]
fn js_bin_verb_reads_piped_stdin_with_no_argv() {
    // The documented input convention holds for the .js verb too: with no
    // argv it posts its stdin, so `echo hi | room:post` composes — a builtin
    // stage feeding a .js stage through a real #pipe.
    let (resource, output) = run_js_verb_line("echo hi | room:post");

    assert_eq!(
        output.exit_code(),
        0,
        "stderr: {}",
        String::from_utf8_lossy(output.stderr())
    );
    assert_eq!(resource.read_file("post").expect("read post"), b"hi\n");
}
