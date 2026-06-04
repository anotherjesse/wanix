use super::{
    HostState, QuickJsValue, host_import_error, optional_quickjs_export, quickjs_export,
    quickjs_value_to_string, read_big_int64_value,
};
use wasmtime::{Caller, Memory};

type ScalarReader =
    fn(&Memory, &mut Caller<'_, HostState>, i32) -> wasmtime::Result<Option<QuickJsValue>>;

const SCALAR_READERS: [ScalarReader; 6] = [
    read_undefined,
    read_null,
    read_bool,
    read_number,
    read_string,
    read_big_int,
];

pub(super) fn quickjs_value_to_scalar(
    memory: &Memory,
    caller: &mut Caller<'_, HostState>,
    value: i32,
) -> wasmtime::Result<QuickJsValue> {
    maybe_quickjs_value_to_scalar(memory, caller, value)?
        .ok_or_else(|| host_import_error("unsupported host callback argument type"))
}

pub(super) fn maybe_quickjs_value_to_scalar(
    memory: &Memory,
    caller: &mut Caller<'_, HostState>,
    value: i32,
) -> wasmtime::Result<Option<QuickJsValue>> {
    for reader in SCALAR_READERS {
        if let Some(value) = reader(memory, caller, value)? {
            return Ok(Some(value));
        }
    }
    Ok(None)
}

fn read_undefined(
    _memory: &Memory,
    caller: &mut Caller<'_, HostState>,
    value: i32,
) -> wasmtime::Result<Option<QuickJsValue>> {
    let qjs_is_undefined = quickjs_export::<i32, i32>(caller, "qjs_is_undefined")?;
    if qjs_is_undefined.call(caller, value)? != 0 {
        Ok(Some(QuickJsValue::Undefined))
    } else {
        Ok(None)
    }
}

fn read_null(
    _memory: &Memory,
    caller: &mut Caller<'_, HostState>,
    value: i32,
) -> wasmtime::Result<Option<QuickJsValue>> {
    let Some(qjs_is_null) = optional_quickjs_export::<i32, i32>(caller, "qjs_is_null")? else {
        return Ok(None);
    };
    if qjs_is_null.call(caller, value)? != 0 {
        Ok(Some(QuickJsValue::Null))
    } else {
        Ok(None)
    }
}

fn read_bool(
    _memory: &Memory,
    caller: &mut Caller<'_, HostState>,
    value: i32,
) -> wasmtime::Result<Option<QuickJsValue>> {
    if !is_optional_type(caller, value, "qjs_is_bool")? {
        return Ok(None);
    }
    let qjs_get_bool = quickjs_export::<i32, i32>(caller, "qjs_get_bool")?;
    Ok(Some(QuickJsValue::Bool(
        qjs_get_bool.call(caller, value)? != 0,
    )))
}

fn read_number(
    _memory: &Memory,
    caller: &mut Caller<'_, HostState>,
    value: i32,
) -> wasmtime::Result<Option<QuickJsValue>> {
    if !is_required_type(caller, value, "qjs_is_number")? {
        return Ok(None);
    }
    let qjs_get_float64 = quickjs_export::<i32, f64>(caller, "qjs_get_float64")?;
    Ok(Some(QuickJsValue::Number(
        qjs_get_float64.call(caller, value)?,
    )))
}

fn read_string(
    memory: &Memory,
    caller: &mut Caller<'_, HostState>,
    value: i32,
) -> wasmtime::Result<Option<QuickJsValue>> {
    let qjs_is_string = quickjs_export::<i32, i32>(caller, "qjs_is_string")?;
    if qjs_is_string.call(&mut *caller, value)? == 0 {
        return Ok(None);
    }
    quickjs_value_to_string(memory, caller, value)
        .map(QuickJsValue::String)
        .map(Some)
}

fn read_big_int(
    memory: &Memory,
    caller: &mut Caller<'_, HostState>,
    value: i32,
) -> wasmtime::Result<Option<QuickJsValue>> {
    let Some(qjs_is_big_int) = optional_quickjs_export::<i32, i32>(caller, "qjs_is_big_int")?
    else {
        return Ok(None);
    };
    if qjs_is_big_int.call(&mut *caller, value)? == 0 {
        return Ok(None);
    }
    read_big_int64_value(memory, caller, value)
        .map(QuickJsValue::BigIntI64)
        .map(Some)
}

fn is_required_type(
    caller: &mut Caller<'_, HostState>,
    value: i32,
    export: &'static str,
) -> wasmtime::Result<bool> {
    let predicate = quickjs_export::<i32, i32>(caller, export)?;
    Ok(predicate.call(caller, value)? != 0)
}

fn is_optional_type(
    caller: &mut Caller<'_, HostState>,
    value: i32,
    export: &'static str,
) -> wasmtime::Result<bool> {
    let Some(predicate) = optional_quickjs_export::<i32, i32>(caller, export)? else {
        return Ok(false);
    };
    Ok(predicate.call(caller, value)? != 0)
}
