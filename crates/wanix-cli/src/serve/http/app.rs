use std::ffi::OsStr;
use std::net::SocketAddr;
use std::path::{Component, Path};

use wanix_fs::{FileSystem, FileType, FsError, FsResult};
use wanix_task::quote_cmd_argv;

use super::{HttpStatus, StaticResponse, request_target};
use crate::serve::ServeRoots;

mod fs_io;
use fs_io::{
    create_dir_if_missing, normalized, read_all, read_text, truncate_file, write_service_text,
};

const APP_ROUTE_ROOT: &str = ".wanix";
const APP_ROUTE_KIND: &str = "app";
const APP_SOURCE_DIR: &str = "apps";
const APP_TRACE_DIR: &str = ".wanix/http";
const APP_CONTENT_TYPE: &str = "text/plain; charset=utf-8";

pub(super) fn app_route_response(
    roots: &ServeRoots,
    relative_path: &Path,
    request: &[u8],
    peer_addr: SocketAddr,
) -> Option<StaticResponse> {
    let name = match app_name(relative_path)? {
        Ok(name) => name,
        Err(status) => return Some(StaticResponse::plain(status, status.reason())),
    };
    Some(app_response(roots, &name, request, peer_addr))
}

pub(in crate::serve) fn app_route_json(roots: &ServeRoots, http_url: &str) -> String {
    if roots.wanix_services {
        format!(
            "{{\"url\":{},\"protocol\":\"wanix-http-app.v1\",\
             \"status\":\"available\",\"method\":\"GET\",\
             \"route\":\"/.wanix/app/<name>\",\"source\":\"apps/<name>.js\",\
             \"response\":\"stdout\",\"scope\":\"loopback\"}}",
            crate::json::json_string(http_url)
        )
    } else {
        "{\"status\":\"disabled\"}".to_owned()
    }
}

fn app_response(
    roots: &ServeRoots,
    name: &str,
    request: &[u8],
    peer_addr: SocketAddr,
) -> StaticResponse {
    if !roots.wanix_services {
        return StaticResponse::plain(
            HttpStatus::NotFound,
            "wanix app routes require --wanix-services",
        );
    }
    if !peer_addr.ip().is_loopback() {
        return StaticResponse::plain(
            HttpStatus::Forbidden,
            "wanix app routes are available only to loopback clients",
        );
    }
    match run_qjs_app(roots.p9_root.as_ref(), name, request) {
        Ok(stdout) => StaticResponse {
            status: HttpStatus::Ok,
            content_type: APP_CONTENT_TYPE,
            body: stdout,
        },
        Err(AppError::MissingScript(path)) => StaticResponse::plain(
            HttpStatus::NotFound,
            &format!("wanix app script {path} was not found"),
        ),
        Err(AppError::TaskExit {
            exit,
            stdout,
            stderr,
        }) => StaticResponse::plain(
            HttpStatus::InternalServerError,
            &format!(
                "wanix app {name} exited {exit}\n\nstdout:\n{}\n\nstderr:\n{}",
                String::from_utf8_lossy(&stdout),
                String::from_utf8_lossy(&stderr)
            ),
        ),
        Err(AppError::Fs(error)) => StaticResponse::plain(
            HttpStatus::InternalServerError,
            &format!("wanix app {name} failed: {error}"),
        ),
    }
}

fn run_qjs_app(fs: &dyn FileSystem, name: &str, request: &[u8]) -> Result<Vec<u8>, AppError> {
    let source_script = app_script(name);
    require_script_file(fs, &source_script)?;
    prepare_trace_dir(fs)?;

    let task_id = read_text(fs, "#task/new/qjs")?.trim().to_owned();
    let stdout_path = format!("{APP_TRACE_DIR}/{task_id}.out");
    let stderr_path = format!("{APP_TRACE_DIR}/{task_id}.err");
    truncate_file(fs, &stdout_path)?;
    truncate_file(fs, &stderr_path)?;

    let target = request_target(request).unwrap_or("/.wanix/app").to_owned();
    let script = format!("{name}.js");
    let cmd = quote_cmd_argv([script.as_str(), target.as_str()]);
    let task_path = format!("#task/{task_id}");
    write_service_text(fs, &format!("{task_path}/cmd"), &format!("{cmd}\n"))?;
    write_service_text(
        fs,
        &format!("{task_path}/dir"),
        &format!("{APP_SOURCE_DIR}\n"),
    )?;
    write_service_text(fs, &format!("{task_path}/env"), &http_env(name, &target))?;
    write_service_text(
        fs,
        &format!("{task_path}/ctl"),
        &format!("bind {stdout_path} fd/1\n"),
    )?;
    write_service_text(
        fs,
        &format!("{task_path}/ctl"),
        &format!("bind {stderr_path} fd/2\n"),
    )?;
    write_service_text(fs, &format!("{task_path}/ctl"), "start\n")?;

    let exit = read_text(fs, &format!("{task_path}/exit"))?;
    let stdout = read_all(fs, &stdout_path)?;
    let stderr = read_all(fs, &stderr_path)?;
    if exit.trim() == "0" {
        Ok(stdout)
    } else {
        Err(AppError::TaskExit {
            exit: exit.trim().to_owned(),
            stdout,
            stderr,
        })
    }
}

fn app_name(relative_path: &Path) -> Option<Result<String, HttpStatus>> {
    let mut components = relative_path.components();
    match components.next() {
        Some(Component::Normal(component)) if component == APP_ROUTE_ROOT => {}
        _ => return None,
    }
    match components.next() {
        Some(Component::Normal(component)) if component == APP_ROUTE_KIND => {}
        _ => return None,
    }
    let name = match components.next() {
        Some(Component::Normal(component)) => component,
        _ => return Some(Err(HttpStatus::BadRequest)),
    };
    if components.next().is_some() {
        return Some(Err(HttpStatus::BadRequest));
    }
    Some(route_name(name))
}

fn route_name(name: &OsStr) -> Result<String, HttpStatus> {
    let name = name.to_str().ok_or(HttpStatus::BadRequest)?;
    if !valid_route_name(name) {
        return Err(HttpStatus::BadRequest);
    }
    Ok(name.to_owned())
}

fn valid_route_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

fn app_script(name: &str) -> String {
    format!("{APP_SOURCE_DIR}/{name}.js")
}

fn require_script_file(fs: &dyn FileSystem, path: &str) -> Result<(), AppError> {
    let metadata = match fs.metadata(&normalized(path)?) {
        Ok(metadata) => metadata,
        Err(FsError::NotFound) => return Err(AppError::MissingScript(path.to_owned())),
        Err(error) => return Err(AppError::Fs(error)),
    };
    if metadata.file_type() != FileType::File {
        return Err(AppError::MissingScript(path.to_owned()));
    }
    Ok(())
}

fn prepare_trace_dir(fs: &dyn FileSystem) -> FsResult<()> {
    create_dir_if_missing(fs, APP_ROUTE_ROOT)?;
    create_dir_if_missing(fs, APP_TRACE_DIR)
}

fn http_env(name: &str, target: &str) -> String {
    format!("WANIX_HTTP_APP={name}\nWANIX_HTTP_TARGET={target}\n")
}

enum AppError {
    MissingScript(String),
    TaskExit {
        exit: String,
        stdout: Vec<u8>,
        stderr: Vec<u8>,
    },
    Fs(FsError),
}

impl From<FsError> for AppError {
    fn from(error: FsError) -> Self {
        Self::Fs(error)
    }
}
