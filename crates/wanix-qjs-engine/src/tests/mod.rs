use super::*;
use anyhow::{Result, bail};
use std::sync::OnceLock;
use wasmtime::Engine;

mod binary_values;
mod bytecode;
mod host_callbacks;
mod host_config;
mod intrinsics;
mod jobs;
mod module_loader;
mod promise_rejections;
mod restore;
mod runtime_limits;
mod runtime_values;
mod scalar_values;
mod snapshot_validation;
mod stdlib;

// Tests that need a different wasm path should load an explicit module
// instead of using this process-wide QUICKJS_WASM cache.
static QUICKJS_FIXTURE: OnceLock<std::result::Result<(Engine, QuickJsModule), String>> =
    OnceLock::new();

fn quickjs_fixture() -> Result<(Engine, QuickJsModule)> {
    match QUICKJS_FIXTURE.get_or_init(|| {
        let engine = Engine::default();
        let path =
            std::env::var("QUICKJS_WASM").unwrap_or_else(|_| "fixtures/quickjs.wasm".to_string());
        QuickJsRuntime::module_from_file(&engine, path)
            .map(|module| (engine, module))
            .map_err(|err| format!("{err:#}"))
    }) {
        Ok((engine, module)) => Ok((engine.clone(), module.clone())),
        Err(err) => bail!("{err}"),
    }
}
