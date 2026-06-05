use rust_wasi_quickjs::{
    QuickJsCreateOptions, QuickJsHostConfig, QuickJsRestoreOptions, QuickJsRuntime,
};
use wanix_fs::{FileSystem, FsResult};

use crate::host_api::{define_wanix_module_loader, qjs_error};
use crate::{QuickJsRunner, QuickJsWanixConfig, create_options_with_config, wanix_wasi_host};

struct WanixCreateOptions<N> {
    options: QuickJsCreateOptions,
    namespace: N,
}

struct WanixRestoreOptions<N> {
    options: QuickJsRestoreOptions,
    namespace: N,
}

impl QuickJsRunner {
    /// Creates a QuickJS runtime with live Wanix-backed WASI imports.
    ///
    /// The returned runtime can be snapshotted through the engine API. Use
    /// [`Self::restore_runtime_from_bytes_with_wanix_config`] to resume that VM
    /// image with fresh Wanix host resources. This lifecycle helper intentionally
    /// installs only the live WASI provider and namespace module loader; it does
    /// not install convenience `print`/`console` callback shims, because those
    /// are host callback objects that require separate restore-time reattachment.
    /// Guest `qjs:std` stdout and stderr writes go through the `WasiConfig` fd
    /// attachments supplied by `config`.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when the Wanix WASI host cannot be created, the
    /// QuickJS runtime cannot be instantiated, or the namespace module loader
    /// cannot be attached.
    pub fn create_runtime_with_wanix_config(
        &self,
        config: QuickJsWanixConfig,
    ) -> FsResult<QuickJsRuntime> {
        let create_options = create_options_with_wanix_wasi(config)?;
        self.create_wanix_runtime(create_options)
    }

    /// Restores a QuickJS VM image with live Wanix-backed WASI imports.
    ///
    /// Snapshot bytes remain a QuickJS VM image. The supplied Wanix config
    /// reattaches namespace, preopen, fd, argv/env, and stdio host resources for
    /// the restored runtime. Guest `qjs:std` stdout and stderr writes go through
    /// the `WasiConfig` fd attachments supplied by `config`.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when the snapshot bytes are invalid or
    /// incompatible with this runner's QuickJS module, the Wanix WASI host cannot
    /// be created, the runtime cannot be restored, or the namespace module loader
    /// cannot be attached.
    pub fn restore_runtime_from_bytes_with_wanix_config(
        &self,
        bytes: &[u8],
        config: QuickJsWanixConfig,
    ) -> FsResult<QuickJsRuntime> {
        let restore_options = restore_options_with_wanix_wasi(config)?;
        self.restore_wanix_runtime(bytes, restore_options)
    }

    fn create_wanix_runtime(
        &self,
        create_options: WanixCreateOptions<impl FileSystem + Clone + 'static>,
    ) -> FsResult<QuickJsRuntime> {
        let namespace = create_options.namespace;
        let mut runtime = self
            .module
            .create_runtime_with_options(create_options.options)
            .map_err(qjs_error)?;
        define_wanix_module_loader(&mut runtime, namespace)?;
        Ok(runtime)
    }

    fn restore_wanix_runtime(
        &self,
        bytes: &[u8],
        restore_options: WanixRestoreOptions<impl FileSystem + Clone + 'static>,
    ) -> FsResult<QuickJsRuntime> {
        let namespace = restore_options.namespace;
        let mut runtime = self
            .module
            .restore_runtime_from_bytes_with_options(bytes, restore_options.options)
            .map_err(qjs_error)?;
        define_wanix_module_loader(&mut runtime, namespace)?;
        Ok(runtime)
    }
}

fn create_options_with_wanix_wasi(
    config: QuickJsWanixConfig,
) -> FsResult<WanixCreateOptions<impl FileSystem + Clone + 'static>> {
    let namespace = config.wasi().namespace().clone();
    let host_config = QuickJsHostConfig::new().with_clock_time_ns(config.wasi().clock_time_ns());
    Ok(WanixCreateOptions {
        options: create_options_with_config(host_config).with_wasi_host(wanix_wasi_host(config)?),
        namespace,
    })
}

fn restore_options_with_wanix_wasi(
    config: QuickJsWanixConfig,
) -> FsResult<WanixRestoreOptions<impl FileSystem + Clone + 'static>> {
    let namespace = config.wasi().namespace().clone();
    let host_config = QuickJsHostConfig::new().with_clock_time_ns(config.wasi().clock_time_ns());
    Ok(WanixRestoreOptions {
        options: QuickJsRestoreOptions::new()
            .with_host_config(host_config)
            .with_wasi_host(wanix_wasi_host(config)?),
        namespace,
    })
}
