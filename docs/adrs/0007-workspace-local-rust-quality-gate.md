# ADR 0007: Workspace-Local Rust Quality Gate

## Status

Accepted

## Context

The Wanix Rust port has multiple workspace crates with distinct ownership
boundaries. The quality gate should make the intended crate set explicit so new
cycles do not accidentally skip a runtime crate or reach outside this repo.

## Decision

The required formatting check enumerates Wanix workspace packages explicitly:

```sh
cargo fmt --package wanix-cli --package wanix-fs --package wanix-qjs --package wanix-qjs-engine --package wanix-task --package wanix-vfs --package wanix-wasi --check
```

The clippy and test checks remain workspace-wide:

```sh
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --locked
```

## Consequences

- Cycle commits enforce formatting for all current Wanix workspace crates.
- The explicit package list is duplicated in docs and must be updated when
  crates are added or renamed.
- Workspace-wide clippy and tests still compile and exercise cross-crate
  integration through Cargo's normal package graph.
