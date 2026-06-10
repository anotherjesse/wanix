---
title: Wanix-Backed WASI (not host WASI)
slug: concepts/wanix-backed-wasi
pageType: concept
oneLiner: WASI Preview 1 syscalls resolve through Wanix namespaces and WASI fds instead of the host OS, and any #name device path resolves from the namespace root regardless of cwd.
audience: [developer]
tags: [wasi, runtime, shipped, caveat, command-style]
sourceRefs:
  - crates/wanix-wasi/src/lib.rs:1-27
  - crates/wanix-wasi/src/ctx.rs:22-161
  - crates/wanix-wasi/src/ctx/path.rs:7-41
  - crates/wanix-wasi/src/ctx/init.rs:11-73
  - crates/wanix-wasi/src/ctx/fd_ops.rs:10-113
  - crates/wanix-wasi/src/config.rs:116-242
  - crates/wanix-wasi-host/src/lib.rs:1-13
  - crates/wanix-wasi-host/src/core.rs:36-37
  - rust-walkthrough.md:247-285
seeAlso:
  - concepts/shared-wasi-fd-contract
  - concepts/wasmtime-as-substrate
  - concepts/host-not-ambient-authority
  - concepts/command-style-wasm-linker
  - concepts/service-devices
prerequisites:
  - concepts/shared-wasi-fd-contract
usedInFlows: []
honestLimits:
  - "The wasm linker is a command-style Preview 1 subset: poll_oneoff returns NOSYS (errno 52), so a compiled .wasm task cannot block on fd readiness."
  - "qjs has a richer engine-local host path (live fd readiness, a richer poll_oneoff); the two share WasiCtx, not the linker."
  - "Clocks are deterministic by default (a fixed clock_time_ns), not wall-clock; this is policy, not a real-time source."
---

# Wanix-Backed WASI (not host WASI)

WASI Preview 1 syscalls resolve through Wanix namespaces and WASI fds instead of the host OS, and any `#name` device path resolves from the namespace root regardless of cwd.

A normal WASI runtime hands the guest a slice of the host OS: a host directory becomes a preopen, host files back the fds, the host clock backs `clock_time_get`. Wanix does not. The `WasiCtx` in `crates/wanix-wasi/src/ctx.rs` carries a Wanix `Namespace`, not a host root, and every filesystem syscall a guest makes lands on that namespace. The crate doc says it plainly: "Filesystem calls resolve through Wanix namespaces and WASI file descriptors instead of delegating namespace behavior to the host operating system or a generic WASI filesystem adapter" (`crates/wanix-wasi/src/lib.rs:1-7`). That single substitution is what keeps the authority boundary at Wanix rather than the host.

## Show it: a guest reaches `#kv` from any cwd

Run a JavaScript task; the WASI calls behind `std.writeFile`/`std.loadFile` go through `WasiCtx`:

```sh
cargo build --package wanix-cli
alias wanix='./target/debug/wanix'
wanix qjs examples/qjs-demo.js
```

```js
import * as std from "qjs:std";

// fd 3 is preopened at the namespace root, cwd is "."
std.writeFile("notes.txt", "relative to cwd");      // joins onto cwd
std.writeFile("#kv/greeting", "hello");             // resolves from root
std.out.puts(std.loadFile("#kv/greeting") + "\n");  // -> hello
```

`notes.txt` is a cwd-relative path: it is joined onto the directory fd's base. `#kv/greeting` is not — it resolves from the namespace root no matter where the task's cwd points. That asymmetry is deliberate, and it is in one function.

## Root and cwd: `resolve_path` and `is_rooted_service_path`

When a guest calls `path_open`, the host runs `resolve_path` (`crates/wanix-wasi/src/ctx.rs:126-141`). It looks up the directory fd's base path and required rights, validates the raw guest string with `wasi_path`, and then decides: is this a rooted service path?

```rust
fn resolve_path(&self, dirfd, path, required) -> Result<NormalizedPath, Errno> {
    let (base, rights_base, _) = self.directory_handle(dirfd)?;
    if !rights_base.contains(required) { return Err(Errno::Notcapable); }
    let path = wasi_path(path)?;
    if is_rooted_service_path(&path) { return Ok(path); }   // from root
    join_paths(base, &path).map_err(Errno::from)            // from cwd
}
```

`is_rooted_service_path` checks whether the first path component is one of the known `#name` devices (`crates/wanix-wasi/src/ctx/path.rs:12-19`): `#task`, `#term`, `#kv`, `#pipe`, `#plumb`, `#cas`, `#agent`, `#mesh`, `#cpu`. If so, the path is returned untouched, so the namespace's own resolver finds the device at the root. Everything else is `join_paths`'d onto the directory fd's base. The list is exact on purpose: an unrecognized `#name` (a file literally named `#foo` in your cwd) stays cwd-relative — Wanix does not magic-route arbitrary `#`-prefixed names, only the device set it knows. This is why the [service devices](/concepts/service-devices) are reachable from any task regardless of where it `chdir`'d.

`wasi_path` is the other half of the safety: it rejects absolute paths, trailing slashes, `//`, backslashes, NULs, and any `.`/`..` component with `Errno::Notcapable` (`crates/wanix-wasi/src/ctx/path.rs:21-41`). The guest cannot traverse out of its namespace by spelling a path cleverly; there is no host root underneath to escape to anyway.

## Fds, rights, and clocks are Wanix's to define

A `WasiCtx` starts with fd 3 preopened at the namespace root and stdio bound to whatever the task config supplied (`crates/wanix-wasi/src/ctx/init.rs:11-52`, `crates/wanix-wasi/src/config.rs:116-129`). `fd_read`/`fd_write` dispatch on the handle kind and check the open mode and the per-fd `WasiRights` before touching the file (`crates/wanix-wasi/src/ctx/fd_ops.rs:10-57`): a read on a write-only fd returns `Notcapable`, a read on a directory returns `Isdir`. Rights are projected at open time in `path_open` from the parent directory's inheriting rights and the guest's requested rights (`crates/wanix-wasi/src/ctx/open.rs:87-118`), so a guest never gains more authority than its preopen granted.

Clocks are deterministic by default. `clock_time_get` and the WASI `*_NOW` timestamp flags read `clock_time_ns` from the config, which defaults to a fixed value (`crates/wanix-wasi/src/config.rs:14-18`, `crates/wanix-wasi/src/ctx/path_ops.rs:152-159`). This is a policy choice — reproducible runs and snapshot-stable timestamps — not a real-time source. An embedder that wants wall-clock time sets it explicitly with `with_clock_time_ns`.

## Why the boundary stays at Wanix

Because nothing under the guest is the host OS, "what can this program touch?" has a single answer: its namespace. There is no ambient host directory, no implicit `/tmp`, no host environment leaking in. Authority is the set of binds in the namespace plus the rights on each fd — see [host, not ambient, authority](/concepts/host-not-ambient-authority). The same `WasiCtx` is the substrate for both task tiers: a [qjs task](/concepts/qjs-task) and a [compiled .wasm task](/concepts/compiled-wasm-task-driver) see identical filesystem semantics because they share this one context (the [shared WASI fd contract](/concepts/shared-wasi-fd-contract)).

## Under the hood: two host paths, one context

`WasiCtx` is engine-agnostic. The compiled-`.wasm` runner drives it through the generic [command-style linker](/concepts/command-style-wasm-linker) in `wanix-wasi-host`, which marshals guest memory into `WasiCtx` calls. QuickJS does **not** use that linker — qjs keeps its own engine-local host-import path with live fd readiness, snapshot blockers, restore reattachment, and a richer `poll_oneoff` (`crates/wanix-wasi-host/src/lib.rs:1-13`). They share `WasiCtx` and the task-config contract, not the linker.

For behavior with no Wanix contract, the linker returns deliberate errors rather than faking success. The clearest is `poll_oneoff`, which the command-style core registers as a stub returning `ERRNO_NOSYS` (`crates/wanix-wasi-host/src/core.rs:36-37`). A compiled `.wasm` command cannot block on readiness — it runs straight through and exits. That is honest: Wanix has no readiness contract to offer there yet, so it says NOSYS instead of pretending.

## See also

- [Shared WASI fd contract](/concepts/shared-wasi-fd-contract) — why qjs and wasm see identical fd semantics.
- [Wasmtime as substrate](/concepts/wasmtime-as-substrate) — the execution engine under both tiers.
- [Host, not ambient, authority](/concepts/host-not-ambient-authority) — the authority model this enforces.
- [Command-style wasm linker](/concepts/command-style-wasm-linker) — the generic Preview 1 marshalling for `.wasm`.
- [Service devices](/concepts/service-devices) — the `#name` set that resolves from the namespace root.

## Status / honest limits

- The wasm path is a **command-style Preview 1 subset**: `poll_oneoff` returns `NOSYS` (errno 52) in the shared linker (`crates/wanix-wasi-host/src/core.rs:36-37`), so a compiled `.wasm` task cannot block on fd readiness — it runs to completion and exits.
- **qjs is richer than the wasm linker.** QuickJS carries its own host-import path with live fd readiness and a fuller `poll_oneoff` (`crates/wanix-wasi-host/src/lib.rs:5-8`); the two tiers share `WasiCtx`, not the linker, so do not assume wasm readiness from qjs behavior.
- **Clocks are deterministic by default**, sourced from `clock_time_ns` (`crates/wanix-wasi/src/config.rs:14-18`), not the wall clock. This is reproducibility policy; set `with_clock_time_ns` for a different value.
- The rooted-`#name` set is **exact** (`crates/wanix-wasi/src/ctx/path.rs:12-14`): only those device names resolve from the root. Any other `#`-prefixed name stays cwd-relative.
