use super::guest_memory::guest_range;
use super::{HostState, caller_memory};
use crate::allocation::try_copy_str;
use crate::guest::guest_offset;
use wasmtime::{Caller, Extern, Linker, Memory, TypedFunc, WasmParams, WasmResults};

/// A copied promise rejection notification from QuickJS.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuickJsPromiseRejection {
    reason: String,
    is_handled: bool,
}

impl QuickJsPromiseRejection {
    pub(crate) fn new(reason: String, is_handled: bool) -> Self {
        Self { reason, is_handled }
    }

    /// Returns the rejection reason converted to a string by QuickJS.
    #[must_use]
    pub fn reason(&self) -> &str {
        &self.reason
    }

    /// Returns `true` when QuickJS is notifying that a previously unhandled
    /// rejection has gained a handler.
    #[must_use]
    pub fn is_handled(&self) -> bool {
        self.is_handled
    }

    /// Consumes the event and returns the owned reason string.
    #[must_use]
    pub fn into_reason(self) -> String {
        self.reason
    }
}

pub(crate) type PromiseRejectionHandler = Box<dyn FnMut(QuickJsPromiseRejection) + Send + 'static>;

pub(super) fn define_import(linker: &mut Linker<HostState>) -> anyhow::Result<()> {
    linker.func_wrap(
        "env",
        "host_promise_rejection",
        |mut caller: Caller<'_, HostState>,
         promise: i32,
         reason: i32,
         is_handled: i32|
         -> wasmtime::Result<()> {
            dispatch_promise_rejection(&mut caller, promise, reason, is_handled != 0)
        },
    )?;
    Ok(())
}

fn dispatch_promise_rejection(
    caller: &mut Caller<'_, HostState>,
    promise: i32,
    reason: i32,
    is_handled: bool,
) -> wasmtime::Result<()> {
    if !caller.data().has_promise_rejection_handler() {
        return free_promise_rejection_values(caller, promise, reason);
    }

    let reason_string = rejection_reason_to_string(caller, reason);
    let result = {
        caller
            .data_mut()
            .handle_promise_rejection(QuickJsPromiseRejection::new(reason_string, is_handled));
        Ok(())
    };
    let cleanup = free_promise_rejection_values(caller, promise, reason);
    finish_host_cleanup(result, cleanup)
}

fn rejection_reason_to_string(caller: &mut Caller<'_, HostState>, reason: i32) -> String {
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

const PROMISE_REJECTION_UNSTRINGIFIABLE_REASON: &str =
    "[promise rejection reason could not be stringified]";

fn free_promise_rejection_values(
    caller: &mut Caller<'_, HostState>,
    promise: i32,
    reason: i32,
) -> wasmtime::Result<()> {
    let qjs_free_value = quickjs_export::<i32, ()>(caller, "qjs_free_value")?;
    let promise_cleanup = free_optional_value(caller, &qjs_free_value, promise);
    let reason_cleanup = free_optional_value(caller, &qjs_free_value, reason);
    combine_cleanup(promise_cleanup, reason_cleanup)
}

fn free_optional_value(
    caller: &mut Caller<'_, HostState>,
    qjs_free_value: &TypedFunc<i32, ()>,
    value: i32,
) -> wasmtime::Result<()> {
    if value == 0 {
        return Ok(());
    }
    qjs_free_value.call(caller, value).map_err(|err| {
        host_import_error(format!("failed to free promise rejection value: {err:#}"))
    })
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
        .map_err(|err| host_import_error(format!("missing or mistyped {name} export: {err:#}")))
}

fn finish_host_cleanup<T>(
    result: wasmtime::Result<T>,
    cleanup: wasmtime::Result<()>,
) -> wasmtime::Result<T> {
    match (result, cleanup) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(err), Ok(())) => Err(err),
        (Ok(_value), Err(cleanup_err)) => Err(host_import_error(format!(
            "host cleanup failed after successful result: {cleanup_err:#}"
        ))),
        (Err(err), Err(cleanup_err)) => Err(host_import_error(format!(
            "{err:#}; additionally, host cleanup failed: {cleanup_err:#}"
        ))),
    }
}

fn combine_cleanup(
    first: wasmtime::Result<()>,
    second: wasmtime::Result<()>,
) -> wasmtime::Result<()> {
    match (first, second) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(err), Ok(())) | (Ok(()), Err(err)) => Err(err),
        (Err(first), Err(second)) => Err(host_import_error(format!(
            "{first:#}; additionally, host cleanup failed: {second:#}"
        ))),
    }
}

fn host_import_error(message: impl Into<String>) -> wasmtime::Error {
    wasmtime::Error::msg(message.into())
}
