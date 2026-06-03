# ADR 0008: QuickJS Namespace Modules and Virtual WASI Projection

## Status

Accepted

## Context

The first QuickJS task driver exposed a temporary `Wanix.readText` /
`Wanix.writeText` object so JavaScript could observe a Wanix namespace before
full Wanix-backed WASI imports were wired. The `rust-wasi-quickjs` prototype
also has a read-only virtual WASI filesystem. At the time of this decision, the
fixture did not initialize QuickJS libc or `qjs:std`, so JavaScript could not
yet issue file-read WASI calls directly. ADR 0012 later enabled
`qjs:std`/`qjs:os` for stdio-facing guest WASI proofs.

## Decision

`wanix-qjs` installs a Rust-side ES module loader that resolves imports through
the task namespace. Relative module specifiers such as `./lib.js` are normalized
against the importing module path and loaded as Wanix files.

At runtime setup, `wanix-qjs` also projects the root
`wanix_wasi::WasiConfig` preopen into the prototype's read-only virtual WASI
configuration. The qjs-facing configuration is `QuickJsWanixConfig` so the
copied-file adapter is named as a Wanix policy boundary, not as a durable
QuickJS process or fd model. Additional Wanix preopens are rejected instead of
flattened because the prototype only exposes one copied virtual root. This is
groundwork for future `qjs:std` file reads, not the final dynamic Wanix WASI
implementation.

## Consequences

- QuickJS tasks can compose JavaScript modules from Wanix namespaces outside
  Chrome.
- The CLI can copy a host script directory into a fresh Wanix namespace and run
  scripts with ordinary relative imports.
- The virtual WASI projection is read-only, stale after runtime creation, and
  copies visible files into memory up front.
- The projection rejects non-root/multiple preopens; live Wanix-backed WASI
  imports are required before QuickJS can observe real preopen fd semantics.
- The projection is centralized at the `QuickJsWanixConfig` /
  `wanix_wasi::WasiConfig` boundary, which is the replacement point for custom
  Wanix-owned WASI imports.
- `qjs:std` filesystem reads remain a follow-up before guest JS can read Wanix
  files through actual QuickJS WASI file APIs.
