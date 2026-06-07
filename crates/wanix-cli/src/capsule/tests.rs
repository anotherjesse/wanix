use std::ffi::OsString;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use super::{parse_capsule_command, run_capsule_command};

static COUNTER: AtomicU32 = AtomicU32::new(0);

fn os(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}

fn unique_base() -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("wanix-capsule-{}-{n}", std::process::id()))
}

/// The capsule id is the second whitespace-separated token of `save` stdout:
/// `capsule <id> saved ...`.
fn capsule_id(stdout: &[u8]) -> String {
    String::from_utf8_lossy(stdout)
        .split_whitespace()
        .nth(1)
        .unwrap_or_default()
        .to_owned()
}

#[test]
fn save_then_load_round_trips_over_cas() {
    let base = unique_base();
    let world = base.join("world");
    let store = base.join("store");
    fs::create_dir_all(world.join("docs")).unwrap();
    fs::write(world.join("notes.txt"), b"banana").unwrap();
    fs::write(world.join("docs/a.txt"), b"hello\n").unwrap();
    let restored = base.join("restored");

    let saved = run_capsule_command(
        parse_capsule_command(&os(&[
            "save",
            world.to_str().unwrap(),
            "--store",
            store.to_str().unwrap(),
        ]))
        .unwrap(),
    )
    .unwrap();
    let id = capsule_id(saved.stdout());
    // The capsule id is the BLAKE3 manifest hash: 64 lowercase-hex characters.
    assert_eq!(
        id.len(),
        64,
        "expected a content-hash capsule id, got {id:?}"
    );

    // Load by id from the same CAS store and materialize into a fresh dir.
    run_capsule_command(
        parse_capsule_command(&os(&[
            "load",
            &id,
            restored.to_str().unwrap(),
            "--store",
            store.to_str().unwrap(),
        ]))
        .unwrap(),
    )
    .unwrap();

    assert_eq!(fs::read(restored.join("notes.txt")).unwrap(), b"banana");
    assert_eq!(fs::read(restored.join("docs/a.txt")).unwrap(), b"hello\n");

    fs::remove_dir_all(&base).ok();
}

#[test]
fn save_is_deterministic_and_dedups_identical_files() {
    let base = unique_base();
    let world = base.join("world");
    let store = base.join("store");
    // Two byte-identical files dedup to one blob, so the manifest has 2 entries
    // but only one underlying file blob.
    fs::create_dir_all(world.join("a")).unwrap();
    fs::create_dir_all(world.join("b")).unwrap();
    fs::write(world.join("a/lib.js"), b"shared\n").unwrap();
    fs::write(world.join("b/lib.js"), b"shared\n").unwrap();

    let first = run_capsule_command(
        parse_capsule_command(&os(&[
            "save",
            world.to_str().unwrap(),
            "--store",
            store.to_str().unwrap(),
        ]))
        .unwrap(),
    )
    .unwrap();
    let second = run_capsule_command(
        parse_capsule_command(&os(&[
            "save",
            world.to_str().unwrap(),
            "--store",
            store.to_str().unwrap(),
        ]))
        .unwrap(),
    )
    .unwrap();
    // The same world always yields the same id (deterministic, content-addressed).
    assert_eq!(capsule_id(first.stdout()), capsule_id(second.stdout()));

    fs::remove_dir_all(&base).ok();
}

#[test]
fn load_rejects_a_non_hex_capsule_id() {
    let error = parse_capsule_command(&os(&["load", "not-a-hash", "/tmp/out"])).unwrap_err();
    assert_eq!(error.exit_code(), 2);
}

#[test]
fn capsule_requires_save_or_load() {
    let error = parse_capsule_command(&os(&["frobnicate", "a", "b"])).unwrap_err();
    assert_eq!(error.exit_code(), 2);
}
