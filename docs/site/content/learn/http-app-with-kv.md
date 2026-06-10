---
title: Build an HTTP app backed by #kv
slug: learn/http-app-with-kv
pageType: flow
oneLiner: Scaffold a project, serve it at /.wanix/app/<name>, and back a counter with the #kv device.
audience: [newcomer, developer]
tags: [shipped, cli, mesh, caveat, loopback-only]
sourceRefs:
  - crates/wanix-cli/src/serve/http/app.rs:22-101
  - crates/wanix-cli/src/serve/http/app.rs:104-156
  - crates/wanix-cli/src/serve/http/routes.rs:14-25
  - crates/wanix-cli/src/serve/roots.rs:185-212
  - crates/wanix-kv/src/lib.rs:1-21
  - crates/wanix-cli/src/new.rs:101-176
seeAlso:
  - concepts/http-app-route
  - devices/kv
  - concepts/kv-smallest-database
  - concepts/wanix-services-device-set
  - concepts/single-frame-serve-caveat
  - recipes/04-tiny-http-app-with-kv
prerequisites:
  - learn/js-outside-chrome
usedInFlows: []
honestLimits:
  - "#kv is in-memory: the counter lives only as long as the serve process. Freeze a capsule to persist it."
  - "The /.wanix/app/<name> route is loopback-only and refuses unless serve runs with --wanix-services."
  - "Each request runs a fresh qjs task; serve handles one 9P frame at a time per connection, so the handler must not block."
---

# Build an HTTP app backed by #kv

Scaffold a project, serve it at `/.wanix/app/<name>`, and back a counter with the `#kv` device.

You already ran JavaScript outside Chrome as a `qjs` task. This flow keeps the same one-file handler but moves the state *out* of the handler and into a device. The handler reads a number, adds one, writes it back, and prints it. Nothing in the code remembers the count between requests — `#kv` does, because the device lives for the whole lifetime of the `serve` process. The result is the smallest possible "stateful web app" in Wanix, and the state node is just a file you can `cat`.

## Step 0 — build once, scaffold a project

If you have not built the CLI yet ([build & install](/reference/build-and-install)):

```sh
cargo build --locked --package wanix-cli
alias wanix-rust='./target/debug/wanix-rust'
```

Scaffold a starter project. `wanix new --js counter` writes a `counter/` directory with `main.js`, the guest SDK under `lib/wanix/`, and a `tsconfig.json` (`crates/wanix-cli/src/new.rs:101`). You will add one file of your own:

```sh
wanix-rust new --js counter
```

Now create the `apps/` directory inside the project (`mkdir -p counter/apps`) and write the handler below to `counter/apps/counter.js`. The route looks for the program under `apps/<name>.js` relative to the served root, so the file name is the route name.

```js
import * as std from "qjs:std";

const target = std.getenv("WANIX_HTTP_TARGET");
const key = "#kv/http-counter";
let count = 0;

const previous = std.loadFile(key);
if (previous) {
  const parsed = parseInt(previous, 10);
  if (!isNaN(parsed)) count = parsed;
}

count += 1;
const f = std.open(key, "w");
f.puts(String(count));
f.close();

std.out.puts("counter=" + count + "\n");
std.out.puts("target=" + target + "\n");
```

Note the key: `#kv/http-counter`. `#kv` is addressed as `#kv/<key>` from the namespace root, not from the task's cwd, so the handler reaches the device no matter where it runs.

## Step 1 — serve it with the device set bound

```sh
wanix-rust serve --wanix-services --listen 127.0.0.1:7654 ./counter
```

`--wanix-services` is the switch that binds the device set — `#task`, `#term`, `#kv`, `#pipe`, `#plumb`, `#cas`, `#agent`, `#sites` — into the served namespace (`crates/wanix-cli/src/serve/roots.rs:207` binds `KvDevice` at `#kv`). Without it, `#kv` does not exist and the app route refuses the request (`crates/wanix-cli/src/serve/http/app.rs:55`). The positional `./counter` becomes the `.` of the served namespace, so `apps/counter.js` and `#kv/http-counter` sit under the same root.

## Step 2 — drive the counter through the running serve

The route is wired and shipped on this branch (`crates/wanix-cli/src/serve/http/routes.rs:22`). Hit it with `curl` from the same machine:

```sh
curl -s "http://127.0.0.1:7654/.wanix/app/counter"
# counter=1
# target=/.wanix/app/counter
curl -s "http://127.0.0.1:7654/.wanix/app/counter"
# counter=2
curl -s "http://127.0.0.1:7654/.wanix/app/counter"
# counter=3
```

What the route does for each request, in order (`crates/wanix-cli/src/serve/http/app.rs:104`): allocate a task through `#task/new/qjs`, bind the task's stdout and stderr to trace files under `.wanix/http/`, set `WANIX_HTTP_TARGET` in the task environment from the request line (`app.rs:271`), `start` the task, then read the task's `exit` and return its stdout as the `text/plain` response body. The response also carries `X-Wanix-Task-Id`, `X-Wanix-Stdout-Path`, and `X-Wanix-Stderr-Path` headers so you can open the trace files in the cockpit (`app.rs:158`). The Plan 9 framing: an HTTP request is just another writer driving the `#task` and `#kv` files — the route is an adapter over the one namespace, not a second runtime.

The cockpit drives the same route. Its HTTP-apps view POSTs to `/.wanix/app/counter`, renders the response inline, and links `#kv/http-counter` as a live state node you can open as a file. The count it shows is the count `curl` sees, because both go through the one `KvDevice` in the serve process.

## Step 3 — why the state survives (and when it does not)

Run the handler three times and the count climbs because the `KvDevice` is a single `BTreeMap<String, Vec<u8>>` (`crates/wanix-kv/src/lib.rs:21`) bound once into the served namespace. Each request gets a fresh, stateless qjs task; only the device remembers. That is the whole point — the smallest real database inside Wanix is a filesystem, so the handler, the cockpit, and a remote mesh peer all treat the count as the same file.

The boundary is just as important: `#kv` is an in-memory tier (`crates/wanix-kv/src/lib.rs:6`). Stop `serve` and the count is gone. There is no fsync, no journal, no on-disk file under `apps/`. When you want the value to survive a restart, freeze the world into a `.wcap` capsule (see [`#cas`](/devices/cas) and [the capsule concept](/concepts/wanix-capsule)) rather than blurring the in-memory line by writing host files.

## See also

- [The `/.wanix/app/<name>` route](/concepts/http-app-route) — the route contract this flow exercises.
- [`#kv`](/devices/kv) and [`#kv` is the smallest database](/concepts/kv-smallest-database) — the device and why it is a filesystem.
- [The `--wanix-services` device set](/concepts/wanix-services-device-set) — what `--wanix-services` binds.
- [The single-frame serve caveat](/concepts/single-frame-serve-caveat) — why a handler must not block.
- [Recipe 04: a tiny HTTP app with `#kv`](/recipes/04-tiny-http-app-with-kv) — the same handler, step by step.
- Prerequisite flow: [JavaScript outside Chrome](/learn/js-outside-chrome).

## Status / honest limits

- The `/.wanix/app/<name>` route is **shipped** on this branch (`crates/wanix-cli/src/serve/http/app.rs:22`, registered at `routes.rs:22`). It is **loopback-only** — non-loopback peers get `403 Forbidden` (`app.rs:61`) — and it returns `404` unless `serve` ran with `--wanix-services` (`app.rs:55`).
- `#kv` is **in-memory**. The counter lives only as long as the `serve` process; restart and it resets to 1. Persist by freezing a capsule, not by writing files under `apps/`.
- Each request spawns a fresh qjs task and the response is its stdout. `serve` handles one 9P frame at a time per connection, so the handler must complete without blocking on a long-lived stream (e.g. a `#plumb` recv) on the same connection.
- The route runs guest code through the `#task` exec plane, which is **local-trust only**. It is loopback-gated precisely because it executes code; do not expose it to untrusted peers.
