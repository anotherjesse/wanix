use super::QuickJsModule;
use super::abi::validate_quickjs_module_abi;
use anyhow::{Result, anyhow};
use sha2::{Digest, Sha256};
use std::path::Path;
use wasmtime::error::Context as _;
use wasmtime::{Engine, Module};

impl QuickJsModule {
    /// Reads, compiles, and ABI-preflights a QuickJS WebAssembly module using a
    /// default Wasmtime engine owned by the compiled module.
    ///
    /// Use this when callers do not need to share a custom Wasmtime engine.
    /// The resulting module remains responsible for runtime creation and
    /// restore.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read, Wasmtime cannot compile the
    /// bytes as a module, or the module does not export the required QuickJS
    /// runtime ABI.
    pub fn from_file_with_default_engine(path: impl AsRef<Path>) -> Result<Self> {
        let engine = Engine::default();
        Self::from_file(&engine, path)
    }

    /// Reads, compiles, and ABI-preflights a QuickJS WebAssembly module from disk.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read, Wasmtime cannot compile the
    /// bytes as a module, or the module does not export the required QuickJS
    /// runtime ABI.
    pub fn from_file(engine: &Engine, path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let bytes = std::fs::read(path)
            .map_err(|err| anyhow!("failed to read {}: {err}", path.display()))?;
        Self::from_bytes(engine, &bytes).map_err(|err| {
            anyhow!(
                "failed to load {} as a QuickJS WASM module: {err:#}",
                path.display()
            )
        })
    }

    /// Compiles and ABI-preflights a QuickJS WebAssembly module from bytes
    /// using a default Wasmtime engine owned by the compiled module.
    ///
    /// Use this when callers do not need to share a custom Wasmtime engine.
    /// The input bytes are hashed before being discarded so future snapshots can
    /// prove they were produced by the same module build.
    ///
    /// # Errors
    ///
    /// Returns an error if Wasmtime rejects the module bytes, or if the module
    /// does not export the required QuickJS runtime ABI.
    pub fn from_bytes_with_default_engine(bytes: &[u8]) -> Result<Self> {
        let engine = Engine::default();
        Self::from_bytes(&engine, bytes)
    }

    /// Compiles and ABI-preflights a QuickJS WebAssembly module from bytes.
    ///
    /// The input bytes are hashed before being discarded so future snapshots can
    /// prove they were produced by the same module build.
    ///
    /// # Errors
    ///
    /// Returns an error if Wasmtime rejects the module bytes, or if the module
    /// does not export the required QuickJS runtime ABI.
    pub fn from_bytes(engine: &Engine, bytes: &[u8]) -> Result<Self> {
        let module = Module::new(engine, bytes).context("failed to compile QuickJS WASM module")?;
        Self::finish(module, bytes)
    }

    /// Compiles and ABI-preflights a QuickJS WebAssembly module from bytes,
    /// loading a cached compiled artifact from `cache_dir` when one is present
    /// and trusted.
    ///
    /// The cache is advisory: a missing, stale, or untrusted artifact falls back
    /// to a fresh compile. The cache directory must be owner-private or it is
    /// ignored (see [`wanix_module_cache`]).
    ///
    /// # Errors
    ///
    /// Returns an error if Wasmtime rejects the module bytes, or if the module
    /// does not export the required QuickJS runtime ABI.
    pub fn from_bytes_cached(engine: &Engine, bytes: &[u8], cache_dir: &Path) -> Result<Self> {
        let wasm_sha256 = Sha256::digest(bytes).into();
        let module = wanix_module_cache::load_or_compile(engine, bytes, &wasm_sha256, cache_dir)
            .map_err(|err| anyhow!("failed to compile QuickJS WASM module: {err:#}"))?;
        Self::finish_with_sha(module, wasm_sha256)
    }

    /// Compiles and ABI-preflights a QuickJS WebAssembly module from bytes using
    /// a default Wasmtime engine, loading a cached compiled artifact from
    /// `cache_dir` when one is present and trusted.
    ///
    /// Use this when callers do not need to share a custom Wasmtime engine but
    /// want the compiled-module disk cache.
    ///
    /// # Errors
    ///
    /// Returns an error if Wasmtime rejects the module bytes, or if the module
    /// does not export the required QuickJS runtime ABI.
    pub fn from_bytes_with_default_engine_cached(bytes: &[u8], cache_dir: &Path) -> Result<Self> {
        let engine = Engine::default();
        Self::from_bytes_cached(&engine, bytes, cache_dir)
    }

    /// ABI-preflights an already-compiled module and binds it to the SHA-256 of
    /// the wasm bytes that produced it.
    fn finish(module: Module, bytes: &[u8]) -> Result<Self> {
        Self::finish_with_sha(module, Sha256::digest(bytes).into())
    }

    fn finish_with_sha(module: Module, wasm_sha256: [u8; 32]) -> Result<Self> {
        let abi = validate_quickjs_module_abi(&module)?;
        Ok(Self {
            module,
            wasm_sha256,
            abi,
        })
    }
}
