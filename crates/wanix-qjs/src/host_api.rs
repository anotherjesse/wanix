use anyhow::{anyhow, bail};
use rust_wasi_quickjs::{QuickJsHostValue, QuickJsRuntime};
use wanix_fs::{FileSystem, FsError, FsResult, NormalizedPath, OpenOptions};

use crate::task_context::WanixTaskContext;

mod output;
pub(crate) use output::{
    define_output_callback, define_output_callback_with_exit_state, display_host_value, take_buffer,
};

const WANIX_TASK_GLOBALS_PRELUDE: &str = r#"
(() => {
  globalThis.scriptArgs = Object.freeze(JSON.parse(__wanix_script_args_json()));
})();
"#;
const NAMESPACE_READ_CHUNK_BYTES: usize = 1024;

pub(crate) fn define_wanix_task_globals(
    runtime: &mut QuickJsRuntime,
    context: WanixTaskContext,
) -> FsResult<()> {
    let script_args_json = serde_json::to_string(context.script_args())
        .map_err(|err| FsError::Other(format!("failed to encode scriptArgs: {err}")))?;
    runtime
        .define_global_host_function("__wanix_script_args_json", move |args| {
            no_args(args, "scriptArgs")?;
            Ok(QuickJsHostValue::String(script_args_json.clone()))
        })
        .map_err(qjs_error)?;

    runtime
        .eval_discard(WANIX_TASK_GLOBALS_PRELUDE)
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
    let mut buf = [0; NAMESPACE_READ_CHUNK_BYTES];
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

fn no_args(args: &[QuickJsHostValue], function: &str) -> anyhow::Result<()> {
    if args.is_empty() {
        Ok(())
    } else {
        bail!("{function} expects no arguments")
    }
}
