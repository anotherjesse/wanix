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

fn capsule_id(stdout: &[u8]) -> String {
    String::from_utf8_lossy(stdout)
        .split_whitespace()
        .nth(1)
        .unwrap_or_default()
        .to_owned()
}

#[test]
fn save_then_load_round_trips_and_is_content_addressed() {
    let base = unique_base();
    let world = base.join("world");
    fs::create_dir_all(world.join("docs")).unwrap();
    fs::write(world.join("notes.txt"), b"banana").unwrap();
    fs::write(world.join("docs/a.txt"), b"hello\n").unwrap();
    let archive = base.join("world.wcap");
    let restored = base.join("restored");

    let saved = run_capsule_command(
        parse_capsule_command(&os(&[
            "save",
            world.to_str().unwrap(),
            archive.to_str().unwrap(),
        ]))
        .unwrap(),
    )
    .unwrap();
    let id = capsule_id(saved.stdout());
    assert_eq!(id.len(), 64, "expected a sha256 capsule id, got {id:?}");
    assert!(archive.is_file());

    let loaded = run_capsule_command(
        parse_capsule_command(&os(&[
            "load",
            archive.to_str().unwrap(),
            restored.to_str().unwrap(),
        ]))
        .unwrap(),
    )
    .unwrap();

    assert_eq!(fs::read(restored.join("notes.txt")).unwrap(), b"banana");
    assert_eq!(fs::read(restored.join("docs/a.txt")).unwrap(), b"hello\n");
    // Content-addressed: the restored world reports the same capsule id.
    assert_eq!(capsule_id(loaded.stdout()), id);

    fs::remove_dir_all(&base).ok();
}

#[test]
fn capsule_requires_save_or_load() {
    let error = parse_capsule_command(&os(&["frobnicate", "a", "b"])).unwrap_err();
    assert_eq!(error.exit_code(), 2);
}
