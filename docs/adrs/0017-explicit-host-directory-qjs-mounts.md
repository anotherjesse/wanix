# ADR 0017: Explicit Host Directory Mounts

## Status

Accepted

## Context

Native demos and tests need to expose selected host files to Wanix tasks.
Treating the host filesystem as the default Wanix namespace would erase the
runtime boundary the Rust port is meant to preserve.

Host access is a trust boundary: paths must not escape the selected root, and
guest code should see an explicit Wanix mount rather than ambient host state.

## Decision

Expose host directories only through explicit rooted `LocalFs` mounts into a
Wanix namespace.

Host roots are canonicalized before use, and filesystem operations are checked
so guest paths cannot escape the configured root through `..`, symlink traversal
where escape protection applies, or absolute host paths. CLI demos bind those
roots at explicit guest paths and pass those guest paths to Wanix tasks.

No core Wanix task, WASI, or namespace path should implicitly mean "open this
host path" unless a `LocalFs` mount was configured at that point in the
namespace.

## Consequences

Native qjs, 9P, serve, v86, and workbench demos can use real host files without
making the host filesystem the runtime foundation. The same path-resolution
rules apply whether the caller is JavaScript through WASI, a 9P client, a CLI
command, or a service-file workflow.

Future host exposure such as network filesystems, R2FS, HTTPFS, or wider native
mount policies should preserve the explicit-root trust boundary.
