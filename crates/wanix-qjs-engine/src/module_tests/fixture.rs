use crate::QuickJsModule;
use anyhow::Result;
use wasmtime::{Config, Engine};

pub(super) const MINIMAL_ABI_WAT: &str = include_str!("minimal_abi.wat");

pub(super) fn module_from_wat(wat: &str) -> Result<QuickJsModule> {
    let engine = Engine::default();
    module_from_wat_with_engine(&engine, wat)
}

pub(super) fn module_from_wat_with_engine(engine: &Engine, wat: &str) -> Result<QuickJsModule> {
    QuickJsModule::from_bytes(engine, wat.as_bytes())
}

pub(super) fn with_extra_export() -> String {
    let prefix = MINIMAL_ABI_WAT
        .trim_end()
        .strip_suffix(')')
        .expect("fixture module should end with a closing paren");
    format!("{prefix}\n  (func (export \"extra_export\"))\n)\n")
}

pub(super) fn replace_once(source: &str, from: &str, to: &str) -> String {
    let replaced = source.replacen(from, to, 1);
    assert_ne!(replaced, source, "fixture replacement should match");
    replaced
}

pub(super) fn engine_with_config(configure: impl FnOnce(&mut Config)) -> Result<Engine> {
    let mut config = Config::new();
    configure(&mut config);
    Ok(Engine::new(&config)?)
}

pub(super) fn assert_module_error(wat: &str, expected: &str) {
    assert_module_error_contains(wat, &[expected]);
}

pub(super) fn assert_module_error_with_engine(engine: &Engine, wat: &str, expected: &str) {
    assert_module_error_contains_with_engine(engine, wat, &[expected]);
}

pub(super) fn assert_module_error_contains(wat: &str, expected: &[&str]) {
    let engine = Engine::default();
    assert_module_error_contains_with_engine(&engine, wat, expected);
}

pub(super) fn assert_module_error_contains_with_engine(
    engine: &Engine,
    wat: &str,
    expected: &[&str],
) {
    let err = match module_from_wat_with_engine(engine, wat) {
        Ok(_) => panic!("module should fail ABI preflight"),
        Err(err) => format!("{err:#}"),
    };
    assert!(
        !err.contains("failed to compile QuickJS WASM module"),
        "expected ABI preflight error, got compile error {err:?}"
    );
    for expected in expected {
        assert!(
            err.contains(expected),
            "expected error to contain {expected:?}, got {err:?}"
        );
    }
}
