# ADR 0008: QuickJS Namespace Modules and Superseded Virtual WASI Projection

## Status

Superseded by ADR 0011 and live Wanix-backed WASI runtime paths.

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

The old runtime setup also projected the root `wanix_wasi::WasiConfig` preopen
into the prototype's read-only virtual WASI configuration. That bridge was
temporary groundwork for future `qjs:std` file reads, not the final dynamic
Wanix WASI implementation.

After ADR 0011, `wanix-qjs` runtime paths attach a live
`QuickJsWasiHost` backed by `wanix_wasi::WasiCtx` instead of copying namespace
files into `QuickJsHostConfig`. Engine-level read-only virtual files remain
available only as `wanix-qjs-engine` fixture/support behavior; Wanix qjs task
and runner paths should use live WASI.

## Consequences

- QuickJS tasks can compose JavaScript modules from Wanix namespaces outside
  Chrome.
- The CLI can copy a host script directory into a fresh Wanix namespace and run
  scripts with ordinary relative imports.
- The virtual WASI projection was removed from `wanix-qjs` runtime paths.
- `qjs:std`/`qjs:os` filesystem calls now reach Wanix namespace and fd
  semantics through live WASI providers.
- Engine-level read-only virtual files remain useful for engine tests and
  deterministic fixture support, but they are no longer a Wanix runtime bridge.
