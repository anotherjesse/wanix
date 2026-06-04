# ADR 0005: Workspace Crate Boundaries

## Status

Accepted

## Context

The Rust port needs strong ownership boundaries so Wasmtime, QuickJS, host
filesystem access, protocol adapters, and CLI/browser composition do not leak
into the core Wanix contracts.

The workspace also absorbed the QuickJS/WASI prototype that was originally a
sibling project. That relocation makes it easier to evolve the engine import
boundary with Wanix, but it should not make the engine crate responsible for
Wanix process, namespace, or fd policy.

## Decision

Keep the workspace layered around contract ownership:

- `wanix-fs`: filesystem traits, file traits, metadata, errors, path rules,
  readiness hooks, and explicit host-directory-backed filesystems.
- `wanix-vfs`: Plan 9-style namespace binding and resolution.
- `wanix-task`: task identity, `#task`, task metadata files, fd tables, fd
  binding, and task driver registration.
- `wanix-term`: `#term` terminal device filesystem and readiness behavior.
- `wanix-wasi`: generic Wanix-owned WASI Preview 1 semantics backed by
  namespaces and task fds.
- `wanix-qjs-engine`: Wasmtime-hosted QuickJS/WASI mechanics, fixture loading,
  guest memory decoding, runtime creation/restoration, and provider plumbing.
- `wanix-qjs`: adapter that turns QuickJS engine runtimes into Wanix `qjs` task
  drivers.
- `wanix-protocol`: dependency-free protocol codecs, currently centered on 9P.
- `wanix-9p`: 9P server state and filesystem mapping backed by Wanix traits.
- `wanix-cli` and `serve`: native and browser-facing composition layers.

Live QuickJS WASI hooks are runtime host state carried by create/restore options
or equivalent runtime options, not deterministic `QuickJsHostConfig` data.
`QuickJsHostConfig` stays cloneable, comparable, and suitable for deterministic
fixture configuration; live Wanix providers are attached separately.

Core filesystem and namespace crates must remain free of Wasmtime and QuickJS.
Protocol codecs must remain independent of server, CLI, and browser transport
policy.

## Consequences

Each crate has one main reason to change, and trust boundaries are easier to
review. The engine crate can evolve with Wanix while still being only
QuickJS/Wasmtime plumbing. Wanix task, fd, namespace, and WASI semantics remain
in Wanix crates.

When a new capability crosses crate boundaries, prefer adding a narrow adapter
at the composition layer over moving policy into the engine or protocol crates.

## Replaces

This ADR consolidates ADR 0010 and ADR 0011 into the workspace boundary
decision.
