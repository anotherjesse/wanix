# ADR 0001: Rust-Native Wasmtime Runtime

## Status

Accepted

## Context

Wanix began as a browser-centered Go and JavaScript runtime. The Rust port
rebuilds the runtime contracts that make Wanix useful outside Chrome; it is not
a line-by-line translation of the old tree.

The durable goal is a Rust-native Wanix core that acts as the host or
microkernel runtime. Wasmtime is the guest execution substrate, and
QuickJS/WASI is the first serious task runtime. Browser support, v86, VS Code,
native QEMU, and other clients become frontends or deployment surfaces of that
Rust runtime rather than the foundation that defines process, filesystem, or
namespace semantics.

The Go implementation remains a semantic oracle for existing Wanix behavior,
especially filesystem, namespace, service-file, task, terminal, and 9P
semantics. It is not a package structure or API surface to copy blindly.

## Decision

Build Rust Wanix around Wanix-owned contracts rather than the old
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

Keep lower-level crates independent from runtime engines and client policy.
Core filesystem, namespace, terminal, and task crates must remain free of
Wasmtime and QuickJS. Protocol codecs must remain independent of server, CLI,
and browser transport policy. The current crate map belongs in `AGENTS.md`;
this ADR records the ownership rule.

## Consequences

The Rust port can make progress by rebuilding externally visible capability
instead of translating every Go file. Architectural choices should favor
Wanix-owned semantics at trust boundaries, even when host or Wasmtime defaults
would be easier.

Keep this ADR broad. Milestones, crate inventories, and demo status belong in
current-state docs and commit messages. Add or update narrower ADRs only for
durable runtime, QuickJS/WASI, terminal, protocol, serve, client handoff,
trust-boundary, or workflow decisions.
