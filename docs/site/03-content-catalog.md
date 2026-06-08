# Wanix Site — Content Catalog & Page Manifest

## Wanix Documentation — Complete Page Manifest

This manifest enumerates every page that needs full prose, organized to match the eight-section IA (Home, Learn, Concepts, Devices, Use cases, Reference, Recipes, Find). The **Learn** and **Find** sections are index/landing pages that thread or surface the canonical content rather than duplicate it, so the corpus weight lives in Concepts, Devices, Use cases, Reference, and Recipes.

### Home (1 page)
A single hero/overview page with the 30-second proof (`wanix qjs examples/qjs-demo.js` → "outside Chrome: true"), the thesis ("Plan 9 reincarnated for the age of agents"), and four persona on-ramps that route into the flows.

### Learn (1 catalog page)
One landing page cataloging the nine learning flows by persona with time estimates and prerequisite badges. The flows themselves are *sequences over existing pages*, so each flow's steps map to concept/recipe/device pages already in the manifest (I did not create duplicate "flow-step" pages where the IA's flow step already points at a canonical concept or recipe slug — instead each canonical page carries the flow in its "Used in flows" rail). The genuinely new flow-step landing/intro pages that have no canonical home (e.g. flow intros) are folded into the Learn catalog.

### Concepts (~52 pages)
One canonical page per node in the concept graph, plus a handful the flows reference by slug that are not literally in the node list but are first-class ideas (http-app-route, send-agent-to-the-data, trust-boundary-gaps, missing-half-of-9p, wanix-as-host, remotefs-import-half, the-9p-contract, the-filesystem-trait variants). Each is reference-grade with progressive disclosure, a source-of-truth box, and a mandatory Status/honest-limits block.

### Devices (8 pages + 1 shared contract)
One page per service device (#task, #term, #kv, #pipe, #plumb, #cas, #agent, #cpu), each a stable set of per-file deep-link anchors. The blocking-stream EOF contract is a shared concept page cross-linked from all of them.

### Use cases (6 pages)
Outcome-first "why" pages drawn from the philosophy research: personal compute mesh, agents on your files, portable worlds, browser cockpit, distributed dev environments, traceable namespaces (clearly labeled exploratory).

### Reference (~14 pages)
ADR digests (one per ADR + workflow), crate map + layering, FileSystem/File trait, serve+discovery, quality gates, extension points, contributor landing, CLI command index, handoff JSON, queued follow-ups.

### Recipes (~6 pages)
The five copy-paste recipes mapped one-to-one to the shipped docs/recipes/*.md, plus the two foundational walkthrough sub-pages the JS flow needs (walkthrough-1-run-js, walkthrough-2-process-context) which split rust-walkthrough.md into runnable beats.

### Find (1 hub page)
The findability hub describing search, the graph-backed concept index, the glossary, and the tag browser. The A–Z glossary content is generated from concept one-liners, so it is one page, not many.

Rationale for word targets: concepts 900–1500 (deeper for trust-boundary and mesh ideas), flow-steps/recipes 500–900, use-cases 800–1400, reference 700–1300, developer 800–1500, home 500–900. Every page's sourceRefs point at the real repo files a writer must read; the honest-limits blocks are seeded from the GAPS/CAVEATS research so writers cannot overclaim (FakeEngine vs codex, #kv in-memory, single-frame serve, exec-local-trust, no hard CPU/mem limits, mesh has no ADRs yet).

---

## Full page manifest (101 pages)

### `home` (2)

| Slug | Title | Type | Audience | ~Words | One-liner |
|---|---|---|---|---|---|
| `home` | Wanix — Plan 9 Reincarnated for the Age of Agents | home | all | 700 | A Rust-native, Plan 9-style OS core that runs outside Chrome: everything is a file, every process has its own namespace, and one 9P contract reaches local services and remote machines alike. |
| `learn/index` | Learn Wanix — Guided Flows | overview | all, newcomers first | 900 | A catalog of nine persona-shaped learning flows that thread the canonical concept, device, and recipe pages in order — pick your goal and be carried. |

### `concepts` (67)

| Slug | Title | Type | Audience | ~Words | One-liner |
|---|---|---|---|---|---|
| `concepts/everything-is-a-file` | Everything Is a File | concept | all | 1100 | Every capability — storage, tasks, terminals, key/value, agents — is exposed as a FileSystem you read, write, and list, so one file protocol reaches them all. |
| `concepts/the-filesystem-trait` | The FileSystem Trait | concept | developer | 1300 | One Rust trait (open/metadata/read_dir/mutations) is the universal currency of the whole system; implement it and you are a Wanix capability. |
| `concepts/normalizedpath` | NormalizedPath | concept | developer | 900 | Wanix paths are relative, slash-separated, no . or .. components, with '.' as the root — the Go io/fs.ValidPath shape — so paths can never escape upward. |
| `concepts/per-process-namespaces` | Per-Process Namespaces | concept | all | 1200 | Each task carries its own private, rearrangeable view of the filesystem tree; a child clones the parent's view rather than sharing one global root. |
| `concepts/namespace-binding` | Namespace Binding (bind / mount) | concept | developer | 1000 | bind() splices any FileSystem into the namespace at a chosen path with first/replace/last ordering, giving Plan 9 union mounts. |
| `concepts/namespace-resolution` | Namespace Resolution (longest-prefix, union) | concept | developer | 1000 | A path resolves against all bindings whose destination is a prefix, most-specific binding first, merging directories into one synthesized view. |
| `concepts/service-devices` | Service Devices (#name) | concept | all | 1200 | Named '#' devices (#task, #term, #kv, #pipe, #plumb, #cas, #agent) are file trees you operate by read/write/ls instead of bespoke APIs. |
| `concepts/tasks-own-process-identity` | Tasks Own Process Identity | concept | developer | 1200 | Wanix — not the engine — owns task id, parent, kind, cmd/env/cwd, exit status, namespace, and fds; QuickJS/wasm are just engines a task chooses. |
| `concepts/the-fd-table` | The fd Table | concept | developer | 1000 | Each task has an fd table reserving 0/1/2 for stdio and allocating dynamic fds from 3, holding shared open-file handles that survive a closing source task. |
| `concepts/task-drivers` | Task Drivers | concept | developer | 1100 | A driver's check() auto-selects a runtime by program suffix (.js->qjs, .wasm->wasm) and start() runs it; runtimes register from above so the task core stays engine-free. |
| `concepts/shared-wasi-fd-contract` | The Shared WASI / fd Contract | concept | developer | 1200 | task_wasi_config builds the live WASI config (namespace, cwd preopen, argv/env, stdio fds) once for both runtimes, mirroring dynamic file fds into the task table. |
| `concepts/wasmtime-as-substrate` | Wasmtime as the Execution Substrate | concept | developer | 1200 | Wasmtime hosts guest runtimes but inherits no host filesystem or process semantics by default; Wanix supplies live WASI backed by its own namespaces. |
| `concepts/wanix-backed-wasi` | Wanix-Backed WASI (not host WASI) | concept | developer | 1100 | Preview 1 syscalls resolve through Wanix namespaces and WASI fds instead of the host OS; #name device paths resolve from the namespace root regardless of cwd. |
| `concepts/host-not-ambient-authority` | Host Files Are Not Ambient Authority | concept | developer | 900 | Host directories enter Wanix only through an explicit rooted LocalFs mount with escape checks; there is no ambient host path in the default namespace. |
| `concepts/qjs-task` | qjs Task — JavaScript Outside Chrome | concept | all | 1300 | QuickJS-NG compiled to a WASI reactor on Wasmtime runs .js as a first-class Wanix task with live namespace, fds, argv/env/cwd, and observable exit. |
| `concepts/compiled-wasm-task-driver` | Compiled wasm Task Driver | concept | all | 1200 | Any wasm32-wasi command module (Rust/C/Zig/Go) runs as a Wanix .wasm task at near-native speed, sharing the same sandbox and VFS as qjs. |
| `concepts/command-style-wasm-linker` | Command-Style wasm Linker (poll_oneoff = NOSYS) | concept | developer | 900 | wanix-wasi-host is a generic WASI Preview 1 linker for command guests; poll_oneoff returns ERRNO_NOSYS, so there is no readiness polling — qjs keeps its own richer host path. |
| `concepts/porting-a-normal-binary` | Porting a Normal Binary (What a Wanix Task Sees) | concept | developer | 1200 | A Wanix task is an ordinary WASI program — same open/read/write, same argv/env — so file-and-stdio software ports by recompiling; what changes is what the syscalls refer to, and async/socket/event-loop software is the part that needs rework. |
| `concepts/two-tiers-one-substrate` | Two Tiers, One Substrate | concept | visionary | 900 | Interpreted (qjs) and compiled (wasm) tasks built from one Namespace share a filesystem — a write by one is visible to the other. |
| `concepts/compiled-artifact-cache` | Compiled-Artifact Cache | concept | developer | 1100 | sha256-keyed on-disk cache of Wasmtime-serialized modules cuts qjs cold start ~1000x; the cache dir is an fd-verified owner-private trust boundary. |
| `concepts/quickjs-snapshots-are-vm-images` | QuickJS Snapshots Are VM Images | concept | developer | 1000 | Snapshots capture WebAssembly linear memory + pointers, not serialized task state; live Wanix host state (namespace, fds, providers) must be reattached on restore. |
| `concepts/bounded-execution-policy` | Bounded Execution Policy (not a scheduler) | concept | developer | 900 | qjs driver knobs (event-loop wait budget, ready-IO turns, interrupt poll budget, memory limit) bound guest execution but are not a general scheduler, signal system, or cancellation model. |
| `concepts/guest-js-guardrails` | Guest-JS Guardrails | concept | developer | 800 | Guest code uses qjs:std, qjs:os, scriptArgs, stdio, env, and service files — never globalThis.Wanix helpers or a read-only virtual-file bridge. |
| `concepts/qjs-shell` | qjs-shell (Shell as a Guest Program) | concept | all | 1300 | The bundled interactive shell is JavaScript inside a Wanix task driving #term and #task service files — not a separate process model. |
| `concepts/raw-vs-cooked-input` | Raw vs Cooked Input & Control Bytes | concept | developer | 800 | In raw mode the guest shell owns echo, backspace, Ctrl-C line cancel and Ctrl-D exit; clients forward 0x03/0x04 as terminal input, never as task cancellation. |
| `concepts/resize-winch-lifecycle` | Resize / winch Lifecycle | concept | developer | 800 | Resize travels as 'columns rows\n' through #term/<id>/winch; queued resizes are drained on read-handler turns; sessions release the terminal via ctl close on drop. |
| `concepts/the-9p-contract` | The 9P Contract | concept | developer | 1300 | 9P2000.L (plus Google.1/.2 extensions) is the one wire protocol that lets Linux, v86, editors, browsers, and the mesh browse and mutate any Wanix namespace. |
| `concepts/protocol-vs-server-split` | Protocol vs Server Split | concept | developer | 1000 | wanix-protocol is dependency-free wire codecs; wanix-9p maps fids to FileSystem objects and Linux errnos — transports are thin adapters over the same server. |
| `concepts/remotefs-import-half` | RemoteFs — the Import Half of 9P | concept | developer | 1400 | A synchronous wanix_fs::FileSystem that speaks 9P to a remote server, so binding it splices a remote namespace (files and #-devices alike) into your own. |
| `concepts/missing-half-of-9p` | The Missing Half of 9P | concept | visionary | 1400 | Wanix could always export a namespace; the mesh adds the 9P client so it can also import one — bind a remote node at /n/<node> and its files and devices become local. |
| `concepts/import-export-and-n` | Import / Export and /n/ | concept | all | 1100 | Export served files long ago; RemoteFs is the missing 9P client — bind a peer at /n/<node> and its whole namespace (files and # devices) becomes local. |
| `concepts/9p-over-iroh-quic` | 9P over iroh QUIC under One ALPN | concept | developer | 1200 | One iroh Endpoint per node carries 9P (ALPN wanix/9p/1) over QUIC bidi streams; the synchronous 9P core runs unchanged behind a held-runtime blocking bridge. |
| `concepts/async-sync-bridge` | The async/sync Bridge Confined to One Seam | concept | developer | 1000 | iroh and tokio live only in wanix-mesh; BlockingDuplex drives async QUIC streams on a held runtime Handle (never block_on on a worker), so the whole 9P core stays transport-free. |
| `concepts/streaming-import-fs` | StreamingImportFs — One Stream per Blocking Open | concept | developer | 800 | A near-never-EOF read like #agent/<id>/events would freeze the serial connection, so each blocking streaming open dials its own dedicated bidi stream. |
| `concepts/five-hostile-peer-corrections` | Five Hostile-Peer Corrections | concept | developer | 1100 | Frame-size ceiling vs OOM, honest seekability vs silent corruption, RAII fid/tag guards vs leaks, bounded read_dir, and server-side append — the difference between a client that demos and one you'd trust. |
| `concepts/key-is-the-address` | The Key Is the Address | concept | all | 1100 | A node's persisted ed25519 public key is simultaneously its identity and its dialable iroh EndpointId, so 'mount this node' and 'trust this node' are the same bytes. |
| `concepts/persisted-ed25519-identity` | Persisted ed25519 Node Identity | concept | developer | 900 | A 32-byte ed25519 seed at ~/.wanix/node.key (0600, stable across restarts) is the node identity; the QUIC handshake authenticates it so in-band Tauth stays ENOSYS. |
| `concepts/tauth-is-enosys` | Tauth Is ENOSYS (Identity Is a Transport Property) | concept | developer | 900 | In-band 9P authentication is deliberately unimplemented; the QUIC handshake authenticates the peer's key in the transport, so factotum becomes a property of the connection. |
| `concepts/capability-is-a-bind` | A Capability Is a Bind | concept | visionary | 1500 | A grant is not an ACL; it re-roots the peer's namespace at a subpath via SubtreeFs gated by read/write rights, so there is no 'outside' for them to name. |
| `concepts/attach-policy` | AttachPolicy — the Trust Boundary as One Pure Function | concept | developer | 1200 | evaluate(peer, aname) -> Option<Authorization> is the entire attach gate; None denies, and the boundary lives in one auditable place out of the wire code. |
| `concepts/subtreefs-confine-to-prefix` | SubtreeFs / confine_to_prefix | concept | developer | 1100 | SubtreeFs re-roots a backing FS at a prefix and gates every method through one central require(right) check; confine_to_prefix closes the symlink-escape hole. |
| `concepts/devices-import-for-free` | Devices Import Across the Mesh for Free | concept | all | 1100 | Because every service device is a plain FileSystem, /n/A/#kv/<key> and /n/A/#agent work through the one 9P client with no special-cased code. |
| `concepts/one-identity-two-planes` | One Identity, Two Planes (9P Control + iroh-blobs Data) | concept | developer | 1000 | The 9P control plane and the BLAKE3 bulk data plane (iroh-blobs) accept on the same identity-bound endpoint, so a peer reaches both over one QUIC path. |
| `concepts/content-addressed-data-plane` | Content-Addressed Data Plane (content_hash) | concept | developer | 1000 | A FileSystem can expose a BLAKE3 content_hash so CAS-aware clients fetch large files as verified blobs peer-to-peer instead of crawling them through bounded 9P reads. |
| `concepts/kv-smallest-database` | #kv as the Smallest Database | concept | visionary | 900 | In-process BTreeMap state that the mesh, handler, and cockpit all treat as a file; durable backing is a capsule, not a blur. |
| `concepts/best-effort-epidemic-delivery` | Best-Effort Epidemic Delivery (not a queue) | concept | developer | 900 | #plumb has no durability or ack: a late subscriber misses earlier messages; durable handoff belongs in #kv or a capsule. |
| `concepts/blocking-stream-eof-contract` | The Blocking-Stream EOF Contract | concept | developer | 900 | #pipe, #plumb recv, and #agent events block until data or true EOF (Ok(0)); a periodic 50ms re-check never falsely signals end-of-stream. |
| `concepts/end-to-end-hash-verification` | End-to-End Hash Verification | concept | developer | 900 | Every #cas read re-hashes bytes so a hostile peer cannot serve content under a wrong address; ingest is size-capped at write time. |
| `concepts/wanix-capsule` | wanix capsule — CAS-Backed World Snapshots | concept | all | 1300 | Freeze a world into content-addressed blobs whose deterministic manifest-blob hash is the capsule id; dedup, integrity-verified, fetchable over the mesh blob plane. |
| `concepts/send-agent-to-the-data` | Send the Agent to the Data (Mesh Reach) | concept | visionary | 1300 | Instead of dragging files to the compute, run the job ON the node holding the data against its fast local namespace, reverse-exporting the caller's files — Plan 9 cpu(1) over the open internet. |
| `concepts/approvals-as-files` | Approvals as Files (the Trust Gate) | concept | all | 1000 | Powerful actions park in #agent/<id>/pending; nothing runs until a human writes approve <req> to #agent/<id>/ctl. |
| `concepts/agents-as-operators` | Agents Are the Operators Namespaces Always Needed | concept | visionary | 1200 | Per-process namespaces and file-shaped services were too fiddly for humans but native to an LLM that reads by cat, mutates by write, and lists by ls. |
| `concepts/fakeengine-vs-codex` | FakeEngine vs codex (Exec Local-Trust-Only) | concept | developer | 1000 | The served #agent uses the deterministic FakeEngine; the real codex app-server engine is local-trust only on the CLI path; the file shape is identical. |
| `concepts/rooms-not-houses` | Rooms, Not Houses (Cheap Scalable Isolation) | concept | all | 1000 | Each program runs in a ~0.25 MB WebAssembly sandbox instead of a ~10 MB OS process — ~40x smaller — so thousands of isolated programs run cheaply, scaling with your cores. |
| `concepts/safe-for-untrusted-not-claimable` | Safe-for-Untrusted-Code Is Not Claimable Yet | concept | developer | 800 | Per-room memory isolation is real today, but Wasmtime CPU/memory hard limits (epoch/fuel preemption, linear-memory caps) are not wired up — so do not claim 'safe for arbitrary untrusted code'. |
| `concepts/traceable-namespaces` | Traceable / Forkable Namespaces (Provenance, Rewind) | concept | visionary | 1200 | Make agent work legible: every mutation points back to the command/task/agent-turn that caused it, and the past becomes a place you can mount, diff, fork, or restore. |
| `concepts/wanix-as-host` | Wanix-as-Host, not Browser-as-Host | concept | developer | 1100 | The Rust port moves the runtime boundary out of Chrome: Wanix is a native OS core, and the browser becomes one excellent client of it. |
| `concepts/serve-composition-surface` | Serve as One Local Composition Surface | concept | developer | 1100 | wanix-rust serve combines static HTTP, the discovery doc, direct 9P over WebSocket, the qjs-shell WebSocket, and the HTTP-app route on one listener without making the 9P server own HTTP/browser policy. |
| `concepts/discovery-document` | Discovery Document (/.well-known/wanix.json) | concept | developer | 1100 | A single JSON contract advertising p9/rootfs/qjsShell/httpApp/ethernet routes, the v86 block, services, and bundle — clients consume it instead of hard-coding routes. |
| `concepts/wanix-services-device-set` | --wanix-services Device Set | concept | developer | 1000 | Binds #task, #term, #kv, #pipe, #plumb, #cas, #agent into the served namespace from one INSPECTABLE_SERVICE_DEVICES source, advertised as services.devices. |
| `concepts/three-serve-bundles` | Three Serve Bundles | concept | user | 900 | fs9p (plain browser filesystem), workbench-fs9p (the cockpit / Code OSS shell), and direct-v86 (browser emulator handoff), each a generated HTML page that reads discovery. |
| `concepts/browser-cockpit` | The Browser Cockpit (Operator Surface) | concept | visionary | 1300 | A VS Code web extension that browses the namespace and drives every service device over direct 9P — inspect, agent-repair, duet, HTTP apps, self-check — as the human operator surface for the Rust runtime. |
| `concepts/direct-9p-operator-surface` | Direct-9P Operator Surface (No Side Channel) | concept | developer | 900 | Every cockpit operation is a 9P file read or write through WanixP9Handle; there is no MessagePort/CBOR bridge and no globalThis.Wanix in the Rust-hosted path. |
| `concepts/live-stream-vs-one-shot` | Live-Stream vs One-Shot 9P Access | concept | developer | 900 | p9.ts routes #term/#pipe/#plumb/#agent stream paths through walk+open at offset 0 instead of one-shot readFile/writeFile, because allocator-owned files reject create and subscriptions must not drain to EOF. |
| `concepts/http-app-route` | /.wanix/app/<name> HTTP Route | concept | developer | 1100 | A loopback HTTP route that resolves apps/<name>.{js,wasm}, allocates a #task, binds fds to trace files, runs it, and returns stdout with X-Wanix-Task-Id tracing headers. |
| `concepts/loopback-only-handoffs` | Loopback-Only VM and App Handoffs | concept | developer | 900 | rootfs.json, the HTTP-app route, and direct-v86 are gated to loopback clients and prepared-root boot markers — launch contracts, not a VM supervisor. |
| `concepts/single-frame-serve-caveat` | Single-Frame Serve Caveat (Live recv) | concept | developer | 800 | The single-connection serve handles one frame at a time, so a blocking #plumb recv cannot interleave with a send; live delivery needs a second connection. |
| `concepts/trust-boundary-gaps` | Trust-Boundary Gaps (Do Not Overclaim) | concept | developer | 1300 | The honest 'not yet' page: single-attach-per-connection, the discarded uname/aname seam, no public multi-user auth, no ethernet/vnet, #cpu cancel doesn't stop remote computation, blob plane experimental. |

### `devices` (8)

| Slug | Title | Type | Audience | ~Words | One-liner |
|---|---|---|---|---|---|
| `devices/task` | #task — The Task Device | reference | developer | 1100 | Allocate, configure, start, and observe processes purely through files: #task/new/<kind>, #task/<id>/{cmd,env,dir,exit,fd/<n>}, and #task/self. |
| `devices/term` | #term — The Terminal Device | reference | developer | 1200 | Plan 9-style terminal service: new allocates a resource exposing id/ctl/data/program/winch; data<->program cross-feed with \n->\r\n, winch broadcasts 'cols rows'. |
| `devices/kv` | #kv — Key/Value Store | reference | developer | 1100 | #kv/<key> is a file: read returns the value (snapshot-on-open), write commits the whole buffer on close, listing #kv enumerates keys. |
| `devices/pipe` | #pipe — In-Memory Byte Channels | reference | developer | 1000 | #pipe/new allocates a channel; <id>/data is a unidirectional read/write end with EOF on last-writer drop, composing tasks like a Unix pipe. |
| `devices/plumb` | #plumb — Plumber Bus | reference | developer | 1200 | #plumb/<topic>/send publishes a newline-JSON {kind,from,to,body} envelope; <topic>/recv reads envelopes received since it opened. |
| `devices/cas` | #cas — Content-Addressed Store (venti) | reference | developer | 1200 | #cas/<hash> reads a verified blob, #cas/ingest is write-then-read-hash, #cas/have/<hash> returns 1\n or 0\n. |
| `devices/cpu` | #cpu — Exec Plane (cpu(1) over the Mesh) | reference | developer | 1300 | A caller reverse-exports a scoped read-only namespace and a remote acceptor runs a task whose world is that export — Plan 9 cpu(1) over QUIC, grant-allowlisted. |
| `devices/agent` | #agent — an LLM Session as Files | reference | all | 1300 | new/prompt/events/reply/pending/ctl/status — the entire LLM contract reduces to read/write on a small file tree, with approvals as files. |

### `reference` (10)

| Slug | Title | Type | Audience | ~Words | One-liner |
|---|---|---|---|---|---|
| `reference/adr-index` | ADR Index & Workflow | reference | developer | 1500 | The active ADRs 0001-0005 plus the ADR-vs-commit-message workflow — review the related ADRs before touching any boundary. |
| `reference/crate-map-and-layering` | Crate Map & Dependency Direction | developer | developer | 1500 | Strict downward layering keeps core fs/namespace/protocol crates free of Wasmtime, and confines all async/iroh to the single wanix-mesh edge crate. |
| `reference/filesystem-trait` | The FileSystem / File Trait Reference | developer | developer | 1300 | The trait at hand: method-by-method, the defaults, device-aware semantics, and which methods default to NotSupported. |
| `reference/extension-points` | Extending Wanix from the Edges | developer | developer | 1300 | Three extension points — a service device is a FileSystem, a task driver is a TaskDriver, a transport is a 9P adapter — so you do not need to fork the core. |
| `reference/serve-and-discovery` | Serve, Discovery, and Handoff JSON | developer | developer | 1300 | The serve command surface, the discovery document shape, the well-known routes, and the rootfs/qemu/v86 handoff JSON contracts. |
| `reference/quality-gates` | Quality Gates (just check) | developer | developer | 900 | fmt, module-lines, clippy -D warnings, test, and the composite just check; stay under the 250-350 line module limit; use explicit newtypes over raw i32 flags. |
| `reference/contributor-landing` | Contributor Landing — This Is the Rust Port | developer | developer | 1000 | Orient: this is the Rust port, not the Go tree — ignore the Go Makefile/CONTRIBUTING; build with cargo and gate with just check. |
| `reference/cli-command-index` | CLI Command Index | reference | developer | 1200 | Every wanix-rust subcommand at a glance: qjs, qjs-term/-shell/-snapshot/-resume/-restore, wasm, p9-*, serve, rootfs, qemu, direct-v86, mesh-serve, mount-*, cpu, agent, capsule. |
| `reference/cli-rootfs-qemu-v86` | CLI: rootfs, qemu, direct-v86 (VM Handoffs) | reference | developer | 1100 | rootfs prepares/validates a guest root, qemu emits a wanix-qemu-virtio9p.v1 argv handoff, and direct-v86 generates a browser emulator page — launch contracts, not a VM supervisor. |
| `reference/queued-follow-ups` | Queued Follow-Ups (Pick a First Cleanup) | developer | developer | 1100 | The current backlog: split over-limit modules, port the v86-shared-demo stub, wire #plumb live recv, add serve shutdown+cap, typed discovery JSON, the per-principal namespace seam. |

### `use-cases` (6)

| Slug | Title | Type | Audience | ~Words | One-liner |
|---|---|---|---|---|---|
| `use-cases/personal-compute-mesh` | Your Personal Compute Mesh | use-case | visionary | 1300 | Your laptop, phone, and a cloud node each run a Wanix node; mount any one at /n/<peer> and operate its files and devices as your own, NAT-crossed over iroh QUIC, gated by per-peer grants. |
| `use-cases/agents-on-your-files` | AI Agents Operating on Your Files Across Devices | use-case | visionary | 1400 | #agent repairs a broken program with approvals-as-files; over the mesh it reaches a peer's #kv/#cas/files; two agents on two machines collaborate via #plumb. |
| `use-cases/portable-worlds` | Portable Worlds via Capsules | use-case | decision-maker | 1200 | wanix capsule save freezes a whole world (the directory an agent built) into a CAS-backed, BLAKE3-verified, deduplicated set of blobs; hand someone one hash and they materialize the entire world, verifying every blob. |
| `use-cases/browser-cockpit` | A Browser-Native Operator Cockpit | use-case | visionary | 1400 | A Code OSS / VS Code web extension drives the whole namespace over direct 9P: inspect the service devices, run the agent repair demo, run a qjs->wasm->qjs duet on one shared FS, serve HTTP apps backed by #kv, and self-check the device set. |
| `use-cases/distributed-dev-environments` | Distributed Dev Environments | use-case | decision-maker | 1200 | #cpu runs your task on the node holding the source against its fast local namespace, outputs returning over the wire — Plan 9 cpu(1) over the open internet, with a default read-only jail. |
| `use-cases/traceable-namespaces` | Traceable, Forkable, Rewindable Namespaces (Exploratory) | use-case | visionary | 1200 | An agentic runtime should make the chain prompt->plan->command->task->log->mutation->result visible, addressable, and forkable, so you can ask 'what changed, and why?' and rewind or fork the world. |

### `recipes` (7)

| Slug | Title | Type | Audience | ~Words | One-liner |
|---|---|---|---|---|---|
| `recipes/walkthrough-1-run-js` | Walkthrough 1 — Run JavaScript Outside Chrome | flow-step | user | 700 | Copy 'wanix qjs examples/qjs-demo.js' and see 'outside Chrome: true' — JavaScript running as a Wanix task, not in a browser. |
| `recipes/walkthrough-2-process-context` | Walkthrough 2 — Feed env, cwd, stdin, and argv into a Script | flow-step | user | 700 | Pass --env/--cwd/--stdin and -- argv and watch scriptArgs, std.getenv, fd 0, and #task/self/id flow in as task state — no globalThis.Wanix bridge. |
| `recipes/01-repair-broken-qjs` | Recipe 01 — Repair a Broken qjs Program with an Agent | use-case | user | 900 | Hand a crashing qjs program to a Wanix-backed agent from inside the cockpit and watch it propose a patch, ask for approval, and write the fix back — all as plain reads and writes on the #agent service. |
| `recipes/02-mount-remote-peer` | Recipe 02 — Mount a Remote Wanix Peer and Run a Job on It | use-case | power user | 1000 | Stand up two nodes, have A dial B over iroh QUIC, mount B's directory as a Plan 9 namespace, and send a #cpu job so the code runs on B against B's local data. |
| `recipes/03-freeze-world-to-capsule` | Recipe 03 — Freeze a Wanix World to a Portable Capsule | use-case | power user | 1000 | Freeze a directory the agent (or you) just built onto the content-addressed plane so the same world can be reconstructed deterministically anywhere the same capsule id is reachable. |
| `recipes/04-tiny-http-app-with-kv` | Recipe 04 — A Tiny HTTP App with #kv-Backed Counter State | use-case | user | 1100 | A single-file qjs handler at apps/counter.js increments a per-request counter held in #kv/counter — nothing about the state lives in the handler, only in #kv. |
| `recipes/05-two-agents-collaborate` | Recipe 05 — Two Agents Collaborate via #agent/<id>/reply | use-case | power user | 1000 | Agent A drives a rename refactor, spawns a second session B with a sub-goal, blocks on B's reply (single-read, EOF-terminating), and integrates the findings — all as files. |

### `find` (1)

| Slug | Title | Type | Audience | ~Words | One-liner |
|---|---|---|---|---|---|
| `find/index` | Find — Search, Concept Index, Glossary, Tags | overview | all, expert-favoring | 600 | The findability hub: full-text typed search (hotkey '/'), the graph-backed Concept Index (A-Z + clustered visual map), the A-Z Glossary, and the Tag browser. |

