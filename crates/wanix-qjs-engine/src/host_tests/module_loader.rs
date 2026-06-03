use crate::host::{HostState, ModuleLoader, QuickJsHostConfig, define_env_imports};
use anyhow::{Context, Result};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use wasmtime::{Engine, Linker, Memory, Module, Store, TypedFunc};

const MODULE_LOADER_IMPORT_WAT: &str = r#"
(module
  (import "env" "host_module_normalize" (func $host_module_normalize (param i32 i32) (result i32)))
  (import "env" "host_module_load" (func $host_module_load (param i32 i32) (result i32)))

  (memory (export "memory") 1)
  (global $heap (mut i32) (i32.const 2048))
  (global $last_freed (mut i32) (i32.const 0))

  (data (i32.const 64) "module.js\00")

  (func (export "host_module_normalize") (param i32 i32) (result i32)
    local.get 0
    local.get 1
    call $host_module_normalize)

  (func (export "host_module_load") (param i32 i32) (result i32)
    local.get 0
    local.get 1
    call $host_module_load)

  (func (export "wasm_malloc") (param $size i32) (result i32)
    (local $ptr i32)
    global.get $heap
    local.set $ptr
    global.get $heap
    local.get $size
    i32.add
    global.set $heap
    local.get $ptr)

  (func (export "wasm_free") (param $ptr i32)
    local.get $ptr
    global.set $last_freed)

  (func (export "last_freed") (result i32)
    global.get $last_freed)
)
"#;

struct ModuleLoaderImportHarness {
    store: Store<HostState>,
    memory: Memory,
    host_module_load: TypedFunc<(i32, i32), i32>,
    last_freed: TypedFunc<(), i32>,
}

impl ModuleLoaderImportHarness {
    fn call_host_module_load(&mut self, name: i32, out_len: i32) -> Result<i32> {
        Ok(self
            .host_module_load
            .call(&mut self.store, (name, out_len))?)
    }

    fn last_freed(&mut self) -> Result<i32> {
        Ok(self.last_freed.call(&mut self.store, ())?)
    }
}

fn module_loader_import_harness(loader: ModuleLoader) -> Result<ModuleLoaderImportHarness> {
    let engine = Engine::default();
    let module = Module::new(&engine, MODULE_LOADER_IMPORT_WAT)?;
    let mut linker = Linker::<HostState>::new(&engine);
    define_env_imports(&mut linker)?;
    let mut store = Store::new(&engine, HostState::new(QuickJsHostConfig::new()));
    store.data_mut().set_module_loader(loader);
    let instance = linker.instantiate(&mut store, &module)?;
    let memory = instance
        .get_memory(&mut store, "memory")
        .context("test module should export memory")?;
    store.data_mut().set_memory(memory);
    let host_module_load = instance.get_typed_func(&mut store, "host_module_load")?;
    let last_freed = instance.get_typed_func(&mut store, "last_freed")?;

    Ok(ModuleLoaderImportHarness {
        store,
        memory,
        host_module_load,
        last_freed,
    })
}

#[test]
fn host_module_load_rejects_null_out_len_before_calling_loader() -> Result<()> {
    let called = Arc::new(AtomicUsize::new(0));
    let called_by_loader = Arc::clone(&called);
    let mut harness = module_loader_import_harness(ModuleLoader {
        normalize: None,
        load: Box::new(move |_name| {
            called_by_loader.fetch_add(1, Ordering::SeqCst);
            Ok("export const value = 1;".to_string())
        }),
    })?;
    harness
        .memory
        .write(&mut harness.store, 0, &u32::MAX.to_le_bytes())?;

    let ptr = harness.call_host_module_load(64, 0)?;

    assert_eq!(ptr, 0);
    assert_eq!(called.load(Ordering::SeqCst), 0);
    assert_eq!(read_u32(&harness, 0)?, u32::MAX);
    assert_eq!(harness.last_freed()?, 0);
    Ok(())
}

#[test]
fn host_module_load_frees_source_allocation_when_out_len_write_fails() -> Result<()> {
    let mut harness = module_loader_import_harness(ModuleLoader {
        normalize: None,
        load: Box::new(|_name| Ok("export const value = 1;".to_string())),
    })?;
    let memory_len = harness.memory.data_size(&harness.store);
    let out_len = i32::try_from(memory_len - 2).context("test memory length should fit i32")?;

    let ptr = harness.call_host_module_load(64, out_len)?;

    assert_eq!(ptr, 0);
    assert_eq!(harness.last_freed()?, 2048);
    Ok(())
}

fn read_u32(harness: &ModuleLoaderImportHarness, offset: usize) -> Result<u32> {
    let mut bytes = [0; 4];
    harness.memory.read(&harness.store, offset, &mut bytes)?;
    Ok(u32::from_le_bytes(bytes))
}
