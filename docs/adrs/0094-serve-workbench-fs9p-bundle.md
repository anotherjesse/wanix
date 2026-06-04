# ADR 0094: Serve workbench-fs9p Launcher

## Status

Accepted

## Context

ADR 0093 added a direct 9P backend to the web workbench extension, but there was
still no Rust-served page that launched VS Code web without the old browser
Wanix `<wanix-system>` MessagePort bridge. The existing example launchers still
boot through the browser runtime, while Rust `serve` only exposed the lower-level
`fs9p` smoke page and the v86 boot handoff.

The missing externally visible proof is a filesystem-only workbench path:

- Rust `serve` hosts static assets and `/.well-known/wanix.json`;
- VS Code web loads the checked-out `workbench/` extension package;
- the workspace opens as `wanix:/`; and
- the extension falls back to direct 9P discovery for filesystem operations.

This proof must not imply terminal, task, or process support. Rust `serve` still
exports a directory over 9P, not a complete `#task` or `#term` namespace.

## Decision

Add `wanix-rust serve --bundle workbench-fs9p`. When requested as
`/?bundle=workbench-fs9p`, Rust `serve` returns a generated VS Code web launch
page that:

- loads Code OSS web assets from `/workbench/code/out/`;
- registers `/workbench/` as the builtin `wanix.workbench` web extension;
- opens `wanix:/` as the trusted workspace;
- fetches `/.well-known/wanix.json` before launch so the active Rust serve
  endpoint is visible to the page; and
- supplies only a VS Code IPC activation channel, without an `event.data.wanix`
  filesystem port, so any filesystem backend must come from the extension's
  direct 9P fallback rather than the legacy MessagePort/CBOR bridge.

The generated page accepts optional `assets`, `workspace`, `extension`, and
`debug` query parameters for local experiments, but its default contract is the
repository-local workbench package served from the same root.

## Consequences

The Rust port now has a direct route from `wanix-rust serve` to a VS Code web
workspace rooted at `wanix:/`, with no browser Wanix runtime or legacy Wanix
filesystem port supplied by the page. This is the first Rust-served workbench
launch surface and a better target for browser-side integration regressions than
the standalone 9P smoke page alone.

The first browser smoke proved that the generated page boots Code OSS and opens
the `wanix:/` workspace root. It also exposed the next integration blocker:
this checked-out Code OSS asset set does not yet register the served
`wanix.workbench` extension from `/workbench/`, so Explorer does not yet
populate through the direct 9P backend. That registration/activation issue is
the next workbench filesystem blocker, not a reason to reintroduce the legacy
Wanix filesystem bridge.

The bundle is intentionally a dev/demo path. The Code OSS assets under
`workbench/code/` and the built extension under `workbench/dist/` are generated
or downloaded local artifacts and remain outside the Rust binary. Packaging,
asset installation, terminal creation, `#task`/`#term` exposure, search
providers, and auth/TLS policy are separate follow-up decisions.
