# AppFS And AppResource

Status: **draft idea / design note**. This is not an ADR yet. It captures the
motivation and first contract shape for guest-defined Wanix apps that can expose
HTTP and filesystem surfaces. It is a sibling to
[ToolFS](toolfs.md), and builds on the resource/catalog direction in
[ADR 0007](adrs/0007-resources-catalogs-and-pairing.md).

**Status update (2026-06): v0 shipped; wire v0.2 shipped.** The filesystem
surface and the file2chan adapter live at `crates/wanix-appfs` (discrete ops
as newline-JSON events to the guest, a host pump thread for guest output,
host-owned never-EOF stream files, `open_view(principal)`), and `wanix-rust
app serve` (`crates/wanix-cli/src/app/`) runs the bundled `examples/chatroom`
qjs guest as a mesh-served AppResource — the Chatroom Proof below, pinned by
`crates/wanix-cli/src/app/serve/tests.rs` and walked through in
`docs/site/content/recipes/07-chatroom-over-the-mesh.md`.

The **v0.2 wire** (pinned field-by-field in `crates/wanix-appfs/src/tests.rs`)
adds, on top of the v0 line protocol:

- **Guest hello.** The guest's first line is
  `{"hello":{"proto":1,"files":[...],"streams":[...]}}`; the guest is the tree
  authority (manifest `files`/`streams` are documentation and the fallback
  when the hello declares no tree), and a proto mismatch or missing hello is a
  clear serve-time error.
- **Ranged reads.** `read` requests carry `offset`/`len` (the host asks in
  256 KiB chunks); a reply shorter than `len` means EOF, so the 1 MiB line
  ceiling no longer caps app file size while a small file still costs one
  fetch.
- **Host-stamped time.** Every request carries `at_ms` (host wall-clock
  milliseconds) — the guest's one trusted time source.
- **Optional stat size.** A stat reply may declare `{"size":N}`; the adapter
  reports it as the file's metadata length instead of lying `0`.
- **Principal scheme.** The serve attach policy presents verified mesh peers
  as `iroh:<hex>`; principals stay opaque to the adapter and guest, and the
  scheme keeps future gateway principals collision-free.
- **Op-fatal vs channel-fatal errors.** An oversized or malformed (but
  newline-framed) guest line fails only the in-flight op with the specific
  error; only genuine desync (broken pipe, unattributable garbage, undeclared-
  stream publish, mismatched reply id) latches the channel down.
- **Per-request deadline.** Every reply wait is bounded (default 30 s,
  configurable via `AppFsService::with_op_deadline`); expiry fails the op as
  `Unreachable` naming the op and deadline and latches the channel down.
- **Auto-restart.** `app serve --restart on-failure` supervises the guest with
  capped exponential backoff, swapping each fresh generation's service through
  a `ServiceSlot` read per attach — ticket and endpoint unchanged, old
  connections keep their honestly-dead view, new connections get the live
  service (default remains no restart).

What this still does **not** cover from this design: the HTTP surface and
gateway principals (§HTTP Surface), the turn-based execution model with a host
handle table and stream splicing (§Execution Model — the guest is an ADR 0010
tier-2 resident qjs loop over blocking stdin, not tier-1 turns), CAS-pinned
code provenance (§Provenance), and the rust-wasm adapter.

## Motivation

Wanix has a powerful primitive hiding in plain sight: a resource is a
`FileSystem`, and a namespace is how resources become useful together. The
recent volume work makes that visible at the product level: one resource ticket
names one root, and a task or shell composes several resource roots into one
working namespace.

ToolFS answers one important question:

```text
How do I expose one host-approved operation as files?
```

AppFS asks a different question:

```text
How do I let a Wanix app be the resource?
```

A chatroom should be able to be a mounted filesystem:

```text
/n/general/
  post
  stream
  latest
  who
```

The same room should also be able to be a small web service:

```text
POST /post
GET  /stream
GET  /latest
GET  /who
```

That shape is inspired by Cloudflare Workers and Durable Objects, but the Wanix
substrate is different. Cloudflare gives an object a platform storage and
network environment. Wanix should give an app a namespace: volumes, `#kv`,
`#plumb`, `#cas`, `#agent`, remote mesh resources, and whatever else the host
binds in. Authority is visible as files.

The useful mental model:

```text
Workers/Durable Objects, but the bindings are Wanix namespace capabilities.
Plan 9 file servers, but the app can also speak HTTP.
```

Web and files are both fundamentals. HTTP is how browsers, humans, and ordinary
web clients arrive. Filesystems are how Wanix composes authority, state, tools,
agents, and remote devices once they are inside.

## The Idea

The core primitive should be **AppResource**.

An AppResource is a named, stateful Wanix resource implemented by a guest app
such as qjs or Rust-wasm. It has one resource identity, one manifest, one
granted namespace, one lifecycle policy, and one or more surfaces.

```text
AppResource
  runtime:    qjs | rust-wasm | future runtime
  surfaces:   http | fs | both
  authority:  mounted namespace + verified principal + declared limits
  durability: explicit mounted files, not guest memory
```

**AppFS** is the filesystem surface of an AppResource. It is important, but it
is not the whole idea. Write the durable concept as AppResource first, AppFS
second.

The division of responsibility should stay simple:

```text
host:
  identity, grants, limits, handle tables, stream pumping, lifecycle,
  cancellation, gateway/transport adaptation

guest:
  app decisions, state transitions, small transforms, response selection,
  application semantics

namespace:
  authority, durable state, composed resources

gateway / mesh:
  how clients arrive, how principals are proven, how streams are transported
```

The slogan:

```text
AppResource returns capabilities, not bytes.
Host moves bytes; guest names meaning.
```

## What

An AppResource is a filesystem-shaped or web-shaped app packaged with a small
manifest.

Sketch:

```text
chatroom/
  app.wanix.json
  main.js
```

Possible manifest sketch:

```json
{
  "wanix.resource": "v0",
  "kind": "app",
  "name": "chatroom",
  "runtime": {
    "kind": "qjs",
    "main": "cas:sha256:9f2c..."
  },
  "surfaces": {
    "http": true,
    "fs": true
  },
  "mounts": {
    "/state": {
      "catalog": "chat-state"
    },
    "/bus": {
      "device": "#plumb"
    }
  },
  "limits": {
    "memoryBytes": 67108864,
    "maxBodyBytes": 1048576,
    "maxOpenStreams": 128,
    "maxConcurrentRequests": 32,
    "requestTimeoutMs": 5000
  }
}
```

### Provenance: CAS-pinned code, mount-gated authority

Two deployment rules are part of the v0 contract, not later hardening, because
the most prolific author of AppResources will be agents and retrofitting
immutability under stable tickets is much harder than starting with it:

1. **Code is CAS-pinned.** The manifest references guest code by `cas:` hash;
   the app's running identity is the hash of (manifest + code), exposed in its
   `spec.json`/`status`. "What exactly is running on node X" has a
   cryptographic answer across a fleet — no snowflakes. An upgrade is a new
   hash; the deployment record (hash history per ticket) is the audit log and
   the rollback mechanism. A client can verify "this is the build I audited"
   from the spec.

2. **The deploy gate is mount approval — review capabilities, not code.**
   Nobody audits 200 lines of generated JS per deploy; nobody has to. The
   manifest's `mounts` are the app's *entire* authority, so deploying an app
   creates a pending approval listing its requested mounts and limits — the
   same approvals-as-files pattern the `#agent` device already uses. The owner
   (or a policy agent) approves three mount lines, never the code. Code can be
   anything precisely because authority is explicit (ADR 0000).

The guest sees a small runtime-neutral API. Qjs can expose it in a friendly
Workers-like form:

```js
export default {
  async fetch(req, env, ctx) {
    const log = env.fs.open("/state/log");
    return new Response(log.stream());
  },
};
```

But the contract should not be qjs-only. Rust-wasm should be able to implement
the same value model later.

Core value algebra:

```text
AppValue =
  Empty
  Bytes
  Json
  FileHandle
  StreamHandle
  HttpResponse { status, headers, body }
  Error
```

Where `body` is normally bytes or a `StreamHandle`.

The host owns the handle table. A handle is an opaque capability: the guest can
refer to it, return it, or close it, but cannot forge it.

## Why

### It keeps ToolFS honest

ToolFS and AppResource solve different problems.

```text
ToolFS:
  host chooses executable, argv, env, cwd, limits, and output mapping;
  caller supplies data.

AppResource:
  host chooses authority, identity, limits, lifecycle, and mounted namespace;
  guest app chooses behavior.

#cpu:
  caller chooses program and job spec;
  host runs it.
```

These should not collapse into each other.

ToolFS is a host wrapper. AppResource is a guest-defined resource. `#cpu` is a
remote exec plane. The trust boundary is different in each case.

### It matches the grain of Wanix

Wanix already has:

- namespaces as composition;
- native mesh imports as local files;
- service devices as plain `FileSystem`s;
- qjs and wasm tasks sharing one namespace model;
- resource tickets beginning to name roots;
- browser/cockpit HTTP routes as human entry points.

AppResource does not replace that. It makes it programmable.

### It avoids the byte-pump trap

The tempting first implementation is to ask qjs to implement `read()` and
`write()` directly. That is the wrong center.

If a qjs handler blocks forever inside `read("stream")`, the single guest actor
is now the stream pump. It cannot handle `post`, `who`, cancellation, or new
subscribers cleanly. Rust-wasm is faster and more predictable than qjs, but the
same architecture can still get stuck.

The host must own concurrent edges and blocking stream movement. The guest
should make decisions and return handles.

### It creates one bridge between web and files

A stream body, a mesh file read, a `#plumb` subscription, a `#pipe`, a CAS blob,
and an HTTP response body can all be one host-level `StreamHandle`.

That lets a qjs app say:

```js
return new Response(env.fs.open("/vol/movie.mp4").stream());
```

without sitting in a JavaScript loop copying bytes from the volume to the
client.

## Prior Art To Borrow From

### Cloudflare Workers And Durable Objects

Borrow the stateful actor intuition from Durable Objects: one named
coordination point is a good shape for rooms, game sessions, collaborative
documents, queues, and other "one object owns the truth" resources.

Borrow the familiar `fetch(request, env, ctx)` shape for qjs HTTP. Borrow
Web Streams as the user-facing feel for HTTP bodies.

But do not borrow hidden platform magic. In Wanix, `env` should be derived from
the manifest and namespace. A binding is not a second authority model; it is a
friendly name for a mounted capability.

Useful references:

- [Durable Objects](https://developers.cloudflare.com/durable-objects/)
- [Workers bindings](https://developers.cloudflare.com/workers/runtime-apis/bindings/)
- [Workers Streams](https://developers.cloudflare.com/workers/runtime-apis/streams/)

### Plan 9 And Inferno

Borrow the file-server taste: design the namespace first. A program can be a
file server. A small tree of synthetic files can be a better interface than a
large API surface.

For a chatroom, this is a good tree:

```text
/
  post
  stream
  latest
  who
  status
```

Avoid "one file per method" sprawl. Files are the distribution and composition
boundary; the app/runtime boundary can still use typed events and handles.

Inferno's `file2chan` is especially relevant: it lets a process receive
filesystem requests as messages. Its warning is also relevant: the process
serving the request cannot be the process blocked in the read/write syscall.
That is the exact single-threaded qjs trap AppResource must avoid.

Useful references:

- [The Use of Name Spaces in Plan 9](https://felloff.net/sys/doc/names.html)
- [The Ubiquitous File Server in Plan 9](https://inferno-os.org/papers/servers.pdf)
- [Inferno file2chan](https://inferno-os.org/inferno/man/2/sys-file2chan.html)

### Go, Clojure, Rust, And Wasmtime

Borrow the tiny stream contracts from Go: `Reader`, `Writer`, `Closer`,
`Context`, and explicit cancellation. Pipelines are elegant only if downstream
exit cancels upstream work.

Borrow the transform taste from Clojure transducers and core.async: transforms
should be composable, but buffers and blocking behavior must be explicit.

Borrow the implementation shape from Rust: `hyper` bodies are pull-based
streams with backpressure, `tower::Service` has an admission/readiness point,
and Tokio bounded channels make backpressure real.

Borrow the handle-table model from Wasmtime component resources:
host-owned resources, typed handles, capacity limits, and parent-child
lifetimes. AppResource does not need to adopt the full component model in v0,
but it should copy the resource discipline.

Useful references:

- [Go pipelines and cancellation](https://go.dev/blog/pipelines)
- [Clojure transducers](https://clojure.org/reference/transducers)
- [core.async reference](https://clojure.github.io/core.async/reference.html)
- [Hyper body module](https://docs.rs/hyper/latest/hyper/body/index.html)
- [Wasmtime ResourceTable](https://docs.wasmtime.dev/api/wasmtime/component/struct.ResourceTable.html)

## Execution Model

V0 should be:

```text
single actor guest, concurrent host edge
```

The guest actor is serialized. The host edge is concurrent.

> This is now the decided platform shape, not an AppFS-local choice: an
> AppResource is a **tier-1 turn-based resident task**
> ([ADR 0010](adrs/0010-task-tiers-turns-and-snapshots.md)). The empty stack
> between turns is what makes the app snapshottable, killable, forkable, and
> migratable — the host-pump rule below is load-bearing for operability, not
> just a concurrency dodge.

For HTTP:

1. A request arrives through a local gateway, native iroh HTTP ALPN, or served
   HTTP route.
2. The host authenticates or annotates the caller and builds `ctx`.
3. The host exposes `env` from the AppResource namespace.
4. The guest `fetch` handler returns an `AppValue`.
5. If the response body is a `StreamHandle`, the host splices it to the client
   with backpressure and cancellation.

For AppFS:

1. The mounted AppFS root receives a filesystem operation.
2. The host handles ordinary filesystem mechanics: open handles, offsets,
   pending reads, cancellation, and stream lifetimes.
3. The guest receives events such as `stat`, `readDir`, `open`, `write`,
   `close`, or `ctl`.
4. The guest returns bytes, metadata, errors, or stream handles.
5. Blocking reads park host-owned streams or queues, not the guest actor.

This is closer to "Inferno `file2chan` plus host-managed streams" than to
"qjs directly implements every blocking `File` method."

## Streams

Streams are the contract's sharp edge. They should be first-class from the
beginning, but conservative.

V0 stream rules:

- A `StreamHandle` is opaque and unforgeable.
- A handle is created by the host from the AppResource namespace or by a
  host-approved app API.
- Returning a stream transfers or borrows it according to one explicit rule.
  Prefer linear transfer in v0: one consumer, one close path.
- The host owns pumping bytes between source and destination.
- Client disconnect cancels the consuming stream.
- Cancellation propagates upstream where the source supports it.
- Every stream has limits: buffer bytes, idle timeout, total bytes when
  applicable, and owner lifetime.
- Errors surface through the edge shape: filesystem error for AppFS, HTTP
  response error or stream termination for HTTP.

Do not add guest transforms in the first slice.

Later transform sketch:

```js
return env.fs
  .open("/logs/app.log")
  .stream()
  .lines((line) => line.includes("ERROR") ? line + "\n" : null);
```

Transform rules, when added:

- max line/chunk size;
- max output expansion per input chunk;
- callback timeout;
- bounded queues;
- explicit behavior on transform error;
- per-stream transform state;
- separate lane for blocking or expensive transforms.

## Identity And Trust

The guest should never claim identity.

For native mesh calls, the principal comes from iroh's verified peer identity.
For browser HTTP, the principal comes from the gateway policy. For local
development, it may be a local synthetic principal.

The app receives identity as context:

```text
ctx.principal
ctx.transport
ctx.localPetname?   # display concern, not auth
```

A chatroom `post` accepts the body only. The server stamps author from
`ctx.principal`.

This exposes an existing Wanix seam: the native mesh handler already knows the
verified `remote_id()`, but an open exported root currently does not hand that
principal into the resource implementation. Principal-aware resources need one
of these:

- a principal-scoped root factory for native mesh;
- an "allow all but still wrap with principal" attach policy;
- or a resource-specific context seam that is clearly native-mesh-only.

Do not revive 9P's client-claimed `uname` as authority.

## Durability

Guest memory is a cache. It may disappear on restart, eviction, crash, upgrade,
or snapshot restore.

Durable state must be explicit:

```text
/state       mounted volume or #kv prefix
/log         #cas-backed append/log resource later
/bus         #plumb
/objects     #cas
```

This keeps the contract honest. It also makes AppResource composition visible:
move the app and its mounted state resources together, or intentionally point
two apps at the same state.

Open question: should the manifest require a declared state mount for any app
that advertises itself as durable? The conservative answer is yes.

## HTTP Surface

The qjs HTTP surface should feel familiar:

```js
export default {
  async fetch(request, env, ctx) {
    return new Response("hello\n");
  },
};
```

Wanix-specific differences:

- `env` derives from namespace mounts and manifest names.
- `ctx.principal` derives from transport or gateway identity.
- `ctx.waitUntil` can be considered later, but do not make background work
  implicit in v0.
- `Response` bodies can be bytes or host-owned streams.
- Service-to-service calls should prefer namespace resources and HTTP/FS
  surfaces before adding private RPC.

Possible native mesh path:

```text
WANIX_HTTP_ALPN = b"wanix/http/1"
```

This would live beside the native filesystem ALPN. A local or public gateway can
dial the iroh service and present ordinary HTTP to browsers. The gateway path is
the practical demo path; native HTTP-over-iroh is the cleaner mesh path.

## Filesystem Surface

AppFS should expose a small intentional tree.

For the chatroom proof:

```text
/
  spec.json    read-only app/filesystem contract
  post         write message body
  stream       never-EOF read of newline JSON messages
  latest       bounded read of recent newline JSON messages
  who          read current participants
  status       read app status
```

`post` should not contain author, room id, or trusted timestamp. The host/app
adds authenticated attribution.

`stream` should be a host-owned stream handle. A blocked reader must not block
the guest actor.

`latest` can be ordinary bounded bytes and is a good compatibility path for
single-request clients.

`who` is presence from host-owned session/stream state plus app policy.

## Chatroom Proof

The first proof should be a qjs chat AppResource that exposes both HTTP and FS.

HTTP:

```text
POST /post
GET  /stream
GET  /latest
GET  /who
```

FS:

```text
/post
/stream
/latest
/who
```

State:

```text
/state/messages
/state/members?      # optional; presence may be live only
```

The proof should demonstrate:

- one named resource maps to one actor;
- qjs can implement app semantics;
- the host supplies `ctx.principal`;
- message author is stamped, not client-claimed;
- HTTP body can be a returned stream handle;
- AppFS `stream` can be the same returned stream handle;
- slow readers have an explicit policy;
- durable history survives guest restart when backed by `/state`;
- no hidden authority exists outside the namespace and manifest.

Slow-reader policy must be explicit. Reasonable v0 choices:

```text
sliding buffer, disconnect on overflow
```

Blocking the whole room on one slow reader should not be the default.

## API Sketch

This is illustrative, not a contract.

```js
const subscribers = new Set();

export default {
  async fetch(req, env, ctx) {
    const url = new URL(req.url);
    if (req.method === "POST" && url.pathname === "/post") {
      const body = await req.text();
      return postMessage(body, env, ctx);
    }
    if (req.method === "GET" && url.pathname === "/stream") {
      return new Response(chatStream(ctx), {
        headers: { "content-type": "application/x-ndjson" },
      });
    }
    if (req.method === "GET" && url.pathname === "/latest") {
      return new Response(await latest(env), {
        headers: { "content-type": "application/x-ndjson" },
      });
    }
    return new Response("not found\n", { status: 404 });
  },

  fs: {
    async write(path, bytes, env, ctx) {
      if (path === "post") return postMessage(bytes.text(), env, ctx);
      throw new FsError("not found");
    },

    async open(path, env, ctx) {
      if (path === "stream") return chatStream(ctx);
      if (path === "latest") return bytes(await latest(env));
      throw new FsError("not found");
    },

    async readDir(path) {
      if (path === ".") return ["post", "stream", "latest", "who", "status"];
      throw new FsError("not found");
    },
  },
};
```

`chatStream(ctx)` should not be a JavaScript loop that pumps bytes forever. It
should return a host stream capability registered for this subscriber.

## Build Slices

1. **Contract note and fake host tests.** Pin the AppResource vocabulary,
   value algebra, handle ownership rules, stream cancellation rules, and
   chatroom tree.
2. **Host handle table.** Add an AppResource-local table for `FileHandle` and
   `StreamHandle`, with capacity limits and parent lifetimes.
3. **Qjs HTTP proof.** Run a qjs `fetch` handler behind a local HTTP route.
   Support bytes and simple response metadata first.
4. **Stream return proof.** Let qjs return a host stream from a mounted
   namespace file and have the host splice it to HTTP without qjs pumping.
5. **Principal context.** Thread verified/native or gateway principal into
   `ctx`, with tests proving the guest cannot spoof it through payload.
6. **Chatroom HTTP.** Implement `POST /post`, `GET /latest`, `GET /stream`,
   and `GET /who`.
7. **AppFS adapter.** Expose the same chatroom as a `FileSystem` tree.
8. **Native iroh HTTP ALPN.** Add `wanix/http/1` once the local gateway proof is
   stable.
9. **Rust-wasm adapter.** Implement the same AppValue contract from wasm,
   borrowing WIT/resource concepts where they fit.
10. **Transforms.** Add bounded line/chunk transforms only after raw stream
    handles are solid.

## Non-Goals For V0

- Full WASI Preview 2/component-model adoption.
- Arbitrary RPC between apps.
- Guest-owned blocking stream pumps.
- Distributed actor placement or replication.
- Multi-owner stream duplication.
- Long-running guest transforms.
- Background task semantics such as `waitUntil`.
- Public multi-user auth UX.
- Perfect browser/full-duplex streaming on every gateway transport.

## Open Questions

- Should the top-level primitive be named `AppResource`, `App`, or something
  else? `AppResource` is clunky but says the right thing.
- Is `AppFS` the filesystem surface only, or the umbrella name users will say?
- Should stream handles be strictly linear in v0, or should borrowed handles be
  allowed for pass-through file reads?
- What is the exact principal-aware root seam for native mesh open resources?
- How should gateway principals be represented when HTTP arrives without native
  iroh identity?
- Does a durable app require a declared `/state` mount?
- What restart policy is visible to the app and to clients?
- How much of the qjs API should mimic Fetch/Web Streams, and how much should
  be Wanix-specific from day one?
- How should AppResource catalog entries describe surfaces and health?
- ~~Should AppResource and ToolFS share `spec.json` conventions?~~ Decided:
  yes — the shared `"wanix.resource"` envelope (see docs/toolfs.md §Spec
  Shape); AppResource is `"kind": "app"`.
- When a resource exposes both HTTP and FS, should they be one ticket with
  several advertised surfaces, or separate resource tickets?

## Summary

AppResource is the missing live-app resource in the ADR 0007 story.

ToolFS lets a host wrap one approved operation. Volumes let users name durable
state. AppResource lets a small qjs or wasm program become a named Wanix
resource with HTTP and filesystem surfaces.

The core boundary should stay simple:

```text
guest decides;
host moves;
namespace grants;
transport identifies.
```

That gives Wanix a Worker-like programming model without losing its Plan 9
center. The first proof should be a qjs chatroom that can be used as a web app
and mounted as a filesystem, with stream handles and verified identity doing the
real work underneath.

