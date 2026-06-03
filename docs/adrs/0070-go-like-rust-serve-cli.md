# ADR 0070: Go-Like Rust Serve CLI

## Status

Accepted.

## Context

Rust `wanix-rust serve` now exposes static HTTP and browser-reachable 9P
WebSocket routes, but its first CLI shape still looked like a transport test
harness: callers had to pass both `--root` and `--addr`.

The Go `wanix serve` command is the existing demo entrypoint for browser/v86
experiments. It defaults to serving the current directory, accepts a positional
directory, uses `--listen`, and prints a bundle URL when `--bundle` is supplied.
The Rust command should preserve the useful demo workflow even while the runtime
semantics continue moving to Rust.

## Decision

Make Rust `serve` more Go-like:

- `wanix-rust serve` defaults to root `.` and address `127.0.0.1:7654`;
- a single positional directory is accepted as the root;
- `--root DIR` remains supported for explicit scripts;
- `--listen HOST:PORT` is accepted as an alias for `--addr HOST:PORT`;
- `--listen :PORT` normalizes to `0.0.0.0:PORT` for binding;
- `--bundle NAME` reports a browser URL with `?bundle=NAME`;
- status output reports the served root and the browser URL while keeping the
  static/9P route behavior unchanged.

## Consequences

Rust serve is now usable as the native browser/v86 demo entrypoint instead of
only as a low-level transport command. Existing scripted uses of `--root`,
`--addr`, and `--once` still work.

This does not implement qemu/v86 bundle assembly, VS Code route wiring, or the
Ethernet/vnet bridge. It makes those next steps easier to run and explain.
