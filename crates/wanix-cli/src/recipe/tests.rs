use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;

use wanix_fs::{FileSystem, MemFs};

use super::command::{RecipeCommand, parse_recipe_command};
use super::exec::{run_run_in, run_save_in};
use super::{Recipe, RecipeBind, read_recipe, resolve_binds_in, write_recipe};
use crate::catalog::{CatalogEntry, register_served, write_entry};
use crate::mesh::mounts::test_support::serve_native;

fn args(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("wanix-recipe-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
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
fn parse_takes_save_and_run_and_rejects_malformed_invocations() {
    let saved = parse_recipe_command(&args(&[
        "save",
        "transcribe",
        "--mount",
        "notes=/vol/notes",
        "--mount",
        "whisper",
        "--run",
        "cat < /vol/notes/a.txt",
        "--description",
        "demo",
        "--force",
    ]))
    .unwrap();
    assert_eq!(
        saved,
        RecipeCommand::Save {
            name: "transcribe".to_owned(),
            description: Some("demo".to_owned()),
            run: Some("cat < /vol/notes/a.txt".to_owned()),
            mounts: vec!["notes=/vol/notes".to_owned(), "whisper".to_owned()],
            force: true,
        }
    );

    let run = parse_recipe_command(&args(&["run", "transcribe", "--", "extra", "words"])).unwrap();
    assert_eq!(
        run,
        RecipeCommand::Run {
            name: "transcribe".to_owned(),
            extra: vec!["extra".to_owned(), "words".to_owned()],
        }
    );

    for bad in [
        vec![],                              // no subcommand
        vec!["bogus"],                       // unknown subcommand
        vec!["save"],                        // missing name
        vec!["save", "ok"],                  // no --mount at all
        vec!["save", "Bad", "--mount", "a"], // bad recipe name
        vec!["save", "ok", "--mount"],       // flag missing value
        vec!["run"],                         // missing name
        vec!["run", "a", "b"],               // extra operand outside --
        vec!["run", "iroh://abc"],           // a ticket is never a recipe name
    ] {
        assert!(
            parse_recipe_command(&args(&bad)).is_err(),
            "{bad:?} should be a usage error"
        );
    }
}

/// Pins the on-disk recipe artifact: TOML at `<dir>/<name>.recipe`, a NAME
/// target carrying the address it resolved to at save time as `hint`, an
/// address target carrying none, and paths stored normalized.
#[test]
fn save_records_resolved_hints_with_pinned_file_format() {
    let recipes = temp_dir("save-format");
    let catalog = temp_dir("save-format-catalog");
    let host = Arc::new(MemFs::new());
    let (server, url) = serve_native(host as Arc<dyn FileSystem>, 240);
    write_entry(&catalog, &entry("notes", &url), false).unwrap();
    let direct = format!(
        "iroh://{}",
        wanix_id::NodeIdentity::from_secret_bytes([241u8; 32])
            .peer_id()
            .to_hex()
    );

    let output = run_save_in(
        &recipes,
        &catalog,
        "transcribe".to_owned(),
        Some("demo recipe".to_owned()),
        Some("cat < /vol/notes/a.txt".to_owned()),
        &["notes=/vol/notes".to_owned(), format!("{direct}=/vol/raw")],
        false,
    )
    .unwrap();
    let stdout = String::from_utf8(output.stdout().to_vec()).unwrap();
    assert!(stdout.contains("saved recipe transcribe"), "{stdout}");
    assert!(
        stdout.contains(&format!("bind notes -> /vol/notes (hint {url})")),
        "{stdout}"
    );

    let raw = std::fs::read_to_string(recipes.join("transcribe.recipe")).unwrap();
    let expected = format!(
        "name = \"transcribe\"\ndescription = \"demo recipe\"\n\
         run = \"cat < /vol/notes/a.txt\"\n\n\
         [[binds]]\ntarget = \"notes\"\npath = \"vol/notes\"\nhint = \"{url}\"\n\n\
         [[binds]]\ntarget = \"{direct}\"\npath = \"vol/raw\"\n"
    );
    assert_eq!(raw, expected);

    // Round trip through the reader.
    let recipe = read_recipe(&recipes, "transcribe").unwrap();
    assert_eq!(recipe.binds[0].hint.as_deref(), Some(url.as_str()));
    assert_eq!(recipe.binds[1].hint, None);

    // Overwrite is refused without --force, like catalog add.
    let refused = run_save_in(
        &recipes,
        &catalog,
        "transcribe".to_owned(),
        None,
        None,
        &["notes".to_owned()],
        false,
    )
    .unwrap_err();
    assert!(refused.to_string().contains("--force"), "{refused}");

    drop(server);
    let _ = std::fs::remove_dir_all(&recipes);
    let _ = std::fs::remove_dir_all(&catalog);
}

#[test]
fn save_rejects_unknown_names_and_pathless_ticket_targets() {
    let recipes = temp_dir("save-reject");
    let catalog = temp_dir("save-reject-catalog");

    // A NAME target must resolve at save time (the hint is recorded then).
    let unknown = run_save_in(
        &recipes,
        &catalog,
        "r".to_owned(),
        None,
        None,
        &["absent=/vol".to_owned()],
        false,
    )
    .unwrap_err();
    assert!(
        unknown.to_string().contains("no catalog entry \"absent\""),
        "{unknown}"
    );
    assert!(
        unknown.to_string().contains("catalog add absent"),
        "{unknown}"
    );

    // A ticket target has no name to derive a default path from.
    let pathless = run_save_in(
        &recipes,
        &catalog,
        "r".to_owned(),
        None,
        None,
        &["iroh://abc".to_owned()],
        false,
    )
    .unwrap_err();
    assert!(
        pathless.to_string().contains("needs an explicit =PATH"),
        "{pathless}"
    );

    let _ = std::fs::remove_dir_all(&recipes);
    let _ = std::fs::remove_dir_all(&catalog);
}

/// The end-to-end proof: two independently served resources are registered in
/// the catalog by the `--register` machinery, a recipe binds both BY NAME with
/// one run line, and `recipe run` composes the mounts and produces output that
/// crossed both mounts (plus `--` words appended to the line).
#[test]
fn recipe_run_composes_two_named_resources_and_runs_the_line() {
    let notes = Arc::new(MemFs::new());
    notes.write_file("a.txt", b"note-a\n").unwrap();
    let photos = Arc::new(MemFs::new());
    photos.write_file("b.txt", b"photo-b\n").unwrap();
    let (notes_server, notes_url) = serve_native(notes.clone() as Arc<dyn FileSystem>, 242);
    let (photos_server, photos_url) = serve_native(photos.clone() as Arc<dyn FileSystem>, 243);

    let recipes = temp_dir("run-e2e");
    let catalog = temp_dir("run-e2e-catalog");
    register_served(
        &catalog,
        "notes",
        "volume",
        &[("notes".to_owned(), notes_url)],
    )
    .unwrap();
    register_served(
        &catalog,
        "photos",
        "volume",
        &[("photos".to_owned(), photos_url)],
    )
    .unwrap();

    run_save_in(
        &recipes,
        &catalog,
        "gather".to_owned(),
        None,
        Some("cat < /vol/notes/a.txt; cat < /vol/photos/b.txt; echo".to_owned()),
        &[
            "notes=/vol/notes".to_owned(),
            "photos=/vol/photos".to_owned(),
        ],
        false,
    )
    .unwrap();

    let output = run_run_in(
        &recipes,
        &catalog,
        "gather",
        &["done".to_owned()],
        &mut std::io::empty(),
    )
    .unwrap();
    assert_eq!(
        output.exit_code(),
        0,
        "stderr: {}",
        String::from_utf8_lossy(output.stderr())
    );
    // Output crossed BOTH name-resolved mounts, and the `--` word landed on
    // the run line (`echo done`).
    assert_eq!(output.stdout(), b"note-a\nphoto-b\ndone\n");
    // The launch-time resolutions are on the audit trail.
    let stderr = String::from_utf8_lossy(output.stderr()).into_owned();
    assert!(stderr.contains("name 'notes' ->"), "{stderr}");
    assert!(stderr.contains("name 'photos' ->"), "{stderr}");
    assert!(!stderr.contains("DRIFTED"), "{stderr}");

    drop(notes_server);
    drop(photos_server);
    let _ = std::fs::remove_dir_all(&recipes);
    let _ = std::fs::remove_dir_all(&catalog);
}

/// Drift (ADR 0007 open question 6): the catalog is authoritative at launch,
/// and a catalog that moved since save warns loudly while dialing the NEW
/// address — proven by content only the re-registered resource serves.
#[test]
fn recipe_run_warns_on_hint_drift_and_uses_the_catalog_address() {
    let old = Arc::new(MemFs::new());
    old.write_file("x.txt", b"old\n").unwrap();
    let new = Arc::new(MemFs::new());
    new.write_file("x.txt", b"new\n").unwrap();
    let (old_server, old_url) = serve_native(old as Arc<dyn FileSystem>, 244);
    let (new_server, new_url) = serve_native(new as Arc<dyn FileSystem>, 245);

    let recipes = temp_dir("drift");
    let catalog = temp_dir("drift-catalog");
    write_entry(&catalog, &entry("vol", &old_url), false).unwrap();
    // A bare NAME mount defaults to n/NAME, so the run line reads /n/vol/.
    run_save_in(
        &recipes,
        &catalog,
        "drifty".to_owned(),
        None,
        Some("cat < /n/vol/x.txt".to_owned()),
        &["vol".to_owned()],
        false,
    )
    .unwrap();
    // The hint pinned the old address; a re-register (always-overwrite) moves
    // the name to the new resource.
    let saved = read_recipe(&recipes, "drifty").unwrap();
    assert_eq!(saved.binds[0].hint.as_deref(), Some(old_url.as_str()));
    assert_eq!(saved.binds[0].path, "n/vol");
    write_entry(&catalog, &entry("vol", &new_url), true).unwrap();

    let output = run_run_in(&recipes, &catalog, "drifty", &[], &mut std::io::empty()).unwrap();
    let stderr = String::from_utf8_lossy(output.stderr()).into_owned();
    assert!(stderr.contains("DRIFTED since save"), "{stderr}");
    assert!(stderr.contains(&old_url), "{stderr}");
    assert!(stderr.contains(&new_url), "{stderr}");
    // The catalog won: the bytes are the re-registered resource's, not the
    // saved hint's.
    assert_eq!(output.stdout(), b"new\n", "stderr: {stderr}");

    drop(old_server);
    drop(new_server);
    let _ = std::fs::remove_dir_all(&recipes);
    let _ = std::fs::remove_dir_all(&catalog);
}

/// A name that vanished from the catalog falls back to the saved hint with a
/// loud warning; one with neither entry nor hint is a clear error.
#[test]
fn resolve_binds_falls_back_to_the_hint_when_the_entry_vanished() {
    let catalog = temp_dir("fallback-catalog");
    let hint = format!(
        "iroh://{}",
        wanix_id::NodeIdentity::from_secret_bytes([246u8; 32])
            .peer_id()
            .to_hex()
    );
    let recipe = Recipe {
        name: "ghost".to_owned(),
        description: None,
        run: Some("true".to_owned()),
        binds: vec![RecipeBind {
            target: "gone".to_owned(),
            path: "vol".to_owned(),
            hint: Some(hint.clone()),
        }],
    };
    let (mounts, notes) = resolve_binds_in(&catalog, &recipe).unwrap();
    assert_eq!(mounts.len(), 1);
    assert_eq!(mounts[0].addr, hint);
    assert!(
        notes
            .iter()
            .any(|note| note.contains("no longer in the catalog")),
        "{notes:?}"
    );

    let hintless = Recipe {
        binds: vec![RecipeBind {
            target: "gone".to_owned(),
            path: "vol".to_owned(),
            hint: None,
        }],
        ..recipe
    };
    let error = resolve_binds_in(&catalog, &hintless).unwrap_err();
    assert!(error.to_string().contains("no catalog entry"), "{error}");
    assert!(error.to_string().contains("catalog add gone"), "{error}");

    let _ = std::fs::remove_dir_all(&catalog);
}

/// A run-less recipe is an interactive session: the collected path refuses it
/// with a pointer at the tty binary, mirroring interactive `sh`.
#[test]
fn run_less_recipe_is_refused_on_the_collected_path() {
    let recipes = temp_dir("interactive-refusal");
    let catalog = temp_dir("interactive-refusal-catalog");
    write_recipe(
        &recipes,
        &Recipe {
            name: "shell".to_owned(),
            description: None,
            run: None,
            binds: Vec::new(),
        },
        false,
    )
    .unwrap();

    let error = run_run_in(&recipes, &catalog, "shell", &[], &mut std::io::empty()).unwrap_err();
    assert_eq!(error.exit_code(), 2);
    assert!(error.to_string().contains("interactive shell"), "{error}");

    let missing = run_run_in(&recipes, &catalog, "absent", &[], &mut std::io::empty()).unwrap_err();
    assert!(
        missing.to_string().contains("no recipe \"absent\""),
        "{missing}"
    );
    assert!(
        missing.to_string().contains("recipe save absent"),
        "{missing}"
    );

    let _ = std::fs::remove_dir_all(&recipes);
    let _ = std::fs::remove_dir_all(&catalog);
}
