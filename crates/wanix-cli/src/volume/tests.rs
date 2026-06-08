use std::ffi::OsString;
use std::path::PathBuf;

use super::{
    VolumeCommand, create_volume_in, list_volumes_in, parse_volume_command,
    resolve_existing_volume, validate_volume_name,
};

fn args(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}

/// A hermetic per-test volumes root under the system temp dir (no `tempfile`
/// dev-dep; each test uses a distinct tag so parallel tests do not collide).
fn temp_root(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("wanix-voltest-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn parse_volume_create_and_ls() {
    assert_eq!(
        parse_volume_command(&args(&["create", "notes"])).unwrap(),
        VolumeCommand::Create {
            name: "notes".to_owned()
        }
    );
    assert_eq!(
        parse_volume_command(&args(&["ls"])).unwrap(),
        VolumeCommand::Ls
    );
}

#[test]
fn parse_volume_rejects_bad_shapes() {
    for bad in [
        vec![],
        vec!["bogus"],
        vec!["create"],
        vec!["create", "a", "b"],
        vec!["create", "../escape"],
        vec!["ls", "extra"],
    ] {
        assert!(
            parse_volume_command(&args(&bad)).is_err(),
            "{bad:?} should be a usage error"
        );
    }
}

#[test]
fn volume_name_validation_accepts_and_rejects() {
    for good in ["notes", "photos", "my-notes", "v1.0", "a", "A_B-c.9"] {
        validate_volume_name(good).unwrap();
    }
    for bad in [
        "", "..", ".", "../x", "a/b", "a\\b", ".hidden", "-x", "x-", "x.", "na me", "💾",
    ] {
        assert!(
            validate_volume_name(bad).is_err(),
            "{bad:?} should be rejected"
        );
    }
}

#[test]
fn create_errors_on_existing_and_ls_lists_sorted() {
    let root = temp_root("create-ls");

    // Empty root lists as nothing.
    assert_eq!(list_volumes_in(&root).unwrap().stdout(), b"");

    create_volume_in(&root, "photos").unwrap();
    create_volume_in(&root, "notes").unwrap();
    assert!(root.join("notes").is_dir());
    assert!(root.join("photos").is_dir());

    // create is NOT idempotent: re-creating an existing volume is an error.
    let error = create_volume_in(&root, "notes").unwrap_err();
    assert_eq!(error.exit_code(), 1);
    assert!(error.to_string().contains("already exists"));

    // ls is sorted, newline-terminated.
    assert_eq!(list_volumes_in(&root).unwrap().stdout(), b"notes\nphotos\n");

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn resolve_existing_volume_finds_present_and_rejects_missing() {
    let root = temp_root("resolve");
    create_volume_in(&root, "notes").unwrap();

    assert_eq!(
        resolve_existing_volume(&root, "notes").unwrap(),
        root.join("notes")
    );

    let missing = resolve_existing_volume(&root, "ghost").unwrap_err();
    assert_eq!(missing.exit_code(), 1);
    assert!(missing.to_string().contains("does not exist"));

    // An invalid name is rejected before any filesystem lookup (usage error).
    let invalid = resolve_existing_volume(&root, "../escape").unwrap_err();
    assert_eq!(invalid.exit_code(), 2);

    let _ = std::fs::remove_dir_all(&root);
}
