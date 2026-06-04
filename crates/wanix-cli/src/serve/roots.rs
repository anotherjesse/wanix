use std::fs;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use wanix_fs::{FileSystem, LocalFs};
use wanix_qjs::QuickJsTaskDriver;
use wanix_task::TaskTable;
use wanix_term::TermDevice;
use wanix_vfs::{BindOptions, BindPosition, Namespace};

use crate::{CliError, quickjs_runner};

#[derive(Clone)]
pub(super) struct ServeRoots {
    pub(super) static_root: PathBuf,
    pub(super) p9_root: Arc<dyn FileSystem>,
    pub(super) local_addr: SocketAddr,
    pub(super) bundle: Option<String>,
    pub(super) wanix_services: bool,
}

impl ServeRoots {
    pub(super) fn new(
        root_path: &Path,
        local_addr: SocketAddr,
        bundle: Option<String>,
        wanix_services: bool,
    ) -> Result<Self, CliError> {
        let static_root = fs::canonicalize(root_path).map_err(|error| {
            CliError::new(
                format!("failed to open serve root {}: {error}", root_path.display()),
                1,
            )
        })?;
        let p9_root = serve_p9_root(root_path, wanix_services)?;
        Ok(Self {
            static_root,
            p9_root,
            local_addr,
            bundle,
            wanix_services,
        })
    }
}

fn serve_p9_root(root_path: &Path, wanix_services: bool) -> Result<Arc<dyn FileSystem>, CliError> {
    let host_root = open_host_p9_root(root_path)?;
    match wanix_services {
        true => serve_services_root(host_root),
        false => Ok(host_root),
    }
}

fn open_host_p9_root(root_path: &Path) -> Result<Arc<dyn FileSystem>, CliError> {
    Ok(Arc::new(LocalFs::new(root_path).map_err(|error| {
        CliError::new(
            format!(
                "failed to open serve 9P root {}: {error}",
                root_path.display()
            ),
            1,
        )
    })?))
}

fn serve_services_root(host_root: Arc<dyn FileSystem>) -> Result<Arc<dyn FileSystem>, CliError> {
    let table = serve_task_table()?;
    let namespace = serve_services_namespace(host_root, &table)?;
    Ok(Arc::new(namespace))
}

fn serve_services_namespace(
    host_root: Arc<dyn FileSystem>,
    table: &TaskTable,
) -> Result<Namespace, CliError> {
    let mut namespace = Namespace::new();
    bind_host_and_terminal(&mut namespace, host_root)?;
    bind_task_service(&mut namespace, table)?;
    Ok(namespace)
}

fn bind_host_and_terminal(
    namespace: &mut Namespace,
    host_root: Arc<dyn FileSystem>,
) -> Result<(), CliError> {
    let terminal = Arc::new(TermDevice::new());
    namespace.bind(host_root, ".", ".", BindOptions::default())?;
    namespace.bind(terminal, ".", "#term", BindOptions::default())?;
    Ok(())
}

fn bind_task_service(namespace: &mut Namespace, table: &TaskTable) -> Result<(), CliError> {
    let root_task = table.allocate_root_with_namespace("noop", namespace.clone())?;
    namespace.bind(
        Arc::new(table.filesystem_for(root_task.id())),
        ".",
        "#task",
        BindOptions {
            position: BindPosition::Replace,
        },
    )?;
    Ok(())
}

fn serve_task_table() -> Result<TaskTable, CliError> {
    let table = TaskTable::new();
    table.register_noop_driver("noop")?;
    table.register_driver("qjs", Arc::new(QuickJsTaskDriver::new(quickjs_runner()?)))?;
    Ok(table)
}
