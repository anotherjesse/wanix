---
title: Add a task driver or a transport
slug: learn/add-driver-or-transport
pageType: flow
oneLiner: Plug a new task runtime into #task or carry 9P over a new wire, without forking the core.
audience: [developer]
tags: [extension-point, task-drivers, ninep, transport, local-trust-only, caveat]
sourceRefs:
  - crates/wanix-task/src/driver.rs:6-18
  - crates/wanix-task/src/table.rs:48-59
  - crates/wanix-task/src/table.rs:138-154
  - crates/wanix-wasm/src/driver.rs:29-54
  - crates/wanix-wasm/src/task_stdio.rs:8
  - crates/wanix-9p/src/transport.rs:95-126
  - crates/wanix-9p/src/lib.rs:375-385
  - crates/wanix-9p-client/src/remote.rs:56-90
  - crates/wanix-cli/src/mount.rs:26
seeAlso:
  - concepts/task-drivers
  - concepts/compiled-wasm-task-driver
  - concepts/shared-wasi-fd-contract
  - concepts/protocol-vs-server-split
  - concepts/the-9p-contract
  - concepts/remotefs-import-half
  - reference/extension-points
  - reference/crate-map-and-layering
prerequisites:
  - learn/add-a-service-device
usedInFlows: []
honestLimits:
  - "Exec devices (#task and the drivers behind them) are local-trust only; the server does not sandbox arbitrary untrusted task kinds, and there are no hard CPU or memory limits yet."
  - "Tauth stays ENOSYS — there is no 9P auth handshake for your transport to negotiate."
  - "serve handles one 9P frame at a time per connection, so a transport cannot interleave a blocking read with a write on the same connection."
  - "The shipped CLI mount binds a single slot /n/remote; per-peer /n/<peer-id> is designed but unshipped."
canonicalCaveatFor: []
---

# Add a task driver or a transport

Plug a new task runtime into `#task` or carry 9P over a new wire, without forking the core.

Wanix has two clean extension seams, and they sit at opposite ends of the same contract. A **task driver** teaches `#task` how to run a new kind of program; a **transport** teaches the 9P server and client how to talk over a new wire. Both are small adapters that register *from above* — the core crates never learn about your driver or your socket. This flow shows the exact trait surface each one implements, the layering law that keeps it honest, and the constraints you must design around. Prerequisite: do the [add a service device](/learn/add-a-service-device) flow first, since a driver and a transport both assume you already know how a `FileSystem` becomes a namespace entry.

Build once, then follow either branch:

```sh
cargo build --package wanix-cli
alias wanix-rust='./target/debug/wanix-rust'
```

## Branch A: a task driver is `check` + `start`

Watch the effect first. A `.wasm` cmd auto-starts as a task and reports an exit:

```sh
wanix-rust wasm examples/rust-guest.wasm --list /dir
```

The whole driver contract is two methods. From `crates/wanix-task/src/driver.rs:6-18`:

```rust
pub trait TaskDriver: Send + Sync {
    fn check(&self, _task: &Task) -> bool { false }   // can I auto-start this?
    fn start(&self, task: &Task) -> FsResult<()>;     // run it
}
```

`check` is the auto-select predicate; `start` runs the program and records the exit through `Task::set_exit`. That is the entire surface a third runtime has to satisfy — a Python interpreter, a Lua VM, a native ELF loader. The worked model is `WasmTaskDriver` in `crates/wanix-wasm/src/driver.rs:29-54`: its `check` claims any task whose program ends in `.wasm` (line 30-32), and its `start` reads the module bytes *from the task's own namespace*, compiles them, builds a live WASI config, runs `_start`, and records the guest's exit code.

**Register from above.** Drivers are inserted by the orchestrator, not discovered by the core. `TaskTable::register_driver(kind, Arc<dyn TaskDriver>)` (`crates/wanix-task/src/table.rs:48-59`) adds your driver under a kind name. When a task is allocated as `"auto"`, `TaskTable::start` walks the registered drivers and runs the first whose `check` returns true, then sets the task's kind to match (`crates/wanix-task/src/table.rs:138-154`). That is the suffix auto-select you saw with `.wasm`: nothing in `wanix-task` mentions wasm or QuickJS — the binding happens entirely at registration time.

**Reuse the shared WASI config, do not reinvent it.** If your runtime is WASI-shaped, build its sandbox through the one shared builder. `wanix-wasm` re-exports it rather than owning fd wiring: `pub(crate) use wanix_wasi::task_wasi_config;` (`crates/wanix-wasm/src/task_stdio.rs:8`). The namespace/cwd/env/argv binding and the fd-mirroring contract live in `wanix-wasi` so every WASI runtime — QuickJS and compiled wasm today — follows one [shared WASI fd contract](/concepts/shared-wasi-fd-contract). A new WASI task driver should call the same builder, not hand-roll fds 0/1/2.

**The layering law.** `wanix-task` must never depend on a runtime engine. `check`/`start` take a `&Task` and return `FsResult` — no Wasmtime, no engine type crosses that boundary. Your driver crate depends *down* on `wanix-task` and on whatever engine it wraps; the core stays free of it. See [task drivers](/concepts/task-drivers) and [the compiled-wasm driver](/concepts/compiled-wasm-task-driver) for the canonical write-up.

## Branch B: a transport is a thin adapter over the sync 9P core

The wire is also a seam, and it is thinner than it looks. The server side is one method on `P9Server`: `serve_stream<R: Read, W: Write>` reads frames from any blocking byte stream and writes responses back (`crates/wanix-9p/src/transport.rs:95-126`). The client side is `RemoteFs::connect(Box<dyn Duplex>)` — hand it any blocking bidirectional stream and it negotiates a session and binds the served root as a local `FileSystem` (`crates/wanix-9p-client/src/remote.rs:56-90`). A new **9P transport** is the adapter that produces those streams: a TLS socket, a serial line, a WebTransport datagram channel, a pair of pipes. TCP already proves the shape — the `tcp://` arm of `dial_remote` in `crates/wanix-cli/src/mount.rs` yields a `RemoteFs`.

There is also a rarer move: a new **encoding** over the same `FileSystem` contract, not just a new byte transport for 9P. The [native mesh wire](/concepts/missing-half-of-9p) (`wanix-mesh-wire`) is the worked example — between two Wanix nodes the mesh dials `dial_native` and gets a `NativeFs` (a `postcard`-framed, typed-error codec, one QUIC stream per call/open-file) instead of a `RemoteFs`. Both are transport-agnostic and async-free; both ride the same sync `Duplex` boundary and the same `BlockingDuplex` bridge in `wanix-mesh`. Reach for a new encoding only between trait-speakers (Wanix↔Wanix); for a foreign peer, add a 9P transport adapter.

**The protocol-vs-server split is the reuse boundary.** Framing, codecs, and version negotiation live in `wanix-protocol` (dependency-free, wire-only); fid state and filesystem mapping live in `wanix-9p`. A transport touches neither. Do not reinvent frame splitting — feed bytes to the existing core and let it split frames for you. This is the [protocol-vs-server split](/concepts/protocol-vs-server-split) and the wider [9P contract](/concepts/the-9p-contract); your job is to move bytes, not to re-encode `Twalk`.

**The frame-boundary rule.** `serve_stream` is strictly serial: it reads, decodes complete frames, handles each one, writes the response, then loops (`crates/wanix-9p/src/transport.rs:104-125`). A response is fully written before the next request is read. Your transport must preserve frame boundaries and must not assume the server will read a second request while a first is still being answered. `RemoteFs` mirrors this on the client: it holds one connection behind a mutex and keeps a single request outstanding (`crates/wanix-9p-client/src/remote.rs:1-8`).

## What you do not have to build

Two negotiations you might expect are deliberately absent, so do not design around them.

- **No auth handshake.** `Tauth` returns `ENOSYS` and binds no fid (`crates/wanix-9p/src/lib.rs:375-385`). There is no in-band 9P authentication for your transport to layer onto; identity and the default-deny attach policy live in the transport's own handshake (ed25519 on the iroh path), not in a 9P auth message.
- **One frame at a time per connection.** Because `serve_stream` answers one frame before reading the next, a blocking read on a streaming file (a `#plumb` recv that waits for an event) cannot interleave with a write on the *same* connection. Live pub/sub over your transport needs a second connection or concurrent frame handling — that is unbuilt, not a bug to paper over.

## Run the gate

Both branches are subject to the same dependency-direction and module-line guardrails. Run the required check before every cycle commit:

```sh
just check
```

Keep tokio and iroh out of everything but `wanix-mesh`; keep Wasmtime out of the core filesystem and namespace crates; keep modules under the 250–350 line guardrail. The [extension points](/reference/extension-points) reference and the [crate map](/reference/crate-map-and-layering) spell out exactly which crate your new code belongs in.

## See also

- Concepts: [task drivers](/concepts/task-drivers) · [the compiled-wasm task driver](/concepts/compiled-wasm-task-driver) · [the shared WASI fd contract](/concepts/shared-wasi-fd-contract) · [protocol vs. server split](/concepts/protocol-vs-server-split) · [the 9P contract](/concepts/the-9p-contract) · [RemoteFs, the import half](/concepts/remotefs-import-half)
- Reference: [extension points](/reference/extension-points) · [crate map and layering](/reference/crate-map-and-layering)
- Prerequisite flow: [add a service device](/learn/add-a-service-device)

## Status / honest limits

- **Exec is local-trust only.** `#task` and the drivers behind it run as cheap, scalable isolation for code you already trust — not a sandbox for arbitrary untrusted task kinds. There are no hard CPU or memory limits yet; do not expose a driver to untrusted or public peers.
- **No 9P auth.** `Tauth` is `ENOSYS` (`crates/wanix-9p/src/lib.rs:375-385`); a transport authenticates in its own handshake, not in-band.
- **Single-frame serve.** One 9P frame is handled per connection at a time (`crates/wanix-9p/src/transport.rs:95-126`); a blocking read cannot interleave with a write on the same connection.
- **Single mount slot.** The shipped CLI mount binds one slot, `/n/remote` (`crates/wanix-cli/src/mount.rs:26`); per-peer `/n/<peer-id>` is designed but not shipped. Treat `/n/<peer>` as a labelled convention only.
