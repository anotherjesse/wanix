---
title: Extending Wanix from the Edges
slug: reference/extension-points
pageType: developer
oneLiner: Three extension points — a service device is a FileSystem, a task driver is a TaskDriver, a transport is a 9P adapter — so you do not need to fork the core.
audience: [developer]
tags: [extensibility, shipped, mesh, cli, local-trust-only, caveat]
sourceRefs:
  - crates/wanix-kv/src/lib.rs:1-177
  - crates/wanix-task/src/driver.rs:1-29
  - crates/wanix-task/src/table.rs:47-64
  - crates/wanix-9p/src/transport.rs:82-127
  - crates/wanix-9p-client/src/transport.rs:1-32
  - crates/wanix-9p-client/src/remote.rs:40-90
  - crates/wanix-cli/src/serve/roots.rs:189-191
seeAlso:
  - concepts/the-filesystem-trait
  - concepts/task-drivers
  - concepts/protocol-vs-server-split
  - concepts/devices-import-for-free
  - concepts/wanix-services-device-set
  - learn/add-a-service-device
  - learn/add-driver-or-transport
prerequisites:
  - reference/crate-map-and-layering
  - concepts/the-filesystem-trait
usedInFlows:
  - {flow: add-a-service-device, step: 1}
  - {flow: add-driver-or-transport, step: 1}
honestLimits:
  - The served exec devices (#task/#agent/#cpu) are local-trust only; new task drivers do not change that and there are no hard CPU/memory limits yet.
  - serve handles one 9P frame at a time per connection, so a new transport adapter must preserve that serial framing rather than assume concurrency.
  - Tauth stays ENOSYS; a new transport carries no 9P auth handshake, so authorization happens at attach via capability binds, not in the transport.
canonicalCaveatFor: []
---

# Extending Wanix from the Edges

Three extension points — a service device is a `FileSystem`, a task driver is a `TaskDriver`, a transport is a 9P adapter — so you do not need to fork the core.

Most systems make you patch the kernel to add a feature. Wanix does not, because the kernel is a small set of contracts and everything interesting plugs into one of three of them. A new device is a filesystem. A new language runtime is a driver. A new wire is an adapter around the existing 9P core. Each point is a trait you implement and a one-line registration from the orchestration layer; none of them touches `wanix-fs`, `wanix-vfs`, `wanix-task`, or `wanix-protocol`. This page shows each extension point, the exact trait, and the layering rule it obeys.

## The three points

| You want to add | Implement | Registered from | You get for free |
| --- | --- | --- | --- |
| A new device (`#thing`) | `wanix_fs::FileSystem` | the namespace bind in `serve` | mesh import, 9P export, cockpit inspector |
| A new task runtime | `wanix_task::TaskDriver` | `TaskTable::register_driver` | `#task` files, autostart, observable exit |
| A new wire | a thin adapter over `P9Server` / `RemoteFs` | your `serve`/mount call site | the whole 9P codec and fid state |

The order matters: the first is the cheapest and the most common, the third is the rarest. Reach for a new transport only when none of TCP, WebSocket, process pipes, or iroh QUIC fit.

## Service device = `impl FileSystem`

`#kv` is the template. Read `crates/wanix-kv/src/lib.rs:120-166`: `KvDevice` is a struct around an `Arc<RwLock<BTreeMap<String, Vec<u8>>>>`, and it becomes a device purely by implementing `FileSystem`. Reading `#kv/<key>` is `open` with the read bit returning a `KvReadFile` (`lib.rs:129-131`); writing is `open` with the write/create bit returning a `KvWriteFile` (`lib.rs:125-128`); listing `#kv` is `read_dir` over the map (`lib.rs:142-154`); deleting a key is `remove_file` (`lib.rs:156-165`). That is the whole device. There is no networking code, no protocol code, no task code.

The payoff is the quiet superpower from [the crate map](/reference/crate-map-and-layering): because the device is *just a filesystem*, it gets three things at no cost.

- **Mesh import for free.** Mounting a remote 9P export is itself a `FileSystem` (`RemoteFs`), so a peer's `#kv` is reachable through the same namespace machinery as a local bind. See [devices import for free](/concepts/devices-import-for-free).
- **9P export for free.** `serve` wraps a root filesystem in a `P9Server` and exports it; your device, bound into that root, is exported with no per-device wire work.
- **Cockpit inspection for free.** Adding the device to the inspectable set surfaces it in the browser operator view over direct 9P.

To ship it: implement `FileSystem`, then bind it into the served namespace next to the existing devices in `crates/wanix-cli/src/serve/roots.rs` (the `#task`/`#term`/`#kv`/`#plumb`/`#cas`/`#agent` set lives in `roots::INSPECTABLE_SERVICE_DEVICES`). The full step-by-step is [add a service device](/learn/add-a-service-device). One honesty note for stateful devices: `#kv` is in-memory (`lib.rs:21,63`), so its state lives only as long as the serve process. Persistence is freezing the world to a [capsule](/concepts/wanix-capsule), not a device feature.

## Task driver = `impl TaskDriver`

A task runtime is two methods. `crates/wanix-task/src/driver.rs:6-18` is the entire trait:

```rust
pub trait TaskDriver: Send + Sync {
    fn check(&self, _task: &Task) -> bool { false }   // can I auto-start this task?
    fn start(&self, task: &Task) -> FsResult<()>;     // run it
}
```

`check` is how a `cmd` auto-resolves to a kind: the qjs driver answers true when the program ends with `.js`, the wasm driver when it ends with `.wasm` (`crates/wanix-qjs/src/driver.rs:83-84`, `crates/wanix-wasm/src/driver.rs:30-31`). `start` reads the module from the task's namespace, builds a live WASI config from the task's namespace/cwd/env/argv and fds 0/1/2, runs the guest, and records the exit through `Task::set_exit`. That contract — task identity, fd table, namespace, exit — is owned by `wanix-task`; the driver only supplies the engine mechanics. The arrow runs runtime → task, never the reverse, which is exactly why you can add a third driver without editing the task crate. See [task drivers](/concepts/task-drivers).

Registration is one call from the orchestration layer, not a core edit. `crates/wanix-task/src/table.rs:47-59` exposes `register_driver(kind, Arc<dyn TaskDriver>)`, and the serve path wires the shipped set in three lines (`crates/wanix-cli/src/serve/roots.rs:189-191`):

```rust
table.register_noop_driver("noop")?;
table.register_driver("qjs", Arc::new(QuickJsTaskDriver::new(quickjs_runner()?)))?;
table.register_driver("wasm", Arc::new(WasmTaskDriver::new()))?;
```

Add your driver with a fourth line, give it a `check`, and a matching `cmd` auto-starts through `#task/new`. The walkthrough is [add a driver or transport](/learn/add-driver-or-transport). The boundary this does **not** move: the exec devices (`#task`, `#agent`, `#cpu`) are local-trust only, and a new driver inherits that — it is cheap, scalable isolation, not a sandbox for arbitrary untrusted code, and there are no hard CPU or memory limits yet.

## Transport = a thin adapter over `P9Server` or `RemoteFs`

The 9P codec, fid state, metadata mapping, and compatibility probes all live in `wanix-9p` and `wanix-9p-client` (see [protocol vs. server split](/concepts/protocol-vs-server-split)). A new transport adds *none* of that. It adds a way to get bytes in and out.

On the **export** side, `P9Server::serve_stream` takes any `Read` + `Write` and drives the whole server loop over it: it reads request frames, dispatches each, and writes the response frames until EOF (`crates/wanix-9p/src/transport.rs:95-126`). Your adapter's only job is to hand `serve_stream` a reader and a writer — a TCP socket, a WebSocket data channel, a process pipe — and let the existing codec do the rest.

On the **import** side, `RemoteFs::connect` takes a `Box<dyn Duplex>`, where `Duplex` is the blanket trait over `Read + Write + Send` (`crates/wanix-9p-client/src/transport.rs:24-32`, `crates/wanix-9p-client/src/remote.rs:56-61`). Any blocking bidirectional byte stream satisfies it; the crate brings no transport of its own. `connect_with_aname` (`remote.rs:74-79`) is the capability-scoped variant the mesh uses to import a grant-gated subtree.

Two rules a transport adapter must honor:

1. **Preserve frame boundaries and serial framing.** `serve_stream` processes one frame at a time per connection (`transport.rs:117-124`). serve handles one 9P frame at a time per connection, so a blocking read (such as a `#plumb/<topic>/recv`) cannot interleave with a write on the same connection — your adapter must not assume it can. Live pub/sub needs a second connection or concurrent frame handling, not a clever single-channel transport.
2. **Authorize at attach, not in the wire.** Tauth stays ENOSYS — there is no 9P auth handshake — so a transport carries no credentials of its own. Authorization is a capability bind evaluated at `Tattach` against the verified peer identity (the mesh edge supplies the peer; see [the attach-and-capability contract](/reference/attach-and-capability-contract)). A capability is a bind, not an ACL.

## How each fits the layering rules

All three points obey the one rule from [the crate map](/reference/crate-map-and-layering): arrows point down. A device depends only on `wanix-fs` (plus `wanix-vfs` for binding). A driver depends on `wanix-task` and its own engine, never the reverse. A transport reuses the synchronous 9P core and stays out of it. Crucially, none of the three is allowed to drag tokio or iroh into the core — that async edge belongs to `wanix-mesh` alone. When your extension compiles and `just check` is green (fmt + module-lines + clippy `-D warnings` + test), the layering held. Keep new modules under the 250/350-line guardrail, and prefer explicit newtypes over raw `i32` flags at any trust boundary.

## See also

- [The FileSystem trait](/concepts/the-filesystem-trait) — the contract a service device implements.
- [Task drivers](/concepts/task-drivers) — why runtimes plug into `wanix-task` from above.
- [Protocol vs. server split](/concepts/protocol-vs-server-split) — the codec a transport reuses.
- [Devices import for free](/concepts/devices-import-for-free) — the mesh payoff of "device = filesystem."
- [The Wanix services device set](/concepts/wanix-services-device-set) — what `serve --wanix-services` binds today.
- [Add a service device](/learn/add-a-service-device) and [Add a driver or transport](/learn/add-driver-or-transport) — the guided flows.
- [Crate map & dependency direction](/reference/crate-map-and-layering) — the layering rule all three obey.

## Status / honest limits

- A new **task driver** does not widen the trust boundary. The exec devices (`#task`, `#agent`, `#cpu`) are local-trust only and are not exposed to untrusted or public peers; they offer cheap, scalable isolation, not a safe sandbox for arbitrary untrusted code, and there are no hard CPU or memory limits yet.
- A new **transport** must preserve `serve`'s one-frame-at-a-time-per-connection framing (`crates/wanix-9p/src/transport.rs:117-124`); it cannot make a blocking `#plumb` recv interleave with a write on the same connection.
- Tauth stays ENOSYS: a transport carries no 9P auth handshake. Authorization is a capability bind evaluated at attach against the verified peer, not anything the wire negotiates.
- A new **stateful device** modeled on `#kv` is in-memory by default (`crates/wanix-kv/src/lib.rs:21,63`); state lives only for the serve process unless you freeze the world to a capsule.
