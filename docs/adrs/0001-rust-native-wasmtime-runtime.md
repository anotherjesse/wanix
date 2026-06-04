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
- Browser, workbench, v86, native QEMU, and CLI paths compose with the Rust
  runtime through explicit protocols and service files.
- Migration uses Go behavior and tests as references when semantics are
  ambiguous, but Rust crate boundaries should follow the contracts being
  rebuilt.

## Consequences

The Rust port can make progress by rebuilding externally visible capability
instead of translating every Go file. Architectural choices should favor
Wanix-owned semantics at trust boundaries, even when host or Wasmtime defaults
would be easier.

This ADR is intentionally broad. More specific ADRs record crate boundaries,
QuickJS task semantics, Wanix-owned WASI, snapshots, terminals, 9P, serve,
workbench, v86, and QEMU handoff contracts.

## Replaces

This ADR absorbs the durable direction from ADR 0004. The separate Go-oracle
record was removed because the oracle rule is part of the runtime direction,
not a standalone architecture boundary.
