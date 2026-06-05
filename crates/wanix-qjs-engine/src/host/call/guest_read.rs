use super::{HostState, JS_VALUE_PTR_LEN, finish_host_cleanup, host_import_error, quickjs_export};
use crate::allocation::try_copy_str;
use crate::guest::guest_offset;
use crate::host::guest_memory::guest_range;
use wasmtime::{Caller, Memory};

pub(super) fn quickjs_value_to_string(
    memory: &Memory,
    caller: &mut Caller<'_, HostState>,
    value: i32,
) -> wasmtime::Result<String> {
    let qjs_get_string = quickjs_export::<i32, i32>(caller, "qjs_get_string")?;
    let c_string = qjs_get_string.call(&mut *caller, value)?;
    if c_string == 0 {
        return Err(host_import_error(
            "qjs_get_string returned a null C string pointer",
        ));
    }
    read_and_free_quickjs_c_string(memory, caller, c_string)
}

pub(super) fn quickjs_pending_exception_string(
    memory: &Memory,
    caller: &mut Caller<'_, HostState>,
) -> wasmtime::Result<String> {
    let qjs_get_exception = quickjs_export::<(), i32>(caller, "qjs_get_exception")?;
    let exception = qjs_get_exception.call(&mut *caller, ())?;
    if exception == 0 {
        return Err(host_import_error(
            "qjs_get_exception returned a null JSValue pointer",
        ));
    }
    let result = quickjs_value_to_string(memory, caller, exception);
    let qjs_free_value = quickjs_export::<i32, ()>(caller, "qjs_free_value")?;
    let cleanup = qjs_free_value.call(caller, exception);
    finish_host_cleanup(result, cleanup)
}

pub(super) fn read_and_free_quickjs_c_string(
    memory: &Memory,
    caller: &mut Caller<'_, HostState>,
    ptr: i32,
) -> wasmtime::Result<String> {
    let string = read_guest_c_string(memory, caller, ptr);
    let qjs_free_cstring = quickjs_export::<i32, ()>(caller, "qjs_free_cstring")?;
    let cleanup = qjs_free_cstring.call(caller, ptr);
    finish_host_cleanup(string, cleanup)
}

fn read_guest_c_string(
    memory: &Memory,
    caller: &Caller<'_, HostState>,
    ptr: i32,
) -> wasmtime::Result<String> {
    if ptr == 0 {
        return Err(host_import_error("null guest C string pointer"));
    }
    let data = memory.data(caller);
    let start = guest_offset(ptr);
    if start >= data.len() {
        return Err(host_import_error(
            "guest C string pointer is outside memory",
        ));
    }
    let mut end = start;
    while end < data.len() && data[end] != 0 {
        end += 1;
    }
    if end == data.len() {
        return Err(host_import_error("unterminated guest C string"));
    }
    read_guest_utf8(memory, caller, start, end - start, "guest C string")
}

pub(super) fn read_guest_u32(
    memory: &Memory,
    caller: &Caller<'_, HostState>,
    ptr: i32,
    _label: &str,
) -> wasmtime::Result<u32> {
    let range = guest_range(memory, caller, guest_offset(ptr), 4)?;
    let mut bytes = [0; 4];
    bytes.copy_from_slice(&memory.data(caller)[range]);
    Ok(u32::from_le_bytes(bytes))
}

pub(super) fn read_guest_utf8(
    memory: &Memory,
    caller: &Caller<'_, HostState>,
    ptr: usize,
    len: usize,
    label: &str,
) -> wasmtime::Result<String> {
    let range = guest_range(memory, caller, ptr, len)?;
    let bytes = &memory.data(caller)[range];
    let value = std::str::from_utf8(bytes)
        .map_err(|err| wasmtime::Error::msg(format!("{label} was not valid UTF-8: {err}")))?;
    try_copy_str(value, label).map_err(|err| wasmtime::Error::msg(format!("{err:#}")))
}

pub(super) fn read_guest_i32(
    memory: &Memory,
    caller: &Caller<'_, HostState>,
    ptr: usize,
) -> wasmtime::Result<i32> {
    let range = guest_range(memory, caller, ptr, JS_VALUE_PTR_LEN)?;
    let mut bytes = [0; JS_VALUE_PTR_LEN];
    bytes.copy_from_slice(&memory.data(caller)[range]);
    Ok(i32::from_le_bytes(bytes))
}
