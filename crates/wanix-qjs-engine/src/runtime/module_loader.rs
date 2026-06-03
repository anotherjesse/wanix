use super::QuickJsRuntime;
use crate::host::{ModuleLoadCallback, ModuleLoader, ModuleNormalizeCallback};
use anyhow::{Result, anyhow};
use wasmtime::error::Context as _;

impl QuickJsRuntime {
    /// Installs or replaces the Rust-side synchronous ES module loader.
    ///
    /// Imported module specifiers are passed through unchanged. Use
    /// [`Self::set_module_loader_with_normalizer`] when the host needs to
    /// resolve relative or virtual module names before loading source.
    ///
    /// The loader closure is Rust host state. It is not serialized into
    /// snapshots; restored runtimes must install a loader again before future
    /// module imports can call into Rust.
    ///
    /// # Errors
    ///
    /// Returns an error if the wasm module does not export the module-loader
    /// helper, or if enabling the QuickJS loader fails.
    pub fn set_module_loader<L>(&mut self, load: L) -> Result<()>
    where
        L: FnMut(&str) -> Result<String> + Send + 'static,
    {
        self.install_module_loader(None, Box::new(load))
    }

    /// Installs or replaces the Rust-side synchronous ES module loader and normalizer.
    ///
    /// `normalize` receives `(base_name, specifier)` and returns the canonical
    /// module name that will be passed to `load`. Returned names must not contain
    /// NUL bytes because QuickJS consumes them as C strings.
    ///
    /// The callbacks are Rust host state. They are not serialized into
    /// snapshots; restored runtimes must install callbacks again before future
    /// module imports can call into Rust.
    ///
    /// # Errors
    ///
    /// Returns an error if the wasm module does not export the module-loader
    /// helper, or if enabling the QuickJS loader fails.
    pub fn set_module_loader_with_normalizer<N, L>(&mut self, normalize: N, load: L) -> Result<()>
    where
        N: FnMut(&str, &str) -> Result<String> + Send + 'static,
        L: FnMut(&str) -> Result<String> + Send + 'static,
    {
        self.install_module_loader(Some(Box::new(normalize)), Box::new(load))
    }

    fn install_module_loader(
        &mut self,
        normalize: Option<ModuleNormalizeCallback>,
        load: ModuleLoadCallback,
    ) -> Result<()> {
        let set_module_loader = self
            .qjs_set_module_loader
            .clone()
            .ok_or_else(module_loader_unsupported_error)?;
        set_module_loader
            .call(&mut self.store, 1)
            .context("failed to enable QuickJS module loader")?;
        self.store
            .data_mut()
            .set_module_loader(ModuleLoader { normalize, load });
        Ok(())
    }
}

fn module_loader_unsupported_error() -> anyhow::Error {
    anyhow!(
        "QuickJS WASM module does not export qjs_set_module_loader; module loaders are not supported"
    )
}
