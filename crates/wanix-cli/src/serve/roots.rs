use std::fs;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use wanix_agent::{AgentDevice, FakeEngine};
use wanix_cas::{CasDevice, ContentStore, LocalCasStore};
use wanix_fs::{FileSystem, LocalFs};
use wanix_kv::KvDevice;
use wanix_pipe::PipeDevice;
use wanix_plumb::PlumbDevice;
use wanix_qjs::QuickJsTaskDriver;
use wanix_sites::{Host, SiteSource, SitesDevice};
use wanix_task::TaskTable;
use wanix_term::TermDevice;
use wanix_vfs::{BindOptions, BindPosition, Namespace};
use wanix_wasm::WasmTaskDriver;

use crate::{CliError, quickjs_runner};

#[derive(Clone)]
pub(super) struct ServeRoots {
    pub(super) static_root: PathBuf,
    /// The filesystem the HTTP static path serves through. For `--root DIR`
    /// this is a `LocalFs` over the same directory as `static_root`, so byte
    /// content, directory-index resolution, and MIME are unchanged — only the
    /// I/O path moves from `std::fs` to the `FileSystem` trait (Phase 0). Later
    /// phases serve other filesystems (an in-memory generator output, a CAS
    /// snapshot) through this same field.
    pub(super) site_root: Arc<dyn FileSystem>,
    /// The `#sites` device: a host→filesystem binding table. The HTTP gateway
    /// reads the request `Host` header and serves `sites.resolve(host)` through
    /// the same FS-backed static handler as `site_root`. Shared with the bound
    /// `#sites` in the services namespace so 9P writes and the gateway agree.
    /// An empty device when services are disabled (the gateway then never
    /// consults it and every request falls through to `--root`).
    pub(super) sites: Arc<SitesDevice>,
    pub(super) p9_root: Arc<dyn FileSystem>,
    /// Task driver kinds advertised by service discovery, captured from the
    /// registry at build time so the discovery JSON cannot drift from what
    /// `serve_task_table` actually registers. Empty when services are disabled.
    pub(super) driver_kinds: Vec<String>,
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
        sites: &[(String, PathBuf)],
    ) -> Result<Self, CliError> {
        let static_root = fs::canonicalize(root_path).map_err(|error| {
            CliError::new(
                format!("failed to open serve root {}: {error}", root_path.display()),
                1,
            )
        })?;
        let site_root = open_host_p9_root(root_path)?;
        // One owner-private store backs both `#cas` and `#sites`, so a blob
        // ingested via `#cas` (or written by a publish freeze) is readable by a
        // site by hash. The `#sites` device is built `with_store` so a
        // `cas <root-hash>` binding resolves to an immutable `CasSiteFs`.
        let cas_store = Arc::new(LocalCasStore::open_default());
        let sites_device = Arc::new(SitesDevice::with_store(
            Arc::clone(&cas_store) as Arc<dyn ContentStore>
        ));
        register_startup_sites(&sites_device, sites)?;
        let (p9_root, driver_kinds) = serve_p9_root(
            root_path,
            wanix_services,
            Arc::clone(&sites_device),
            Arc::clone(&cas_store) as Arc<dyn ContentStore>,
        )?;
        Ok(Self {
            static_root,
            site_root,
            sites: sites_device,
            p9_root,
            driver_kinds,
            local_addr,
            bundle,
            wanix_services,
        })
    }
}

/// Registers each `--site HOST=PATH` startup binding as a live `LocalFs`
/// [`SiteSource::Memory`] source, so the gateway serves that host immediately.
fn register_startup_sites(
    device: &SitesDevice,
    sites: &[(String, PathBuf)],
) -> Result<(), CliError> {
    for (raw_host, path) in sites {
        let host = Host::registered(raw_host)
            .ok_or_else(|| CliError::usage(format!("invalid --site host: {raw_host:?}")))?;
        let fs = Arc::new(LocalFs::new(path).map_err(|error| {
            CliError::new(
                format!("failed to open --site root {}: {error}", path.display()),
                1,
            )
        })?);
        device.bind_site(host, SiteSource::Memory(fs));
    }
    Ok(())
}

fn serve_p9_root(
    root_path: &Path,
    wanix_services: bool,
    sites: Arc<SitesDevice>,
    cas_store: Arc<dyn ContentStore>,
) -> Result<(Arc<dyn FileSystem>, Vec<String>), CliError> {
    let host_root = open_host_p9_root(root_path)?;
    match wanix_services {
        true => serve_services_root(host_root, sites, cas_store),
        false => Ok((host_root, Vec::new())),
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

fn serve_services_root(
    host_root: Arc<dyn FileSystem>,
    sites: Arc<SitesDevice>,
    cas_store: Arc<dyn ContentStore>,
) -> Result<(Arc<dyn FileSystem>, Vec<String>), CliError> {
    let table = serve_task_table()?;
    let driver_kinds = table.driver_kinds();
    let namespace = serve_services_namespace(host_root, sites, cas_store, &table)?;
    Ok((Arc::new(namespace), driver_kinds))
}

/// Builds the full `--wanix-services` namespace (host root + #term/#pipe/#kv/
/// #plumb/#cas/#agent + #task) rooted at `root_path`, for use as an agent's
/// confined world so the agent operates the Wanix service devices as files.
///
/// # Errors
///
/// Returns an error when the root or services namespace cannot be built.
pub(crate) fn services_namespace_for_root(
    root_path: &Path,
) -> Result<Arc<dyn FileSystem>, CliError> {
    let host_root = open_host_p9_root(root_path)?;
    let cas_store = Arc::new(LocalCasStore::open_default()) as Arc<dyn ContentStore>;
    let sites = Arc::new(SitesDevice::with_store(Arc::clone(&cas_store)));
    let (namespace, _kinds) = serve_services_root(host_root, sites, cas_store)?;
    Ok(namespace)
}

fn serve_services_namespace(
    host_root: Arc<dyn FileSystem>,
    sites: Arc<SitesDevice>,
    cas_store: Arc<dyn ContentStore>,
    table: &TaskTable,
) -> Result<Namespace, CliError> {
    let mut namespace = Namespace::new();
    bind_host_and_terminal(&mut namespace, host_root, sites, cas_store)?;
    bind_task_service(&mut namespace, table)?;
    Ok(namespace)
}

/// Service devices bound into the `--wanix-services` namespace, advertised in
/// discovery so the cockpit can list and inspect them. Must stay in sync with
/// the binds in [`bind_host_and_terminal`] and [`bind_task_service`]; the
/// `serve_wanix_services_*` tests exercise each one over 9P.
pub(super) const INSPECTABLE_SERVICE_DEVICES: &[&str] = &[
    "#task", "#term", "#kv", "#pipe", "#plumb", "#cas", "#agent", "#sites",
];

fn bind_host_and_terminal(
    namespace: &mut Namespace,
    host_root: Arc<dyn FileSystem>,
    sites: Arc<SitesDevice>,
    cas_store: Arc<dyn ContentStore>,
) -> Result<(), CliError> {
    let terminal = Arc::new(TermDevice::new());
    namespace.bind(host_root, ".", ".", BindOptions::default())?;
    namespace.bind(terminal, ".", "#term", BindOptions::default())?;
    namespace.bind(
        Arc::new(PipeDevice::new()),
        ".",
        "#pipe",
        BindOptions::default(),
    )?;
    namespace.bind(
        Arc::new(KvDevice::new()),
        ".",
        "#kv",
        BindOptions::default(),
    )?;
    // `#plumb` is the plumber bus: `#plumb/<topic>/send` publishes a JSON
    // envelope and `#plumb/<topic>/recv` reads received ones. Served here over a
    // single-node `LocalPlumbPort`; the mesh swaps in a `GossipPlumbPort` so a
    // topic crosses nodes. Like `#kv` it is a plain `FileSystem`, so it imports
    // for free across the mesh (`/n/A/#plumb/<topic>/recv`).
    namespace.bind(
        Arc::new(PlumbDevice::local()),
        ".",
        "#plumb",
        BindOptions::default(),
    )?;
    // `#cas` is the data plane as files: `#cas/<hash>` reads a blob,
    // `#cas/ingest` is write-then-read-hash, `#cas/have/<hash>` probes presence.
    // Like `#kv` it is just a `FileSystem`, so it imports across the mesh for
    // free (`/n/A/#cas/...`). It is backed by the owner-private on-disk store, so
    // blobs an agent ingests here persist and dedup against capsules. It shares
    // the same store instance as `#sites`, so a blob ingested here (or written by
    // a publish freeze) is readable by a site by hash.
    namespace.bind(
        Arc::new(CasDevice::new(cas_store)),
        ".",
        "#cas",
        BindOptions::default(),
    )?;
    // Served #agent uses the deterministic fake engine: the real codex bridge is
    // local-trust only (auth + unattended execution) and stays on the CLI path.
    namespace.bind(
        Arc::new(AgentDevice::new(Arc::new(FakeEngine))),
        ".",
        "#agent",
        BindOptions::default(),
    )?;
    // `#sites` binds a host to a filesystem source: `#sites/<host>` lists/reads/
    // writes the binding, and the serve HTTP gateway serves `resolve(host)` for a
    // matching `Host` header. The same `SitesDevice` Arc backs the gateway, so a
    // 9P write to `#sites/<host>` and the gateway agree. Like the other devices
    // it is a plain `FileSystem`, so it imports across the mesh for free.
    namespace.bind(sites, ".", "#sites", BindOptions::default())?;
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

pub(super) fn serve_task_table() -> Result<TaskTable, CliError> {
    let table = TaskTable::new();
    table.register_noop_driver("noop")?;
    table.register_driver("qjs", Arc::new(QuickJsTaskDriver::new(quickjs_runner()?)))?;
    table.register_driver("wasm", Arc::new(WasmTaskDriver::new()))?;
    Ok(table)
}

#[cfg(test)]
mod tests {
    use super::serve_task_table;

    #[test]
    fn serve_task_table_registers_wasm_driver_alongside_qjs() {
        // The `#task` service exposes `#task/new/<kind>` per registered driver, so
        // registering `wasm` here is what makes `#task/new/wasm` (and auto-start of
        // a `.wasm` cmd) a first-class Wanix task in `wanix serve`.
        let table = serve_task_table().expect("build serve task table");
        let kinds = table.driver_kinds();
        assert!(kinds.contains(&"auto".to_owned()), "kinds: {kinds:?}");
        assert!(kinds.contains(&"qjs".to_owned()), "kinds: {kinds:?}");
        assert!(kinds.contains(&"wasm".to_owned()), "kinds: {kinds:?}");
    }
}
