# ADR 0058: Rust 9P Protocol Framing

## Status

Accepted

## Context

The Go/browser Wanix path already uses 9P in several places: the browser
system element exposes `_open9P`, the WASM bridge buffers 9P frames by size and
correlates replies by tag, and the v86 path uses 9P2000.L over virtio to mount
Wanix filesystems into a Linux guest.

The Rust port needs 9P for the larger serve/v86/VS Code integration path, but
starting with a full server would mix wire framing, task namespace policy, and
filesystem export decisions too early.

## Decision

Add a dependency-free `wanix-protocol` crate and start it with 9P wire helpers:

- generic frame encode/decode for `size[4] type[1] tag[2] payload`;
- tag extraction from raw frame bytes for v86-style request/reply routing;
- a stream frame buffer for partial writes and multiple frames per chunk; and
- typed `Tversion`/`Rversion` helpers for `msize` and version-string
  negotiation, including the existing `9P2000.L` version string.

Generic frames preserve unknown message types so transport code can split and
route messages before the Rust port implements every 9P2000.L operation.

## Consequences

Rust Wanix now has an owned 9P wire boundary that can be reused by future native
server/client work without pulling in Wasmtime, QuickJS, task, namespace, or
filesystem policy.

The next 9P steps are to add typed request/response payloads for the operations
Wanix must serve first, then build a `wanix-fs`/`wanix-vfs` backed server and
wire it into native serve/v86 demos. This ADR does not decide the 9P server
authorization model, fid lifetime policy, directory entry format details, or
how HTTPFS/R2FS protocol pieces should share the crate.
