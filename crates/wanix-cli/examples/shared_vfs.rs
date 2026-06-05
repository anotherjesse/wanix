//! Proof: a Rust `wasm32-wasi` task and a QuickJS task share one Wanix VFS,
//! two-way, through a single namespace.
//!
//! Flow on one shared in-memory filesystem:
//!   1. qjs   writes /shared/from_qjs.txt
//!   2. rust  (compiled wasm) reads it, writes /shared/from_rust.txt
//!   3. qjs   reads /shared/from_rust.txt back
//!
//! Run: cargo run -p wanix-cli --example shared_vfs

use std::sync::Arc;

use wanix_fs::MemFs;
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

fn main() {
    // One backing filesystem, shared by every task below.
    let fs = Arc::new(MemFs::new());
    fs.create_dir_all("shared").expect("make /shared");

    let qjs = QuickJsRunner::from_bundled_wasm().expect("qjs runner");
    let rust = WasiRunner::from_bytes(RUST_GUEST).expect("compile rust guest");

    // 1. qjs writes a file.
    let out = run_qjs(
        &qjs,
        &fs,
        r#"import * as std from "qjs:std";
std.writeFile("/shared/from_qjs.txt", "hello from qjs");
std.out.puts("qjs: wrote /shared/from_qjs.txt\n");
std.out.flush();
"#,
    );
    print!("{out}");

    // 2. rust (compiled wasm) reads qjs's file and writes its own.
    let stdout = CaptureFile::new();
    let config = WasiConfig::new(namespace_on(&fs))
        .with_args(["guest", "/shared/from_qjs.txt", "/shared/from_rust.txt"])
        .with_stdout(Box::new(stdout.clone()), "stdout");
    let exit = rust.run(config).expect("rust wasm task ran");
    print!("{}", stdout.contents());
    println!("rust: exited with code {exit}");

    // 3. qjs reads back what rust wrote.
    let out = run_qjs(
        &qjs,
        &fs,
        r#"import * as std from "qjs:std";
std.out.puts("qjs: read back -> " + std.loadFile("/shared/from_rust.txt") + "\n");
std.out.flush();
"#,
    );
    print!("{out}");

    // Host-side confirmation straight from the shared filesystem.
    let qjs_file = String::from_utf8(fs.read_file("shared/from_qjs.txt").unwrap()).unwrap();
    let rust_file = String::from_utf8(fs.read_file("shared/from_rust.txt").unwrap()).unwrap();
    println!("---");
    println!("shared fs /shared/from_qjs.txt  = {qjs_file:?}");
    println!("shared fs /shared/from_rust.txt = {rust_file:?}");
    assert_eq!(qjs_file, "hello from qjs");
    assert_eq!(rust_file, "rust-wasm saw: hello from qjs");
    println!("OK: qjs <-> rust shared one VFS, two-way.");
}
