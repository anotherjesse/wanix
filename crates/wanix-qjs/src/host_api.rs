use std::sync::{Arc, Mutex};

use anyhow::{anyhow, bail};
use rust_wasi_quickjs::{QuickJsHostValue, QuickJsRuntime};
use wanix_fs::{FileSystem, FsError, FsResult, NormalizedPath, OpenOptions};
use wanix_task::Task;

use crate::{
    fd_api::define_wanix_fd_api,
    task_context::{WanixExitState, WanixTaskContext},
};

const WANIX_HOST_API_PRELUDE: &str = r#"
(() => {
  globalThis.scriptArgs = Object.freeze(JSON.parse(__wanix_script_args_json()));
  const api = {
    readText: (path) => __wanix_read_text(String(path)),
    writeText: (path, text) => __wanix_write_text(String(path), String(text)),
    args: () => Object.freeze(JSON.parse(__wanix_args_json())),
    env: function(name) {
      const env = JSON.parse(__wanix_env_json());
      if (arguments.length === 0) {
        return Object.freeze(env);
      }
      const key = String(name);
      return Object.prototype.hasOwnProperty.call(env, key) ? env[key] : undefined;
    },
    cwd: () => __wanix_cwd(),
    cmd: () => __wanix_cmd(),
  };
  if (typeof __wanix_open === "function") {
    api.open = (path, mode = "r") => __wanix_open(String(path), String(mode));
    api.readFd = (fd, len) => __wanix_read_fd(Number(fd), Number(len));
    api.writeFd = (fd, text) => __wanix_write_fd(Number(fd), String(text));
    api.closeFd = (fd) => __wanix_close_fd(Number(fd));
  }
  globalThis.Wanix = Object.freeze(api);
})();
"#;

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

pub(crate) fn define_wanix_host_api(
    runtime: &mut QuickJsRuntime,
    namespace: impl FileSystem + Clone + 'static,
    context: WanixTaskContext,
    exit_state: Option<WanixExitState>,
    task: Option<Task>,
) -> FsResult<()> {
    let read_namespace = namespace.clone();
    let read_cwd = context.cwd().clone();
    runtime
        .define_global_host_function("__wanix_read_text", move |args| {
            let path = one_string_arg(args, "Wanix.readText")?;
            let text = read_text_path(&read_namespace, &read_cwd, &path)
                .map_err(|err| anyhow!("Wanix.readText({path:?}) failed: {err}"))?;
            Ok(QuickJsHostValue::String(text))
        })
        .map_err(qjs_error)?;

    let write_namespace = namespace.clone();
    let write_cwd = context.cwd().clone();
    let write_exit_state = exit_state.clone();
    runtime
        .define_global_host_function("__wanix_write_text", move |args| {
            if exit_requested(&write_exit_state)? {
                return Ok(QuickJsHostValue::Undefined);
            }
            let (path, text) = two_string_args(args, "Wanix.writeText")?;
            write_text_path(&write_namespace, &write_cwd, &path, text.as_bytes())
                .map_err(|err| anyhow!("Wanix.writeText({path:?}) failed: {err}"))?;
            Ok(QuickJsHostValue::Undefined)
        })
        .map_err(qjs_error)?;

    let cmd = context.cmd().to_owned();
    runtime
        .define_global_host_function("__wanix_cmd", move |args| {
            no_args(args, "Wanix.cmd")?;
            Ok(QuickJsHostValue::String(cmd.clone()))
        })
        .map_err(qjs_error)?;

    let args_json = serde_json::to_string(context.args())
        .map_err(|err| FsError::Other(format!("failed to encode task args: {err}")))?;
    runtime
        .define_global_host_function("__wanix_args_json", move |args| {
            no_args(args, "Wanix.args")?;
            Ok(QuickJsHostValue::String(args_json.clone()))
        })
        .map_err(qjs_error)?;

    let script_args_json = serde_json::to_string(context.script_args())
        .map_err(|err| FsError::Other(format!("failed to encode scriptArgs: {err}")))?;
    runtime
        .define_global_host_function("__wanix_script_args_json", move |args| {
            no_args(args, "scriptArgs")?;
            Ok(QuickJsHostValue::String(script_args_json.clone()))
        })
        .map_err(qjs_error)?;

    let env_json = serde_json::to_string(context.env())
        .map_err(|err| FsError::Other(format!("failed to encode task env: {err}")))?;
    runtime
        .define_global_host_function("__wanix_env_json", move |args| {
            no_args(args, "Wanix.env")?;
            Ok(QuickJsHostValue::String(env_json.clone()))
        })
        .map_err(qjs_error)?;

    let cwd = context.cwd().to_string();
    runtime
        .define_global_host_function("__wanix_cwd", move |args| {
            no_args(args, "Wanix.cwd")?;
            Ok(QuickJsHostValue::String(cwd.clone()))
        })
        .map_err(qjs_error)?;

    if let Some(task) = task {
        define_wanix_fd_api(
            runtime,
            namespace.clone(),
            context.cwd().clone(),
            task,
            exit_state,
        )?;
    }

    runtime
        .eval_discard(WANIX_HOST_API_PRELUDE)
        .map_err(qjs_error)
}

pub(crate) fn define_wanix_module_loader(
    runtime: &mut QuickJsRuntime,
    namespace: impl FileSystem + Clone + 'static,
) -> FsResult<()> {
    let root_cwd = NormalizedPath::new(".").expect("root path is valid");
    runtime
        .set_module_loader_with_normalizer(normalize_module_name, move |name| {
            read_text_path(&namespace, &root_cwd, name)
                .map_err(|err| anyhow!("failed to load Wanix module {name:?}: {err}"))
        })
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

fn read_text_path(
    namespace: &impl FileSystem,
    cwd: &NormalizedPath,
    path: &str,
) -> FsResult<String> {
    let path = resolve_namespace_path(cwd, path)?;
    read_namespace_file(namespace, &path)
}

fn normalize_module_name(base_name: &str, specifier: &str) -> anyhow::Result<String> {
    if specifier.is_empty() || specifier.contains('\0') {
        bail!("module specifier must be a non-empty Wanix path");
    }
    if specifier.starts_with('/') {
        bail!("module specifier must be relative to the Wanix namespace root");
    }
    if specifier.starts_with("./") || specifier.starts_with("../") {
        normalize_relative_module_name(base_name, specifier)
    } else {
        NormalizedPath::new(specifier)
            .map(|path| path.as_str().to_owned())
            .map_err(|err| anyhow!("invalid Wanix module specifier {specifier:?}: {err}"))
    }
}

fn normalize_relative_module_name(base_name: &str, specifier: &str) -> anyhow::Result<String> {
    let base = NormalizedPath::new(base_name)
        .map_err(|err| anyhow!("invalid Wanix module base name {base_name:?}: {err}"))?;
    let mut parts = match base.parent() {
        Some(parent) if parent.as_str() != "." => parent
            .as_str()
            .split('/')
            .map(str::to_owned)
            .collect::<Vec<_>>(),
        _ => Vec::new(),
    };

    for component in specifier.split('/') {
        match component {
            "" | "." => {}
            ".." => {
                if parts.pop().is_none() {
                    bail!("module specifier escapes Wanix namespace root");
                }
            }
            component => parts.push(component.to_owned()),
        }
    }

    let normalized = if parts.is_empty() {
        ".".to_owned()
    } else {
        parts.join("/")
    };
    if normalized == "." {
        bail!("module specifier must name a file");
    }
    NormalizedPath::new(&normalized)
        .map(|path| path.as_str().to_owned())
        .map_err(|err| anyhow!("invalid Wanix module specifier {specifier:?}: {err}"))
}

fn write_text_path(
    namespace: &impl FileSystem,
    cwd: &NormalizedPath,
    path: &str,
    bytes: &[u8],
) -> FsResult<()> {
    let path = resolve_namespace_path(cwd, path)?;
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

pub(crate) fn resolve_namespace_path(cwd: &NormalizedPath, path: &str) -> FsResult<NormalizedPath> {
    let path = NormalizedPath::new(path)?;
    if path.as_str().starts_with('#') || cwd.as_str() == "." {
        return Ok(path);
    }
    if path.as_str() == "." {
        return Ok(cwd.clone());
    }
    NormalizedPath::new(format!("{cwd}/{path}"))
}

fn one_string_arg(args: &[QuickJsHostValue], function: &str) -> anyhow::Result<String> {
    match args {
        [QuickJsHostValue::String(value)] => Ok(value.clone()),
        _ => bail!("{function} expects one string argument"),
    }
}

pub(crate) fn two_string_args(
    args: &[QuickJsHostValue],
    function: &str,
) -> anyhow::Result<(String, String)> {
    match args {
        [
            QuickJsHostValue::String(left),
            QuickJsHostValue::String(right),
        ] => Ok((left.clone(), right.clone())),
        _ => bail!("{function} expects two string arguments"),
    }
}

fn no_args(args: &[QuickJsHostValue], function: &str) -> anyhow::Result<()> {
    if args.is_empty() {
        Ok(())
    } else {
        bail!("{function} expects no arguments")
    }
}

pub(crate) fn exit_requested(exit_state: &Option<WanixExitState>) -> anyhow::Result<bool> {
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
