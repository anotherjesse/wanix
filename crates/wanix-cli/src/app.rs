//! `wanix app`: guest-defined AppResources served over the native mesh wire.
//!
//! An app *is* the resource (`docs/appfs.md`, ADR 0007 §"Worked example: a
//! chatroom"): `app serve` runs the app's qjs guest as a resident detached
//! task, adapts its newline-JSON stdin/stdout protocol through `wanix-appfs`
//! (the file2chan adapter), and exports the resulting `FileSystem` over one
//! native mesh endpoint — one ticket names one app instance. Durable state is
//! an explicit mounted directory (`--state`, visible to the guest at
//! `/state`), never guest memory. This module owns the tiny `app.wanix.json`
//! manifest and the per-app endpoint identity; the serve path lives in
//! [`serve`] and the guest task wiring in [`guest`].

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::CliError;
use crate::volume::wanix_dir;

mod guest;
mod serve;

pub(crate) use serve::{parse_app_serve_command, run_app_serve_streaming};

/// The manifest file name inside an `--app` directory.
pub(crate) const APP_MANIFEST_FILE: &str = "app.wanix.json";

/// The minimal v0 app manifest (`app.wanix.json`): the shared
/// `"wanix.resource"` envelope (docs/toolfs.md §Spec Shape) plus the runtime
/// entry point and the declared AppFS tree. `main` is a file path relative to
/// the app directory in v0; CAS-pinned `cas:` references and mount-approval
/// deployment (docs/appfs.md §Provenance) are explicitly deferred.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AppManifest {
    /// Shared resource envelope version; must be `"v0"`.
    #[serde(rename = "wanix.resource")]
    resource: String,
    /// Resource kind; must be `"app"`.
    kind: String,
    /// The app's name (the default serve/identity name).
    pub(crate) name: String,
    /// The guest runtime entry point.
    pub(crate) runtime: AppRuntime,
    /// Guest-handled file names (discrete ops routed to the guest).
    #[serde(default)]
    pub(crate) files: Vec<String>,
    /// Host-owned never-EOF stream file names (fed by guest publishes).
    #[serde(default)]
    pub(crate) streams: Vec<String>,
}

/// The manifest's runtime entry: which engine runs `main`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AppRuntime {
    /// Runtime kind; `"qjs"` is the only v0 runtime (rust-wasm later).
    pub(crate) kind: String,
    /// The entry script, relative to the app directory.
    pub(crate) main: String,
}

/// Loads and validates `app.wanix.json` from the app directory.
///
/// # Errors
///
/// Returns a CLI error when the manifest is missing, malformed, or declares
/// an unsupported envelope/runtime, with the offending field named.
pub(crate) fn load_app_manifest(app_dir: &Path) -> Result<AppManifest, CliError> {
    let path = app_dir.join(APP_MANIFEST_FILE);
    let raw = std::fs::read_to_string(&path).map_err(|error| {
        CliError::new(
            format!("app serve: cannot read {}: {error}", path.display()),
            1,
        )
    })?;
    let manifest: AppManifest = serde_json::from_str(&raw).map_err(|error| {
        CliError::new(format!("app serve: invalid {}: {error}", path.display()), 1)
    })?;
    validate_manifest(&manifest)
        .map_err(|message| CliError::new(format!("app serve: {}: {message}", path.display()), 1))?;
    Ok(manifest)
}

fn validate_manifest(manifest: &AppManifest) -> Result<(), String> {
    if manifest.resource != "v0" {
        return Err(format!(
            "unsupported \"wanix.resource\" {:?} (this build speaks v0)",
            manifest.resource
        ));
    }
    if manifest.kind != "app" {
        return Err(format!("\"kind\" must be \"app\", got {:?}", manifest.kind));
    }
    if manifest.name.is_empty() {
        return Err("\"name\" must not be empty".to_owned());
    }
    if manifest.runtime.kind != "qjs" {
        return Err(format!(
            "runtime kind {:?} is not supported (v0 runs qjs apps only)",
            manifest.runtime.kind
        ));
    }
    if manifest.runtime.main.starts_with("cas:") {
        return Err(
            "CAS-pinned main is not supported yet (docs/appfs.md §Provenance is deferred); \
             use a file path relative to the app directory"
                .to_owned(),
        );
    }
    if manifest.runtime.main.is_empty() || manifest.runtime.main.starts_with('/') {
        return Err(format!(
            "runtime main {:?} must be a relative path inside the app directory",
            manifest.runtime.main
        ));
    }
    Ok(())
}

/// The per-app mesh-endpoint identity key path,
/// `~/.wanix/app-identities/<name>.key` (the tool/volume-identities
/// precedent). Deliberately OUTSIDE the app and state directories, so the
/// endpoint secret key is never exported to peers that mount the app.
pub(crate) fn app_identity_path(name: &str) -> Result<PathBuf, CliError> {
    Ok(wanix_dir()?
        .join("app-identities")
        .join(format!("{name}.key")))
}

#[cfg(test)]
mod tests {
    use super::{AppManifest, validate_manifest};

    fn manifest(json: &str) -> AppManifest {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn bundled_chatroom_manifest_is_valid() {
        let raw = include_str!("../../../examples/chatroom/app.wanix.json");
        let parsed = manifest(raw);
        validate_manifest(&parsed).unwrap();
        assert_eq!(parsed.name, "chatroom");
        assert_eq!(parsed.runtime.main, "main.js");
        assert_eq!(parsed.streams, vec!["stream".to_owned()]);
    }

    #[test]
    fn manifest_rejects_unsupported_shapes() {
        let base = |resource: &str, kind: &str, runtime: &str, main: &str| {
            format!(
                "{{\"wanix.resource\":\"{resource}\",\"kind\":\"{kind}\",\"name\":\"x\",\
                 \"runtime\":{{\"kind\":\"{runtime}\",\"main\":\"{main}\"}}}}"
            )
        };
        for (raw, expected) in [
            (base("v1", "app", "qjs", "main.js"), "wanix.resource"),
            (base("v0", "tool", "qjs", "main.js"), "\"kind\""),
            (base("v0", "app", "wasm", "main.wasm"), "v0 runs qjs"),
            (base("v0", "app", "qjs", "cas:sha256:9f2c"), "CAS-pinned"),
            (base("v0", "app", "qjs", "/etc/main.js"), "relative path"),
        ] {
            let error = validate_manifest(&manifest(&raw)).unwrap_err();
            assert!(error.contains(expected), "{raw} -> {error}");
        }
    }
}
