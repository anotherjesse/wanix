use std::sync::{Arc, Mutex};

use anyhow::{anyhow, bail};
use rust_wasi_quickjs::{QuickJsHostValue, QuickJsRuntime};
use wanix_fs::{FileSystem, FsError, FsResult, NormalizedPath, OpenOptions};

const WANIX_NAMESPACE_PRELUDE: &str = r#"
globalThis.Wanix = Object.freeze({
  readText: (path) => __wanix_read_text(String(path)),
  writeText: (path, text) => __wanix_write_text(String(path), String(text)),
});
"#;

pub(crate) fn define_output_callback(
    runtime: &mut QuickJsRuntime,
    name: &'static str,
    output: Arc<Mutex<Vec<u8>>>,
) -> FsResult<()> {
    runtime
        .define_global_host_function(name, move |args| {
            let text = args.iter().map(display_host_value).collect::<String>();
            output
                .lock()
                .map_err(|_| anyhow!("output buffer lock poisoned"))?
                .extend_from_slice(text.as_bytes());
            Ok(QuickJsHostValue::Undefined)
        })
        .map_err(qjs_error)
}

pub(crate) fn define_wanix_namespace_api(
    runtime: &mut QuickJsRuntime,
    namespace: impl FileSystem + Clone + 'static,
) -> FsResult<()> {
    let read_namespace = namespace.clone();
    runtime
        .define_global_host_function("__wanix_read_text", move |args| {
            let path = one_string_arg(args, "Wanix.readText")?;
            let text = read_text_path(&read_namespace, &path)
                .map_err(|err| anyhow!("Wanix.readText({path:?}) failed: {err}"))?;
            Ok(QuickJsHostValue::String(text))
        })
        .map_err(qjs_error)?;

    runtime
        .define_global_host_function("__wanix_write_text", move |args| {
            let (path, text) = two_string_args(args, "Wanix.writeText")?;
            write_text_path(&namespace, &path, text.as_bytes())
                .map_err(|err| anyhow!("Wanix.writeText({path:?}) failed: {err}"))?;
            Ok(QuickJsHostValue::Undefined)
        })
        .map_err(qjs_error)?;

    runtime
        .eval_discard(WANIX_NAMESPACE_PRELUDE)
        .map_err(qjs_error)
}

pub(crate) fn read_namespace_file(
    namespace: &impl FileSystem,
    path: &NormalizedPath,
) -> FsResult<String> {
    let mut file = namespace.open(path, OpenOptions::read())?;
    let mut bytes = Vec::new();
    let mut buf = [0; 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        bytes.extend_from_slice(&buf[..n]);
    }
    String::from_utf8(bytes)
        .map_err(|err| FsError::Other(format!("script is not valid UTF-8: {err}")))
}

pub(crate) fn take_buffer(buffer: Arc<Mutex<Vec<u8>>>) -> FsResult<Vec<u8>> {
    let mut buffer = buffer
        .lock()
        .map_err(|_| FsError::Other("output buffer lock poisoned".to_owned()))?;
    Ok(std::mem::take(&mut *buffer))
}

pub(crate) fn qjs_error(error: impl std::fmt::Display) -> FsError {
    FsError::Other(format!("QuickJS error: {error:#}"))
}

fn read_text_path(namespace: &impl FileSystem, path: &str) -> FsResult<String> {
    let path = NormalizedPath::new(path)?;
    read_namespace_file(namespace, &path)
}

fn write_text_path(namespace: &impl FileSystem, path: &str, bytes: &[u8]) -> FsResult<()> {
    let path = NormalizedPath::new(path)?;
    let mut file = namespace.open(
        &path,
        OpenOptions {
            write: true,
            create: true,
            truncate: true,
            ..OpenOptions::default()
        },
    )?;
    let mut written = 0;
    while written < bytes.len() {
        let n = file.write(&bytes[written..])?;
        if n == 0 {
            return Err(FsError::Other("namespace write returned zero".to_owned()));
        }
        written += n;
    }
    Ok(())
}

fn one_string_arg(args: &[QuickJsHostValue], function: &str) -> anyhow::Result<String> {
    match args {
        [QuickJsHostValue::String(value)] => Ok(value.clone()),
        _ => bail!("{function} expects one string argument"),
    }
}

fn two_string_args(args: &[QuickJsHostValue], function: &str) -> anyhow::Result<(String, String)> {
    match args {
        [
            QuickJsHostValue::String(left),
            QuickJsHostValue::String(right),
        ] => Ok((left.clone(), right.clone())),
        _ => bail!("{function} expects two string arguments"),
    }
}

fn display_host_value(value: &QuickJsHostValue) -> String {
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
