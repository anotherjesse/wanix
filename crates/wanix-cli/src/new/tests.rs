use std::ffi::OsString;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use super::{parse_new_command, run_new_command};

static COUNTER: AtomicU32 = AtomicU32::new(0);

fn os_args(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}

fn unique_parent(label: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("wanix-cli-new-{label}-{}-{n}", std::process::id()))
}

#[test]
fn new_js_writes_main_readme_and_bundled_sdk() {
    let parent = unique_parent("js");
    let cmd = parse_new_command(&os_args(&[
        "--js",
        "demo",
        "--dir",
        &parent.display().to_string(),
    ]))
    .unwrap();
    let out = run_new_command(cmd).unwrap();
    let root = parent.join("demo");

    assert!(root.join("main.js").is_file());
    assert!(root.join("README.md").is_file());
    assert!(root.join("tsconfig.json").is_file());
    assert!(root.join("lib/wanix/task.js").is_file());
    assert!(root.join("lib/wanix/fs.js").is_file());
    assert!(root.join("lib/wanix/wanix-qjs.d.ts").is_file());

    let main = fs::read_to_string(root.join("main.js")).unwrap();
    assert!(main.contains("qjs:std"));
    assert!(main.contains("lib/wanix/"));
    assert!(String::from_utf8_lossy(out.stdout()).contains("created js project"));

    fs::remove_dir_all(&parent).unwrap();
}

#[test]
fn new_rust_writes_cargo_config_and_main_with_name() {
    let parent = unique_parent("rust");
    let cmd = parse_new_command(&os_args(&[
        "--rust",
        "widget",
        "--dir",
        &parent.display().to_string(),
    ]))
    .unwrap();
    run_new_command(cmd).unwrap();
    let root = parent.join("widget");

    let cargo = fs::read_to_string(root.join("Cargo.toml")).unwrap();
    assert!(cargo.contains("name = \"widget\""));
    assert!(cargo.contains("[workspace]"));
    assert!(cargo.contains("opt-level = \"s\""));
    assert!(cargo.contains("strip = true"));

    let config = fs::read_to_string(root.join(".cargo/config.toml")).unwrap();
    assert!(config.contains("wasm32-wasip1"));
    assert!(root.join("src/main.rs").is_file());

    let readme = fs::read_to_string(root.join("README.md")).unwrap();
    assert!(readme.contains("wanix wasm"));
    assert!(readme.contains("widget.wasm"));

    fs::remove_dir_all(&parent).unwrap();
}

#[test]
fn new_refuses_non_empty_target() {
    let parent = unique_parent("nonempty");
    let root = parent.join("demo");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("existing.txt"), b"keep").unwrap();

    let cmd = parse_new_command(&os_args(&[
        "--js",
        "demo",
        "--dir",
        &parent.display().to_string(),
    ]))
    .unwrap();
    let error = run_new_command(cmd).unwrap_err();
    assert_eq!(error.exit_code(), 1);
    assert!(error.to_string().contains("must be empty"));

    fs::remove_dir_all(&parent).unwrap();
}

#[test]
fn new_requires_exactly_one_kind() {
    let missing = parse_new_command(&os_args(&["demo"])).unwrap_err();
    assert_eq!(missing.exit_code(), 2);

    let both = parse_new_command(&os_args(&["--js", "--rust", "demo"])).unwrap_err();
    assert_eq!(both.exit_code(), 2);

    let no_name = parse_new_command(&os_args(&["--js"])).unwrap_err();
    assert_eq!(no_name.exit_code(), 2);
}
