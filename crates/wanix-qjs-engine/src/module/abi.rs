use anyhow::{Result, anyhow, bail};
use wasmtime::{ExternType, Module, Mutability, ValType};

mod spec;

pub(crate) use spec::QUICKJS_WASM_ABI_VERSION;
use spec::{AbiVal, REQUIRED_FUNCS, RequiredFunc, WASM_PAGE_SIZE};

#[derive(Clone, Copy)]
pub(super) struct QuickJsModuleAbi {
    minimum_memory_pages: u64,
}

impl QuickJsModuleAbi {
    pub(super) fn minimum_memory_len(self) -> Result<usize> {
        let bytes = self
            .minimum_memory_pages
            .checked_mul(WASM_PAGE_SIZE)
            .ok_or_else(|| anyhow!("QuickJS WASM minimum memory length overflowed"))?;
        usize::try_from(bytes)
            .map_err(|_| anyhow!("QuickJS WASM minimum memory length does not fit in usize"))
    }
}

pub(super) fn validate_quickjs_module_abi(module: &Module) -> Result<QuickJsModuleAbi> {
    let minimum_memory_pages = validate_memory(module)?;
    validate_stack_pointer(module)?;
    for required in REQUIRED_FUNCS {
        validate_func(module, required)?;
    }
    Ok(QuickJsModuleAbi {
        minimum_memory_pages,
    })
}

fn validate_memory(module: &Module) -> Result<u64> {
    let export = required_export(module, "memory")?;
    let ExternType::Memory(memory) = export else {
        bail!(
            "QuickJS WASM export memory has type {}, expected memory",
            export_type_name(&export)
        );
    };
    if memory.is_64() {
        bail!("QuickJS WASM memory export must use 32-bit indexes");
    }
    if memory.is_shared() {
        bail!("QuickJS WASM memory export must be unshared");
    }
    if memory.page_size() != WASM_PAGE_SIZE {
        bail!("QuickJS WASM memory export must use 64 KiB pages");
    }
    Ok(memory.minimum())
}

fn validate_stack_pointer(module: &Module) -> Result<()> {
    let export = required_export(module, "__stack_pointer")?;
    let ExternType::Global(global) = export else {
        bail!(
            "QuickJS WASM export __stack_pointer has type {}, expected mutable i32 global",
            export_type_name(&export)
        );
    };
    if !matches!(global.content(), ValType::I32) {
        bail!("QuickJS WASM export __stack_pointer must be an i32 global");
    }
    if global.mutability() != Mutability::Var {
        bail!("QuickJS WASM export __stack_pointer must be mutable");
    }
    Ok(())
}

fn validate_func(module: &Module, required: &RequiredFunc) -> Result<()> {
    let export = required_export(module, required.name)?;
    let ExternType::Func(func) = export else {
        bail!(
            "QuickJS WASM export {} has type {}, expected function {}",
            required.name,
            export_type_name(&export),
            format_expected_func(required.params, required.results)
        );
    };

    let params = func.params().collect::<Vec<_>>();
    let results = func.results().collect::<Vec<_>>();
    if !abi_values_match(&params, required.params) || !abi_values_match(&results, required.results)
    {
        bail!(
            "QuickJS WASM export {} has signature {}, expected {}",
            required.name,
            format_actual_func(&params, &results),
            format_expected_func(required.params, required.results)
        );
    }
    Ok(())
}

fn required_export(module: &Module, name: &str) -> Result<ExternType> {
    module
        .get_export(name)
        .ok_or_else(|| anyhow::anyhow!("QuickJS WASM module does not export {name}"))
}

fn abi_values_match(actual: &[ValType], expected: &[AbiVal]) -> bool {
    actual.len() == expected.len()
        && actual
            .iter()
            .zip(expected)
            .all(|(actual, expected)| abi_value_matches(actual, *expected))
}

fn abi_value_matches(actual: &ValType, expected: AbiVal) -> bool {
    matches!(
        (actual, expected),
        (ValType::I32, AbiVal::I32) | (ValType::F64, AbiVal::F64)
    )
}

fn format_actual_func(params: &[ValType], results: &[ValType]) -> String {
    format_func(
        params.iter().map(ToString::to_string),
        results.iter().map(ToString::to_string),
    )
}

fn format_expected_func(params: &[AbiVal], results: &[AbiVal]) -> String {
    format_func(
        params.iter().map(|value| value.name().to_string()),
        results.iter().map(|value| value.name().to_string()),
    )
}

fn format_func(
    params: impl IntoIterator<Item = String>,
    results: impl IntoIterator<Item = String>,
) -> String {
    let params = params.into_iter().collect::<Vec<_>>();
    let results = results.into_iter().collect::<Vec<_>>();
    format!("({}) -> ({})", params.join(", "), results.join(", "))
}

fn export_type_name(export: &ExternType) -> &'static str {
    match export {
        ExternType::Func(_) => "function",
        ExternType::Global(_) => "global",
        ExternType::Table(_) => "table",
        ExternType::Memory(_) => "memory",
        ExternType::Tag(_) => "tag",
    }
}
