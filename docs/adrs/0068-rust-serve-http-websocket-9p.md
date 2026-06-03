# ADR 0068: Rust Serve HTTP and WebSocket 9P

## Status

Accepted.

## Context

Rust Wanix now has a filesystem-backed 9P server plus stdio, TCP, and binary
WebSocket exports. The Go `wanix serve` path is still the reference for browser
and v86 composition: it serves static assets with browser isolation headers and
uses WebSocket upgrades as 9P sessions.

The next big-piece milestone should not be another cleanup pass. Browser v86
and VS Code-style integrations need one native endpoint that can deliver assets
and expose the Wanix filesystem transport shape they can actually reach from a
browser.

## Decision

Add `wanix-rust serve --root DIR --addr HOST:PORT [--once]` in `wanix-cli`:

- ordinary HTTP `GET` requests serve files from the host-root directory;
- static responses include `Cross-Origin-Opener-Policy: same-origin`,
  `Cross-Origin-Embedder-Policy: require-corp`, and
  `Access-Control-Allow-Origin: *`;
- directory requests use `index.html`;
- path traversal and escaped separators are rejected before host files are read;
- WebSocket upgrade requests are handed to the existing binary 9P WebSocket
  connection handler, so `serve` and `p9-ws` share request framing and response
  behavior;
- each WebSocket connection gets a fresh `P9Server`, preserving connection-local
  fid state;
- `--once` serves one HTTP request or one WebSocket session and exits for tests
  and scripted demos.

The command is intentionally a CLI/composition feature. `wanix-9p` continues to
own only the frame-to-filesystem server core, while static asset policy,
WebSocket exposure, and future v86/VS Code routing remain outside core crates.

## Consequences

Rust Wanix now has the first native serve shape that can host browser assets and
the browser-reachable 9P export on the same listener. This makes qemu/v86 and
VS Code experiments a question of asset/routing/client policy rather than a
missing transport.

This ADR does not decide public auth, writable export exposure, vnet bridging,
qemu/v86 bundle selection, HTTPS, or VS Code-specific routes. Those should be
added deliberately once the clients are wired to this endpoint.
