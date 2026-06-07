use std::time::{SystemTime, UNIX_EPOCH};

use super::{NodeIdentity, NodeIdentityError, SECRET_KEY_LEN};

fn temp_path(name: &str) -> std::path::PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("wanix-id-{name}-{}-{nanos}", std::process::id()))
}

#[test]
fn secret_bytes_round_trip() {
    let identity = NodeIdentity::generate().unwrap();
    let secret = identity.to_secret_bytes();
    assert_eq!(secret.len(), SECRET_KEY_LEN);

    let restored = NodeIdentity::from_secret_bytes(secret);
    assert_eq!(restored.to_secret_bytes(), secret);
    assert_eq!(restored.peer_id(), identity.peer_id());
    assert_eq!(
        restored.verifying_key().to_bytes(),
        identity.verifying_key().to_bytes()
    );
}

#[test]
fn peer_id_is_the_public_key() {
    let identity = NodeIdentity::from_secret_bytes([7u8; SECRET_KEY_LEN]);
    assert_eq!(
        identity.peer_id().as_bytes(),
        &identity.verifying_key().to_bytes()
    );
}

#[test]
fn load_or_create_is_stable_across_restarts() {
    let path = temp_path("stable");
    let _ = std::fs::remove_file(&path);

    let first = NodeIdentity::load_or_create(&path).unwrap();
    let again = NodeIdentity::load_or_create(&path).unwrap();
    assert_eq!(first.peer_id(), again.peer_id());
    assert_eq!(first.to_secret_bytes(), again.to_secret_bytes());

    std::fs::remove_file(&path).unwrap();
}

#[cfg(unix)]
#[test]
fn persisted_key_is_owner_private() {
    use std::os::unix::fs::PermissionsExt;

    let path = temp_path("perms");
    let _ = std::fs::remove_file(&path);

    let identity = NodeIdentity::load_or_create(&path).unwrap();
    let metadata = std::fs::metadata(&path).unwrap();
    assert_eq!(metadata.permissions().mode() & 0o777, 0o600);

    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(bytes, identity.to_secret_bytes());

    std::fs::remove_file(&path).unwrap();
}

#[test]
fn wrong_length_key_file_is_rejected() {
    let path = temp_path("badlen");
    std::fs::write(&path, b"too short").unwrap();

    let err = NodeIdentity::load_or_create(&path).unwrap_err();
    assert!(matches!(err, NodeIdentityError::BadLength(9)));

    std::fs::remove_file(&path).unwrap();
}

#[test]
fn distinct_generations_have_distinct_keys() {
    let a = NodeIdentity::generate().unwrap();
    let b = NodeIdentity::generate().unwrap();
    assert_ne!(a.peer_id(), b.peer_id());
}
