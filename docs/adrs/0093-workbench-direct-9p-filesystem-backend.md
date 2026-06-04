# ADR 0093: Workbench Direct 9P Filesystem Backend

## Status

Accepted

## Context

ADR 0092 added `serve --bundle fs9p` as a browser-side proof that Rust `serve`
discovery and the direct binary 9P WebSocket route can support basic filesystem
operations outside the old browser Wanix runtime. The VS Code/web workbench
extension already has a `wanix:` `FileSystemProvider`, but its backend was the
classic MessagePort/CBOR `WanixHandle` supplied by a browser `<wanix-system>`.

To move toward Rust `serve` and VS Code integration, the workbench needs a
filesystem backend that consumes `/.well-known/wanix.json` and talks to the Rust
9P export directly. That backend must not pretend to own process/task semantics:
Rust `serve` currently exports a directory over 9P, not `#task`, `#term`, or a
full Wanix task namespace.

## Decision

Add a workbench-local `WanixP9Handle` that matches the filesystem methods the
existing `WanixBridge` expects:

- `stat`, `readDir`, `readFile`, and `writeFile`;
- `makeDir`;
- `rename`, `copy`, `remove`, and recursive `removeAll`; and
- simple stream adapters for file-backed readable/writable use.

The handle connects by fetching `/.well-known/wanix.json`, opening
`routes.p9.websocket`, negotiating 9P2000.L, and issuing 9P operations such as
`Tgetattr`, `Treaddir`, `Tread`, `Tlcreate`, `Twrite`, `Tmkdir`, `Trenameat`,
`Tunlinkat`, and `Tclunk`.

The workbench extension keeps the existing MessagePort/CBOR backend as the
preferred path when an embedding `<wanix-system>` supplies one. Direct 9P
discovery is a fallback for Rust `serve` pages and does not enable terminal
creation unless a future Rust route exposes the needed `#task`/`#term`
semantics.

## Consequences

The existing `WanixBridge` can now be reused for a Rust-served `wanix:`
filesystem without copying VS Code provider logic. This is a concrete step from
the standalone `fs9p` demo toward a workbench route that opens a served Rust
Wanix filesystem.

The direct backend has intentionally narrow semantics. Error mapping is still
coarse, file copy is implemented as read/write for files only, and stream
helpers are file-buffered rather than live task streams. Those limits are
acceptable for the filesystem-provider slice and keep terminal/process behavior
out of the wrong layer.
