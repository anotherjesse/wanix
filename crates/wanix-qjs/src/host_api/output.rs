use std::sync::{Arc, Mutex};

use anyhow::anyhow;
use rust_wasi_quickjs::{QuickJsHostValue, QuickJsRuntime};
use wanix_fs::{FsError, FsResult};

use crate::host_api::qjs_error;
use crate::task_context::WanixExitState;

pub(crate) fn define_output_callback(
    runtime: &mut QuickJsRuntime,
    name: &'static str,
    output: Arc<Mutex<Vec<u8>>>,
) -> FsResult<()> {
    define_output_callback_inner(runtime, name, output, None)
}

pub(crate) fn define_output_callback_with_exit_state(
    runtime: &mut QuickJsRuntime,
    name: &'static str,
    output: Arc<Mutex<Vec<u8>>>,
    exit_state: WanixExitState,
) -> FsResult<()> {
    define_output_callback_inner(runtime, name, output, Some(exit_state))
}

fn define_output_callback_inner(
    runtime: &mut QuickJsRuntime,
    name: &'static str,
    output: Arc<Mutex<Vec<u8>>>,
    exit_state: Option<WanixExitState>,
) -> FsResult<()> {
    runtime
        .define_global_host_function(name, move |args| {
            if exit_requested(&exit_state)? {
                return Ok(QuickJsHostValue::Undefined);
            }
            let text = args.iter().map(display_host_value).collect::<String>();
            output
                .lock()
                .map_err(|_| anyhow!("output buffer lock poisoned"))?
                .extend_from_slice(text.as_bytes());
            Ok(QuickJsHostValue::Undefined)
        })
        .map_err(qjs_error)
}

pub(crate) fn take_buffer(buffer: Arc<Mutex<Vec<u8>>>) -> FsResult<Vec<u8>> {
    let mut buffer = buffer
        .lock()
        .map_err(|_| FsError::Other("output buffer lock poisoned".to_owned()))?;
    Ok(std::mem::take(&mut *buffer))
}

fn exit_requested(exit_state: &Option<WanixExitState>) -> anyhow::Result<bool> {
    match exit_state {
        Some(exit_state) => exit_state.is_requested(),
        None => Ok(false),
    }
}

pub(crate) fn display_host_value(value: &QuickJsHostValue) -> String {
    match value {
        QuickJsHostValue::Undefined => "undefined".to_owned(),
        QuickJsHostValue::Null => "null".to_owned(),
        QuickJsHostValue::Bool(value) => value.to_string(),
        QuickJsHostValue::Number(value) => value.to_string(),
        QuickJsHostValue::String(value) => value.clone(),
        QuickJsHostValue::BigIntI64(value) => value.to_string(),
        _ => "[unsupported]".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::{QuickJsHostValue, display_host_value};

    #[test]
    fn display_host_value_formats_current_scalar_values() {
        let cases = [
            (QuickJsHostValue::Undefined, "undefined"),
            (QuickJsHostValue::Null, "null"),
            (QuickJsHostValue::Bool(true), "true"),
            (QuickJsHostValue::Bool(false), "false"),
            (QuickJsHostValue::Number(12.5), "12.5"),
            (QuickJsHostValue::String("text".to_owned()), "text"),
            (QuickJsHostValue::BigIntI64(-42), "-42"),
        ];

        for (value, expected) in cases {
            assert_eq!(display_host_value(&value), expected);
        }
    }
}
