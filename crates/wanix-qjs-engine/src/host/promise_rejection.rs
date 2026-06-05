use super::HostState;
use wasmtime::{Caller, Extern, Linker, TypedFunc, WasmParams, WasmResults};

mod reason;
use reason::rejection_reason_to_string;

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
