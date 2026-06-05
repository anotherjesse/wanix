use super::guest_memory::guest_len;
use super::{HostCallbackMode, HostState, QuickJsCopiedValue, QuickJsValue, caller_memory};
use crate::guest::{guest_i32_add, guest_offset, i64_from_guest_u32_halves};
use wasmtime::{Caller, Extern, Linker, Memory, TypedFunc, WasmParams, WasmResults};

mod args;
mod binary;
mod guest_read;
mod output;
mod scalar;
use args::read_host_callback_args;
use guest_read::{quickjs_pending_exception_string, read_guest_u32, read_guest_utf8};
use output::{host_error_to_js_exception, quickjs_callback_value_to_js};

const JS_VALUE_PTR_LEN: usize = 4;
const BIG_INT64_WORD_SIZE: u32 = 4;
const BIG_INT64_OUTPUT_SIZE: i32 = 8;

pub(super) fn define_import(linker: &mut Linker<HostState>) -> anyhow::Result<()> {
    linker.func_wrap(
        "env",
        "host_call",
        |mut caller: Caller<'_, HostState>,
         name_ptr: i32,
         name_len: i32,
         _this_value: i32,
         argc: i32,
         argv: i32|
         -> wasmtime::Result<i32> {
            if caller.data().memory().is_none() {
                return Ok(0);
            }
            match dispatch_host_call(&mut caller, name_ptr, name_len, argc, argv) {
                Ok(value) => Ok(value),
                Err(err) => host_error_to_js_exception(&mut caller, err),
            }
        },
    )?;
    Ok(())
}

fn dispatch_host_call(
    caller: &mut Caller<'_, HostState>,
    name_ptr: i32,
    name_len: i32,
    argc: i32,
    argv: i32,
) -> wasmtime::Result<i32> {
    let memory = caller_memory(caller)?;
    let name = read_guest_utf8(
        &memory,
        caller,
        guest_offset(name_ptr),
        guest_len(name_len)?,
        "host callback name",
    )?;
    let mode = caller
        .data()
        .host_callback_mode(&name)
        .map_err(|err| host_import_error(format!("{err:#}")))?;
    let args = read_host_callback_args(&memory, caller, argc, argv, mode)?;
    let value = caller
        .data_mut()
        .call_host_callback(&name, &args)
        .map_err(|err| host_import_error(format!("{err:#}")))?;
    quickjs_callback_value_to_js(caller, value)
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

fn read_big_int64_value(
    memory: &Memory,
    caller: &mut Caller<'_, HostState>,
    value: i32,
) -> wasmtime::Result<i64> {
    let wasm_malloc = quickjs_export::<i32, i32>(caller, "wasm_malloc")?;
    let out = wasm_malloc.call(&mut *caller, BIG_INT64_OUTPUT_SIZE)?;
    if out == 0 {
        return Err(host_import_error(
            "wasm_malloc returned null for QuickJS BigInt output words",
        ));
    }

    let result = (|| {
        let hi_out = guest_i32_add(out, BIG_INT64_WORD_SIZE)
            .ok_or_else(|| host_import_error("BigInt output pointer overflowed"))?;
        let qjs_get_big_int64 =
            quickjs_export::<(i32, i32, i32), i32>(caller, "qjs_get_big_int64")?;
        let ret = qjs_get_big_int64.call(&mut *caller, (value, out, hi_out))?;
        if ret != 0 {
            let message = quickjs_pending_exception_string(memory, caller)
                .unwrap_or_else(|err| format!("failed to take QuickJS exception: {err:#}"));
            return Err(host_import_error(format!(
                "qjs_get_big_int64 failed: {message}"
            )));
        }
        let lo = read_guest_u32(memory, caller, out, "BigInt low word")?;
        let hi = read_guest_u32(memory, caller, hi_out, "BigInt high word")?;
        Ok(i64_from_guest_u32_halves(lo, hi))
    })();

    let wasm_free = quickjs_export::<i32, ()>(caller, "wasm_free")?;
    let cleanup = wasm_free.call(caller, out);
    finish_host_cleanup(result, cleanup)
}

fn finish_host_cleanup<T>(
    result: wasmtime::Result<T>,
    cleanup: wasmtime::Result<()>,
) -> wasmtime::Result<T> {
    match (result, cleanup) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(err), Ok(())) => Err(err),
        (Ok(_value), Err(cleanup_err)) => Err(wasmtime::Error::msg(format!(
            "host cleanup failed after successful result: {cleanup_err:#}"
        ))),
        (Err(err), Err(cleanup_err)) => Err(wasmtime::Error::msg(format!(
            "{err:#}; additionally, host cleanup failed: {cleanup_err:#}"
        ))),
    }
}

fn quickjs_export<P, R>(
    caller: &mut Caller<'_, HostState>,
    name: &str,
) -> wasmtime::Result<TypedFunc<P, R>>
where
    P: WasmParams,
    R: WasmResults,
{
    let Some(Extern::Func(func)) = caller.get_export(name) else {
        return Err(host_import_error(format!(
            "QuickJS WASM module does not export {name}"
        )));
    };
    func.typed::<P, R>(&*caller)
        .map_err(|err| wasmtime::Error::msg(format!("missing or mistyped {name} export: {err:#}")))
}

fn optional_quickjs_export<P, R>(
    caller: &mut Caller<'_, HostState>,
    name: &str,
) -> wasmtime::Result<Option<TypedFunc<P, R>>>
where
    P: WasmParams,
    R: WasmResults,
{
    let Some(export) = caller.get_export(name) else {
        return Ok(None);
    };
    let Extern::Func(func) = export else {
        return Err(host_import_error(format!(
            "missing or mistyped {name} export: export is not a function"
        )));
    };
    func.typed::<P, R>(&*caller)
        .map(Some)
        .map_err(|err| wasmtime::Error::msg(format!("missing or mistyped {name} export: {err:#}")))
}

fn ensure_non_null_js_value(ptr: i32, source: &str) -> wasmtime::Result<i32> {
    if ptr == 0 {
        return Err(host_import_error(format!(
            "{source} returned a null JSValue pointer"
        )));
    }
    Ok(ptr)
}

fn host_import_error(message: impl Into<String>) -> wasmtime::Error {
    wasmtime::Error::msg(message.into())
}
