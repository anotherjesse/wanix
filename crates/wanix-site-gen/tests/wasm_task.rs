//! Integration test: run the checked-in SSG wasm artifact as a real Wanix
//! `.wasm` task over an in-memory output `FileSystem`.
//!
//! This proves the Phase 1 end-to-end shape: the `site-gen.wasm` module
//! (built from `wasm-src/`, vendored under `fixtures/`) is bound into a task
//! namespace, started through `WasmTaskDriver`, reads the fixture corpus from
//! the shared `MemFs`, and writes `<section>/<page>/index.html` back into that
//! same `MemFs` — with no host disk for the output and no wasm toolchain at
//! test time.

use std::sync::Arc;

use wanix_fs::MemFs;
use wanix_task::TaskTable;
use wanix_vfs::{BindOptions, Namespace};
use wanix_wasm::WasmTaskDriver;

/// The vendored SSG artifact (rebuild via `wasm-src/README.md`).
const SITE_GEN_WASM: &[u8] = include_bytes!("../fixtures/site-gen.wasm");

/// Fixture corpus pages seeded into the in-memory namespace.
const CORPUS: &[(&str, &str)] = &[
    (
        "content/home.md",
        "---\ntitle: \"Wanix Home\"\n---\n# Wanix\n\nGo to [EIAF](/concepts/everything-is-a-file).\n",
    ),
    (
        "content/concepts/everything-is-a-file.md",
        "---\ntitle: Everything Is a File\n---\n# Everything Is a File\n\nBack [home](/).\n",
    ),
    (
        "content/concepts/the-filesystem-trait.md",
        "---\ntitle: The Filesystem Trait\n---\n# The Filesystem Trait\n\nSee [EIAF](/concepts/everything-is-a-file).\n",
    ),
];

#[test]
fn wasm_ssg_task_generates_site_into_shared_memfs() {
    let fs = Arc::new(MemFs::new());
    fs.write_file("site-gen.wasm", SITE_GEN_WASM)
        .expect("seed wasm module");
    for (path, body) in CORPUS {
        fs.create_dir_all(parent_of(path)).expect("seed corpus dir");
        fs.write_file(path, body.as_bytes()).expect("seed corpus");
    }

    let mut ns = Namespace::new();
    ns.bind(fs.clone(), ".", ".", BindOptions::default())
        .expect("bind shared fs at root");

    let table = TaskTable::new();
    table
        .register_driver("wasm", Arc::new(WasmTaskDriver::new()))
        .expect("register wasm driver");
    let task = table
        .allocate_root_with_namespace("auto", ns)
        .expect("allocate task");
    task.set_cmd("site-gen.wasm content site").expect("set cmd");

    table.start(task.id()).expect("auto-start wasm SSG task");
    assert_eq!(task.kind(), "wasm", "auto task resolved to the wasm driver");
    assert_eq!(task.exit(), "0", "SSG task should exit 0");

    // Page count: one index.html per source page, written through the shared FS.
    let home =
        String::from_utf8(fs.read_file("site/index.html").expect("home index")).expect("utf8");
    let eiaf = fs
        .read_file("site/concepts/everything-is-a-file/index.html")
        .expect("eiaf index");
    let trait_page = fs
        .read_file("site/concepts/the-filesystem-trait/index.html")
        .expect("trait index");
    assert!(!eiaf.is_empty() && !trait_page.is_empty());

    // Frontmatter stripped from the generated output.
    assert!(
        !home.contains("title: \"Wanix Home\""),
        "frontmatter must be stripped: {home}"
    );
    assert!(home.contains("<h1>Wanix</h1>"), "body rendered: {home}");

    // Internal slug link rewritten to the served directory form, and that
    // target resolves to a generated file in the shared FS.
    assert!(
        home.contains("href=\"/concepts/everything-is-a-file/\""),
        "internal link rewritten to served path: {home}"
    );
    assert!(
        fs.read_file("site/concepts/everything-is-a-file/index.html")
            .is_ok(),
        "rewritten link target resolves to a generated file"
    );
}

/// Returns the parent directory of a `/`-separated path (`""` if none).
fn parent_of(path: &str) -> &str {
    match path.rfind('/') {
        Some(i) => &path[..i],
        None => "",
    }
}
