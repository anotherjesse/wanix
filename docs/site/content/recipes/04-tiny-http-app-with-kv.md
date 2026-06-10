---
title: Recipe 04 — A Tiny HTTP App with #kv-Backed Counter State
slug: recipes/04-tiny-http-app-with-kv
pageType: use-case
oneLiner: A single-file qjs handler at apps/counter.js increments a per-request counter held in #kv — nothing about the state lives in the handler, only in the device.
audience: [developer, newcomer]
tags: [recipe, cli, shipped, local-trust-only, caveat, kv, http-app]
sourceRefs:
  - crates/wanix-kv/src/lib.rs:1-177
  - crates/wanix-cli/src/serve/http/app.rs:22-156
  - crates/wanix-cli/src/serve/http/routes.rs:9-22
  - crates/wanix-cli/src/serve/roots.rs:185-212
  - workbench/src/web/http-app-demo.ts:9-135
seeAlso:
  - devices/kv
  - concepts/kv-smallest-database
  - concepts/http-app-route
  - concepts/devices-import-for-free
  - recipes/03-freeze-world-to-capsule
prerequisites:
  - devices/kv
  - concepts/kv-smallest-database
  - concepts/http-app-route
usedInFlows:
  - {flow: http-app-with-kv, step: 3}
honestLimits:
  - "The #kv store is in-memory: state lives only as long as the serve process. Freeze to a capsule to persist."
  - "The /.wanix/app/<name> route is loopback-only and requires --wanix-services; it is not exposed to remote or untrusted peers."
  - "Each request spawns a fresh qjs task (#task is local-trust only, no hard CPU/memory limits); cheap isolation, not a sandbox for arbitrary untrusted code."
canonicalCaveatFor: []
---

# Recipe 04 — A Tiny HTTP App with #kv-Backed Counter State

A single-file qjs handler at `apps/counter.js` increments a per-request counter held in `#kv` — nothing about the state lives in the handler, only in the device.

**What & why.** Most "hello, counter" tutorials wire a database, a connection pool, and a schema before the number even goes up. Wanix asks a smaller question: what if the counter's state were just a file, and the handler never owned it? You write one qjs file that reads a key, adds one, writes it back, and prints the result. You run `wanix serve --wanix-services`, which binds the `#kv` device into the served namespace. You `curl` the route and watch the number climb across requests — even though the handler holds nothing between calls. The state lives in `#kv`; the handler is pure plumbing. That separation is the whole point: state has one home, the device is a `FileSystem`, and "a tiny web app" turns out to be "a file you read and a file you write."

## The handler: read, increment, write

Create a project root with an `apps/` directory inside it (`mkdir -p project-root/apps` — the serve step below points at `./project-root`, which must exist) and write this to `project-root/apps/counter.js`. It reads `#kv/http-counter` (an empty or missing key counts as `0`), increments, writes it back, and prints the new value as the response body. The route's runner sets `WANIX_HTTP_TARGET` in the task environment (`crates/wanix-cli/src/serve/http/app.rs:123`), so the handler can see the request path.

```js
import * as std from "qjs:std";

const target = std.getenv("WANIX_HTTP_TARGET");
// #kv is a namespace binding addressed as "#kv/<key>", resolved from the
// namespace root (not the task cwd). The key is shared across every qjs
// task invocation, so the count persists between HTTP requests.
const KEY = "#kv/http-counter";

let count = 0;
const previous = std.loadFile(KEY);
if (previous) {
  const parsed = parseInt(previous, 10);
  if (!isNaN(parsed)) count = parsed;
}

count += 1;
const f = std.open(KEY, "w");
f.puts(String(count));
f.close();

std.out.puts("counter=" + count + "\n");
std.out.puts("target=" + target + "\n");
```

Two things to notice. First, the handler is *stateless* between invocations — the only place a number survives a task exit is the `KvDevice` map (`crates/wanix-kv/src/lib.rs:21`, a `BTreeMap<String, Vec<u8>>` behind one `RwLock`). Second, there is no commit dance: opening for write calls `ensure_key` so a stat between open and close succeeds (`lib.rs:93-100`), and `KvWriteFile` replaces the value under one write lock on close. A reader gets either the old value or the new one, never a torn intermediate.

## Run serve with #kv bound

```sh
cargo build --locked --package wanix-cli       # see /reference/build-and-install
alias wanix='./target/debug/wanix'

wanix serve --wanix-services --listen 127.0.0.1:7654 ./project-root
```

`--wanix-services` is the single flag that turns the device set on. Without it, the services namespace path never runs, and `#kv` is not bound. With it, `roots.rs:206-211` runs `namespace.bind(Arc::new(KvDevice::new()), ".", "#kv", ...)`, and the served root advertises the inspectable device set `#task`, `#term`, `#kv`, `#pipe`, `#plumb`, `#cas`, `#agent`, `#sites` (`crates/wanix-cli/src/serve/roots.rs:187`). The positional `./project-root` becomes the `.` of the served namespace, so `./project-root/apps/counter.js` shows up at `apps/counter.js` and `#kv/http-counter` sits next to it under the same root.

There is one served namespace and many adapters bind against it: direct 9P, WebSocket 9P, the HTTP app route, and `POST /agent` all share the same filesystem. There is no per-route filesystem.

## Invoke it: GET /.wanix/app/counter

The HTTP-app route is shipped on this branch. A `GET` to `/.wanix/app/<name>` resolves `apps/<name>.js` (or `apps/<name>.wasm`), spawns a fresh task through `#task/new/qjs`, binds its stdout and stderr to trace files, runs it, and returns stdout as the body (`crates/wanix-cli/src/serve/http/app.rs:104-156`; the route wires in at `routes.rs:22`).

```sh
curl -sS http://127.0.0.1:7654/.wanix/app/counter
# counter=1
# target=/.wanix/app/counter

curl -sS http://127.0.0.1:7654/.wanix/app/counter
# counter=2
# target=/.wanix/app/counter

curl -sS http://127.0.0.1:7654/.wanix/app/counter
# counter=3
```

The number climbs because every request reads and writes the *same* `KvDevice` singleton living inside the serve process — the bind at `roots.rs:206-211` keeps that map alive for the lifetime of `serve`. Each request also returns trace headers (`X-Wanix-Task-Id`, `X-Wanix-Stdout-Path`, `X-Wanix-Stderr-Path`, see `app.rs:158-167`) pointing at `.wanix/http/<task-id>.out` so you can read exactly what the handler printed.

Two access boundaries are enforced in the route, not by convention. The handler refuses without services: a request to the route without `--wanix-services` returns `404 wanix app routes require --wanix-services` (`app.rs:55-59`). And it is loopback-only: any non-loopback peer gets `403` (`app.rs:61-66`). The route advertises itself in discovery with `"scope":"loopback"` (`app.rs:35-47`).

## Why #kv (not files under apps/)

You could write the count to `apps/counter.count.txt` instead. Three things would degrade. **Write churn**: every increment becomes an open/truncate/write/close on the host filesystem, plus an inotify storm if the cockpit is watching `apps/`. With `#kv` it is one locked `BTreeMap` mutation. **Durability clarity**: `#kv` states its boundary plainly — this is in-process state; on serve restart it is gone (`crates/wanix-kv/src/lib.rs:6-7`). A host file *looks* durable but only commits on whatever sync semantics the host gives you. When you actually want persistence, the answer is to freeze the interesting keys into a capsule (see [Recipe 03](/recipes/03-freeze-world-to-capsule)), not to blur the `#kv` line. **Mesh transparency**: `#kv` is a plain `FileSystem` (`lib.rs:120` `impl FileSystem for KvDevice`), so a peer that mounts the device over the mesh reads the same value the local handler writes — devices [import for free](/concepts/devices-import-for-free). A bag of host files does not get that.

Put bluntly: `#kv` is the smallest real database inside Wanix precisely *because* it is a filesystem and not a database. The handler treats it as a file, the mesh treats it as a file, the cockpit treats it as a file.

## The cockpit's role: catalog and preview

The browser cockpit is the catalog and preview surface for this recipe. Its built-in `counter` template (`workbench/src/web/http-app-demo.ts:112-135`) is the same handler shown above, keyed on `#kv/http-counter` (`COUNTER_STATE_KEY = "http-counter"`, line 15). The cockpit fetches the route, writes a `counter.response.txt` preview, publishes a `.wanix/http-apps.md` catalog of discovered `apps/<name>.js|.wasm` programs, and registers a data-store entry pointing at the `#kv` state node — so opening it in the editor reads the *live* device value as a file, with the route labelled `/.wanix/app/counter` (line 92). Code node is `apps/counter.js`; state node is `#kv/http-counter`; the cockpit never confuses the two.

## See also

- [#kv device](/devices/kv) — the key/value service this recipe writes into.
- [#kv is the smallest database](/concepts/kv-smallest-database) — why a `BTreeMap` behind a `FileSystem` is the right primitive here.
- [The HTTP-app route](/concepts/http-app-route) — the `/.wanix/app/<name>` contract end to end.
- [Devices import for free](/concepts/devices-import-for-free) — why the same `#kv` value reads identically across the mesh.
- [Recipe 03 — Freeze a world to a capsule](/recipes/03-freeze-world-to-capsule) — how to make this counter survive a restart.
- [HTTP app with #kv flow](/learn/http-app-with-kv) — the guided walkthrough this recipe anchors.

## Status / honest limits

These are engineering boundaries, stated once.

- **`#kv` is in-memory.** The store is a `BTreeMap` in the serve process (`crates/wanix-kv/src/lib.rs:21`). When `serve` exits, the counter is gone. Durable and content-addressed backing is a follow-up; persist by freezing keys into a capsule.
- **The route is loopback-only and services-gated.** `/.wanix/app/<name>` returns `403` to non-loopback peers and `404` without `--wanix-services` (`crates/wanix-cli/src/serve/http/app.rs:55-66`). It is not a public endpoint.
- **Exec is local-trust.** Each request spawns a fresh qjs task through `#task`, which is local-trust only with no hard CPU or memory limits. This is cheap, scalable isolation, not a sandbox safe for arbitrary untrusted code.
- **One handler invocation per request.** The route allocates a task, runs it to exit, and returns stdout (`app.rs:104-156`); there is no long-lived process or per-route filesystem.
