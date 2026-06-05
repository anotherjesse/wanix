use super::{QuickJsRuntime, instantiate};
use crate::guest::guest_i32;
use crate::{QuickJsCreateOptions, QuickJsHostConfig, QuickJsIntrinsics, QuickJsModule};
use anyhow::{Result, bail};
use std::path::Path;
use wasmtime::Engine;
use wasmtime::error::Context as _;

impl QuickJsRuntime {
    /// Reads and compiles a QuickJS WebAssembly module from disk.
    ///
    /// This is a convenience wrapper around [`QuickJsModule::from_file`].
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read or Wasmtime cannot compile
    /// the module.
    pub fn module_from_file(engine: &Engine, path: impl AsRef<Path>) -> Result<QuickJsModule> {
        QuickJsModule::from_file(engine, path)
    }

    /// Creates and initializes a runtime with the default host configuration.
    ///
    /// Prefer [`QuickJsModule::create_runtime`](crate::QuickJsModule::create_runtime)
    /// when you already have a module.
    ///
    /// # Errors
    ///
    /// Returns an error if `engine` did not compile `module`, the wasm module
    /// cannot be instantiated, required exports are missing, or QuickJS
    /// initialization fails.
    pub fn create(engine: &Engine, module: &QuickJsModule) -> Result<Self> {
        Self::create_with_host_config(engine, module, QuickJsHostConfig::default())
    }

    /// Creates and initializes a runtime with explicit host import settings.
    ///
    /// Prefer
    /// [`QuickJsModule::create_runtime_with_host_config`](crate::QuickJsModule::create_runtime_with_host_config)
    /// when you already have a module.
    ///
    /// # Errors
    ///
    /// Returns an error if `engine` did not compile `module`, the wasm module
    /// cannot be instantiated, required exports are missing, or QuickJS
    /// initialization fails.
    pub fn create_with_host_config(
        engine: &Engine,
        module: &QuickJsModule,
        config: QuickJsHostConfig,
    ) -> Result<Self> {
        module.ensure_engine(engine)?;
        module.create_runtime_with_host_config(config)
    }

    /// Creates and initializes a runtime with explicit QuickJS create options.
    ///
    /// Prefer
    /// [`QuickJsModule::create_runtime_with_options`](crate::QuickJsModule::create_runtime_with_options)
    /// when you already have a module.
    ///
    /// # Errors
    ///
    /// Returns an error if `engine` did not compile `module`, the wasm module
    /// cannot be instantiated, required exports are missing, explicit
    /// intrinsic selection is requested without `qjs_init2`, or QuickJS
    /// initialization fails.
    pub fn create_with_options(
        engine: &Engine,
        module: &QuickJsModule,
        options: QuickJsCreateOptions,
    ) -> Result<Self> {
        module.ensure_engine(engine)?;
        module.create_runtime_with_options(options)
    }

    /// Creates and initializes a runtime with selective QuickJS intrinsics.
    ///
    /// Prefer
    /// [`QuickJsModule::create_runtime_with_intrinsics`](crate::QuickJsModule::create_runtime_with_intrinsics)
    /// when you already have a module.
    ///
    /// # Errors
    ///
    /// Returns an error if `engine` did not compile `module`, the wasm module
    /// cannot be instantiated, `qjs_init2` is missing or mistyped, or QuickJS
    /// initialization fails.
    pub fn create_with_intrinsics(
        engine: &Engine,
        module: &QuickJsModule,
        intrinsics: QuickJsIntrinsics,
    ) -> Result<Self> {
        Self::create_with_options(
            engine,
            module,
            QuickJsCreateOptions::new().with_intrinsics(intrinsics),
        )
    }

    pub(crate) fn create_for_module(
        module: &QuickJsModule,
        config: QuickJsHostConfig,
    ) -> Result<Self> {
        Self::create_for_module_with_options(
            module,
            QuickJsCreateOptions::new().with_host_config(config),
        )
    }

    pub(crate) fn create_for_module_with_options(
        module: &QuickJsModule,
        options: QuickJsCreateOptions,
    ) -> Result<Self> {
        let (config, intrinsics, wasi_host) = options.into_parts();
        let mut instantiated = Self::instantiate(module.engine(), module, config, wasi_host)?;

        instantiated
            .initialize
            .call(&mut instantiated.vm.store, ())
            .context("failed to initialize WASI reactor")?;

        let init_result = match intrinsics {
            Some(intrinsics) => call_qjs_init2(&mut instantiated, intrinsics)?,
            None => instantiated
                .qjs_init
                .call(&mut instantiated.vm.store, ())
                .context("failed to call qjs_init")?,
        };
        if init_result != 0 {
            bail!("QuickJS runtime initialization failed with code {init_result}");
        }

        Ok(instantiated.vm)
    }
}

fn call_qjs_init2(
    instantiated: &mut instantiate::InstantiatedRuntime,
    intrinsics: QuickJsIntrinsics,
) -> Result<i32> {
    let qjs_init2 = instantiated.qjs_init2.clone().ok_or_else(|| {
        anyhow::anyhow!(
            "QuickJS WASM module does not export qjs_init2; configurable intrinsics are not supported"
        )
    })?;
    Ok(qjs_init2
        .call(&mut instantiated.vm.store, guest_i32(intrinsics.bits()))
        .context("failed to call qjs_init2")?)
}
