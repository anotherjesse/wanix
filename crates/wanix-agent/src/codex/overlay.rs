use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use wanix_fs::{FsError, FsResult};

static OVERLAY_COUNTER: AtomicU64 = AtomicU64::new(0);

fn other(message: impl Into<String>) -> FsError {
    FsError::Other(message.into())
}

/// codex's real home (`$CODEX_HOME`, else `$HOME/.codex`).
fn real_codex_home() -> FsResult<PathBuf> {
    if let Ok(home) = std::env::var("CODEX_HOME") {
        return Ok(PathBuf::from(home));
    }
    let home = std::env::var("HOME").map_err(|_| other("HOME is not set"))?;
    Ok(PathBuf::from(home).join(".codex"))
}

fn link_or_copy(source: &Path, destination: &Path) {
    if !source.exists() {
        return;
    }
    #[cfg(unix)]
    let linked = std::os::unix::fs::symlink(source, destination).is_ok();
    #[cfg(not(unix))]
    let linked = false;
    if !linked {
        let _ = fs::copy(source, destination);
    }
}

/// Builds a private `CODEX_HOME` overlay that reuses the real codex auth and
/// registers a `wanix` environment whose exec-server is this binary
/// (`agent-exec-server --root <world_root>`). Returns the overlay dir; the
/// environment id is always `wanix`.
///
/// # Errors
///
/// Returns an error when the overlay cannot be created or this executable's
/// path cannot be resolved.
pub(super) fn build_codex_overlay(world_root: &str) -> FsResult<PathBuf> {
    let real = real_codex_home()?;
    let nonce = OVERLAY_COUNTER.fetch_add(1, Ordering::Relaxed);
    let overlay =
        std::env::temp_dir().join(format!("wanix-codex-home-{}-{nonce}", std::process::id()));
    fs::create_dir_all(&overlay)
        .map_err(|error| other(format!("create codex overlay: {error}")))?;

    // Symlink auth.json so token refreshes propagate back; copy the rest.
    link_or_copy(&real.join("auth.json"), &overlay.join("auth.json"));
    link_or_copy(&real.join("config.toml"), &overlay.join("config.toml"));
    link_or_copy(
        &real.join("installation_id"),
        &overlay.join("installation_id"),
    );

    let exe = std::env::current_exe()
        .map_err(|error| other(format!("resolve current executable: {error}")))?;
    let environments = format!(
        "default = \"wanix\"\ninclude_local = false\n\n\
         [[environments]]\n\
         id = \"wanix\"\n\
         program = {program}\n\
         args = [\"agent-exec-server\", \"--root\", {root}, \"--services\"]\n\
         cwd = {root}\n\
         initialize_timeout_sec = 60\n",
        program = toml_string(&exe.to_string_lossy()),
        root = toml_string(world_root),
    );
    fs::write(overlay.join("environments.toml"), environments)
        .map_err(|error| other(format!("write environments.toml: {error}")))?;
    Ok(overlay)
}

/// Minimal TOML basic-string quoting (escape backslash and double-quote).
fn toml_string(value: &str) -> String {
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}
