# ADR 0008: QuickJS Namespace Modules and Virtual WASI Projection

## Status

Accepted

## Context

The first QuickJS task driver exposed a temporary `Wanix.readText` /
`Wanix.writeText` object so JavaScript could observe a Wanix namespace before
full Wanix-backed WASI imports were wired. The `rust-wasi-quickjs` prototype
also has a read-only virtual WASI filesystem, but the current fixture does not
initialize QuickJS libc or `qjs:std`, so JavaScript cannot yet issue file-read
WASI calls directly.

## Decision

`wanix-qjs` installs a Rust-side ES module loader that resolves imports through
the task namespace. Relative module specifiers such as `./lib.js` are normalized
against the importing module path and loaded as Wanix files.

At runtime setup, `wanix-qjs` also snapshots visible regular files from the
namespace into the prototype's read-only virtual WASI configuration. This is
groundwork for future `qjs:std` file reads, not the final dynamic Wanix WASI
implementation.

## Consequences

- QuickJS tasks can compose JavaScript modules from Wanix namespaces outside
  Chrome.
- The CLI can copy a host script directory into a fresh Wanix namespace and run
  scripts with ordinary relative imports.
- The virtual WASI projection is read-only, stale after runtime creation, and
  copies visible files into memory up front.
- `qjs:std` support remains a prototype-side follow-up before guest JS can read
  Wanix files through actual QuickJS WASI file APIs.
