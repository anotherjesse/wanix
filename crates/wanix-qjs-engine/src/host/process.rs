use super::guest_memory::{guest_offset_at, guest_range};
use super::{ERRNO_INVAL, ERRNO_SUCCESS, HostState, caller_memory};
use crate::guest::guest_offset;
use wasmtime::{Caller, Linker, Memory};

const WASI_U32_SIZE: usize = 4;

pub(super) fn define_imports(linker: &mut Linker<HostState>) -> anyhow::Result<()> {
    linker.func_wrap("wasi_snapshot_preview1", "args_sizes_get", args_sizes_get)?;
    linker.func_wrap("wasi_snapshot_preview1", "args_get", args_get)?;
    linker.func_wrap(
        "wasi_snapshot_preview1",
        "environ_sizes_get",
        environ_sizes_get,
    )?;
    linker.func_wrap("wasi_snapshot_preview1", "environ_get", environ_get)?;
    Ok(())
}

fn args_sizes_get(
    caller: Caller<'_, HostState>,
    count_ptr: i32,
    buf_size_ptr: i32,
) -> wasmtime::Result<i32> {
    string_array_sizes_get(caller, ProcessStringArray::Args, count_ptr, buf_size_ptr)
}

fn args_get(
    caller: Caller<'_, HostState>,
    argv_ptr: i32,
    argv_buf_ptr: i32,
) -> wasmtime::Result<i32> {
    string_array_get(caller, ProcessStringArray::Args, argv_ptr, argv_buf_ptr)
}

fn environ_sizes_get(
    caller: Caller<'_, HostState>,
    count_ptr: i32,
    buf_size_ptr: i32,
) -> wasmtime::Result<i32> {
    string_array_sizes_get(caller, ProcessStringArray::Env, count_ptr, buf_size_ptr)
}

fn environ_get(
    caller: Caller<'_, HostState>,
    environ_ptr: i32,
    environ_buf_ptr: i32,
) -> wasmtime::Result<i32> {
    string_array_get(
        caller,
        ProcessStringArray::Env,
        environ_ptr,
        environ_buf_ptr,
    )
}

#[derive(Clone, Copy)]
enum ProcessStringArray {
    Args,
    Env,
}

fn string_array_sizes_get(
    mut caller: Caller<'_, HostState>,
    array: ProcessStringArray,
    count_ptr: i32,
    buf_size_ptr: i32,
) -> wasmtime::Result<i32> {
    let strings = match process_strings(&caller, array)? {
        Ok(strings) => strings,
        Err(errno) => return Ok(errno.preview1_result()),
    };
    let encoded = match encode_strings(strings)? {
        Ok(encoded) => encoded,
        Err(errno) => return Ok(errno),
    };
    let count = u32::try_from(encoded.len())
        .map_err(|_| wasmtime::Error::msg("WASI string array count exceeds u32"))?;
    let buf_size = encoded_buf_size(&encoded)?;
    let memory = caller_memory(&caller)?;
    guest_range(&memory, &caller, guest_offset(count_ptr), WASI_U32_SIZE)?;
    guest_range(&memory, &caller, guest_offset(buf_size_ptr), WASI_U32_SIZE)?;
    memory.write(&mut caller, guest_offset(count_ptr), &count.to_le_bytes())?;
    memory.write(
        &mut caller,
        guest_offset(buf_size_ptr),
        &buf_size.to_le_bytes(),
    )?;
    Ok(ERRNO_SUCCESS)
}

fn string_array_get(
    mut caller: Caller<'_, HostState>,
    array: ProcessStringArray,
    ptrs_ptr: i32,
    buf_ptr: i32,
) -> wasmtime::Result<i32> {
    let strings = match process_strings(&caller, array)? {
        Ok(strings) => strings,
        Err(errno) => return Ok(errno.preview1_result()),
    };
    let encoded = match encode_strings(strings)? {
        Ok(encoded) => encoded,
        Err(errno) => return Ok(errno),
    };
    let memory = caller_memory(&caller)?;
    write_string_array(&memory, &mut caller, &encoded, ptrs_ptr, buf_ptr)?;
    Ok(ERRNO_SUCCESS)
}

fn process_strings(
    caller: &Caller<'_, HostState>,
    array: ProcessStringArray,
) -> wasmtime::Result<Result<Vec<String>, super::QuickJsWasiErrno>> {
    let Some(host) = caller.data().wasi_host() else {
        return Ok(Ok(Vec::new()));
    };
    let mut host = host
        .lock()
        .map_err(|_| wasmtime::Error::msg("QuickJS WASI host lock poisoned"))?;
    Ok(match array {
        ProcessStringArray::Args => host.args(),
        ProcessStringArray::Env => host.env(),
    })
}

fn encode_strings(strings: Vec<String>) -> wasmtime::Result<Result<Vec<Vec<u8>>, i32>> {
    let mut encoded = Vec::new();
    encoded
        .try_reserve_exact(strings.len())
        .map_err(|_| wasmtime::Error::msg("WASI string array allocation failed"))?;
    for string in strings {
        if string.as_bytes().contains(&0) {
            return Ok(Err(ERRNO_INVAL));
        }
        encoded.push(string.into_bytes());
    }
    Ok(Ok(encoded))
}

fn encoded_buf_size(strings: &[Vec<u8>]) -> wasmtime::Result<u32> {
    strings.iter().try_fold(0u32, |total, string| {
        let len = u32::try_from(string.len())
            .map_err(|_| wasmtime::Error::msg("WASI string length exceeds u32"))?;
        total
            .checked_add(len)
            .and_then(|total| total.checked_add(1))
            .ok_or_else(|| wasmtime::Error::msg("WASI string buffer size overflow"))
    })
}

fn write_string_array(
    memory: &Memory,
    caller: &mut Caller<'_, HostState>,
    strings: &[Vec<u8>],
    ptrs_ptr: i32,
    buf_ptr: i32,
) -> wasmtime::Result<()> {
    if !strings.is_empty() {
        guest_range(
            memory,
            &*caller,
            guest_offset_at(ptrs_ptr, strings.len() - 1, WASI_U32_SIZE)?,
            WASI_U32_SIZE,
        )?;
    }
    let buf_size = usize::try_from(encoded_buf_size(strings)?)
        .map_err(|_| wasmtime::Error::msg("WASI string buffer size exceeds usize"))?;
    let buf_start = guest_offset(buf_ptr);
    guest_range(memory, &*caller, buf_start, buf_size)?;

    let mut offset = 0usize;
    for (index, string) in strings.iter().enumerate() {
        let string_ptr = buf_start
            .checked_add(offset)
            .filter(|ptr| *ptr <= u32::MAX as usize)
            .ok_or_else(|| wasmtime::Error::msg("WASI string pointer overflow"))?;
        let string_ptr = u32::try_from(string_ptr)
            .map_err(|_| wasmtime::Error::msg("WASI string pointer exceeds u32"))?;
        memory.write(
            &mut *caller,
            guest_offset_at(ptrs_ptr, index, WASI_U32_SIZE)?,
            &string_ptr.to_le_bytes(),
        )?;
        memory.write(&mut *caller, buf_start + offset, string)?;
        offset += string.len();
        memory.write(&mut *caller, buf_start + offset, &[0])?;
        offset += 1;
    }
    Ok(())
}
