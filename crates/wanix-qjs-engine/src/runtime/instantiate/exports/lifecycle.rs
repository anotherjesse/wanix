use crate::host::HostState;
use anyhow::Result;
use wasmtime::{Instance, Store, TypedFunc};

use super::{QjsCompileFunc, optional_typed, typed};

pub(in crate::runtime::instantiate) struct LifetimeExports {
    pub(in crate::runtime::instantiate) qjs_destroy: TypedFunc<(), ()>,
    pub(in crate::runtime::instantiate) qjs_eval: TypedFunc<(i32, i32, i32, i32), i32>,
    pub(in crate::runtime::instantiate) qjs_free_cstring: TypedFunc<i32, ()>,
    pub(in crate::runtime::instantiate) qjs_free_value: TypedFunc<i32, ()>,
}

impl LifetimeExports {
    pub(in crate::runtime::instantiate) fn bind(
        instance: &Instance,
        store: &mut Store<HostState>,
    ) -> Result<Self> {
        Ok(Self {
            qjs_destroy: typed(instance, store, "qjs_destroy")?,
            qjs_eval: typed(instance, store, "qjs_eval")?,
            qjs_free_cstring: typed(instance, store, "qjs_free_cstring")?,
            qjs_free_value: typed(instance, store, "qjs_free_value")?,
        })
    }
}

pub(in crate::runtime::instantiate) struct BytecodeExports {
    pub(in crate::runtime::instantiate) qjs_compile: Option<QjsCompileFunc>,
    pub(in crate::runtime::instantiate) qjs_free_bytecode: Option<TypedFunc<i32, ()>>,
    pub(in crate::runtime::instantiate) qjs_eval_bytecode: Option<TypedFunc<(i32, i32), i32>>,
}

impl BytecodeExports {
    pub(in crate::runtime::instantiate) fn bind(
        instance: &Instance,
        store: &mut Store<HostState>,
    ) -> Result<Self> {
        Ok(Self {
            qjs_compile: optional_typed(instance, store, "qjs_compile")?,
            qjs_free_bytecode: optional_typed(instance, store, "qjs_free_bytecode")?,
            qjs_eval_bytecode: optional_typed(instance, store, "qjs_eval_bytecode")?,
        })
    }
}

pub(in crate::runtime::instantiate) struct WasmMemoryExports {
    pub(in crate::runtime::instantiate) wasm_malloc: TypedFunc<i32, i32>,
    pub(in crate::runtime::instantiate) wasm_free: TypedFunc<i32, ()>,
}

impl WasmMemoryExports {
    pub(in crate::runtime::instantiate) fn bind(
        instance: &Instance,
        store: &mut Store<HostState>,
    ) -> Result<Self> {
        Ok(Self {
            wasm_malloc: typed(instance, store, "wasm_malloc")?,
            wasm_free: typed(instance, store, "wasm_free")?,
        })
    }
}
