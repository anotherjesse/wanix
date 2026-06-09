//! `--mount-mesh IROH_URL=GUEST` binding: dial each spec over the native mesh
//! wire and bind the imported remote namespace at its guest path.
//!
//! This is shared task plumbing, not a qjs-only feature: `qjs-shell`, `wasm`,
//! and `sh` all bind mesh mounts the same way. The one contract every caller
//! must honor is the keepalive: each returned [`IrohMount`] owns the tokio
//! runtime its imported `FileSystem` drives QUIC ops on, so the keepalives must
//! outlive every namespace operation that can touch the mount — dropping one
//! shuts that runtime down and panics the next op.

use std::sync::Arc;

use wanix_fs::FileSystem;
use wanix_task::Task;
use wanix_vfs::{BindOptions, Namespace};

use super::{IrohMount, dial_iroh_remote};
use crate::CliError;
use crate::qjs_args::MeshMountSpec;

/// Dials each `--mount-mesh` spec and binds the imported remote into the task
/// namespace at its guest path.
///
/// Returns the dialer [`IrohMount`] keepalives; **the caller must hold the
/// returned vector for as long as the task may touch the mount**. The qjs-term
/// runtime path stores it on the prepared-execution object so it outlives the
/// task.
///
/// # Errors
///
/// Returns a CLI error when a dial fails or the guest path cannot be bound.
pub(crate) fn bind_mesh_mounts(
    task: &Task,
    mounts: &[MeshMountSpec],
) -> Result<Vec<IrohMount>, CliError> {
    dial_and_bind(mounts, |remote, guest_path| {
        task.bind(remote, ".", guest_path, BindOptions::default())
            .map_err(CliError::from)
    })
}

/// [`bind_mesh_mounts`] for callers that compose a [`Namespace`] before a task
/// exists (the `wasm` runner and the `sh` namespace builder). The same
/// keepalive contract applies: hold the returned vector across every operation
/// on the namespace.
///
/// # Errors
///
/// Returns a CLI error when a dial fails or the guest path cannot be bound.
pub(crate) fn bind_mesh_mounts_into(
    namespace: &mut Namespace,
    mounts: &[MeshMountSpec],
) -> Result<Vec<IrohMount>, CliError> {
    dial_and_bind(mounts, |remote, guest_path| {
        namespace
            .bind(remote, ".", guest_path, BindOptions::default())
            .map_err(CliError::from)
    })
}

fn dial_and_bind(
    mounts: &[MeshMountSpec],
    mut bind: impl FnMut(Arc<dyn FileSystem>, &str) -> Result<(), CliError>,
) -> Result<Vec<IrohMount>, CliError> {
    let mut keepalives = Vec::with_capacity(mounts.len());
    for mount in mounts {
        let dialed = dial_iroh_remote(&mount.addr, "")?;
        bind(dialed.remote.clone(), mount.guest_path.as_str())?;
        keepalives.push(dialed);
    }
    Ok(keepalives)
}

/// Loopback native-wire serving helpers shared by the mesh-mount proofs here
/// and the qjs/wasm/sh mount tests in their own modules.
#[cfg(test)]
pub(crate) mod test_support {
    use std::net::{Ipv4Addr, SocketAddr};
    use std::sync::Arc;

    use wanix_fs::FileSystem;
    use wanix_id::NodeIdentity;
    use wanix_mesh::{MeshNode, NativeServeConfig};
    use wanix_task::TaskTable;

    use crate::mesh::IROH_SCHEME;

    fn loopback() -> SocketAddr {
        SocketAddr::new(Ipv4Addr::LOCALHOST.into(), 0)
    }

    /// Serves `host` over the native mesh wire and returns the node plus the
    /// dialable `iroh://` URL (built the way `mesh-serve` announces it).
    pub(crate) fn serve_native(host: Arc<dyn FileSystem>, identity_seed: u8) -> (MeshNode, String) {
        let identity = NodeIdentity::from_secret_bytes([identity_seed; 32]);
        let mut server = MeshNode::bind_local(&identity, loopback()).unwrap();
        server.serve_native(NativeServeConfig::open(host));
        let peer = server.peer_id();
        let addrs: Vec<String> = server
            .ticket()
            .ip_addrs()
            .map(|addr| format!("addr={addr}"))
            .collect();
        (server, format!("{IROH_SCHEME}{peer}?{}", addrs.join("&")))
    }

    /// A fresh single-task `noop` table, mirroring `allocate_qjs_term_task`'s
    /// self-contained table (the returned `Task` does not borrow the table).
    pub(crate) fn noop_task() -> wanix_task::Task {
        let table = TaskTable::new();
        table.register_noop_driver("noop").unwrap();
        table.allocate_root("noop").unwrap()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use wanix_fs::{FileSystem, MemFs, NormalizedPath, OpenOptions};
    use wanix_vfs::Namespace;

    use super::test_support::{noop_task, serve_native};
    use super::{bind_mesh_mounts, bind_mesh_mounts_into};
    use crate::qjs_args::MeshMountSpec;

    fn read_through(namespace: &Namespace, path: &str) -> Vec<u8> {
        let mut file = namespace
            .open(&NormalizedPath::new(path).unwrap(), OpenOptions::read())
            .unwrap();
        let mut bytes = Vec::new();
        let mut chunk = [0_u8; 4096];
        loop {
            let read = file.read(&mut chunk).unwrap();
            if read == 0 {
                break;
            }
            bytes.extend_from_slice(&chunk[..read]);
        }
        bytes
    }

    fn write_through(namespace: &Namespace, path: &str, bytes: &[u8]) {
        let mut file = namespace
            .open(
                &NormalizedPath::new(path).unwrap(),
                OpenOptions {
                    read: false,
                    write: true,
                    create: true,
                    truncate: true,
                },
            )
            .unwrap();
        assert_eq!(file.write(bytes).unwrap(), bytes.len());
    }

    /// Slice 1 runtime proof: a `--mount-mesh` spec dialed by `bind_mesh_mounts`
    /// binds a remote native-wire volume into a task namespace, and the task can
    /// read a seeded file and write a new one (the served root observes it) while
    /// the returned keepalive is held. Dropping the keepalive only after the task
    /// mirrors the field order the qjs-shell runtime path relies on.
    #[test]
    fn mesh_mount_binds_into_task_namespace_and_stays_alive() {
        let host = Arc::new(MemFs::new());
        host.write_file("seed.txt", b"served-by-A").unwrap();
        let (server, url) = serve_native(host.clone() as Arc<dyn FileSystem>, 3);
        let task = noop_task();

        let spec = MeshMountSpec {
            addr: url,
            guest_path: NormalizedPath::new("vol").unwrap(),
        };
        let keepalive = bind_mesh_mounts(&task, std::slice::from_ref(&spec)).unwrap();
        assert_eq!(keepalive.len(), 1);

        // Read a seeded file through the task namespace: the mount is wired and the
        // keepalive runtime is alive for the read.
        let namespace = task.namespace();
        assert_eq!(read_through(&namespace, "vol/seed.txt"), b"served-by-A");

        // Write through the mount; the served root observes it across the wire.
        write_through(&namespace, "vol/from-task.txt", b"from-task");
        assert_eq!(host.read_file("from-task.txt").unwrap(), b"from-task");

        // Mirror PreparedQjsTermExecution drop order: task before the keepalive.
        drop(task);
        drop(keepalive);
        drop(server);
    }

    /// Slice 2 proof: two independent mesh mounts (two dialer nodes, as two
    /// separate `qjs-shell` processes would be) against ONE served open volume —
    /// a write through one mount is visible through the other. This is the
    /// resource-composition experience before any naming layer exists.
    #[test]
    fn two_task_namespaces_share_one_open_volume() {
        let host = Arc::new(MemFs::new());
        let (server, url) = serve_native(host.clone() as Arc<dyn FileSystem>, 4);
        let spec = MeshMountSpec {
            addr: url,
            guest_path: NormalizedPath::new("vol").unwrap(),
        };

        // Two independent task namespaces, each dialing its own mount/keepalive.
        let task_a = noop_task();
        let keep_a = bind_mesh_mounts(&task_a, std::slice::from_ref(&spec)).unwrap();
        let task_b = noop_task();
        let keep_b = bind_mesh_mounts(&task_b, std::slice::from_ref(&spec)).unwrap();

        // Shell A writes /vol/shared.txt; shell B reads it back over its own mount.
        let payload = b"written-by-A-read-by-B";
        let namespace_a = task_a.namespace();
        write_through(&namespace_a, "vol/shared.txt", payload);

        let namespace_b = task_b.namespace();
        assert_eq!(read_through(&namespace_b, "vol/shared.txt"), payload);
        // The single served root holds the shared byte stream.
        assert_eq!(host.read_file("shared.txt").unwrap(), payload);

        drop(task_a);
        drop(task_b);
        drop(keep_a);
        drop(keep_b);
        drop(server);
    }

    /// Per-resource composition (ADR 0007 step 4): one namespace mounts TWO
    /// independently served volume resources via repeated `--mount-mesh`, and
    /// writes stay scoped to their target volume. This is the per-resource-ticket
    /// model — no aggregate `/vol/*` root, the client composes the tickets.
    /// Exercised through `bind_mesh_mounts_into`, the task-free namespace shape
    /// the `wasm` and `sh` paths use.
    #[test]
    fn one_namespace_composes_two_independently_served_volumes() {
        let notes = Arc::new(MemFs::new());
        let photos = Arc::new(MemFs::new());
        let (notes_server, notes_url) = serve_native(notes.clone() as Arc<dyn FileSystem>, 5);
        let (photos_server, photos_url) = serve_native(photos.clone() as Arc<dyn FileSystem>, 6);

        let specs = [
            MeshMountSpec {
                addr: notes_url,
                guest_path: NormalizedPath::new("vol/notes").unwrap(),
            },
            MeshMountSpec {
                addr: photos_url,
                guest_path: NormalizedPath::new("vol/photos").unwrap(),
            },
        ];

        let mut namespace = Namespace::new();
        let keepalives = bind_mesh_mounts_into(&mut namespace, &specs).unwrap();
        assert_eq!(keepalives.len(), 2, "both mounts must be held alive");

        write_through(&namespace, "vol/notes/a.txt", b"note-a");
        write_through(&namespace, "vol/photos/b.txt", b"photo-b");

        // Writes are scoped: each landed only in its own served volume.
        assert_eq!(notes.read_file("a.txt").unwrap(), b"note-a");
        assert_eq!(photos.read_file("b.txt").unwrap(), b"photo-b");
        assert!(
            notes.read_file("b.txt").is_err(),
            "a photos write must not leak into the notes volume"
        );
        assert!(
            photos.read_file("a.txt").is_err(),
            "a notes write must not leak into the photos volume"
        );

        // Both are readable from the one composed namespace.
        assert_eq!(read_through(&namespace, "vol/notes/a.txt"), b"note-a");
        assert_eq!(read_through(&namespace, "vol/photos/b.txt"), b"photo-b");

        drop(namespace);
        drop(keepalives);
        drop(notes_server);
        drop(photos_server);
    }
}
