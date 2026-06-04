//! On-disk cache for compiled QuickJS WASM modules.
//!
//! Cranelift-compiling the ~1.7 MiB QuickJS fixture costs ~550 ms per process,
//! which dominates qjs task cold-start. Wasmtime can serialize a compiled
//! module to a host/version-bound artifact and `deserialize` it in well under a
//! millisecond, so callers that load the same wasm build repeatedly cache the
//! artifact keyed by the wasm SHA-256.
//!
//! The cache is advisory: a missing, stale, or incompatible artifact falls back
//! to a fresh compile, and the engine refuses artifacts that do not match its
//! configuration, so a Wasmtime upgrade simply recompiles under a new key.

use std::path::{Path, PathBuf};

use anyhow::Result;
use wasmtime::{Engine, Module};

/// Artifact filename for one wasm build under a cache directory.
///
/// The Wasmtime engine embeds its own version/config compatibility marker in
/// the serialized bytes and rejects mismatches on `deserialize`, so keying on
/// the wasm SHA-256 alone is sufficient: an incompatible artifact is detected
/// and recompiled rather than trusted.
fn artifact_path(cache_dir: &Path, wasm_sha256: &[u8; 32]) -> PathBuf {
    let mut name = String::with_capacity(64 + 6);
    for byte in wasm_sha256 {
        name.push_str(&format!("{byte:02x}"));
    }
    name.push_str(".cwasm");
    cache_dir.join(name)
}

/// Loads a compiled module from the cache, or compiles and caches it.
///
/// Returns the compiled [`Module`]. Cache read/write failures are non-fatal:
/// the function always falls back to compiling from `bytes`, so a read-only or
/// missing cache directory only forfeits the speedup.
pub(super) fn load_or_compile(
    engine: &Engine,
    bytes: &[u8],
    wasm_sha256: &[u8; 32],
    cache_dir: &Path,
) -> Result<Module> {
    let path = artifact_path(cache_dir, wasm_sha256);

    if let Ok(artifact) = std::fs::read(&path) {
        // SAFETY: `deserialize` validates the engine compatibility marker the
        // matching `serialize` wrote and errors on mismatch or corruption; we
        // treat any error as a cache miss and recompile below.
        if let Ok(module) = unsafe { Module::deserialize(engine, &artifact) } {
            return Ok(module);
        }
    }

    let module = Module::new(engine, bytes)?;

    // Best-effort write; ignore failures (read-only dir, races, full disk).
    if let Ok(artifact) = module.serialize() {
        let _ = write_atomic(&path, &artifact, wasm_sha256);
    }

    Ok(module)
}

/// Writes `bytes` to `path` atomically via a unique temp file + rename so a
/// crashed or concurrent writer never leaves a truncated artifact behind.
fn write_atomic(path: &Path, bytes: &[u8], wasm_sha256: &[u8; 32]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    // Disambiguate concurrent writers by the artifact key plus the writer's pid;
    // the final rename is atomic so the last writer wins harmlessly.
    let suffix = format!(
        "{:02x}{:02x}.{}.tmp",
        wasm_sha256[0],
        wasm_sha256[1],
        std::process::id()
    );
    let tmp = path.with_extension(suffix);
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}
