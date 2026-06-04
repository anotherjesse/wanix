# ADR 0001: Rust-Native Wasmtime Runtime

## Status

Accepted

## Context

Wanix began as a browser-centered Go and JavaScript runtime. The Rust port is
not a line-by-line translation of that tree; it is a chance to restate the
runtime contracts that make Wanix useful outside Chrome.

The durable goal is a Rust-native Wanix core that acts as the host or
microkernel runtime. Wasmtime is the guest execution substrate, and
QuickJS/WASI is the first serious task runtime. Browser support, v86, VS Code,
and other clients become frontends or deployment surfaces of that Rust runtime
rather than the foundation that defines process, filesystem, or namespace
semantics.

The Go implementation remains valuable because it records existing Wanix
behavior, especially filesystem, namespace, service-file, task, terminal, and
9P semantics. It should be treated as a semantic oracle during migration, not
as a package structure or API surface to copy blindly.

## Decision

Build Rust Wanix around the Wanix contracts rather than around the old
implementation layout:

- Wanix owns filesystem behavior, namespace binding and resolution, task
  identity, task service files, fd tables, terminal devices, and externally
  visible process state.
- Wasmtime hosts guest runtimes. It is not allowed to inherit host filesystem or
  process semantics by default.
- QuickJS/WASI is the initial task runtime and the proof that JavaScript can run
  as a Wanix process outside Chrome.
- Host files enter Wanix only through explicit rooted mounts with escape checks;
  ambient host paths are not part of the default namespace.
- Browser, workbench, v86, native QEMU, and CLI paths compose with the Rust
  runtime through explicit protocols and service files.
- Migration uses Go behavior and tests as references when semantics are
  ambiguous, but Rust crate boundaries should follow the contracts being
  rebuilt.

Keep the workspace layered around contract ownership:

- `wanix-fs`: filesystem traits, metadata, errors, path rules, readiness hooks,
  in-memory fixtures, and explicit host-directory-backed filesystems.
- `wanix-vfs`: Plan 9-style namespace binding and resolution.
- `wanix-task`: task identity, `#task`, metadata files, fd tables, fd binding,
  and task driver registration.
- `wanix-term`: `#term` terminal device filesystem and readiness behavior.
- `wanix-wasi`: Wanix-owned WASI Preview 1 semantics backed by namespaces and
  task fds.
- `wanix-qjs-engine`: Wasmtime-hosted QuickJS/WASI mechanics, fixture loading,
  guest memory decoding, runtime creation/restoration, and provider plumbing.
- `wanix-qjs`: adapter that turns QuickJS engine runtimes into Wanix `qjs` task
  drivers.
- `wanix-protocol`: dependency-free protocol codecs, currently centered on 9P.
- `wanix-9p`: 9P server state and filesystem mapping backed by Wanix traits.
- `wanix-cli` and `serve`: native and browser-facing composition layers.

Core filesystem, namespace, and task crates must remain free of Wasmtime and
QuickJS. Protocol codecs must remain independent of server, CLI, and browser
transport policy.

## Consequences

The Rust port can make progress by rebuilding externally visible capability
instead of translating every Go file. Architectural choices should favor
Wanix-owned semantics at trust boundaries, even when host or Wasmtime defaults
would be easier.

Keep this ADR broad. Use more specific ADRs only for durable runtime,
QuickJS/WASI, terminal, protocol, serve, client handoff, trust-boundary, or
workflow decisions.
