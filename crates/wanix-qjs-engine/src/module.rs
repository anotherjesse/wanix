use crate::{
    QuickJsCreateOptions, QuickJsHostConfig, QuickJsIntrinsics, QuickJsRestoreOptions,
    QuickJsRuntime, Snapshot,
};
use anyhow::{Result, bail};
use wasmtime::{Engine, Module};

mod abi;
mod load;

pub(crate) use abi::QUICKJS_WASM_ABI_VERSION;
use abi::QuickJsModuleAbi;

/// A compiled, ABI-preflighted QuickJS WebAssembly module and its exact byte identity.
///
/// Snapshots are bound to the SHA-256 of the wasm bytes that produced them.
/// Reuse a `QuickJsModule` when creating, restoring, or validating snapshots for
/// one QuickJS build.
#[derive(Clone)]
pub struct QuickJsModule {
    module: Module,
    wasm_sha256: [u8; 32],
    abi: QuickJsModuleAbi,
}

impl QuickJsModule {
    /// Returns the SHA-256 identity of the original wasm bytes.
    #[must_use]
    pub fn wasm_sha256(&self) -> [u8; 32] {
        self.wasm_sha256
    }

    /// Creates and initializes a runtime for this module with the default host
    /// configuration.
    ///
    /// # Errors
    ///
    /// Returns an error if the wasm module cannot be instantiated, required
    /// exports are missing, or QuickJS initialization fails.
    pub fn create_runtime(&self) -> Result<QuickJsRuntime> {
        self.create_runtime_with_host_config(QuickJsHostConfig::default())
    }

    /// Creates and initializes a runtime for this module with explicit host
    /// import settings.
    ///
    /// # Errors
    ///
    /// Returns an error if the wasm module cannot be instantiated, required
    /// exports are missing, or QuickJS initialization fails.
    pub fn create_runtime_with_host_config(
        &self,
        config: QuickJsHostConfig,
    ) -> Result<QuickJsRuntime> {
        QuickJsRuntime::create_for_module(self, config)
    }

    /// Creates and initializes a runtime with explicit QuickJS create options.
    ///
    /// Use this when fresh-runtime policy includes both host imports and
    /// creation-time QuickJS options such as selective intrinsics. Restore APIs
    /// intentionally do not accept these options because snapshots already
    /// contain an initialized QuickJS context.
    ///
    /// # Errors
    ///
    /// Returns an error if the wasm module cannot be instantiated, required
    /// exports are missing, explicit intrinsic selection is requested without
    /// `qjs_init2`, or QuickJS initialization fails.
    pub fn create_runtime_with_options(
        &self,
        options: QuickJsCreateOptions,
    ) -> Result<QuickJsRuntime> {
        QuickJsRuntime::create_for_module_with_options(self, options)
    }

    /// Creates and initializes a runtime with selective QuickJS intrinsics.
    ///
    /// Base objects are always installed by the reference adapter. The provided
    /// mask selects additional built-ins such as `Date`, `eval`, `JSON`,
    /// `Promise`, typed arrays, and base64 helpers.
    ///
    /// # Errors
    ///
    /// Returns an error if the wasm module cannot be instantiated, `qjs_init2`
    /// is missing or mistyped, or QuickJS initialization fails.
    pub fn create_runtime_with_intrinsics(
        &self,
        intrinsics: QuickJsIntrinsics,
    ) -> Result<QuickJsRuntime> {
        self.create_runtime_with_options(QuickJsCreateOptions::new().with_intrinsics(intrinsics))
    }

    /// Restores a runtime for this module from a snapshot with the default host
    /// configuration.
    ///
    /// # Errors
    ///
    /// Returns an error if the snapshot is not compatible with this module, the
    /// wasm module cannot be instantiated, memory cannot be grown, or the saved
    /// QuickJS pointers cannot be reattached and verified.
    pub fn restore_runtime(&self, snapshot: &Snapshot) -> Result<QuickJsRuntime> {
        self.restore_runtime_with_host_config(snapshot, QuickJsHostConfig::default())
    }

    /// Restores a runtime for this module from a snapshot with explicit host
    /// import settings.
    ///
    /// The host config is not serialized inside the snapshot. Passing it here
    /// intentionally reattaches host behavior to the resumed runtime.
    ///
    /// # Errors
    ///
    /// Returns an error if the snapshot is not compatible with this module, the
    /// wasm module cannot be instantiated, memory cannot be grown, or the saved
    /// QuickJS pointers cannot be reattached and verified.
    pub fn restore_runtime_with_host_config(
        &self,
        snapshot: &Snapshot,
        config: QuickJsHostConfig,
    ) -> Result<QuickJsRuntime> {
        self.restore_runtime_with_options(
            snapshot,
            QuickJsRestoreOptions::new().with_host_config(config),
        )
    }

    /// Restores a runtime for this module with explicit restore options.
    ///
    /// Use this when restore-time policy includes both deterministic host config
    /// and live host state such as a WASI provider. QuickJS creation options
    /// such as intrinsic selection are intentionally unavailable because the
    /// snapshot already contains an initialized QuickJS context.
    ///
    /// # Errors
    ///
    /// Returns an error if the snapshot is not compatible with this module, the
    /// wasm module cannot be instantiated, memory cannot be grown, or the saved
    /// QuickJS pointers cannot be reattached and verified.
    pub fn restore_runtime_with_options(
        &self,
        snapshot: &Snapshot,
        options: QuickJsRestoreOptions,
    ) -> Result<QuickJsRuntime> {
        QuickJsRuntime::restore_for_module(self, snapshot, options)
    }

    /// Parses snapshot bytes, validates them against this module, and restores a
    /// runtime with the default host configuration.
    ///
    /// # Errors
    ///
    /// Returns an error if the snapshot bytes are malformed or not compatible
    /// with this module, the wasm module cannot be instantiated, memory cannot
    /// be grown, or the saved QuickJS pointers cannot be reattached and verified.
    pub fn restore_runtime_from_bytes(&self, bytes: &[u8]) -> Result<QuickJsRuntime> {
        self.restore_runtime_from_bytes_with_host_config(bytes, QuickJsHostConfig::default())
    }

    /// Parses snapshot bytes, validates them against this module, and restores a
    /// runtime with explicit host import settings.
    ///
    /// The host config is not serialized inside the snapshot. Passing it here
    /// intentionally reattaches host behavior to the resumed runtime.
    ///
    /// # Errors
    ///
    /// Returns an error if the snapshot bytes are malformed or not compatible
    /// with this module, the wasm module cannot be instantiated, memory cannot
    /// be grown, or the saved QuickJS pointers cannot be reattached and verified.
    pub fn restore_runtime_from_bytes_with_host_config(
        &self,
        bytes: &[u8],
        config: QuickJsHostConfig,
    ) -> Result<QuickJsRuntime> {
        self.restore_runtime_from_bytes_with_options(
            bytes,
            QuickJsRestoreOptions::new().with_host_config(config),
        )
    }

    /// Parses snapshot bytes, validates them against this module, and restores a
    /// runtime with explicit restore options.
    ///
    /// # Errors
    ///
    /// Returns an error if the snapshot bytes are malformed or not compatible
    /// with this module, the wasm module cannot be instantiated, memory cannot
    /// be grown, or the saved QuickJS pointers cannot be reattached and verified.
    pub fn restore_runtime_from_bytes_with_options(
        &self,
        bytes: &[u8],
        options: QuickJsRestoreOptions,
    ) -> Result<QuickJsRuntime> {
        let snapshot = Snapshot::from_bytes_for_module(bytes, self)?;
        self.restore_runtime_with_options(&snapshot, options)
    }

    pub(crate) fn wasmtime_module(&self) -> &Module {
        &self.module
    }

    pub(crate) fn engine(&self) -> &Engine {
        self.module.engine()
    }

    pub(crate) fn ensure_engine(&self, engine: &Engine) -> Result<()> {
        if !Engine::same(engine, self.module.engine()) {
            bail!("QuickJS module was compiled with a different Wasmtime engine");
        }
        Ok(())
    }

    pub(crate) fn minimum_memory_len(&self) -> Result<usize> {
        self.abi.minimum_memory_len()
    }
}
