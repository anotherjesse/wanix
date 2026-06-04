# ADR 0092: Browser Filesystem And Workbench Integration

## Status

Accepted

## Context

VS Code/workbench integration is a client path for Rust Wanix, not a separate
runtime foundation. It should discover Rust serve routes, talk direct 9P, and
drive Wanix service files when it needs tasks or terminals.

The old MessagePort filesystem bridge was useful for the browser-era runtime,
but the Rust path should prefer direct 9P and the same `#task`/`#term` service
contracts used by native clients.

## Decision

The browser filesystem and workbench path uses Rust serve discovery and direct
9P:

- `serve --bundle fs9p` can expose a browser filesystem smoke page that fetches
  discovery, opens the direct 9P WebSocket route, and proves paginated
  browse/read/write operations against the served root.
- The workbench extension can back its `wanix:` filesystem provider with direct
  9P operations for stat, paginated directory listing, read, write, rename, and
  delete, plus bounded client-side file and text search over `wanix:/`.
- `serve --bundle workbench-fs9p` is a local generated VS Code web workbench
  launch path that points the extension at Rust serve discovery.
- When discovery advertises services, the workbench path can open a qjs shell
  terminal route and can start a `qjs` Wanix task by driving `#task` and
  `#term` over direct 9P, including terminal resize delivery through
  `#term/<id>/winch`.
- Running the active `wanix:` JavaScript file as a `qjs` task should use the
  same task command/env/dir/fd service files as native clients.

Generated launcher details, activation timing, and browser-smoke warnings live
in tests and current-state docs unless they change the route or service
contract.

## Consequences

The editor path validates the same big pieces as the native path: serve
discovery, direct 9P, service namespaces, terminal routes, and QuickJS-backed
Wanix tasks. It also keeps browser/editor integration as a frontend to Rust
Wanix rather than a replacement runtime model.

Future workbench work should add ADRs only for durable API, route, auth, trust,
or lifecycle changes.

## Replaces

This ADR consolidates ADR 0093, ADR 0094, and ADR 0098 into the browser
filesystem and workbench integration contract.
