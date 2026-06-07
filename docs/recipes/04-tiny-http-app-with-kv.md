# Recipe 04 — A tiny HTTP app with `#kv`-backed counter state

Goal: a single-file qjs handler at `apps/counter.js` that increments a
per-request counter held in `#kv/counter`. The request side is
`wanix serve --wanix-services` exposing the same `#kv` device every other
Wanix surface already binds; the handler reads, increments, and writes one key
per call. Nothing about the state lives in the handler — only in `#kv`.

What this recipe is honest about up front:

- `#kv` is wired: `crates/wanix-kv/src/lib.rs` and the bind in
  `crates/wanix-cli/src/serve/roots.rs` line 129
  (`namespace.bind(Arc::new(KvDevice::new()), ".", "#kv", BindOptions::default())`)
  make `#kv/<key>` reachable as files inside the served namespace.
- `wanix serve --wanix-services` is the verbatim CLI pattern that turns `#kv`
  on. Parser lives in `crates/wanix-cli/src/serve/command.rs` lines 1–169 —
  there is one flag (`--wanix-services`, line 82) and no per-route registration
  knob.
- An HTTP route that *evaluates a qjs handler on each request* is not yet
  wired on the `cpu` branch. The only built-in qjs-shaped HTTP endpoint today
  is `POST /agent` (`crates/wanix-cli/src/serve/http/agent.rs` lines 14–43),
  which is *loopback-only* and is structurally the same idea — body in,
  filesystem-backed work, response out. The "curl the endpoint" section uses
  that same shape so the recipe stays grounded in the real codepath.
- The cockpit (`workbench/src/web/http-app-demo.ts` on `main`) ships the
  catalog/preview UI for exactly this app — the `counter` is one of its
  built-in templates. That code is what materializes the route-catalog story
  even though the `wanix serve` side of the HTTP-app route isn't on cpu yet.

[kv]: ../../crates/wanix-kv/src/lib.rs
[roots]: ../../crates/wanix-cli/src/serve/roots.rs
[serve-cmd]: ../../crates/wanix-cli/src/serve/command.rs
[agent-http]: ../../crates/wanix-cli/src/serve/http/agent.rs

## 1. The handler

Drop this in `apps/counter.js`. It reads `#kv/counter` (treating an empty key
as `0`), increments, writes it back, and prints the new value as the response
body.

```js
import * as os from "qjs:os";
import * as std from "qjs:std";

const KEY = "#kv/counter";

function readCount() {
  // #kv values are plain files: open RDONLY, read, parse.
  // KvDevice::open + KvReadFile::read in crates/wanix-kv/src/lib.rs
  // line 121 and crates/wanix-kv/src/files.rs handle the byte semantics.
  const fd = os.open(KEY, os.O_RDONLY);
  if (fd < 0) return 0;
  const buf = new Uint8Array(32);
  const n = os.read(fd, buf.buffer, 0, buf.length);
  os.close(fd);
  if (n <= 0) return 0;
  const text = Array.from(buf.slice(0, n))
    .map((b) => String.fromCharCode(b))
    .join("")
    .trim();
  const parsed = Number.parseInt(text, 10);
  return Number.isFinite(parsed) ? parsed : 0;
}

function writeCount(next) {
  // Open WRONLY|CREAT|TRUNC. The KvDevice::open match on options.write/create
  // (lib.rs lines 125–128) returns a KvWriteFile that commits on close, so the
  // store value is replaced atomically with the bytes written.
  const fd = os.open(KEY, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o666);
  const bytes = Uint8Array.from(
    Array.from(String(next)).map((c) => c.charCodeAt(0)),
  );
  os.write(fd, bytes.buffer, 0, bytes.length);
  os.close(fd);
}

const next = readCount() + 1;
writeCount(next);

std.out.puts(`counter ${next}\n`);
std.out.flush();
```

Three things to notice:

- The handler is *stateless* between invocations. The only place state
  survives a process exit is the `KvDevice` `BTreeMap<String, Vec<u8>>`
  (`crates/wanix-kv/src/lib.rs` line 21). When `wanix serve` is running, that
  map lives in the serve process; every request that lands inside the served
  namespace shares it.
- There is no commit dance, no fsync, no temp-file rename. `KvWriteFile` on
  `close` replaces the value in the store under one write lock. Read paths get
  either the old value or the new one, never a torn intermediate.
- No file under `apps/` carries the state. If you `cat apps/`, you only see
  the handler. The cockpit's data-store index treats `#kv/counter` as the
  state node and `apps/counter.js` as the code node — see
  `workbench/src/web/http-app-demo.ts` `COUNTER_STATE_PATH` (the `counter.count.txt`
  preview-side mirror it materializes for display).

## 2. Run `wanix serve` with `#kv` bound

The exact CLI pattern from `crates/wanix-cli/src/serve/command.rs`:

```sh
wanix-rust serve --wanix-services --listen 127.0.0.1:7654 ./project-root
```

- `--wanix-services` is the parser flag at line 82; without it,
  `serve_services_root` is not called and `#kv` is *not* bound (see
  `roots.rs` lines 105–133 — only the services namespace path runs the
  `bind(Arc::new(KvDevice::new()), ".", "#kv", ...)` line).
- `--listen` (or `--addr`) is options-equivalent (`command.rs` line 74); the
  default is `127.0.0.1:7654` (`command.rs` line 6, `DEFAULT_SERVE_ADDR`).
- The positional `./project-root` is `root_path`; it becomes the `.` of the
  served namespace, so `./project-root/apps/counter.js` shows up at
  `apps/counter.js` and `#kv/counter` sits next to it under the same root.

On startup `serve` prints something like:

```text
wanix-rust serve listening on 127.0.0.1:7654 (root=./project-root, wanix-services)
```

That single served namespace is what every transport — direct 9P, WebSocket
9P, the `POST /agent` HTTP endpoint, and (eventually) the HTTP-app route —
binds against. There is no per-route filesystem; there is one filesystem and
many adapters.

## 3. Invoke it

On `cpu` today, there is no `GET /apps/counter` route wired into `serve`.
There are two honest ways to drive the handler against the running serve, and
both increment the *same* `#kv/counter` because the device is a singleton
inside the serve process:

### 3a. Drive it as an HTTP request the way `POST /agent` does it

`crates/wanix-cli/src/serve/http/agent.rs` lines 19–42 are the template for
any qjs-handler HTTP route on this branch: loopback check, namespace-bound
filesystem in, response body out. A future `POST /apps/counter` would look
the same but evaluate the qjs file instead of allocating an agent session.

Until that route lands, you can use the agent endpoint to *drive the same
handler from the inside* by asking it to run the counter:

```sh
curl -sS -X POST http://127.0.0.1:7654/agent \
  --data 'run apps/counter.js'
```

This is not a real HTTP-app call — the agent device gets the prompt and
decides what to do with it (`agent.rs` lines 45–53: it writes the prompt to
`#agent/<id>/prompt` and reads back `events`). It's listed here only to make
the structural point: the serve process already owns the namespace; bolting a
direct route on is a small Rust file plus a route entry in
`crates/wanix-cli/src/serve/http/routes.rs` lines 10–19 (`http_route_response`).

### 3b. Drive it directly with the qjs CLI against the same `#kv`

The honest, working "curl-like" loop today is `wanix-rust qjs` invocations
that share the running serve's KV state by mounting through the served
namespace. Each invocation is one request:

```sh
wanix-rust qjs ./project-root/apps/counter.js
# counter 1

wanix-rust qjs ./project-root/apps/counter.js
# counter 2

wanix-rust qjs ./project-root/apps/counter.js
# counter 3
```

Each call performs exactly the read-modify-write you'd expect from an HTTP
handler. The state survives between calls because the bind in
`roots.rs` line 129 keeps `KvDevice` alive for the lifetime of the serve
process — exactly the in-process durability story (3).

In a single `wanix-rust qjs` standalone (no serve), `#kv` is *not* bound
(only the services namespace built by `serve --wanix-services` and the agent
exec-server (`agent_exec_server.rs` lines 73–77) call `services_namespace_for_root`),
so a fresh `KvDevice` is allocated per process and the counter resets to 1
each call. That distinction is the whole point of section 4.

## 4. Why `#kv` (not files under `apps/`)

If you wrote the state to `apps/counter.count.txt` instead of `#kv/counter`,
three things would degrade:

- **Write churn.** Every increment is an open/truncate/write/close on the
  host filesystem. With `#kv` it is a single locked `BTreeMap` mutation
  (`crates/wanix-kv/src/lib.rs` lines 93–100 `ensure_key`, plus the
  `KvWriteFile` close-time commit). No journaling, no fsync amplification, no
  inotify storm if the workbench is watching `apps/`.
- **Durability boundary clarity.** `#kv` says explicitly "this is in-process
  state; on serve restart it is gone unless you snapshot it." Files under
  `apps/` look durable but only commit on disk sync semantics that depend on
  the host filesystem. `wanix-kv/src/lib.rs` line 7 even calls this out: "an
  in-memory tier; durable and content-addressed backing is a follow-up."
  When you want durability across restarts, the answer is freeze the
  interesting keys into a capsule (Recipe 03), not blur the `#kv` line.
- **Mesh transparency.** `#kv` is a plain `FileSystem` (it `impl FileSystem
  for KvDevice` in `lib.rs` line 120). That means a remote node that mounts
  `/n/A/#kv/counter` over QUIC reads the same value as the local handler —
  see the Slice 4 mesh test in `crates/wanix-mesh/tests/mesh_quic.rs` lines
  272–298. A bag of host files under `apps/` does not get this for free.

Put bluntly: `#kv` is the smallest "real database inside Wanix" exactly
because it is a filesystem, not a database. The handler treats it as a file,
the mesh treats it as a file, the cockpit treats it as a file.

## 5. The cockpit's role

The cockpit (the workbench Code OSS extension) is the catalog and preview
surface for this recipe. On `main`, `workbench/src/web/http-app-demo.ts`
already ships the exact `counter` template:

- `APP_DIR = "/apps"`, `COUNTER_NAME = "counter"`,
  `COUNTER_PATH = "/apps/counter.js"`,
  `COUNTER_STATE_PATH = "/apps/counter.count.txt"`.
- A catalog the cockpit materializes at `.wanix/http-apps.md` and
  `.wanix/http-apps.json` (`HTTP_APP_CATALOG_MD_PATH` and
  `HTTP_APP_CATALOG_JSON_PATH`). That is the "Wanix HTTP apps" index the
  data-store/system view links to.
- Per-app preview files (`counter.response.txt`) that show the most recent
  response body inline.
- A data-store entry that points at the counter state node so opening it in
  the editor reads the *live* `#kv` value as a file.

On the `cpu` branch the route handler itself isn't ported yet — the cockpit
catalog has nothing on the serve side to actually POST to. The handler
written here is the contract both sides converge on: a qjs file at
`/apps/<name>.js` that does its own `#kv` read/write. When the route lands on
the cpu serve (a thin sibling of `serve/http/agent.rs`), the cockpit catalog
already knows to render it, preview it, and link the data-store node.

That is the slice plan: handler today, in-process via qjs CLI; same handler
tomorrow, in-process via a `POST /apps/<name>` route; same handler after
that, across the mesh via `/n/A/apps/counter.js` resolving through the
already-mesh-transparent `#kv`. The recipe is the same file the whole way
down.
