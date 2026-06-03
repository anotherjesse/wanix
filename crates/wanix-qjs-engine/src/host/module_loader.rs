use super::guest_memory::guest_range;
use super::{HostState, caller_memory};
use crate::allocation::try_copy_str;
use crate::guest::{guest_offset, host_len_i32};
use wasmtime::{Caller, Extern, Linker, TypedFunc};

pub(crate) type ModuleLoadCallback =
    Box<dyn FnMut(&str) -> anyhow::Result<String> + Send + 'static>;
pub(crate) type ModuleNormalizeCallback =
    Box<dyn FnMut(&str, &str) -> anyhow::Result<String> + Send + 'static>;

pub(crate) struct ModuleLoader {
    pub(crate) normalize: Option<ModuleNormalizeCallback>,
    pub(crate) load: ModuleLoadCallback,
}

pub(super) fn define_imports(linker: &mut Linker<HostState>) -> anyhow::Result<()> {
    linker.func_wrap(
        "env",
        "host_module_normalize",
        |mut caller: Caller<'_, HostState>, base_name: i32, name: i32| -> wasmtime::Result<i32> {
            match dispatch_module_normalize(&mut caller, base_name, name) {
                Ok(ptr) => Ok(ptr),
                Err(_err) => Ok(0),
            }
        },
    )?;
    linker.func_wrap(
        "env",
        "host_module_load",
        |mut caller: Caller<'_, HostState>, name: i32, out_len: i32| -> wasmtime::Result<i32> {
            match dispatch_module_load(&mut caller, name, out_len) {
                Ok(ptr) => Ok(ptr),
                Err(_err) => Ok(0),
            }
        },
    )?;
    Ok(())
}

fn dispatch_module_normalize(
    caller: &mut Caller<'_, HostState>,
    base_name: i32,
    name: i32,
) -> wasmtime::Result<i32> {
    let memory = caller_memory(caller)?;
    let base_name = if base_name == 0 {
        String::new()
    } else {
        read_guest_c_string(&memory, caller, base_name)?
    };
    let name = read_guest_c_string(&memory, caller, name)?;
    let normalized = caller
        .data_mut()
        .normalize_module(&base_name, &name)
        .map_err(|err| module_loader_error(format!("{err:#}")))?;
    validate_c_string_value(&normalized, "normalized module name")?;
    write_guest_c_string(caller, &normalized)
}

fn dispatch_module_load(
    caller: &mut Caller<'_, HostState>,
    name: i32,
    out_len: i32,
) -> wasmtime::Result<i32> {
    if out_len == 0 {
        return Err(module_loader_error("null module loader out_len pointer"));
    }
    let memory = caller_memory(caller)?;
    let name = read_guest_c_string(&memory, caller, name)?;
    let source = caller
        .data_mut()
        .load_module(&name)
        .map_err(|err| module_loader_error(format!("{err:#}")))?;
    let ptr = write_guest_c_string(caller, &source)?;
    let write_len = write_guest_u32(caller, out_len, source_len_u32(&source));
    match write_len {
        Ok(()) => Ok(ptr),
        Err(err) => {
            let _ = free_guest_allocation(caller, ptr);
            Err(err)
        }
    }
}

fn source_len_u32(source: &str) -> u32 {
    u32::try_from(source.len()).unwrap_or(u32::MAX)
}

fn validate_c_string_value(value: &str, label: &str) -> wasmtime::Result<()> {
    if value.as_bytes().contains(&0) {
        return Err(module_loader_error(format!(
            "{label} must not contain NUL bytes"
        )));
    }
    Ok(())
}

fn write_guest_c_string(caller: &mut Caller<'_, HostState>, value: &str) -> wasmtime::Result<i32> {
    let bytes = value.as_bytes();
    let len = u32::try_from(bytes.len())
        .map_err(|_| module_loader_error("module loader string is too large"))?;
    let alloc_len = len
        .checked_add(1)
        .ok_or_else(|| module_loader_error("module loader string allocation overflowed"))?;
    let alloc_len_i32 = host_len_i32(alloc_len)
        .ok_or_else(|| module_loader_error("module loader string is too large"))?;

    let wasm_malloc = quickjs_export::<i32, i32>(caller, "wasm_malloc")?;
    let ptr = wasm_malloc.call(&mut *caller, alloc_len_i32)?;
    if ptr == 0 {
        return Err(module_loader_error(
            "wasm_malloc returned null for module loader string",
        ));
    }

    let result = write_guest_bytes(caller, ptr, bytes).and_then(|()| {
        let terminator = guest_offset(ptr)
            .checked_add(bytes.len())
            .ok_or_else(|| module_loader_error("module loader string pointer overflowed"))?;
        let memory = caller_memory(caller)?;
        memory.write(&mut *caller, terminator, &[0])?;
        Ok(ptr)
    });

    if result.is_err() {
        let _ = free_guest_allocation(caller, ptr);
    }
    result
}

fn write_guest_bytes(
    caller: &mut Caller<'_, HostState>,
    ptr: i32,
    bytes: &[u8],
) -> wasmtime::Result<()> {
    if bytes.is_empty() {
        return Ok(());
    }
    let memory = caller_memory(caller)?;
    memory.write(caller, guest_offset(ptr), bytes)?;
    Ok(())
}

fn write_guest_u32(
    caller: &mut Caller<'_, HostState>,
    ptr: i32,
    value: u32,
) -> wasmtime::Result<()> {
    let memory = caller_memory(caller)?;
    let range = guest_range(&memory, caller, guest_offset(ptr), 4)?;
    if range.len() < 4 {
        return Err(module_loader_error(
            "module loader out_len range is too small",
        ));
    }
    memory.write(caller, range.start, &value.to_le_bytes())?;
    Ok(())
}

fn free_guest_allocation(caller: &mut Caller<'_, HostState>, ptr: i32) -> wasmtime::Result<()> {
    let wasm_free = quickjs_export::<i32, ()>(caller, "wasm_free")?;
    wasm_free.call(caller, ptr)
}

fn read_guest_c_string(
    memory: &wasmtime::Memory,
    caller: &Caller<'_, HostState>,
    ptr: i32,
) -> wasmtime::Result<String> {
    if ptr == 0 {
        return Err(module_loader_error("null module loader C string pointer"));
    }
    let data = memory.data(caller);
    let start = guest_offset(ptr);
    if start >= data.len() {
        return Err(module_loader_error(
            "module loader C string pointer is outside memory",
        ));
    }
    let mut end = start;
    while end < data.len() && data[end] != 0 {
        end += 1;
    }
    if end == data.len() {
        return Err(module_loader_error("unterminated module loader C string"));
    }
    read_guest_utf8(memory, caller, start, end - start, "module loader C string")
}

fn read_guest_utf8(
    memory: &wasmtime::Memory,
    caller: &Caller<'_, HostState>,
    ptr: usize,
    len: usize,
    label: &str,
) -> wasmtime::Result<String> {
    let range = guest_range(memory, caller, ptr, len)?;
    let bytes = &memory.data(caller)[range];
    let value = std::str::from_utf8(bytes)
        .map_err(|err| module_loader_error(format!("{label} was not valid UTF-8: {err}")))?;
    try_copy_str(value, label).map_err(|err| module_loader_error(format!("{err:#}")))
}

fn quickjs_export<P, R>(
    caller: &mut Caller<'_, HostState>,
    name: &str,
) -> wasmtime::Result<TypedFunc<P, R>>
where
    P: wasmtime::WasmParams,
    R: wasmtime::WasmResults,
{
    let Some(Extern::Func(func)) = caller.get_export(name) else {
        return Err(module_loader_error(format!(
            "QuickJS WASM module does not export {name}"
        )));
    };
    func.typed::<P, R>(&*caller)
        .map_err(|err| module_loader_error(format!("missing or mistyped {name} export: {err:#}")))
}

fn module_loader_error(message: impl Into<String>) -> wasmtime::Error {
    wasmtime::Error::msg(message.into())
}
