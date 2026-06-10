---
title: --wanix-services Device Set
slug: concepts/wanix-services-device-set
pageType: concept
oneLiner: Binds #task, #term, #kv, #pipe, #plumb, #cas, #agent into the served namespace from one INSPECTABLE_SERVICE_DEVICES source, advertised as services.devices.
audience: [developer]
tags: [serve, mesh, services, local-trust-only, caveat, cli, shipped]
sourceRefs:
  - crates/wanix-cli/src/serve/roots.rs:105-193
  - crates/wanix-cli/src/serve/discovery.rs:154-188
  - crates/wanix-cli/src/serve/http/app.rs:35-66
  - crates/wanix-cli/src/mesh/serve.rs:120-194
seeAlso:
  - concepts/service-devices
  - concepts/serve-composition-surface
  - concepts/discovery-document
  - concepts/fakeengine-vs-codex
  - reference/extension-points
prerequisites:
  - concepts/service-devices
  - concepts/serve-composition-surface
usedInFlows:
  - {flow: learn/add-a-service-device, step: 2}
honestLimits:
  - The served #agent here runs the deterministic FakeEngine, not a live LLM; real codex is the local-trust `wanix agent` CLI path only.
  - #kv is in-memory; its state lives only for the life of the serve process unless you freeze the world to a capsule.
  - The qjs-shell websocket and the /.wanix/app/<name> HTTP route exist only when --wanix-services is set; the app route is also loopback-only.
  - --wanix-services binds the #task/#agent exec devices, so it is refused on the public mesh endpoint even with --peer/--grant; exec is local-trust only.
---

# --wanix-services Device Set

Pass `--wanix-services` and `serve` stops exporting a bare directory: it binds the host root *plus* a fixed set of service devices into one namespace, then advertises exactly which devices it bound so a client never has to guess.

## What & why

A plain `wanix-rust serve --root DIR` exports `DIR` as 9P and nothing else. That is honest but inert — there is no task table to launch a `qjs` script, no `#kv` to hold app state, no `#agent` to drive. The `--wanix-services` flag composes those capabilities into the served namespace from a single source of truth, so the cockpit (or any direct-9P client) gets the same device set every time, and discovery tells it the set by name. The whole point of one bind list is that "what got bound," "what discovery advertises," and "what the tests exercise" cannot drift apart.

## Show, then name: turn on the device set

```sh
cargo build --package wanix-cli
alias wanix-rust='./target/debug/wanix-rust'

wanix-rust serve --root /tmp/world --wanix-services
```

Now the served root carries seven `#`-named devices alongside your files. Over 9P:

```sh
ls '#kv' '#task/new'            # storage keys; launchable task kinds
echo on > '#kv/feature.flag'   # #kv is a file
id=$(cat '#task/new/qjs')      # allocate a qjs task
```

The flag flips one boolean (`ServeRoots.wanix_services`), and that boolean is what every downstream surface keys off (`crates/wanix-cli/src/serve/roots.rs:30`).

## One function binds the host root and the devices

`serve_services_namespace` builds the whole thing (`crates/wanix-cli/src/serve/roots.rs:105-113`). It starts an empty `Namespace`, binds the host directory at `.`, then `bind_host_and_terminal` overlays `#term`, `#pipe`, `#kv`, `#plumb`, `#cas`, and `#agent` (`crates/wanix-cli/src/serve/roots.rs:122-172`), and finally `bind_task_service` binds `#task` from the task table (`crates/wanix-cli/src/serve/roots.rs:174-185`). Each device is a plain `FileSystem` — `KvDevice::new()`, `PipeDevice::new()`, `CasDevice` over the owner-private store — bound under its `#`-name, so it composes by the same rules as any other bind. (See [service devices](/concepts/service-devices) for the per-device contracts.)

`#task` is bound `Replace` rather than overlaid, because it is rooted at a freshly allocated root task that carries this exact namespace as its world — so a `qjs`/`wasm` task started through `#task/new/<kind>` sees the same `#kv` and `#cas` you do.

## INSPECTABLE_SERVICE_DEVICES is the single source

The list of devices is a single `const` (`crates/wanix-cli/src/serve/roots.rs:119-120`):

```rust
pub(super) const INSPECTABLE_SERVICE_DEVICES: &[&str] =
    &["#task", "#term", "#kv", "#pipe", "#plumb", "#cas", "#agent"];
```

Discovery reads that same constant to populate `services.devices` (`crates/wanix-cli/src/serve/discovery.rs:154-167`):

```json
"services": {
  "task": "#task",
  "term": "#term",
  "drivers": ["auto","noop","qjs","wasm"],
  "devices": ["#task","#term","#kv","#pipe","#plumb","#cas","#agent"]
}
```

When `--wanix-services` is off, `services` is the JSON literal `null` (`crates/wanix-cli/src/serve/discovery.rs:168-170`) — discovery says "no devices," not an empty stub. The `drivers` array is not hand-written: it is captured from the task registry at build time (`ServeRoots.driver_kinds`, `crates/wanix-cli/src/serve/roots.rs:84-87`), so the advertised launchable kinds cannot drift from what `#task/new/<kind>` actually accepts. Today that is `auto`, `noop`, `qjs`, and `wasm`. This is the [discovery document](/concepts/discovery-document) contract: one constant, one registry, no parallel hand-maintained list.

## The flag also unlocks the shell and HTTP-app routes

`--wanix-services` is the gate for two more surfaces beyond the device binds:

- **The qjs-shell websocket.** With services on, discovery advertises `qjsShell` as `available` (protocol `wanix-qjs-shell.v1`, raw-bytes mode, with the cwd query, resize, and exit-frame contract); with services off it reports `{"status":"disabled"}` (`crates/wanix-cli/src/serve/discovery.rs:173-188`).
- **The HTTP-app route.** `GET /.wanix/app/<name>` runs `apps/<name>.js` (or `.wasm`) and returns its stdout. The handler refuses with `404` unless `wanix_services` is set, and with `403` unless the peer is loopback (`crates/wanix-cli/src/serve/http/app.rs:55-66`). The counter demo backs its state with `#kv/http-counter`, so refreshing the page increments a value that lives in the same in-memory `#kv` you can `cat` over 9P. (This route is SHIPPED on this branch.)

Both routes need a task table and `#kv` to mean anything, which is why they ride the same flag.

## How to add a device to the bind set

Adding a service device to `--wanix-services` is three edits in `crates/wanix-cli/src/serve/roots.rs`, in lockstep:

1. **Bind it** in `bind_host_and_terminal` — construct the device (a plain `FileSystem`) and `namespace.bind(Arc::new(MyDevice::new()), ".", "#mydev", BindOptions::default())`.
2. **List it** in `INSPECTABLE_SERVICE_DEVICES` so discovery's `services.devices` advertises it.
3. **Test it** — the `serve_wanix_services_*` tests open each device over 9P; add a case so the new device is exercised, not just bound.

The const's doc comment states the invariant out loud: the list "must stay in sync with the binds" (`crates/wanix-cli/src/serve/roots.rs:115-118`). Keep the three in step and a new device is inspectable, advertised, and covered the moment it lands. The [add-a-service-device flow](/learn/add-a-service-device) walks this end to end; [extension points](/reference/extension-points) lists the other seams.

## See also

- [Service devices](/concepts/service-devices) — the `#`-named device catalog and each one's read/write contract.
- [The serve composition surface](/concepts/serve-composition-surface) — how `serve` layers process, TCP, websocket, and HTTP routes over one root.
- [The discovery document](/concepts/discovery-document) — what `/.well-known/discovery` advertises and why clients read it instead of guessing.
- [FakeEngine vs codex](/concepts/fakeengine-vs-codex) — why the served `#agent` is deterministic and codex stays on the CLI.
- [Extension points](/reference/extension-points) — the seams for adding drivers, transports, and devices.

## Status / honest limits

- The served `#agent` runs `FakeEngine`, a deterministic stub, not a live LLM. The real codex bridge needs auth and unattended execution, so it stays on the local-trust `wanix agent` CLI path; the served bind hard-codes the fake engine (`crates/wanix-cli/src/serve/roots.rs:163-170`).
- `#kv` is in-memory: `KvDevice::new()` holds state only for the life of the serve process. To persist, freeze the world to a [capsule](/concepts/wanix-capsule).
- The qjs-shell websocket and the `/.wanix/app/<name>` route exist only with `--wanix-services`, and the app route is loopback-only (`403` otherwise).
- `--wanix-services` binds the `#task`/`#agent` exec devices — remote code execution. The mesh path keeps exec local-trust only, so `mesh-serve --wanix-services` is refused on any non-loopback endpoint even with `--peer`/`--grant` (a LAN `--addr` is mDNS-discoverable, so it is not local trust); the one allowed path is a loopback `--addr 127.0.0.1:PORT` socket (`crates/wanix-cli/src/mesh/serve.rs`). This buys cheap, scalable isolation, not safety for arbitrary untrusted code: there are no hard CPU or memory limits yet.
