use super::{
    HostState, PROMISE_REJECTION_UNSTRINGIFIABLE_REASON, finish_host_cleanup, host_import_error,
    quickjs_export,
};
use crate::allocation::try_copy_str;
use crate::guest::guest_offset;
use crate::host::caller_memory;
use crate::host::guest_memory::guest_range;
use wasmtime::{Caller, Memory};

pub(super) fn rejection_reason_to_string(
    caller: &mut Caller<'_, HostState>,
    reason: i32,
) -> String {
    let qjs_get_string = match quickjs_export::<i32, i32>(caller, "qjs_get_string") {
        Ok(qjs_get_string) => qjs_get_string,
        Err(_err) => return PROMISE_REJECTION_UNSTRINGIFIABLE_REASON.to_string(),
    };
    let c_string = match qjs_get_string.call(&mut *caller, reason) {
        Ok(c_string) => c_string,
        Err(_err) => return PROMISE_REJECTION_UNSTRINGIFIABLE_REASON.to_string(),
    };
    if c_string == 0 {
        return PROMISE_REJECTION_UNSTRINGIFIABLE_REASON.to_string();
    }
    match read_and_free_quickjs_c_string(caller, c_string) {
        Ok(reason) => reason,
        Err(_err) => PROMISE_REJECTION_UNSTRINGIFIABLE_REASON.to_string(),
    }
}

fn read_and_free_quickjs_c_string(
    caller: &mut Caller<'_, HostState>,
    ptr: i32,
) -> wasmtime::Result<String> {
    let memory = caller_memory(caller)?;
    let string = read_guest_c_string(&memory, caller, ptr);
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
        return Err(host_import_error("null promise rejection reason string"));
    }
    let data = memory.data(caller);
    let start = guest_offset(ptr);
    if start >= data.len() {
        return Err(host_import_error(
            "promise rejection reason string pointer is outside memory",
        ));
    }
    let mut end = start;
    while end < data.len() && data[end] != 0 {
        end += 1;
    }
    if end == data.len() {
        return Err(host_import_error(
            "unterminated promise rejection reason string",
        ));
    }
    read_guest_utf8(
        memory,
        caller,
        start,
        end - start,
        "promise rejection reason string",
    )
}

fn read_guest_utf8(
    memory: &Memory,
    caller: &Caller<'_, HostState>,
    ptr: usize,
    len: usize,
    label: &str,
) -> wasmtime::Result<String> {
    let range = guest_range(memory, caller, ptr, len)?;
    let bytes = &memory.data(caller)[range];
    let value = std::str::from_utf8(bytes)
        .map_err(|err| host_import_error(format!("{label} was not valid UTF-8: {err}")))?;
    try_copy_str(value, label).map_err(|err| host_import_error(format!("{err:#}")))
}
