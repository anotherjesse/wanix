//! Cross-engine differential tests over ONE shared Wanix VFS.
//!
//! These automate the `shared_vfs` example proof: a QuickJS task and a compiled
//! Rust `wasm32-wasi` task both run against the same [`MemFs`] namespace and must
//! observe identical filesystem state. The directory-listing test is a true
//! differential: qjs (`os.readdir` + `os.stat`) and rust-wasm (`std::fs::read_dir`)
//! each produce a sorted `name type` listing of the same directory, and the two
//! listings must match byte-for-byte — proving both engines route through the
//! same `WasiCtx::fd_read_dir` backend on the same VFS.

use std::sync::Arc;

use wanix_fs::{FileSystem, MemFs, NormalizedPath};
use wanix_qjs::{QuickJsRunner, QuickJsWanixConfig};
use wanix_vfs::{BindOptions, Namespace};
use wanix_wasi::WasiConfig;
use wanix_wasm::{CaptureFile, WasiRunner};

const RUST_GUEST: &[u8] = include_bytes!("../../wanix-wasm/fixtures/rust-guest.wasm");

fn namespace_on(fs: &Arc<MemFs>) -> Namespace {
    let mut ns = Namespace::new();
    ns.bind(fs.clone(), ".", ".", BindOptions::default())
        .expect("bind shared fs at root");
    ns
}

fn run_qjs(qjs: &QuickJsRunner, fs: &Arc<MemFs>, source: &str) -> String {
    let stdout = CaptureFile::new();
    let wasi = WasiConfig::new(namespace_on(fs)).with_stdout(Box::new(stdout.clone()), "stdout");
    let config = QuickJsWanixConfig::new(wasi);
    qjs.run_source_with_wanix_config(source, config)
        .expect("qjs task ran");
    stdout.contents()
}

fn run_rust(rust: &WasiRunner, fs: &Arc<MemFs>, args: &[&str]) -> (i32, String) {
    let stdout = CaptureFile::new();
    let config = WasiConfig::new(namespace_on(fs))
        .with_args(args.iter().copied())
        .with_stdout(Box::new(stdout.clone()), "stdout");
    let exit = rust.run(config).expect("rust wasm task ran");
    (exit, stdout.contents())
}

/// Parses the rust guest's `--list` output into sorted `name type` rows,
/// dropping the leading summary line.
fn rust_listing(out: &str) -> Vec<String> {
    let mut rows: Vec<String> = out
        .lines()
        .filter(|l| !l.starts_with("rust-wasm:"))
        .map(str::to_string)
        .collect();
    rows.sort();
    rows
}

/// QuickJS program that prints one sorted `name type` line per entry in `dir`,
/// matching the rust guest's vocabulary (`dir` / `symlink` / `file`).
fn qjs_list_source(dir: &str) -> String {
    format!(
        r#"import * as std from "qjs:std";
import * as os from "qjs:os";

const dir = {dir:?};
const [names, err] = os.readdir(dir);
if (err !== 0) {{ throw new Error("readdir " + dir + ": " + err); }}
const rows = [];
for (const name of names) {{
  if (name === "." || name === "..") continue;
  const [st, serr] = os.lstat(dir + "/" + name);
  if (serr !== 0) {{ throw new Error("lstat " + name + ": " + serr); }}
  const fmt = st.mode & os.S_IFMT;
  let kind = "file";
  if (fmt === os.S_IFDIR) kind = "dir";
  else if (fmt === os.S_IFLNK) kind = "symlink";
  rows.push(name + " " + kind);
}}
rows.sort();
for (const row of rows) std.out.puts(row + "\n");
std.out.flush();
"#
    )
}

#[test]
fn qjs_and_rust_wasm_see_identical_directory_listing() {
    let fs = Arc::new(MemFs::new());
    fs.create_dir_all("dir/sub").expect("make /dir/sub");
    fs.create_dir_all("dir/empty").expect("make /dir/empty");
    fs.write_file("dir/alpha.txt", b"a").expect("write alpha");
    fs.write_file("dir/beta.txt", b"bb").expect("write beta");
    fs.write_file("dir/gamma", b"ccc").expect("write gamma");

    let qjs = QuickJsRunner::from_bundled_wasm().expect("qjs runner");
    let rust = WasiRunner::from_bytes(RUST_GUEST).expect("compile rust guest");

    let (exit, rust_out) = run_rust(&rust, &fs, &["guest", "--list", "/dir"]);
    assert_eq!(exit, 0, "rust guest should exit cleanly: {rust_out:?}");
    let rust_rows = rust_listing(&rust_out);

    let qjs_out = run_qjs(&qjs, &fs, &qjs_list_source("/dir"));
    let qjs_rows: Vec<String> = qjs_out.lines().map(str::to_string).collect();

    // The differential assertion: both engines, over the SAME namespace, must
    // report exactly the same entries with the same types.
    assert_eq!(
        rust_rows, qjs_rows,
        "qjs and rust-wasm disagree on /dir listing\nrust={rust_out:?}\nqjs={qjs_out:?}"
    );

    // And the concrete expected listing, so a backend change can't silently make
    // both engines agree on the WRONG answer.
    assert_eq!(
        rust_rows,
        vec![
            "alpha.txt file".to_string(),
            "beta.txt file".to_string(),
            "empty dir".to_string(),
            "gamma file".to_string(),
            "sub dir".to_string(),
        ],
        "unexpected shared listing: {rust_out:?}"
    );
}

#[test]
fn qjs_writes_rust_renames_qjs_observes_move() {
    // Differential proof for the new path_rename import: qjs writes a file,
    // rust-wasm `std::fs::rename`s it on the SAME namespace, then qjs observes
    // (via os.stat) that the new path exists with the original bytes and the
    // old path is gone. Both engines route rename/stat through one WasiCtx VFS.
    let fs = Arc::new(MemFs::new());
    fs.create_dir_all("shared").expect("make /shared");

    let qjs = QuickJsRunner::from_bundled_wasm().expect("qjs runner");
    let rust = WasiRunner::from_bytes(RUST_GUEST).expect("compile rust guest");

    // 1. qjs creates the source file.
    let qjs_out = run_qjs(
        &qjs,
        &fs,
        r#"import * as std from "qjs:std";
std.writeFile("/shared/src.txt", "move me");
std.out.puts("qjs: wrote src\n");
std.out.flush();
"#,
    );
    assert!(qjs_out.contains("qjs: wrote src"), "qjs write: {qjs_out:?}");

    // 2. rust-wasm renames it through the path_rename import.
    let (exit, rust_out) = run_rust(
        &rust,
        &fs,
        &["guest", "--rename", "/shared/src.txt", "/shared/dst.txt"],
    );
    assert_eq!(exit, 0, "rust rename step exit: {rust_out:?}");
    assert!(
        rust_out.contains("renamed /shared/src.txt -> /shared/dst.txt"),
        "rust rename output: {rust_out:?}"
    );

    // 3. qjs, on the same namespace, observes the move: dst exists with the
    // original bytes; src no longer resolves.
    let observed = run_qjs(
        &qjs,
        &fs,
        r#"import * as std from "qjs:std";
import * as os from "qjs:os";
const [, srcErr] = os.stat("/shared/src.txt");
const [, dstErr] = os.stat("/shared/dst.txt");
std.out.puts("src_present " + (srcErr === 0) + "\n");
std.out.puts("dst_present " + (dstErr === 0) + "\n");
if (dstErr === 0) std.out.puts("dst_body " + std.loadFile("/shared/dst.txt") + "\n");
std.out.flush();
"#,
    );
    assert!(
        observed.contains("src_present false"),
        "qjs should see src gone after rename: {observed:?}"
    );
    assert!(
        observed.contains("dst_present true"),
        "qjs should see dst after rename: {observed:?}"
    );
    assert!(
        observed.contains("dst_body move me"),
        "qjs should read renamed bytes: {observed:?}"
    );

    // Host-side confirmation straight from the shared backing filesystem.
    assert_eq!(
        fs.read_file("shared/dst.txt").expect("dst exists"),
        b"move me",
        "renamed file should keep original bytes on the backing fs"
    );
    assert!(
        fs.read_file("shared/src.txt").is_err(),
        "source should be gone on the backing fs after rename"
    );
}

/// Returns whether `path` exists on the backing fs (dir or file).
fn fs_exists(fs: &Arc<MemFs>, path: &str) -> bool {
    let np = NormalizedPath::new(path).expect("normalize path");
    fs.metadata(&np).is_ok()
}

#[test]
fn qjs_creates_rust_removes_directory() {
    // Differential proof for the new path_remove_directory import: qjs/host
    // creates an EMPTY directory, rust-wasm `std::fs::remove_dir`s it on the
    // SAME namespace, then qjs (os.stat) and the backing fs both confirm it is
    // gone. Both engines route rmdir/stat through one WasiCtx VFS.
    let fs = Arc::new(MemFs::new());
    fs.create_dir_all("shared/empty")
        .expect("make /shared/empty");
    assert!(fs_exists(&fs, "shared/empty"), "dir should exist initially");

    let qjs = QuickJsRunner::from_bundled_wasm().expect("qjs runner");
    let rust = WasiRunner::from_bytes(RUST_GUEST).expect("compile rust guest");

    // 1. rust-wasm removes the empty directory through path_remove_directory.
    let (exit, rust_out) = run_rust(&rust, &fs, &["guest", "--rmdir", "/shared/empty"]);
    assert_eq!(exit, 0, "rust rmdir step exit: {rust_out:?}");
    assert!(
        rust_out.contains("removed dir /shared/empty"),
        "rust rmdir output: {rust_out:?}"
    );

    // 2. qjs, on the same namespace, observes the directory is gone.
    let observed = run_qjs(
        &qjs,
        &fs,
        r#"import * as std from "qjs:std";
import * as os from "qjs:os";
const [, err] = os.stat("/shared/empty");
std.out.puts("dir_present " + (err === 0) + "\n");
std.out.flush();
"#,
    );
    assert!(
        observed.contains("dir_present false"),
        "qjs should see dir gone after rmdir: {observed:?}"
    );

    // 3. Host-side confirmation straight from the shared backing filesystem.
    assert!(
        !fs_exists(&fs, "shared/empty"),
        "directory should be gone on the backing fs after rmdir"
    );
    let np = NormalizedPath::new("shared/empty").expect("normalize");
    assert!(
        fs.read_dir(&np).is_err(),
        "read_dir of removed directory should error on the backing fs"
    );
}

#[test]
fn rust_rmdir_nonempty_returns_error() {
    // Parity with ENOTEMPTY: a directory containing a child cannot be removed.
    // rust-wasm's remove_dir must fail and the directory must survive on the
    // backing fs; qjs must still see it present — both engines agree the
    // removal was rejected.
    let fs = Arc::new(MemFs::new());
    fs.create_dir_all("shared/full").expect("make /shared/full");
    fs.write_file("shared/full/child.txt", b"keep me")
        .expect("write child");

    let qjs = QuickJsRunner::from_bundled_wasm().expect("qjs runner");
    let rust = WasiRunner::from_bytes(RUST_GUEST).expect("compile rust guest");

    let (exit, rust_out) = run_rust(&rust, &fs, &["guest", "--rmdir", "/shared/full"]);
    assert_eq!(exit, 0, "rust rmdir step exit: {rust_out:?}");
    assert!(
        rust_out.contains("rmdir failed /shared/full"),
        "rust rmdir of non-empty dir should report failure: {rust_out:?}"
    );

    // The non-empty directory and its child must still exist on the backing fs.
    assert!(
        fs_exists(&fs, "shared/full"),
        "non-empty directory should survive a rejected rmdir"
    );
    assert_eq!(
        fs.read_file("shared/full/child.txt").expect("child exists"),
        b"keep me",
        "child file should be untouched after rejected rmdir"
    );

    // qjs, on the same namespace, agrees the directory is still present.
    let observed = run_qjs(
        &qjs,
        &fs,
        r#"import * as std from "qjs:std";
import * as os from "qjs:os";
const [, err] = os.stat("/shared/full");
std.out.puts("dir_present " + (err === 0) + "\n");
std.out.flush();
"#,
    );
    assert!(
        observed.contains("dir_present true"),
        "qjs should still see the non-empty dir after rejected rmdir: {observed:?}"
    );
}

#[test]
fn qjs_creates_symlink_rust_wasm_readlinks_same_target() {
    // Differential proof for the new path_readlink import: qjs creates a symlink
    // on the SHARED namespace, and rust-wasm (`fs::read_link`) reads back the
    // exact same target bytes. Both engines route symlink/readlink through one
    // WasiCtx VFS, so the targets must match byte-for-byte.
    let fs = Arc::new(MemFs::new());
    fs.create_dir_all("shared").expect("make /shared");
    fs.write_file("shared/real.txt", b"payload")
        .expect("seed real.txt");

    let qjs = QuickJsRunner::from_bundled_wasm().expect("qjs runner");
    let rust = WasiRunner::from_bytes(RUST_GUEST).expect("compile rust guest");

    // 1. qjs creates the symlink (target is relative `real.txt`).
    let qjs_out = run_qjs(
        &qjs,
        &fs,
        r#"import * as std from "qjs:std";
import * as os from "qjs:os";
const err = os.symlink("real.txt", "/shared/qjs-link.txt");
std.out.puts("symlink_err " + err + "\n");
std.out.flush();
"#,
    );
    assert!(
        qjs_out.contains("symlink_err 0"),
        "qjs symlink should succeed: {qjs_out:?}"
    );

    // 2. rust-wasm reads the link back through path_readlink.
    let (exit, rust_out) = run_rust(&rust, &fs, &["guest", "--readlink", "/shared/qjs-link.txt"]);
    assert_eq!(exit, 0, "rust readlink step exit: {rust_out:?}");
    assert!(
        rust_out.lines().any(|l| l == "real.txt"),
        "rust-wasm should read qjs's symlink target: {rust_out:?}"
    );

    // Host-side confirmation: the backing fs stores the exact target bytes.
    let np = NormalizedPath::new("shared/qjs-link.txt").expect("normalize");
    assert_eq!(
        fs.read_link(&np).expect("link exists"),
        b"real.txt",
        "backing fs should store the qjs symlink target"
    );
}

#[test]
fn rust_wasm_creates_symlink_qjs_readlinks_same_target() {
    // The reverse direction: rust-wasm creates a symlink via path_symlink and
    // qjs (`os.readlink`) reads back the identical target on the same namespace.
    let fs = Arc::new(MemFs::new());
    fs.create_dir_all("shared").expect("make /shared");
    fs.write_file("shared/real.txt", b"payload")
        .expect("seed real.txt");

    let qjs = QuickJsRunner::from_bundled_wasm().expect("qjs runner");
    let rust = WasiRunner::from_bytes(RUST_GUEST).expect("compile rust guest");

    // 1. rust-wasm creates the symlink through path_symlink.
    let (exit, rust_out) = run_rust(
        &rust,
        &fs,
        &["guest", "--symlink", "real.txt", "/shared/rust-link.txt"],
    );
    assert_eq!(exit, 0, "rust symlink step exit: {rust_out:?}");
    assert!(
        rust_out.lines().any(|l| l == "ok"),
        "rust-wasm symlink should succeed: {rust_out:?}"
    );

    // 2. qjs reads the link back; os.readlink returns [target, errno].
    let qjs_out = run_qjs(
        &qjs,
        &fs,
        r#"import * as std from "qjs:std";
import * as os from "qjs:os";
const [target, err] = os.readlink("/shared/rust-link.txt");
std.out.puts("readlink_err " + err + "\n");
std.out.puts("target " + target + "\n");
std.out.flush();
"#,
    );
    assert!(
        qjs_out.contains("readlink_err 0"),
        "qjs readlink should succeed: {qjs_out:?}"
    );
    assert!(
        qjs_out.contains("target real.txt"),
        "qjs should read rust-wasm's symlink target byte-for-byte: {qjs_out:?}"
    );

    // Host-side confirmation: the backing fs stores the exact target bytes.
    let np = NormalizedPath::new("shared/rust-link.txt").expect("normalize");
    assert_eq!(
        fs.read_link(&np).expect("link exists"),
        b"real.txt",
        "backing fs should store the rust-wasm symlink target"
    );
}

#[test]
fn qjs_and_rust_wasm_share_one_vfs_two_way() {
    // Backs the `shared_vfs` example as a real automated test: qjs writes a file,
    // rust-wasm reads it and writes its own, qjs reads that back.
    let fs = Arc::new(MemFs::new());
    fs.create_dir_all("shared").expect("make /shared");

    let qjs = QuickJsRunner::from_bundled_wasm().expect("qjs runner");
    let rust = WasiRunner::from_bytes(RUST_GUEST).expect("compile rust guest");

    let qjs_out = run_qjs(
        &qjs,
        &fs,
        r#"import * as std from "qjs:std";
std.writeFile("/shared/from_qjs.txt", "hello from qjs");
std.out.puts("qjs: wrote\n");
std.out.flush();
"#,
    );
    assert!(
        qjs_out.contains("qjs: wrote"),
        "qjs write step: {qjs_out:?}"
    );

    let (exit, rust_out) = run_rust(
        &rust,
        &fs,
        &["guest", "/shared/from_qjs.txt", "/shared/from_rust.txt"],
    );
    assert_eq!(exit, 0, "rust step exit: {rust_out:?}");

    let back = run_qjs(
        &qjs,
        &fs,
        r#"import * as std from "qjs:std";
std.out.puts(std.loadFile("/shared/from_rust.txt"));
std.out.flush();
"#,
    );
    assert_eq!(back, "rust-wasm saw: hello from qjs");

    // Host-side confirmation straight from the shared backing filesystem.
    let qjs_file = String::from_utf8(fs.read_file("shared/from_qjs.txt").unwrap()).unwrap();
    let rust_file = String::from_utf8(fs.read_file("shared/from_rust.txt").unwrap()).unwrap();
    assert_eq!(qjs_file, "hello from qjs");
    assert_eq!(rust_file, "rust-wasm saw: hello from qjs");
}
