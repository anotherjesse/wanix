# ADR 0010: Workspace-Local QuickJS Engine Crate

## Status

Accepted

## Context

Wanix has been depending on `~/lw/rust-wasi-quickjs` as a sibling path
prototype. That was useful while the QuickJS/Wasmtime runtime surface was being
proved out, but the next Wanix milestone needs to change the WASI boundary so
Wanix-owned task, namespace, fd, cwd, env, and exit semantics flow into
QuickJS-backed tasks.

Keeping the engine outside the Wanix workspace now adds friction: API changes
span two repositories, the workspace lockfile does not fully describe the Rust
Wanix runtime, and the prototype's standalone read-only WASI projection is easy
to mistake for the target Wanix-owned WASI model.

## Decision

Move the QuickJS/Wasmtime engine code into the Wanix Rust workspace as
`crates/wanix-qjs-engine`.

The first relocation preserves behavior and the existing Rust import path:
`wanix-qjs` depends on package `wanix-qjs-engine` under the local dependency
name `rust_wasi_quickjs`, while the engine crate's library target remains
`rust_wasi_quickjs`.

Keep `wanix-qjs-engine` responsible for QuickJS WASM module loading,
instantiation, copied JavaScript values, callbacks, module loading, and
snapshot mechanics. Keep Wanix process identity, fd tables, namespaces, cwd,
env, command state, and exit status in `wanix-task`, `wanix-vfs`, and
`wanix-wasi`, with `wanix-qjs` adapting between the layers.

Do not vendor the prototype's `reference/quickjs-wasi` checkout or build
artifacts into Wanix in this relocation. The workspace crate carries the Rust
source, tests, examples, docs, scripts, and checked-in QuickJS WASM fixture
needed by the current engine behavior.

## Consequences

Wanix can evolve the QuickJS/WASI import boundary in one workspace and one
quality gate. Future patches can replace the engine crate's virtual WASI
projection with Wanix-owned syscall hooks without crossing a sibling repo
boundary.

The engine crate remains deliberately below `wanix-qjs`: it must not learn
Wanix task identity or global fd semantics. Those belong to Wanix runtime
crates, while the engine crate supplies reusable QuickJS/Wasmtime mechanics.

The sibling `~/lw/rust-wasi-quickjs` checkout is left untouched by this
relocation so any in-progress prototype work can still be inspected or compared
as an oracle.
