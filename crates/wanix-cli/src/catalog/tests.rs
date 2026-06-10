use std::ffi::OsString;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use wanix_fs::{FileSystem, MemFs};
use wanix_id::NodeIdentity;
use wanix_mesh::NativeServeConfig;

use super::command::{
    CatalogCommand, LsProbe, parse_catalog_command, run_add_in, run_ls_in, run_rm_in, run_show_in,
};
use super::{
    CatalogEntry, register_served, resolve_name_in, validate_catalog_address,
    validate_catalog_name, write_entry,
};
use crate::mesh::resource::bind_endpoints;

fn args(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("wanix-catalog-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A real, on-curve peer id hex nobody serves (iroh validates the curve point).
fn peer_hex(seed: u8) -> String {
    NodeIdentity::from_secret_bytes([seed; 32])
        .peer_id()
        .to_hex()
}

fn ticket(seed: u8) -> String {
    format!("iroh://{}?addr=127.0.0.1:1", peer_hex(seed))
}

fn entry(name: &str, address: &str) -> CatalogEntry {
    CatalogEntry {
        name: name.to_owned(),
        description: None,
        tags: Vec::new(),
        address: address.to_owned(),
    }
}

#[test]
fn name_grammar_accepts_humane_names_and_rejects_ticket_and_path_shapes() {
    for good in ["front-door", "notes", "whisper-stt", "a", "x0", "2nd-disk"] {
        validate_catalog_name(good).unwrap_or_else(|error| panic!("{good}: {error}"));
    }
    for bad in [
        "",
        "Front-Door",      // uppercase
        "a.b",             // dots (could be a path or file)
        "a/b",             // slashes (a path)
        "home/front-door", // scoped form is for later catalogs, not entry names
        "iroh://abc",      // a ticket is never a name
        "cas:abc",         // a cas ref is never a name
        "-leading",
        "trailing-",
        "with space",
        "under_score",
    ] {
        let error = validate_catalog_name(bad).unwrap_err();
        assert_eq!(error.exit_code(), 2, "{bad:?} must be a usage error");
        assert!(
            error.to_string().contains("invalid catalog name"),
            "{bad:?}: {error}"
        );
    }
}

#[test]
fn address_validation_takes_iroh_tickets_only_and_names_future_kinds() {
    validate_catalog_address(&ticket(235)).unwrap();
    validate_catalog_address(&format!("iroh://{}", peer_hex(235))).unwrap();

    for (address, expect) in [
        ("cas:sha256:abc", "future address kind"),
        ("local:/vol/data", "future address kind"),
        ("tcp://127.0.0.1:5640", "must start with iroh://"),
        ("front-door", "must start with iroh://"),
        ("iroh://deadbeef", "64 hex digits"),
    ] {
        let error = validate_catalog_address(address).unwrap_err();
        assert!(error.to_string().contains(expect), "{address}: {error}");
    }
}

#[test]
fn add_show_rm_round_trip_with_pinned_file_format() {
    let dir = temp_dir("crud");
    let address = ticket(235);
    let parsed = parse_catalog_command(&args(&[
        "add",
        "front-door",
        &address,
        "--description",
        "the home gateway",
        "--tags",
        "gateway,home",
    ]))
    .unwrap();
    let CatalogCommand::Add { entry, force } = parsed else {
        panic!("expected add");
    };
    assert!(!force);
    let output = run_add_in(&dir, &entry, force).unwrap();
    assert_eq!(output.exit_code(), 0);
    let stdout = String::from_utf8(output.stdout().to_vec()).unwrap();
    assert!(
        stdout.contains("added catalog entry front-door"),
        "{stdout}"
    );
    assert!(stdout.contains(&address), "{stdout}");

    // Pin the storage contract: one JSON file per entry at <dir>/<name>.json
    // with exactly these keys (the Names phase parses this shape).
    let raw = std::fs::read(dir.join("front-door.json")).unwrap();
    let json: serde_json::Value = serde_json::from_slice(&raw).unwrap();
    assert_eq!(
        json,
        serde_json::json!({
            "name": "front-door",
            "description": "the home gateway",
            "tags": ["gateway", "home"],
            "address": address,
        })
    );

    let shown =
        String::from_utf8(run_show_in(&dir, "front-door").unwrap().stdout().to_vec()).unwrap();
    assert_eq!(
        shown,
        format!(
            "name: front-door\naddress: {address}\ndescription: the home gateway\n\
             tags: gateway, home\n"
        )
    );

    run_rm_in(&dir, "front-door").unwrap();
    assert!(!dir.join("front-door.json").exists());
    let missing = run_show_in(&dir, "front-door").unwrap_err();
    assert!(
        missing
            .to_string()
            .contains("no catalog entry \"front-door\""),
        "{missing}"
    );
    assert!(
        missing.to_string().contains("catalog add front-door"),
        "{missing}"
    );
    let nothing = run_rm_in(&dir, "front-door").unwrap_err();
    assert!(
        nothing.to_string().contains("nothing to remove"),
        "{nothing}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn optional_fields_are_omitted_from_the_entry_file() {
    let dir = temp_dir("minimal");
    let address = ticket(235);
    write_entry(&dir, &entry("notes", &address), false).unwrap();
    let raw = std::fs::read(dir.join("notes.json")).unwrap();
    let json: serde_json::Value = serde_json::from_slice(&raw).unwrap();
    assert_eq!(
        json,
        serde_json::json!({ "name": "notes", "address": address })
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn add_refuses_overwrite_without_force() {
    let dir = temp_dir("overwrite");
    let first = ticket(235);
    let second = format!("iroh://{}", peer_hex(237));
    write_entry(&dir, &entry("notes", &first), false).unwrap();

    let refused = write_entry(&dir, &entry("notes", &second), false).unwrap_err();
    assert_eq!(refused.exit_code(), 2);
    let message = refused.to_string();
    assert!(message.contains("already exists"), "{message}");
    assert!(
        message.contains(&first),
        "the refusal names the current address: {message}"
    );
    assert!(message.contains("--force"), "{message}");
    assert_eq!(resolve_name_in(&dir, "notes").unwrap(), first);

    write_entry(&dir, &entry("notes", &second), true).unwrap();
    assert_eq!(resolve_name_in(&dir, "notes").unwrap(), second);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn resolve_name_is_the_naming_seam_and_never_takes_a_ticket() {
    let dir = temp_dir("resolve");
    let address = ticket(235);
    write_entry(&dir, &entry("notes", &address), false).unwrap();
    assert_eq!(resolve_name_in(&dir, "notes").unwrap(), address);

    // A ticket or a path is never a name: resolution rejects them on grammar
    // before touching the catalog, so callers can route on spelling alone.
    for not_a_name in ["iroh://abc", "/vol/data", "cas:sha256:abc"] {
        let error = resolve_name_in(&dir, not_a_name).unwrap_err();
        assert!(
            error.to_string().contains("invalid catalog name"),
            "{not_a_name}: {error}"
        );
    }
    let missing = resolve_name_in(&dir, "absent").unwrap_err();
    assert!(
        missing.to_string().contains("no catalog entry"),
        "{missing}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn parse_rejects_malformed_invocations() {
    for bad in [
        vec![],                                       // no subcommand
        vec!["bogus"],                                // unknown subcommand
        vec!["add", "name-only"],                     // missing address
        vec!["add", "a", "b", "c"],                   // extra positional
        vec!["add", "UPPER", "iroh://x"],             // bad name
        vec!["add", "ok", "tcp://1.2.3.4:1"],         // bad address
        vec!["add", "ok", "iroh://x", "--tags"],      // flag missing value
        vec!["add", "ok", "iroh://x", "--tags", ","], // empty tag list
        vec!["show"],                                 // missing name
        vec!["show", "a", "b"],                       // extra operand
        vec!["rm"],                                   // missing name
        vec!["ls", "--bogus"],                        // unknown flag
    ] {
        assert!(
            parse_catalog_command(&args(&bad)).is_err(),
            "{bad:?} should be a usage error"
        );
    }
}

#[test]
fn ls_without_probe_lists_instantly_and_sorted() {
    let dir = temp_dir("ls-no-probe");
    let b = ticket(235);
    let a = format!("iroh://{}", peer_hex(237));
    write_entry(&dir, &entry("zeta", &b), false).unwrap();
    write_entry(&dir, &entry("alpha", &a), false).unwrap();

    let output = run_ls_in(&dir, None).unwrap();
    let stdout = String::from_utf8(output.stdout().to_vec()).unwrap();
    assert_eq!(stdout, format!("alpha\t-\t{a}\nzeta\t-\t{b}\n"));

    // An empty catalog lists as nothing, not an error.
    let empty = temp_dir("ls-empty");
    assert!(run_ls_in(&empty, None).unwrap().stdout().is_empty());
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&empty);
}

/// The liveness proof: one live loopback serve and one dead entry, probed
/// concurrently within one bounded deadline — the live entry renders `online`,
/// the dead one `offline` (the pre-ACL outage shape, ADR 0008).
#[test]
fn ls_probes_render_online_and_offline_within_the_deadline() {
    use std::time::Instant;

    let dir = temp_dir("ls-probe");
    let root: Arc<dyn FileSystem> = Arc::new(MemFs::new());
    let served = bind_endpoints(
        "volume",
        vec![(
            "live".to_owned(),
            NodeIdentity::from_secret_bytes([231u8; 32]),
            NativeServeConfig::open(root),
        )],
        Some(SocketAddr::new(Ipv4Addr::LOCALHOST.into(), 0)),
    )
    .unwrap();
    write_entry(&dir, &entry("live", &served[0].ticket_url), false).unwrap();
    // A real on-curve peer nobody serves, with a dead direct-route hint so the
    // probe never depends on outside-network behavior.
    write_entry(&dir, &entry("dead", &ticket(236)), false).unwrap();

    let probe = LsProbe {
        identity: NodeIdentity::generate().unwrap(),
        deadline: Duration::from_secs(3),
    };
    let started = Instant::now();
    let output = run_ls_in(&dir, Some(&probe)).unwrap();
    // Concurrent probes: the listing is bounded by one deadline plus generous
    // slack, never the per-entry sum and never an indefinite hang.
    assert!(
        started.elapsed() < Duration::from_secs(15),
        "ls took {:?}",
        started.elapsed()
    );
    let stdout = String::from_utf8(output.stdout().to_vec()).unwrap();
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 2, "{stdout}");
    assert!(
        lines[0].starts_with("dead\toffline\t"),
        "the unserved entry is an outage, not an error: {stdout}"
    );
    assert!(lines[1].starts_with("live\tonline\t"), "{stdout}");

    drop(served);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn probe_failures_render_as_unknown_with_a_single_line_detail() {
    use crate::mesh::{ProbeOutcome, probe_iroh};

    // A malformed address in a hand-edited entry file is "unknown", not a
    // crash, and the detail stays on one line for the tab-separated listing.
    let identity = NodeIdentity::generate().unwrap();
    let outcome = probe_iroh(&identity, "iroh://deadbeef", Duration::from_millis(100));
    let ProbeOutcome::Unknown(detail) = outcome else {
        panic!("a malformed ticket must probe as unknown, got {outcome:?}");
    };
    assert!(detail.contains("64 hex digits"), "{detail}");
    assert!(!detail.contains('\n'), "{detail}");
}

/// `--register` end to end: one live serve binds an endpoint, registration
/// writes the announced ticket under the registered name, and the naming seam
/// resolves it back — the address a launch-time mount would dial.
#[test]
fn register_served_writes_the_announced_ticket_for_one_serve() {
    let dir = temp_dir("register");
    let root: Arc<dyn FileSystem> = Arc::new(MemFs::new());
    let served = bind_endpoints(
        "volume",
        vec![(
            "notes".to_owned(),
            NodeIdentity::from_secret_bytes([232u8; 32]),
            NativeServeConfig::open(root),
        )],
        Some(SocketAddr::new(Ipv4Addr::LOCALHOST.into(), 0)),
    )
    .unwrap();
    let endpoints: Vec<(String, String)> = served
        .iter()
        .map(|endpoint| (endpoint.name.clone(), endpoint.ticket_url.clone()))
        .collect();

    let lines = register_served(&dir, "my-notes", "volume", &endpoints).unwrap();
    assert_eq!(lines.len(), 1);
    assert!(
        lines[0].starts_with("# registered catalog entry my-notes -> iroh://"),
        "registration lines are comment records: {}",
        lines[0]
    );

    let written = super::read_entry(&dir, "my-notes").unwrap();
    assert_eq!(written.address, served[0].ticket_url);
    assert_eq!(written.tags, vec!["volume".to_owned()]);
    assert_eq!(
        resolve_name_in(&dir, "my-notes").unwrap(),
        served[0].ticket_url
    );

    // Re-announcing updates the entry in place (no --force dance): a restarted
    // serve has a fresh route hint and the name must follow it.
    let moved = vec![("notes".to_owned(), format!("iroh://{}", peer_hex(238)))];
    register_served(&dir, "my-notes", "volume", &moved).unwrap();
    assert_eq!(resolve_name_in(&dir, "my-notes").unwrap(), moved[0].1);

    drop(served);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn register_served_prefixes_names_when_a_serve_announces_several_resources() {
    let dir = temp_dir("register-multi");
    let endpoints = vec![
        ("notes".to_owned(), ticket(235)),
        ("photos".to_owned(), format!("iroh://{}", peer_hex(237))),
    ];
    let lines = register_served(&dir, "media", "volume", &endpoints).unwrap();
    assert_eq!(lines.len(), 2);
    assert_eq!(
        resolve_name_in(&dir, "media-notes").unwrap(),
        endpoints[0].1
    );
    assert_eq!(
        resolve_name_in(&dir, "media-photos").unwrap(),
        endpoints[1].1
    );

    // A resource whose name breaks the catalog grammar fails loudly instead of
    // writing a sneaky almost-name.
    let bad = vec![
        ("ok".to_owned(), ticket(235)),
        ("My_Vol".to_owned(), ticket(237)),
    ];
    let error = register_served(&dir, "data", "volume", &bad).unwrap_err();
    assert!(
        error.to_string().contains("invalid catalog name"),
        "{error}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
