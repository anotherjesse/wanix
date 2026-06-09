use std::ffi::OsString;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;

use wanix_fs::{FileSystem, NormalizedPath, OpenOptions};
use wanix_id::NodeIdentity;
use wanix_vfs::{BindOptions, Namespace};

use super::{
    VolumeSelection, bind_volume_endpoints, parse_volume_serve_command, reject_fixed_port_multi,
    volume_serve_example_line, volume_serve_line,
};

fn args(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("wanix-volserve-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn loopback() -> SocketAddr {
    SocketAddr::new(Ipv4Addr::LOCALHOST.into(), 0)
}

fn write_through(namespace: &Namespace, path: &str, bytes: &[u8]) {
    let mut file = namespace
        .open(
            &NormalizedPath::new(path).unwrap(),
            OpenOptions {
                read: false,
                write: true,
                create: true,
                truncate: true,
            },
        )
        .unwrap();
    assert_eq!(file.write(bytes).unwrap(), bytes.len());
}

#[test]
fn parse_accepts_explicit_volumes_and_all() {
    let explicit = parse_volume_serve_command(&args(&[
        "--volume",
        "notes",
        "--volume",
        "photos",
        "--addr",
        "127.0.0.1:0",
    ]))
    .unwrap();
    assert_eq!(
        explicit.selection,
        VolumeSelection::Explicit(vec!["notes".to_owned(), "photos".to_owned()])
    );

    let all = parse_volume_serve_command(&args(&["--all", "--insecure-open"])).unwrap();
    assert_eq!(all.selection, VolumeSelection::All);
}

#[test]
fn parse_allows_single_volume_on_a_fixed_port() {
    // A fixed nonzero port is fine for ONE volume — the multi-volume rule does not
    // apply.
    parse_volume_serve_command(&args(&["--volume", "notes", "--addr", "127.0.0.1:8080"])).unwrap();
}

#[test]
fn parse_enforces_public_endpoint_posture() {
    // No --addr and no --insecure-open is a public endpoint: refused (matches
    // mesh-serve). The check is at parse time, so the collected path refuses it too.
    let error = parse_volume_serve_command(&args(&["--volume", "notes"])).unwrap_err();
    assert_eq!(error.exit_code(), 2);
    assert!(error.to_string().contains("public endpoint"), "got {error}");

    // --insecure-open is the explicit public opt-in; a local --addr is also fine.
    parse_volume_serve_command(&args(&["--volume", "notes", "--insecure-open"])).unwrap();
    parse_volume_serve_command(&args(&["--all", "--insecure-open"])).unwrap();
    parse_volume_serve_command(&args(&["--volume", "notes", "--addr", "127.0.0.1:0"])).unwrap();
}

#[test]
fn fixed_port_rule_rejects_multiple_volumes() {
    let fixed: SocketAddr = "127.0.0.1:8080".parse().unwrap();
    // The rule that backs `--all` at runtime (count known only after resolving):
    // a fixed nonzero port cannot serve more than one volume.
    assert!(reject_fixed_port_multi(Some(fixed), 2).is_err());
    // One volume on a fixed port, port 0 for many, or a public endpoint are fine.
    assert!(reject_fixed_port_multi(Some(fixed), 1).is_ok());
    assert!(reject_fixed_port_multi("127.0.0.1:0".parse().ok(), 2).is_ok());
    assert!(reject_fixed_port_multi(None, 2).is_ok());
}

#[test]
fn volume_serve_line_is_a_stable_tab_record() {
    // Pinned so a later catalog-register step can parse it: NAME\tTICKET\n.
    assert_eq!(
        volume_serve_line("notes", "iroh://abc?addr=127.0.0.1:5610"),
        "notes\tiroh://abc?addr=127.0.0.1:5610\n"
    );
}

#[test]
fn volume_serve_example_line_is_a_copy_pasteable_comment() {
    // The human-facing example is a `# ` comment (record parsers skip it) and
    // pastes directly into the matching client command.
    let line = volume_serve_example_line("iroh://abc?addr=127.0.0.1:5610");
    assert!(line.starts_with("# "), "{line}");
    assert!(!line.contains('\t'), "{line}");
    assert!(
        line.contains("wanix-rust mount-ls 'iroh://abc?addr=127.0.0.1:5610'"),
        "{line}"
    );
}

#[test]
fn parse_rejects_bad_combinations() {
    for bad in [
        vec![],                                                             // nothing selected
        vec!["--all", "--volume", "notes"], // --all mixed with --volume
        vec!["--volume", "notes", "--volume", "notes"], // duplicate
        vec!["--volume", "../escape"],      // invalid name
        vec!["--volume", "a", "--volume", "b", "--addr", "127.0.0.1:8080"], // fixed port, multi
        vec!["--bogus"],                    // unknown flag
    ] {
        assert!(
            parse_volume_serve_command(&args(&bad)).is_err(),
            "{bad:?} should be a usage error"
        );
    }
}

/// The core slice proof: one process binds an independent endpoint per volume
/// with a distinct identity, and each endpoint serves exactly its own volume
/// root (no aggregate). Writes through each ticket stay scoped to their volume.
#[test]
fn serves_each_volume_as_an_independent_scoped_endpoint() {
    let notes_dir = temp_dir("notes");
    let photos_dir = temp_dir("photos");
    let volumes = vec![
        (
            "notes".to_owned(),
            notes_dir.clone(),
            NodeIdentity::from_secret_bytes([7u8; 32]),
        ),
        (
            "photos".to_owned(),
            photos_dir.clone(),
            NodeIdentity::from_secret_bytes([8u8; 32]),
        ),
    ];

    let served = bind_volume_endpoints(volumes, Some(loopback())).unwrap();
    assert_eq!(served.len(), 2);
    // Distinct peer ids: each volume is an independent mesh resource/ticket.
    assert_ne!(served[0].node.peer_id(), served[1].node.peer_id());

    // Mount each ticket through the production client into its own namespace.
    let notes_mount = crate::mesh::dial_iroh_remote(&served[0].ticket_url, "").unwrap();
    let mut notes_ns = Namespace::new();
    notes_ns
        .bind(notes_mount.remote.clone(), ".", "x", BindOptions::default())
        .unwrap();
    let photos_mount = crate::mesh::dial_iroh_remote(&served[1].ticket_url, "").unwrap();
    let mut photos_ns = Namespace::new();
    photos_ns
        .bind(
            photos_mount.remote.clone(),
            ".",
            "x",
            BindOptions::default(),
        )
        .unwrap();

    write_through(&notes_ns, "x/a.txt", b"note-a");
    write_through(&photos_ns, "x/b.txt", b"photo-b");

    // Writes are scoped to their own on-disk volume directory.
    assert_eq!(std::fs::read(notes_dir.join("a.txt")).unwrap(), b"note-a");
    assert_eq!(std::fs::read(photos_dir.join("b.txt")).unwrap(), b"photo-b");
    assert!(
        !notes_dir.join("b.txt").exists(),
        "a photos write must not appear in the notes volume"
    );
    assert!(
        !photos_dir.join("a.txt").exists(),
        "a notes write must not appear in the photos volume"
    );

    // Regression: each endpoint root IS the single volume, not an aggregate — the
    // notes endpoint lists only its own entry, with no sibling-volume names.
    let entries: Vec<String> = notes_ns
        .read_dir(&NormalizedPath::new("x").unwrap())
        .unwrap()
        .into_iter()
        .map(|entry| entry.name().to_owned())
        .collect();
    assert_eq!(entries, vec!["a.txt".to_owned()]);

    drop(notes_mount);
    drop(photos_mount);
    drop(served);
    let _ = std::fs::remove_dir_all(&notes_dir);
    let _ = std::fs::remove_dir_all(&photos_dir);
}
