# ADR 0012: QuickJS Fixture And Namespace Modules

## Status

Accepted

## Context

Wanix checks in a QuickJS WASI fixture so tests, demos, snapshots, and CLI
commands use a repeatable guest runtime. That fixture is a compatibility
boundary when it changes exported modules, imported WASI calls, or snapshot
identity.

The old read-only virtual WASI projection was a bridge used before live
Wanix-backed WASI was available. The remaining fixture value is the guest
standard library and module surface, not a Wanix runtime filesystem.

## Decision

Keep the checked-in QuickJS WASI fixture as the JavaScript runtime artifact for
Wanix `qjs` tasks. It intentionally exposes:

- `qjs:std` and `qjs:os` namespace module aliases used by Wanix tests and
  demos;
- `scriptArgs`, stdio, environment, file, directory, symlink, readlink,
  timestamp, truncate, sleep, timer, and fd readiness helpers that route through
  WASI;
- module normalization needed for Wanix guest code; and
- a stable module hash boundary for snapshot compatibility.

The engine crate may keep read-only virtual files as engine-only fixture support
for isolated tests, but Wanix runtime paths must use live Wanix-backed WASI
providers for filesystem, fd, and service-path behavior.

Fixture rebuild details, helper-by-helper source changes, and SHA churn belong
in engine crate build notes or commit messages unless a fixture change alters
the durable guest module/import contract.

## Consequences

Guest JavaScript has a stable `qjs:std`/`qjs:os` surface while Wanix retains
ownership of runtime semantics. Removing Wanix dependencies on virtual files
keeps the engine crate focused on QuickJS/Wasmtime mechanics.

Older snapshots can be invalidated by fixture changes. That is acceptable when
the guest module/import contract changes and the new fixture hash makes the
compatibility boundary explicit.

## Replaces

This ADR consolidates the current namespace-module part of ADR 0008 and the
fixture decisions from ADR 0038 and ADR 0039 into the QuickJS fixture boundary.
