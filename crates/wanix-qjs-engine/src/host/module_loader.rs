mod guest;

use super::{HostState, caller_memory};
use guest::{
    free_guest_allocation, module_loader_error, read_guest_c_string, validate_c_string_value,
    write_guest_c_string, write_guest_u32,
};
use wasmtime::{Caller, Linker};

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
