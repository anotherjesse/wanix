# ADR 0092: Serve fs9p Browser Filesystem Bundle

## Status

Accepted

## Context

Rust `serve` already exposes static HTTP, `/.well-known/wanix.json`, and the
direct binary 9P WebSocket route used by v86. That proves the server side of the
browser-facing filesystem contract, but the Rust port still lacked a small
browser client that consumes the discovery document and speaks that 9P route
directly.

The eventual VS Code/web workbench path needs the same shape: discover a Wanix
runtime from the page origin, open a browser-safe filesystem transport, and map
file operations onto Wanix-owned filesystem semantics. Naming this proof
`vscode` would overstate the current capability because it is not a VS Code
`FileSystemProvider` yet.

## Decision

Add `wanix-rust serve --bundle fs9p` as a generated browser filesystem smoke
bundle. The generated page:

- fetches `/.well-known/wanix.json`;
- opens `discovery.routes.p9.websocket` as a binary WebSocket;
- negotiates and attaches a 9P2000.L session; and
- exposes basic directory listing, read, write/create/truncate, rename, and
  delete operations through 9P messages.

Other bundle names still fall through to the static root. The bundle is a
browser-side integration proof over the existing `serve` 9P route, not a new
filesystem policy layer and not a replacement for the future VS Code workbench
extension.

## Consequences

The Rust port now has a browser-visible filesystem demo that exercises the same
discovery and 9P transport surface a VS Code/web workbench integration can build
on. It also gives qemu/v86/editor work a smaller debugging target than a full VM
boot when the issue is the browser filesystem transport itself.

Future cycles can factor the page's 9P client into a reusable workbench module,
add richer error/status mapping, and implement a VS Code `FileSystemProvider`
against the same discovery and direct 9P route.
