use wanix_fs::NormalizedPath;

use super::super::http::{HttpStatus, StaticResponse, percent_decode};

pub(in crate::serve) const QJS_SHELL_WEBSOCKET_PATH: &str = "/.well-known/qjs-shell";

pub(in crate::serve) fn is_qjs_shell_websocket_path(raw_path: Option<&str>) -> bool {
    raw_path.map(|path| path.split_once('?').map_or(path, |(path, _)| path))
        == Some(QJS_SHELL_WEBSOCKET_PATH)
}

pub(in crate::serve) fn qjs_shell_cwd_from_target(
    raw_path: Option<&str>,
) -> Result<NormalizedPath, StaticResponse> {
    let raw_path =
        raw_path.ok_or_else(|| StaticResponse::plain(HttpStatus::BadRequest, "bad request"))?;
    let Some((_path, query)) = raw_path.split_once('?') else {
        return Ok(default_qjs_shell_cwd());
    };
    for pair in query.split('&') {
        let (raw_key, raw_value) = pair.split_once('=').unwrap_or((pair, ""));
        let key = query_percent_decode(raw_key)?;
        if key == "cwd" {
            let value = query_percent_decode(raw_value)?;
            return qjs_shell_cwd_from_query_value(&value);
        }
    }
    Ok(default_qjs_shell_cwd())
}

fn default_qjs_shell_cwd() -> NormalizedPath {
    NormalizedPath::root()
}

fn qjs_shell_cwd_from_query_value(value: &str) -> Result<NormalizedPath, StaticResponse> {
    let path = match value {
        "" => {
            return Err(StaticResponse::plain(
                HttpStatus::BadRequest,
                "invalid qjs shell cwd",
            ));
        }
        "/" => ".",
        path => path.strip_prefix('/').unwrap_or(path),
    };
    let path = if path.is_empty() { "." } else { path };
    NormalizedPath::new(path)
        .map_err(|_| StaticResponse::plain(HttpStatus::BadRequest, "invalid qjs shell cwd"))
}

fn query_percent_decode(value: &str) -> Result<String, StaticResponse> {
    percent_decode(&value.replace('+', " "))
        .map_err(|_| StaticResponse::plain(HttpStatus::BadRequest, "invalid query string"))
}
