# ADR 0007: Workspace-Local Rust Quality Gate

## Status

Accepted

## Context

`cargo fmt --all` formats all workspace packages and local path dependencies.
While `wanix-qjs` depends on the sibling `rust-wasi-quickjs` prototype, that
command reaches outside the Wanix repository and can fail on prototype files
that are intentionally outside this port's commit scope.

## Decision

The required formatting check enumerates Wanix workspace packages explicitly:

```sh
cargo fmt --package wanix-cli --package wanix-fs --package wanix-qjs --package wanix-task --package wanix-vfs --package wanix-wasi --check
```

The clippy and test checks remain workspace-wide:

```sh
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --locked
```

## Consequences

- Cycle commits enforce formatting for all Wanix crates without mutating or
  requiring cleanup in sibling prototype repositories.
- Local path dependencies can still be compiled, linted as dependencies, and
  tested through Wanix integration tests.
- Revisit this once `rust-wasi-quickjs` is vendored, published, or folded into
  the Wanix workspace.
