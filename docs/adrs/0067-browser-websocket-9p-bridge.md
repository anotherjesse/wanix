# ADR 0067: Browser WebSocket 9P Bridge

## Status

Accepted.

## Context

The Go `wanix serve` path accepts WebSocket upgrades and treats binary messages
as 9P request bytes for browser-side v86. Rust Wanix now has a filesystem-backed
9P server, a stdio bridge, a native TCP listener, metadata replies, and core
mutation operations, but browser clients cannot use raw TCP directly.

The next browser-facing step should expose the same Rust 9P server over a binary
WebSocket transport without committing yet to a full static asset server,
network/vnet bridge, or public auth policy.

## Decision

Add `wanix-rust p9-ws --root DIR --addr HOST:PORT [--once]`:

- the command exports a `LocalFs` root through `P9Server`;
- each WebSocket connection gets a fresh `P9Server`, so fid state is
  connection-local;
- incoming binary WebSocket messages are appended to a 9P frame buffer;
- each decoded 9P request is handled synchronously and each response is sent as
  one binary WebSocket message;
- text messages are ignored, ping messages receive pongs, and close messages
  end the session when no partial 9P frame is buffered;
- `--once` serves one WebSocket connection and exits for deterministic tests.

The bridge lives in `wanix-cli` because WebSocket binding and exposure are
deployment policy. `wanix-9p` remains the frame-to-filesystem server core.

## Consequences

Rust Wanix can now serve a host-root 9P export to browser experiments that speak
binary WebSocket messages, matching the important transport shape used by the Go
serve/v86 path. This makes the next serve cycle a composition problem rather
than a protocol/server problem.

Future work should wire this bridge into a Rust `serve` command with static
asset headers, choose an auth/exposure policy for writable exports, and decide
how v86 should discover the WebSocket URL.
