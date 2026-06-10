//! WebDoor: one generic HTTP→namespace gateway on `serve`.
//!
//! `serve --bind NAME=DIR --bind NAME=iroh://PEER ...` composes, per NAME, one
//! namespace (every source for that NAME unioned at its root) and serves it at
//! the origin `http://NAME.localhost:PORT` via `Host`-header routing — so a
//! static webapp and the mesh-mounted room it talks to share ONE origin and
//! fetch each other with no CORS. `*.localhost` resolves to loopback by
//! resolver convention, so names cost nothing. The bare `http://localhost:PORT/`
//! serves an index of the bound names; an unknown name is a 404 pointing at
//! that index. The request→response mapping lives in
//! [`super::http::gateway`]; this module owns the binding grammar, the
//! per-name composition, and the mesh-mount keepalives.
//!
//! # Gateway principals — v0 honesty
//!
//! The gateway dials every `iroh://` bind with **its own persisted dialer key**
//! (`~/.wanix/dialer.key`), so a mounted room sees ONE principal — the gateway
//! itself — for every browser user. Session isolation, if any, happens at the
//! gateway/HTTP layer only; it is never mesh principal enforcement, and the
//! gateway must NOT fake per-user attribution into the room. A future
//! per-user path is delegation certs / gateway principals threaded into the
//! attach (docs/appfs.md §Identity And Trust); that requires mesh wire
//! changes and is explicitly out of scope here.
//!
//! # Trust boundary (ADR 0006)
//!
//! The WebDoor turns HTTP requests into namespace reads *and writes* against
//! operator-chosen directories and mesh mounts with no authentication, so —
//! mirroring the `--wanix-services` exec-device rule — `--bind` is refused
//! whenever the HTTP door is bound to a non-loopback address. Off-loopback
//! gateway auth is recorded follow-up work, not a flag.

use std::collections::BTreeMap;
use std::io::Write;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use wanix_fs::{FileSystem, LocalFs};
use wanix_sites::Host;
use wanix_vfs::{BindOptions, BindPosition, Namespace};

use crate::mesh::{IROH_SCHEME, IrohMount, MeshTicket, dial_iroh_remote};
use crate::serve::discovery::is_loopback_addr;
use crate::{CliError, write_process_output};

/// One parsed `--bind NAME=SOURCE` argument.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WebBind {
    pub(super) host: Host,
    pub(super) source: WebBindSource,
}

/// What one `--bind` points the name at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum WebBindSource {
    /// A host directory, served through a `LocalFs`.
    Dir(PathBuf),
    /// An `iroh://PEER[?addr=...]` mesh ticket, dialed at serve startup and
    /// mounted for the serve's whole lifetime (the gateway holds the
    /// keepalive).
    Mesh(String),
}

impl WebBind {
    /// Parses `NAME=DIR` or `NAME=iroh://PEER[?addr=...]`. A NAME without a
    /// dot gets `.localhost` appended, so `--bind chat=DIR` and
    /// `--bind chat.localhost=DIR` name the same origin.
    pub(crate) fn parse(raw: &str) -> Result<Self, CliError> {
        let Some((name, source)) = raw.split_once('=') else {
            return Err(CliError::usage(format!(
                "serve --bind expects NAME=DIR or NAME=iroh://PEER, got {raw:?}"
            )));
        };
        let host = parse_bind_name(name)?;
        let source = source.trim();
        if source.is_empty() {
            return Err(CliError::usage(format!(
                "serve --bind {name} names an empty source; pass a directory or an iroh:// ticket"
            )));
        }
        if source.starts_with(IROH_SCHEME) {
            // Validate the ticket shape now so a typo fails at parse time, not
            // mid-startup after other binds already dialed.
            MeshTicket::parse(source)?;
            return Ok(Self {
                host,
                source: WebBindSource::Mesh(source.to_owned()),
            });
        }
        Ok(Self {
            host,
            source: WebBindSource::Dir(PathBuf::from(source)),
        })
    }
}

fn parse_bind_name(name: &str) -> Result<Host, CliError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(CliError::usage(
            "serve --bind expects NAME=SOURCE with a non-empty NAME",
        ));
    }
    if name.eq_ignore_ascii_case("localhost") {
        return Err(CliError::usage(
            "serve --bind cannot bind the bare localhost (it serves the name index); \
             pick a name like chat (served as chat.localhost)",
        ));
    }
    let qualified = match name.contains('.') {
        true => name.to_owned(),
        false => format!("{name}.localhost"),
    };
    Host::parse(&qualified).ok_or_else(|| {
        CliError::usage(format!(
            "serve --bind name {name:?} must be a routable site name \
             (e.g. chat or chat.localhost), not an IP literal or bare localhost"
        ))
    })
}

/// The gateway's name→namespace table plus the mesh keepalives that hold its
/// `iroh://` mounts alive for the serve's lifetime.
pub(super) struct WebDoor {
    origins: BTreeMap<Host, Arc<dyn FileSystem>>,
    /// Each dialed mount owns the tokio runtime its filesystem drives QUIC ops
    /// on; dropping one kills every in-flight op on that mount, so they live
    /// exactly as long as the door (shared via `Arc<WebDoor>` on `ServeRoots`).
    _keepalives: Vec<IrohMount>,
}

impl WebDoor {
    pub(super) fn empty() -> Self {
        Self {
            origins: BTreeMap::new(),
            _keepalives: Vec::new(),
        }
    }

    /// Composes one namespace per bound name and dials every mesh source.
    ///
    /// Sources for the same name union at the origin root; each bind is
    /// appended at [`BindPosition::Last`], so on a path conflict the lookup
    /// consults sources in `--bind` order and the earliest `--bind` wins.
    ///
    /// # Errors
    ///
    /// Returns a CLI error when a directory cannot be opened or a mesh dial
    /// fails — a gateway origin with a silently missing member would be a
    /// confusing half-site, so startup fails loudly instead.
    pub(super) fn build(binds: &[WebBind]) -> Result<Self, CliError> {
        let mut door = Self::empty();
        let mut composed: BTreeMap<Host, Namespace> = BTreeMap::new();
        for bind in binds {
            let namespace = composed.entry(bind.host.clone()).or_default();
            let fs = door.open_source(&bind.source)?;
            namespace
                .bind(fs, ".", ".", bind_last())
                .map_err(CliError::from)?;
        }
        for (host, namespace) in composed {
            door.origins.insert(host, Arc::new(namespace));
        }
        Ok(door)
    }

    fn open_source(&mut self, source: &WebBindSource) -> Result<Arc<dyn FileSystem>, CliError> {
        match source {
            WebBindSource::Dir(path) => Ok(Arc::new(LocalFs::new(path).map_err(|error| {
                CliError::new(
                    format!("serve --bind: failed to open {}: {error}", path.display()),
                    1,
                )
            })?)),
            WebBindSource::Mesh(ticket) => {
                let mount = dial_iroh_remote(ticket, "")?;
                let remote = mount.remote.clone();
                self._keepalives.push(mount);
                Ok(remote)
            }
        }
    }

    /// Registers an already-built filesystem as an origin (tests; in-process
    /// devices). Unioned before existing members like an earlier `--bind`.
    #[cfg(test)]
    pub(super) fn bind_origin(&mut self, host: Host, fs: Arc<dyn FileSystem>) {
        self.origins.insert(host, fs);
    }

    pub(super) fn is_empty(&self) -> bool {
        self.origins.is_empty()
    }

    pub(super) fn resolve(&self, host: &Host) -> Option<Arc<dyn FileSystem>> {
        self.origins.get(host).cloned()
    }

    pub(super) fn names(&self) -> impl Iterator<Item = &Host> {
        self.origins.keys()
    }
}

fn bind_last() -> BindOptions {
    BindOptions {
        position: BindPosition::Last,
    }
}

/// Refuses `--bind` on a non-loopback HTTP door, then builds the door and
/// announces each bound origin on stderr. Returns `None` when no `--bind` was
/// given (the gateway then never engages).
///
/// # Errors
///
/// Returns a usage error for the trust-boundary refusal and a CLI error when a
/// bind source cannot be opened or dialed.
pub(super) fn start_webdoor(
    binds: &[WebBind],
    local_addr: SocketAddr,
    process_stderr: &mut dyn Write,
) -> Result<Option<Arc<WebDoor>>, CliError> {
    if binds.is_empty() {
        return Ok(None);
    }
    if !is_loopback_addr(local_addr) {
        return Err(CliError::usage(
            "serve --bind exposes namespace reads AND writes (directories, mesh mounts) over \
             unauthenticated HTTP; it is refused because the HTTP door is bound to a \
             non-loopback address (the ADR 0006 trust rule — off-loopback gateway auth is \
             recorded follow-up work). Bind to loopback (e.g. 127.0.0.1:PORT) or drop --bind",
        ));
    }
    let door = WebDoor::build(binds)?;
    for name in door.names() {
        write_process_output(
            process_stderr,
            "stderr",
            format!(
                "wanix-rust serve: gateway origin http://{name}:{}/\n",
                local_addr.port()
            )
            .as_bytes(),
        )?;
    }
    Ok(Some(Arc::new(door)))
}

#[cfg(test)]
mod tests;
