use crate::host::HostState;
use anyhow::{Result, anyhow};
use wasmtime::error::Context as _;
use wasmtime::{Global, Instance, Memory, Store, TypedFunc};

mod construction;
mod lifecycle;
mod values;

use construction::{
    ConstructorExports, HostControlExports, MemoryPolicyExports, PropertyExports,
    ValueConstantExports,
};
use lifecycle::{BytecodeExports, LifetimeExports, WasmMemoryExports};
use values::{AccessorExports, JobExports, SnapshotExports, TypeCheckExports};

type QjsCompileFunc = TypedFunc<(i32, i32, i32, i32, i32, i32), i32>;

pub(super) struct RuntimeInitExports {
    pub(super) initialize: TypedFunc<(), ()>,
    pub(super) qjs_init: TypedFunc<(), i32>,
    pub(super) qjs_init2: Option<TypedFunc<i32, i32>>,
}

impl RuntimeInitExports {
    pub(super) fn bind(instance: &Instance, store: &mut Store<HostState>) -> Result<Self> {
        Ok(Self {
            initialize: instance
                .get_typed_func::<(), ()>(&mut *store, "_initialize")
                .context("missing _initialize export")?,
            qjs_init: instance
                .get_typed_func::<(), i32>(&mut *store, "qjs_init")
                .context("missing qjs_init export")?,
            qjs_init2: optional_typed(instance, store, "qjs_init2")?,
        })
    }
}

pub(super) struct RuntimeExports {
    pub(super) lifetime: LifetimeExports,
    pub(super) bytecode: BytecodeExports,
    pub(super) constructors: ConstructorExports,
    pub(super) host: HostControlExports,
    pub(super) memory_policy: MemoryPolicyExports,
    pub(super) values: ValueConstantExports,
    pub(super) properties: PropertyExports,
    pub(super) type_checks: TypeCheckExports,
    pub(super) accessors: AccessorExports,
    pub(super) jobs: JobExports,
    pub(super) snapshot: SnapshotExports,
    pub(super) wasm_memory: WasmMemoryExports,
}

impl RuntimeExports {
    pub(super) fn bind(instance: &Instance, store: &mut Store<HostState>) -> Result<Self> {
        Ok(Self {
            lifetime: LifetimeExports::bind(instance, store)?,
            bytecode: BytecodeExports::bind(instance, store)?,
            constructors: ConstructorExports::bind(instance, store)?,
            host: HostControlExports::bind(instance, store)?,
            memory_policy: MemoryPolicyExports::bind(instance, store)?,
            values: ValueConstantExports::bind(instance, store)?,
            properties: PropertyExports::bind(instance, store)?,
            type_checks: TypeCheckExports::bind(instance, store)?,
            accessors: AccessorExports::bind(instance, store)?,
            jobs: JobExports::bind(instance, store)?,
            snapshot: SnapshotExports::bind(instance, store)?,
            wasm_memory: WasmMemoryExports::bind(instance, store)?,
        })
    }
}

pub(super) fn bind_memory(instance: &Instance, store: &mut Store<HostState>) -> Result<Memory> {
    let memory = instance
        .get_memory(&mut *store, "memory")
        .ok_or_else(|| anyhow!("QuickJS WASM module does not export memory"))?;
    store.data_mut().set_memory(memory);
    Ok(memory)
}

pub(super) fn bind_stack_pointer(
    instance: &Instance,
    store: &mut Store<HostState>,
) -> Result<Global> {
    instance
        .get_global(store, "__stack_pointer")
        .ok_or_else(|| anyhow!("QuickJS WASM module does not export __stack_pointer"))
}

fn typed<P, R>(
    instance: &Instance,
    store: &mut Store<HostState>,
    name: &str,
) -> Result<TypedFunc<P, R>>
where
    P: wasmtime::WasmParams,
    R: wasmtime::WasmResults,
{
    Ok(instance
        .get_typed_func::<P, R>(&mut *store, name)
        .with_context(|| format!("missing or mistyped {name} export"))?)
}

fn optional_typed<P, R>(
    instance: &Instance,
    store: &mut Store<HostState>,
    name: &str,
) -> Result<Option<TypedFunc<P, R>>>
where
    P: wasmtime::WasmParams,
    R: wasmtime::WasmResults,
{
    if instance.get_func(&mut *store, name).is_none() {
        return Ok(None);
    }
    typed(instance, store, name).map(Some)
}
