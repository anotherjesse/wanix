---
title: /.wanix/app/<name> HTTP Route
slug: concepts/http-app-route
pageType: concept
oneLiner: A loopback HTTP route that resolves apps/<name>.{js,wasm}, allocates a #task, binds fd/1 and fd/2 to trace files, runs the program, and returns its stdout with X-Wanix-Task-Id tracing headers.
audience: [developer]
tags: [shipped, local-trust-only, caveat, cli, services]
sourceRefs:
  - crates/wanix-cli/src/serve/http/app.rs:49-156
  - crates/wanix-cli/src/serve/http/app.rs:158-273
  - crates/wanix-cli/src/serve/http/agent.rs:14-44
  - workbench/src/web/http-app-demo.ts:15-135
seeAlso:
  - devices/kv
  - concepts/kv-smallest-database
  - devices/task
  - concepts/loopback-only-handoffs
  - concepts/serve-composition-surface
  - concepts/wanix-services-device-set
prerequisites:
  - concepts/wanix-services-device-set
  - devices/kv
  - devices/task
usedInFlows:
  - {flow: http-app-with-kv, step: 3}
honestLimits:
  - The route is loopback-only and gated behind serve --wanix-services; remote peers get 403/404.
  - A request runs a brand-new #task per call and reads its exit synchronously; there is no streaming response and no per-task CPU/memory limit.
  - #kv is the only thing that survives between requests, and #kv is in-memory; persistence requires freezing to a capsule.
canonicalCaveatFor: []
---

# /.wanix/app/<name> HTTP Route

A loopback HTTP route that resolves `apps/<name>.{js,wasm}`, allocates a `#task`, binds fd/1 and fd/2 to trace files, runs the program, and returns its stdout with `X-Wanix-Task-Id` tracing headers.

## What & why

Once `serve` exports a namespace with `#task` and `#kv` in it, you already have everything a tiny web app needs: a place to keep a program, a way to run it, and a place to keep state. The `/.wanix/app/<name>` route stitches those together over plain HTTP. A `GET` to `/.wanix/app/counter` does not invoke a framework — it opens a file under `apps/`, drives the `#task` device through its `ctl` file, and hands you back what the program printed. The whole request is file plumbing, which means the request handler is small, the behaviour is inspectable, and the same `#kv` key the app touches is readable from any other client on the namespace.

## Show: a GET that runs a program

Bring up a services-enabled serve, drop a JavaScript file under `apps/`, and curl the route:

```sh
cargo build --package wanix-cli
alias wanix-rust='./target/debug/wanix-rust'

mkdir -p root/apps
cat > root/apps/hello.js <<'EOF'
import * as std from "qjs:std";
std.out.puts("wanix http app " + std.getenv("WANIX_HTTP_APP") +
             " saw " + std.getenv("WANIX_HTTP_TARGET") + "\n");
EOF

wanix-rust serve --wanix-services --p9-root root &
curl -i http://127.0.0.1:8080/.wanix/app/hello
```

```
HTTP/1.1 200 OK
Content-Type: text/plain; charset=utf-8
X-Wanix-Task-Id: 3
X-Wanix-Stdout-Path: /.wanix/http/3.out
X-Wanix-Stderr-Path: /.wanix/http/3.err

wanix http app hello saw /.wanix/app/hello
```

No process was forked, no port was bound by the app, and no handler was registered. The route name `hello` was resolved to a file, a task ran it, and its standard output became the response body.

## The resolution path, one step at a time

The handler is `app_response` and `run_app` in `crates/wanix-cli/src/serve/http/app.rs`. Reading them top to bottom is the whole contract:

1. **Resolve the program.** `app_program` (`app.rs:233-264`) looks for `apps/<name>.js` first, then `apps/<name>.wasm`. The first that exists as a regular file wins and decides the task kind; if neither exists the route returns `404` with `wanix app program apps/<name>.js or apps/<name>.wasm was not found`. The name itself is validated to ASCII alphanumerics plus `-`/`_` (`valid_route_name`, `app.rs:226-231`), so the route can never reach outside `apps/`.
2. **Allocate a task.** A `.js` program reads `#task/new/qjs`; a `.wasm` program reads `#task/new/wasm` (`AppProgram::task_new_path`, `app.rs:189-196`). The read returns a fresh task id (`run_app`, `app.rs:108`).
3. **Bind fd/1 and fd/2 to trace files.** `prepare_trace_dir` makes `.wanix/http/` (`app.rs:266-269`), then `run_app` truncates `<task-id>.out` and `<task-id>.err` and writes two binds through the task's `ctl` file: `bind /.wanix/http/<id>.out fd/1` and `bind .../<id>.err fd/2` (`app.rs:109-133`). The program's stdout and stderr now land in real, readable files in the namespace.
4. **Set cmd, dir, and env.** The handler writes the task's `cmd` (the program filename plus the request target as argv), sets `dir` to `apps`, and sets `env` to `WANIX_HTTP_APP=<name>` and `WANIX_HTTP_TARGET=<request-path>` (`run_app`, `app.rs:114-123`; `http_env`, `app.rs:271-273`). That is how the app above read its own name and the path it was hit at.
5. **Start and wait.** A final `start` is written to `ctl`, then the handler reads the task's `exit` file — a blocking read that returns when the task finishes (`app.rs:134-136`). On exit `0` it reads back the stdout trace file and returns it as the body; on any other exit it returns `500` with both captured streams inline (`app.rs:139-156`).

The Plan 9 term for what just happened: the route used `#task` purely as a *device* — `new`, `cmd`, `dir`, `env`, `ctl`, `exit` are all files — and used a `bind` to redirect the task's standard descriptors to ordinary files. There is no app server. There is a filesystem and a control file.

## The tracing headers

Every successful (and every task-failure) response carries three headers, attached by `app_trace_headers` (`app.rs:158-168`):

- `X-Wanix-Task-Id` — the `#task/<id>` that served the request.
- `X-Wanix-Stdout-Path` — the rooted path of the captured stdout, e.g. `/.wanix/http/3.out`.
- `X-Wanix-Stderr-Path` — the captured stderr path.

These are not decoration. They make a request *replayable from the filesystem*: the cockpit reads them straight off the `Response` (`workbench/src/web/http-app-demo.ts:681-691`) and links the trace files into its preview, so a developer can open `/.wanix/http/<id>.err` and see exactly what the failing run wrote.

## State that outlives the request: #kv

A new task is allocated per request, so nothing in the task survives the call. Durable state has to live in a device. The shipped counter demo puts it in `#kv` (`workbench/src/web/http-app-demo.ts:112-135`):

```js
import * as std from "qjs:std";
const key = "#kv/http-counter";          // resolved from the namespace root, not cwd
let count = parseInt(std.loadFile(key) || "0", 10) || 0;
count += 1;
const f = std.open(key, "w"); f.puts(String(count)); f.close();
std.out.puts("counter=" + count + "\n");
```

Each `GET /.wanix/app/counter` increments and returns the count. The key is `#kv/http-counter` (`http-app-demo.ts:15`), and because a WASI guest resolves any `#name` device from the namespace root regardless of its `dir`, the task reaches `#kv` even though its working directory is `apps`. The count is shared across every invocation because `#kv` is one device, not per-task state.

## Loopback-only and services-gated

Two guards run before anything executes (`app_response`, `app.rs:49-66`). If `serve` was not started with `--wanix-services` the route is `404`; if the peer's IP is not loopback the route is `403` (`wanix app routes are available only to loopback clients`). This is deliberate: the route runs a `#task`, and exec devices are a local-trust surface, so the route never exposes arbitrary code execution to a non-loopback peer. The discovery document advertises the route as `wanix-http-app.v1` with `"scope":"loopback"` only when services are on (`app_route_json`, `app.rs:35-47`).

`POST /agent` is the structurally identical sibling for the agent device: loopback-only, services-gated, allocate-drive-stream against `#agent` instead of `#task` (`crates/wanix-cli/src/serve/http/agent.rs:14-44`). If the app route lags on a given branch, the agent endpoint is the reference for the same pattern.

## See also

- [#kv device](/devices/kv) and [#kv, the smallest database](/concepts/kv-smallest-database) — where request-surviving state lives.
- [#task device](/devices/task) — the `new`/`cmd`/`dir`/`env`/`ctl`/`exit` files this route drives.
- [Loopback-only handoffs](/concepts/loopback-only-handoffs) — why local-trust surfaces refuse non-loopback peers.
- [The serve composition surface](/concepts/serve-composition-surface) and [the --wanix-services device set](/concepts/wanix-services-device-set) — what has to be running for this route to exist.
- Recipe: [tiny HTTP app with #kv](/recipes/04-tiny-http-app-with-kv); flow: [HTTP app with #kv](/learn/http-app-with-kv).

## Status / honest limits

- **Loopback-only, services-gated.** The route requires `serve --wanix-services` and refuses any non-loopback peer (`app.rs:55-66`). It is not an internet-facing app server.
- **One task per request, read synchronously.** Each call allocates a fresh `#task`, starts it, and blocks on its `exit` file (`app.rs:104-156`). There is no streaming response and no per-task CPU/memory limit; this is cheap isolation, not a sandbox safe for arbitrary untrusted code.
- **Only `#kv` persists across requests, and `#kv` is in-memory.** State survives between calls only because it lives in the `#kv` device, whose contents last only as long as the serve process. To keep state, freeze the world to a [capsule](/concepts/wanix-capsule).
- **A shipped sibling, if you need a reference.** `POST /agent` (`agent.rs:14-44`) follows the same allocate-drive-read shape as this route.
