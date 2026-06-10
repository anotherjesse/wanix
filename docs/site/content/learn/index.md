---
title: Learn Wanix — Guided Flows
slug: learn/index
pageType: home
oneLiner: A catalog of persona-shaped learning flows that thread the canonical concept, device, and recipe pages in order so you finish each goal with something working and honest caveats.
audience: [newcomer, developer, visionary]
tags: [flow, overview, shipped, local-trust-only, mesh, cli, caveat]
sourceRefs:
  - AGENTS.md
  - README.md
  - docs/integration/plan.md
  - docs/recipes/
  - docs/mesh-blueprint.md
seeAlso:
  - concepts/everything-is-a-file
  - concepts/per-process-namespaces
  - concepts/missing-half-of-9p
  - devices/kv
  - devices/agent
  - devices/cpu
  - devices/term
  - recipes/02-mount-remote-peer
  - recipes/04-tiny-http-app-with-kv
  - recipes/01-repair-broken-qjs
  - recipes/05-two-agents-collaborate
  - recipes/06-compose-volume-and-tools
  - recipes/07-chatroom-over-the-mesh
  - recipes/08-real-tools-with-config
  - recipes/09-web-door-gateway
  - reference/contributor-landing
  - concepts/the-9p-contract
  - reference/crate-map-and-layering
  - reference/adr-index
prerequisites: []
usedInFlows: []
honestLimits:
  - The served #agent uses a deterministic FakeEngine, not a live LLM; real codex is local-trust only.
  - "#kv is an in-memory tier — state survives only while the serve process lives; freeze it into a capsule for durability across restarts."
  - The serve 9P WebSocket handles one frame at a time per connection, so a blocking read cannot interleave with a write on the same connection.
  - "#cpu exec and --wanix-services are local-trust-only; --wanix-services is refused on the public mesh endpoint because it is remote code execution."
  - Mesh imports currently land at /n/remote, not yet the per-peer /n/<peer-id> the design promises.
  - The `wanix-rust cpu` dial verb ships, but no CLI serve mode binds the #cpu acceptor yet, so a cross-node cpu job has no shipped server side to dial.
---

# Learn Wanix — Guided Flows

Wanix is a Rust-native Plan 9 for one machine *and* many: everything is a file, every process gets its own namespace, and a mesh imports remote namespaces as local files — over the native FileSystem-over-iroh wire between Wanix nodes, with 9P as the foreign edge. That is a lot of surface area to meet all at once. **Flows** are the antidote — each one picks a single goal, names a reader it is written for, and walks the canonical concept, device, and recipe pages in the right order so you finish with something working instead of a pile of tabs.

## Flows vs. concepts: a sequence, not a reference

A **concept** page (`everything-is-a-file`, `per-process-namespaces`, `missing-half-of-9p`) and a **device** page (`#kv`, `#agent`, `#cpu`) are canonical: they explain one idea completely, and they are where facts live. A **flow** is the opposite shape — it is a hand-held *sequence* that visits those canonical pages in an order chosen for a goal, stops to run real commands, and is honest about where the shipped reality stops short of the design. Concepts are the map; flows are a route across it. When a flow needs a fact, it links the concept rather than restating it, so the truth stays in exactly one place.

Every flow below starts from the same native entry point. Build once (full story: [build & install](../reference/build-and-install)), alias the binary, and run the hero:

```sh
# From the workspace root. The first build takes a few minutes.
cargo build --locked --package wanix-cli
alias wanix-rust='./target/debug/wanix-rust'
wanix-rust qjs examples/qjs-demo.js
```

---

## Flow: JavaScript outside Chrome in 10 minutes — *for Maya*

**Goal:** run a `.js` file as a real Wanix task with live WASI, stdio, and an observable exit status — no browser anywhere. **Route:** [everything-is-a-file](../concepts/everything-is-a-file) → the `qjs` task runtime → `#task` → the [terminal/shell](../devices/term) device.

QuickJS is the engine *inside* the task; Wanix owns the identity, namespace, fds, and exit status around it. Three commands take you from "hello" to an interactive shell:

```sh
wanix-rust qjs examples/qjs-demo.js
wanix-rust qjs-term --stdin "hello terminal" examples/qjs-term-demo.js
printf 'write note.txt hello\nls\ncat note.txt\nexit\n' | wanix-rust qjs-shell
```

Newcomer note: guest JavaScript uses `qjs:std`, `qjs:os`, `scriptArgs`, and service files — *not* a `globalThis.Wanix` object. That bridge is retired.

---

## Flow: Wire two machines into a mesh — *for Devraj*

**Goal:** give two nodes ed25519 identities, dial one from the other over iroh QUIC, and mount the remote namespace as local files. **Route:** [the missing half of 9P](../concepts/missing-half-of-9p) → [per-process namespaces](../concepts/per-process-namespaces) → recipe [Mount a remote peer](../recipes/02-mount-remote-peer).

```sh
# Node B exports a directory over iroh QUIC (the native wire) and prints
# its verified address as an iroh:// ticket.
wanix-rust mesh-serve --root "$ROOT_B" --key "$NODE_B_KEY" \
    --addr 127.0.0.1:5680 --wanix-services

# Node A imports B's namespace as local files.
wanix-rust mount-ls "$NODE_B" work
wanix-rust mount-cat "$NODE_B" work/dataset.txt
```

The peer's address *is* its ed25519 public key — no DNS, no IP-as-identity. Attach is default-deny and capability-gated. Three honest caveats the recipe owns: `--wanix-services` is refused on the public endpoint (it is remote code execution); today the mount lands at `/n/remote`, not yet the per-peer `/n/<peer-id>` the design promises; and the [`#cpu`](../devices/cpu) "run a job where the data lives" half is dial-only right now — `wanix-rust cpu` ships, but no CLI serve mode binds the `#cpu` acceptor yet, so there is no shipped server side to dial.

---

## Flow: Build an HTTP app backed by #kv — *for Maya in builder mode*

**Goal:** a one-file qjs handler whose state lives in `#kv`, not in the handler. **Route:** [everything-is-a-file](../concepts/everything-is-a-file) → `#kv` → recipe [A tiny HTTP app with #kv](../recipes/04-tiny-http-app-with-kv).

The handler reads `#kv/http-counter`, increments, writes it back. Nothing about the state lives in code:

```sh
mkdir -p project-root/apps        # the handler lives under the served root
# ...write the recipe's handler to project-root/apps/counter.js...
wanix-rust serve --wanix-services --listen 127.0.0.1:7654 ./project-root
curl -s http://127.0.0.1:7654/.wanix/app/counter   # counter=1, then 2, then 3 ...
```

Each request spawns a fresh qjs task; the count climbs anyway because the `KvDevice` lives in the serve process. The route is loopback-only and requires `--wanix-services`. `#kv` is an in-memory tier; for durability across restarts, freeze it into a capsule.

---

## Flow: Compose volumes and tools across the mesh — *for Devraj*

**Goal:** serve a data volume and tools as independent mesh resources, mount them into one shell namespace, and pipe a remote file through a remote tool back into the remote volume. **Route:** [compose volumes and tools](../learn/compose-volumes-and-tools) → [jobs are files](../concepts/jobs-are-files) → recipe [06](../recipes/06-compose-volume-and-tools), then wrap *your own host programs* from a `tools.toml` — recipe [08](../recipes/08-real-tools-with-config) (live `events --follow`, real timeouts, real aborts).

---

## Flow: Build a chatroom (a guest-defined room) — *for Maya going social*

**Goal:** serve a small qjs program as a mesh-mounted chatroom: transport-verified attribution, display nicks, live delivery with `mount-cat --follow`, durable restart. **Route:** [build a chatroom](../learn/build-a-chatroom) → [guest-defined resources](../concepts/guest-defined-resources) → recipe [07](../recipes/07-chatroom-over-the-mesh).

---

## Flow: Publish apps via the web door — *for Maya shipping it*

**Goal:** give that room (or any file-shaped app) a browser audience: named origins by `Host` routing, SSE off a never-EOF stream, honest 503s. **Route:** [publish apps via the web door](../learn/publish-apps-via-web-door) → recipe [09](../recipes/09-web-door-gateway). The honest boundary up front: the gateway is ONE principal to the room — every web user posts as the gateway's key (per-user web identity is named follow-up work).

---

## Flow: Add a service device — *for Theo*

**Goal:** ship a new device that is a plain `FileSystem`, so it imports across the mesh for free. **Route:** [everything-is-a-file](../concepts/everything-is-a-file) → study `#pipe`/`#kv` as the simplest models → the [contribute](../reference/contributor-landing) reference.

A service device is just a crate that implements `FileSystem` over `wanix-fs` — `wanix-kv`, `wanix-pipe`, `wanix-plumb`, and `wanix-cas` are the worked examples. Because the mesh carries 9P and every device is a filesystem, the moment your device binds into the served namespace it is reachable at `/n/A/#yourdevice/...` with no extra work. Keep tokio and iroh *out* of it: `wanix-mesh` is the only async edge.

---

## Flow: Add a task driver or a transport — *for Theo*

**Goal:** add a third WASI task runtime, or a new wire transport, without breaking the layering. **Route:** the `wasm` and `qjs` task drivers → [the 9P contract](../concepts/the-9p-contract) → ADR 0002 (task runtime) and ADR 0004 (9P contract).

A task driver registers `check`/`start` with `#task` (see how `wanix-wasm`'s `WasmTaskDriver` claims `.wasm`); a transport reuses the synchronous 9P core in `wanix-9p`/`wanix-protocol` rather than reinventing framing. The dependency direction is law: `wanix-task` must never depend on a runtime engine, and core filesystem/namespace crates stay free of Wasmtime.

---

## Flow: The agent on your files — *for Kemi*

**Goal:** drive an LLM session that *is* a set of files — prompt, events, approvals — over a confined namespace. **Route:** [everything-is-a-file](../concepts/everything-is-a-file) → `#agent` → recipe [Repair a broken qjs program](../recipes/01-repair-broken-qjs) → recipe [Two agents collaborate](../recipes/05-two-agents-collaborate).

```sh
A=$(cat '#agent/new')
echo "Rename foo() to bar() across the project. Plan first." > "#agent/$A/prompt"
cat "#agent/$A/events"      # watch it think; approvals appear under pending
```

(`#agent/...` are paths *inside a served Wanix namespace*, not your host shell — drive them with `wanix-rust mount-cat`/`mount-write` as the flow shows.) Approvals are files; `ctl` takes `approve`/`deny`/`close`; one agent delegates to another by blocking on its `reply`. The served `#agent` uses a deterministic `FakeEngine` — real codex is local-trust only.

---

## Flow: The Plan 9 ideas tour — *for Aria*

**Goal:** see *why* the three ideas compose, not just how to type them. **Route:** [everything-is-a-file](../concepts/everything-is-a-file) → [per-process namespaces](../concepts/per-process-namespaces) → [the missing half of 9P](../concepts/missing-half-of-9p), then any device page as a worked example.

The arc: if everything is a file, and each process composes its own namespace, then importing a *remote* namespace is the same operation as binding a local one — that is the missing half Plan 9 always implied and the mesh finally ships. Heritage worn lightly: you do not need to have used Plan 9 to follow it.

---

## Flow: Contribute to the Wanix core — *for Priya*

**Goal:** land a change that passes review on the first pass. **Route:** the crate map in [AGENTS.md](../reference/crate-map-and-layering) → the [ADR index](../reference/adr-index) → `just check`.

Read the dependency direction and module-line guardrails (250–350 lines), pick the ADR that governs your area, and run the required gate before every cycle commit:

```sh
just check
```

---

## Prerequisite badges and estimated times

Each flow header carries a badge so you can pick by appetite:

| Flow | Prereqs | Est. time |
| --- | --- | --- |
| JS outside Chrome | Rust toolchain (optional cockpit step needs Go) | ~10 min |
| Wire a mesh | one flow done; two terminals | ~20 min |
| HTTP app with #kv | JS-outside-Chrome flow | ~15 min |
| Compose volumes & tools | JS-outside-Chrome flow; two terminals | ~15 min |
| Build a chatroom | JS-outside-Chrome flow | ~15 min |
| Publish via the web door | the chatroom flow | ~15 min |
| Add a service device | Rust; read `#kv`/`#pipe` | ~1 hr |
| Add a driver/transport | the service-device flow | ~2 hr |
| Agent on your files | JS-outside-Chrome flow | ~15 min |
| Plan 9 ideas tour | none — start here cold | ~20 min reading |
| Contribute to core | Rust; `just check` green | ongoing |

"No prereqs" flows are safe cold starts for a first-day reader; "one flow done" flows assume you can already run a `qjs` task.

## Status / honest limits

Each flow links the concept and recipe pages that own its caveats in full; this index restates the ones that are easy to over-read across the whole catalog:

- **The agent is a FakeEngine on the served path.** The served `#agent` device runs a deterministic `FakeEngine`, not a live LLM. Real codex is local-trust only (the CLI `wanix agent` path against a confined world).
- **`#kv` is in-memory.** State persists only while the serve process is alive. The HTTP-app flow's counter survives between calls because `KvDevice` lives in that process — not across restarts. Freeze a world into a capsule for durability.
- **The serve 9P WebSocket is single-frame-at-a-time.** A blocking read (e.g. `#plumb/<topic>/recv`) cannot be interleaved with a write on the same connection; live pub/sub needs a second connection.
- **`#cpu` exec and `--wanix-services` are local-trust-only.** `--wanix-services` is remote code execution and is refused on the public mesh endpoint. Treat the mesh and cpu flows as trusted-peer demos, not multi-user auth.
- **The mesh mounts at `/n/remote`.** Today an imported peer lands at `/n/remote`, not yet the per-peer `/n/<peer-id>` the design describes.
- **`#cpu` is dial-only in the shipped CLI.** `wanix-rust cpu` exists and the acceptor is proven in `wanix-mesh` tests, but no CLI serve mode binds the `#cpu` acceptor yet — there is no shipped server side for a cross-node cpu job to dial.

## See also / next

- Concepts: [everything is a file](../concepts/everything-is-a-file) · [per-process namespaces](../concepts/per-process-namespaces) · [the missing half of 9P](../concepts/missing-half-of-9p)
- Devices: [`#kv`](../devices/kv) · [`#agent`](../devices/agent) · [`#cpu`](../devices/cpu) · [`#term`](../devices/term)
- Recipes: start with [scaffold a project](../recipes/00-scaffold-a-project) — every flow above lands on one of the numbered recipes.
- Going deeper: [the missing half of 9P](../concepts/missing-half-of-9p) and the [ADR index](../reference/adr-index).
