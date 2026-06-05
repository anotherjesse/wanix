use super::binary::quickjs_value_to_callback;
use super::guest_read::read_guest_i32;
use super::scalar::quickjs_value_to_scalar;
use super::{HostCallbackMode, HostState, JS_VALUE_PTR_LEN, QuickJsCopiedValue, host_import_error};
use crate::host::guest_memory::{guest_len, guest_offset_at};
use wasmtime::{Caller, Memory};

pub(super) fn read_host_callback_args(
    memory: &Memory,
    caller: &mut Caller<'_, HostState>,
    argc: i32,
    argv: i32,
    mode: HostCallbackMode,
) -> wasmtime::Result<Vec<QuickJsCopiedValue>> {
    let argc = guest_len(argc)?;
    let mut args = Vec::new();
    args.try_reserve_exact(argc).map_err(|err| {
        wasmtime::Error::msg(format!("host callback args allocation failed: {err}"))
    })?;
    if argc == 0 {
        return Ok(args);
    }
    if argv == 0 {
        return Err(host_import_error(format!(
            "host callback argv pointer is null for {argc} arguments"
        )));
    }
    for index in 0..argc {
        let ptr = read_guest_i32(
            memory,
            caller,
            guest_offset_at(argv, index, JS_VALUE_PTR_LEN)?,
        )?;
        let arg = match mode {
            HostCallbackMode::Scalar => quickjs_value_to_scalar(memory, caller, ptr)?.into(),
            HostCallbackMode::BinaryCapable => quickjs_value_to_callback(memory, caller, ptr)?,
        };
        args.push(arg);
    }
    Ok(args)
}
