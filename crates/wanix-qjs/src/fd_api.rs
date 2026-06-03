use anyhow::{anyhow, bail};
use rust_wasi_quickjs::{QuickJsHostValue, QuickJsRuntime};
use wanix_fs::{FileSystem, FsResult, NormalizedPath, OpenOptions};
use wanix_task::{Fd, Task};

use crate::{
    host_api::{
        display_host_value, exit_requested, qjs_error, resolve_namespace_path, two_string_args,
    },
    task_context::WanixExitState,
};

pub(crate) fn define_fd_output_callback(
    runtime: &mut QuickJsRuntime,
    name: &'static str,
    task: Task,
    fd: Fd,
    exit_state: WanixExitState,
) -> FsResult<()> {
    runtime
        .define_global_host_function(name, move |args| {
            if exit_state.is_requested()? {
                return Ok(QuickJsHostValue::Undefined);
            }
            let text = args.iter().map(display_host_value).collect::<String>();
            write_all_fd(&task, fd, text.as_bytes(), name)?;
            Ok(QuickJsHostValue::Undefined)
        })
        .map_err(qjs_error)
}

pub(crate) fn define_wanix_fd_api(
    runtime: &mut QuickJsRuntime,
    namespace: impl FileSystem + Clone + 'static,
    cwd: NormalizedPath,
    task: Task,
    exit_state: Option<WanixExitState>,
) -> FsResult<()> {
    let open_namespace = namespace.clone();
    let open_task = task.clone();
    let open_cwd = cwd.clone();
    let open_exit_state = exit_state.clone();
    runtime
        .define_global_host_function("__wanix_open", move |args| {
            if exit_requested(&open_exit_state)? {
                return Ok(QuickJsHostValue::Number(-1.0));
            }
            let (path, mode) = two_string_args(args, "Wanix.open")?;
            let options = open_mode(&mode)?;
            let path = resolve_namespace_path(&open_cwd, &path)
                .map_err(|err| anyhow!("Wanix.open({path:?}) failed: {err}"))?;
            let file = open_namespace
                .open(&path, options)
                .map_err(|err| anyhow!("Wanix.open({path:?}) failed: {err}"))?;
            let fd = open_task
                .open_fd(file, path)
                .map_err(|err| anyhow!("Wanix.open failed to allocate fd: {err}"))?;
            Ok(QuickJsHostValue::Number(f64::from(fd.get())))
        })
        .map_err(qjs_error)?;

    let read_task = task.clone();
    let read_exit_state = exit_state.clone();
    runtime
        .define_global_host_function("__wanix_read_fd", move |args| {
            if exit_requested(&read_exit_state)? {
                return Ok(QuickJsHostValue::String(String::new()));
            }
            let (fd, len) = fd_len_args(args, "Wanix.readFd")?;
            let mut buf = vec![0; len];
            let count = read_task
                .read_fd(fd, &mut buf)
                .map_err(|err| anyhow!("Wanix.readFd({}) failed: {err}", fd.get()))?;
            buf.truncate(count);
            String::from_utf8(buf)
                .map(QuickJsHostValue::String)
                .map_err(|err| anyhow!("Wanix.readFd({}) returned non-UTF-8 data: {err}", fd.get()))
        })
        .map_err(qjs_error)?;

    let write_task = task.clone();
    let write_exit_state = exit_state.clone();
    runtime
        .define_global_host_function("__wanix_write_fd", move |args| {
            if exit_requested(&write_exit_state)? {
                return Ok(QuickJsHostValue::Number(0.0));
            }
            let (fd, text) = fd_text_args(args, "Wanix.writeFd")?;
            let count = write_task
                .write_fd(fd, text.as_bytes())
                .map_err(|err| anyhow!("Wanix.writeFd({}) failed: {err}", fd.get()))?;
            Ok(QuickJsHostValue::Number(count as f64))
        })
        .map_err(qjs_error)?;

    runtime
        .define_global_host_function("__wanix_close_fd", move |args| {
            if exit_requested(&exit_state)? {
                return Ok(QuickJsHostValue::Undefined);
            }
            let fd = one_fd_arg(args, "Wanix.closeFd")?;
            task.close_fd(fd)
                .map_err(|err| anyhow!("Wanix.closeFd({}) failed: {err}", fd.get()))?;
            Ok(QuickJsHostValue::Undefined)
        })
        .map_err(qjs_error)
}

fn open_mode(mode: &str) -> anyhow::Result<OpenOptions> {
    match mode {
        "r" => Ok(OpenOptions::read()),
        "rw" => Ok(OpenOptions::read_write()),
        "w" => Ok(OpenOptions {
            write: true,
            create: true,
            truncate: true,
            ..OpenOptions::default()
        }),
        "w+" => Ok(OpenOptions {
            read: true,
            write: true,
            create: true,
            truncate: true,
        }),
        _ => bail!("Wanix.open mode must be one of r, rw, w, or w+"),
    }
}

fn one_fd_arg(args: &[QuickJsHostValue], function: &str) -> anyhow::Result<Fd> {
    match args {
        [QuickJsHostValue::Number(value)] => Ok(Fd::new(fd_number(*value, function)?)),
        _ => bail!("{function} expects one fd number"),
    }
}

fn fd_len_args(args: &[QuickJsHostValue], function: &str) -> anyhow::Result<(Fd, usize)> {
    match args {
        [QuickJsHostValue::Number(fd), QuickJsHostValue::Number(len)] => {
            let fd = Fd::new(fd_number(*fd, function)?);
            let len = usize_number(*len, function)?;
            Ok((fd, len))
        }
        _ => bail!("{function} expects an fd number and byte length"),
    }
}

fn fd_text_args(args: &[QuickJsHostValue], function: &str) -> anyhow::Result<(Fd, String)> {
    match args {
        [QuickJsHostValue::Number(fd), QuickJsHostValue::String(text)] => {
            Ok((Fd::new(fd_number(*fd, function)?), text.clone()))
        }
        _ => bail!("{function} expects an fd number and string"),
    }
}

fn fd_number(value: f64, function: &str) -> anyhow::Result<u32> {
    if !value.is_finite() || value.fract() != 0.0 || !(0.0..=f64::from(u32::MAX)).contains(&value) {
        bail!("{function} expects an unsigned integer fd");
    }
    Ok(value as u32)
}

fn usize_number(value: f64, function: &str) -> anyhow::Result<usize> {
    const MAX_READ_LEN: usize = 1024 * 1024;
    if !value.is_finite() || value.fract() != 0.0 || value < 0.0 || value > MAX_READ_LEN as f64 {
        bail!("{function} expects a byte length in 0..=1048576");
    }
    Ok(value as usize)
}

fn write_all_fd(task: &Task, fd: Fd, bytes: &[u8], function: &str) -> anyhow::Result<()> {
    let mut written = 0;
    while written < bytes.len() {
        let count = task
            .write_fd(fd, &bytes[written..])
            .map_err(|err| anyhow!("{function} failed to write fd {}: {err}", fd.get()))?;
        if count == 0 {
            bail!(
                "{function} failed to write fd {}: wrote zero bytes",
                fd.get()
            );
        }
        written += count;
    }
    Ok(())
}
