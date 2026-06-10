# ADR 0007: Resources, Catalogs, and Pairing (DRAFT / PROPOSED)

## Status

**Proposed — draft for discussion.** This record is broader than a typical
terse ADR on purpose: it carries the *motivation story* for how a user builds
things in Wanix, then proposes the primitives that story implies, then collects
the open questions we still need to answer. Once the contracts here stabilize,
split the durable boundaries (the catalog format, the resource address model,
the host-wrapper service contract, the Layer-2/3 authorization model) into
consecutive accepted ADRs and let this draft retire into commit history.
The Layer-1 naming layer has since shipped — the `~/.wanix/catalog` address
book, `--register` self-registration, launch-time name resolution, and recipes
(build-order steps 5–8 below) — and the BinVerbs confinement contract is
accepted and pinned (its own section below); Layer 2/3 authorization remains
the open half.

---

## The story: how you build things in Wanix

Here is the user's-eye view we are building toward. Read this as the product,
not the implementation.

You keep a **catalog** of resources. Each entry is just a name, a description,
some tags, and an address. The catalog is *yours* — it is your address book for
everything you can reach across your machines and your collaborators' machines.

The resources in your catalog are things you (or others) created:

- A **data volume** — a read/write filesystem you spun up to hold photos,
  notes, datasets, app state. You created it with a standalone command, or you
  parked it on a small **volume server** (one Wanix binary that serves several
  named volumes, each with its own resource ticket). Either way, the volume is
  now a resource, so it goes in the catalog.

- A **service** — you took a normal local CLI program on your Mac (say,
  `whisper` for speech-to-text) and wrapped it with a **host wrapper**. The
  wrapper runs on your Mac, has its own address, and exposes that one program as
  a Wanix device you talk to through files (write input, read output, drive it
  with a `ctl` file). That, too, is a resource in the catalog.

- An **agent**, a **compute endpoint**, a **frozen capsule**, or another whole
  **node**. All resources. All catalog entries.

Then, when you want to build something — an interactive session, a script, a
small web service — you go shopping in your own catalog. *"Let's grab that macOS
Whisper STT service."* It mounts at `/n/whisper`. *"And my notes volume."* It
mounts at `/vol/notes`. Now you have a **namespace** assembled from your
resources, and you use it however you like: from `qjs`, from a shell, from a
`wasm` task. You write audio bytes to `/n/whisper`, you read text back. You
write a file to `/vol/notes`. The program does not care that Whisper is a
subprocess on a laptop three rooms away — it is just a file in the namespace.

And the act of "grab these resources and wire them into a namespace" is itself
something you can name and save — a **recipe** — which is, of course, another
resource in the catalog.

That is the whole story. *Make resources. Catalog them. Compose a namespace by
name. Use it.*

---

## The reframe: composition is already built; naming and wrapping are not

The reason this story is reachable is that the hard machinery already exists.
What is missing is mostly a *naming layer* and one *new kind of device*.

**Already shipped (the mechanism):**

- The mesh imports a remote namespace over the native FileSystem-over-iroh wire
  (`WANIX_FS_ALPN`) at `/n/<peer>`, where `<peer>` is the raw 64-hex ed25519
  public key (`crates/wanix-mesh/src/dialer.rs`).
- Every device is a plain `FileSystem`, so `#kv`/`#cas`/`#plumb`/`#agent`/`#cpu`
  import across the mesh for free (`/n/A/#kv/...`).
- Binding an imported `FileSystem` into a `Namespace` *is* "compose resources
  into a namespace." On the mesh this import is `NativeFs`; on the foreign edge
  (`tcp://`, Linux/v86/QEMU, external 9P clients, the current cockpit) it is
  `RemoteFs`.
- Streaming files (never-EOF reads such as `#agent/<id>/events`) ride their own
  native-wire bidi stream by construction, so a blocking read does not freeze
  sibling operations.
- `mesh-serve --wanix-services` exports a namespace of devices over the native
  mesh wire; `serve --wanix-services` keeps exporting the same shape over the
  direct 9P edge for browser/editor clients.
- Layer-0 identity is already cryptographic: iroh authenticates the peer's
  ed25519 pubkey on every connection. The existing `AttachPolicy`/`GrantTable`
  machinery is the enforcement primitive, but the v1 native wire currently
  resolves only the empty/root scope. The v0 resource-server path should avoid
  non-root scope entirely by making each ticket address exactly one resource
  root. Native subresource selection is a later optimization/authorization shape,
  not required for the first volume/catalog loop.
- **Persistent mesh mounts into a task/shell namespace (shipped).**
  `--mount-mesh iroh://...=/vol` dials a remote namespace over the native wire
  and binds it into the task namespace, holding the dialer keepalive for the
  session lifetime; repeated flags compose several resources into one namespace.
  The keepalive home is the shared `crates/wanix-cli/src/mesh/mounts.rs`
  plumbing, carried by `qjs-shell`, `wasm`, and `sh`. (Open remainder: threading
  the flag to `qjs`/`qjs-term`.)
- **A single-process per-resource volume server (shipped).** `wanix volume
  create` makes persistent volumes under `~/.wanix/volumes`; `wanix volume serve
  (--volume NAME ... | --all)` runs one native mesh endpoint per volume — each
  with a distinct, stable identity under `~/.wanix/volume-identities` (outside the
  exported root) — and prints one ticket per volume. Clients compose the tickets
  with repeated `--mount-mesh`; there is no aggregate root. (`mesh-serve --volume
  NAME` remains the single-volume debug shorthand.)

> Wire-protocol direction (ADR 0004): the Wanix↔Wanix mesh path is now the
> hand-rolled native `wanix-mesh-wire` over iroh QUIC, not 9P-over-iroh and not
> `irpc`. 9P is quarantined to the foreign edge (Linux/VM, external 9P tools,
> `tcp://`, and the current cockpit). The catalog/recipe story composes
> `FileSystem`s into namespaces regardless of encoding, but its mesh-facing
> resource and authorization shapes should be designed for the native wire.

**The missing half at drafting time — since shipped:**

- **A persistent registry of resources** — shipped as the local catalog:
  `wanix catalog add/show/rm/ls`, one JSON entry per name under
  `~/.wanix/catalog/`, `--register NAME` on every resource serve, and
  launch-time resolution of bare names at every ticket-accepting surface
  (`--mount-mesh`, the `mount-*` verbs, `serve --bind`, recipe binds).
- **Projecting a single local CLI program onto the mesh** — shipped as ToolFS
  (ADR 0009) plus the `ProcRunner` fixed-policy inversion: `tool serve
  --config tools.toml` wraps real host programs with operator-fixed
  command/argv, no shell, empty child env, and real timeout/abort kills.
  `#cpu` remains the arbitrary-caller-supplied-spec direction, not this.
- **A named "wire these together" artifact** distinct from a state snapshot —
  shipped as recipes (`recipe save/run`, TOML under `~/.wanix/recipes/`).

So the remaining work is pairing/authorization and sync — not naming,
wrapping, or composition.

---

## Proposed primitives

### 1. The catalog — and it is just a volume

A catalog entry is:

```
{ name, description, tags[], address }
```

The catalog itself is a **data volume**, which means the *first* thing we build
with the volume primitive is the catalog — and it syncs across your devices over
the mesh like any other volume. Model the device on `#kv`/`#agent`: each entry
is a file; listing the directory lists your resources; tags enable queries
("grab all my STT services").

The `address` is a **tagged union**, and the tag dictates how to mount and
whether the resource can be offline:

- `iroh://<pubkey>[?addr=...]` — a **live** node, volume, or service. In the v0
  resource-server model the endpoint root *is* the resource; a volume server may
  run many endpoints and print one ticket per volume. May be down; needs a
  probe.
- `cas:<hash>` — a **static** blob or `.wcap` capsule. Always loadable, never
  offline.
- `local:<path>` — something on this machine.

This live-vs-static split is real and should be explicit from the start: a
capsule is a frozen *world*, a CAS blob is frozen *data*, an iroh entry is a
*live reference* that can fail to attach. The catalog unifies them under
name/tags but they do not behave the same.

### 2. The volume server

"A single Wanix binary that exposes volumes." The important v0 shape is **one
process, many resource endpoints**, not one aggregate `/vol/*` root. The process
owns local volume dirs and starts one native mesh endpoint per served volume:

```
volume server process
  notes   -> MeshNode A -> iroh://A...?addr=...  (root = notes LocalFs)
  photos  -> MeshNode B -> iroh://B...?addr=...  (root = photos LocalFs)
  data    -> MeshNode C -> iroh://C...?addr=...  (root = data LocalFs)
```

The client remains the composer:

```
qjs-shell \
  --mount-mesh iroh://A...?addr=...=/vol/notes \
  --mount-mesh iroh://B...?addr=...=/vol/photos
```

This keeps the bearer-capability story simple: if someone has the `notes`
ticket, they can reach the `notes` resource root; they cannot discover `photos`
by editing a path component in the ticket. It also works with the native wire as
implemented today because `dial_native` asks for the endpoint root and the
endpoint root is exactly the resource.

`wanix volume create photos` is a thin persistent-dir wrapper. A single-volume
serve (`mesh-serve --volume photos`) is still a useful debugging/development
shorthand, but the resource-server mode should print/list one ticket per served
volume rather than exporting all volumes under one shared namespace.

Remote volume administration is a separate resource, not an accidental side
effect of serving data volumes. Creating/deleting/renaming volumes and editing
ACLs stays CLI-local until an explicit `#volume`/admin device and authorization
model exists.

### 3. The host wrapper — the genuinely new device

"Expose a local CLI program as a service called via a `ctl` file." The critical
design decision: **do not build this on `#cpu`.**

`#cpu` is the dangerous direction — the *caller* supplies the job spec (program,
argv, env), i.e. arbitrary remote exec (`crates/wanix-cpu/src/spec.rs`). The
host wrapper is the security **inversion**: the **host** freezes the program and
the argv template; the caller supplies only *input data*. That inversion is what
makes it safe to project your Mac's Whisper onto the mesh.

Architecturally the wrapper is a near-clone of `#agent` (already the codebase's
template for "wrap a stateful local thing as files"):

```
#whisper/
  new            → read allocates a session, returns id
  <id>/in        → write input bytes (audio)
  <id>/ctl       → write "run\n"   (or auto-run on close-of-in)
  <id>/out       → read output (blocking until ready)
  <id>/status    → read state
```

The wrapper process binds an iroh endpoint from its own `~/.wanix/node.key`,
exports that one device over the native mesh wire, and spawns
`whisper <fixed-flags>` per session, feeding `in`→stdin and stdout→`out`. It can
run open in the Layer-1 prototype; once Layer 2 lands it should default-deny
through the same per-resource endpoint ACL machinery as volumes.

Two contract flavors, declared per service:

- **request/response** (Whisper: write a whole file, run, read transcript) —
  works today, even over the single-frame serve connection.
- **streaming** (live transcription: `in`/`out` are pipes) — needs the
  concurrent-frame work (see Open Questions). Ship request/response first.

### 4. The recipe — wiring, as distinct from state

A capsule freezes *state* (CAS-backed, `wanix capsule`). A **recipe** freezes
*wiring*: a list of `(catalog-name → mount-path)` binds plus what to launch,
resolved against the catalog at launch:

```
# whisper-notes recipe
bind  whisper-stt  -> /n/whisper
bind  my-notes     -> /vol/notes
run   qjs transcribe.js
```

A recipe is itself a catalog resource, so a whole workspace is shareable; the
recipient resolves names against their own catalog (or you ship addresses
inline). This is the artifact that turns "a protocol" into "a thing you hand
someone."

**Convergence to watch:** an AppResource ([docs/appfs.md](../appfs.md)) is a
recipe (the manifest's mounts = wiring) plus CAS-pinned code plus a state
volume; a capsule freezes state; a tier-1 task snapshot (ADR 0010) freezes a
running task. These should become one CAS-rooted artifact *family* with one
loader, not three freeze formats — restructure this section around that once
the AppResource manifest stabilizes (discussion in
[docs/design/agent-native-next-layer.md](../design/agent-native-next-layer.md)).

---

## Trust: two separate problems, not one

It is tempting to call trust one "two-sided" problem (my catalog ↔ their grant)
and treat the catalog UX as fundamentally a *pairing* flow. That conflates two
genuinely independent concerns:

1. **Naming / reach** — how I *refer to and connect to* a resource. This is the
   catalog. One-sided: my bookmark.
2. **Authorization** — *whether I may use it*, and *who decides*. This is access
   control.

They look welded only because the current default is *deny*, so today you cannot
use a resource without a grant existing. But that is a default choice, not an
inherent coupling — and "open" already exists in the code as `mesh-serve
--insecure-open`. Flip the default and the two fully decouple. **You can build
the entire catalog / volume / host-wrapper / recipe story running fully open,
with zero authorization work.**

It is worth laying the concerns out as layers, because the bottom one is free
and changes what "open" even means:

- **Layer 0 — Identity & authentication (already done).** iroh authenticates
  *every* connection by the caller's ed25519 pubkey
  (`crates/wanix-id`, `crates/wanix-mesh`). You **always know who is visiting**,
  cryptographically, even in open mode. There is no anonymous visitor here;
  "open" just means *you do not check the pubkey against an allow-list*. So
  "who is this person accessing me?" is already answered — the only question is
  whether you *act* on it.

- **Layer 1 — Naming / reach (the catalog).** One-sided, my bookmark. Needed to
  build. This whole document is mostly Layer 1.

- **Layer 2 — Authorization (per-resource ACL).** Start open, then per-resource
  allow-lists keyed on the Layer-0 pubkey. The default-deny policy machinery
  (`AttachPolicy`/`GrantTable` in `crates/wanix-id`) is already the enforcement
  primitive, but the operator surface is still 9P-shaped and static
  (`--grant ANAME:PREFIX:RIGHTS --peer <hex>`). With per-resource endpoints, the
  first private shape can authorize the endpoint root itself. Scoped subresources
  become necessary only when one endpoint intentionally exposes more than one
  independently grantable resource.

- **Layer 3 — Ownership & delegation.** Who may *edit* a resource's ACL, can
  access be delegated, can it be revoked. This is the genuinely unbuilt deep
  end: the current grant model has no owner-principal-who-edits-grants and no
  delegation.

  **Layer-3-lite already works locally.** Within one node, delegating an
  attenuated capability to a sub-agent needs no certificates at all: bind the
  sub-agent's task a *subset namespace* (fewer mounts, narrower roots) — the
  namespace is the attenuation (ADR 0000). Certificates become necessary only
  when the delegate crosses the mesh and must prove its attenuated authority
  to a third party. The decided direction there is delegation certs
  (UCAN/biscuit-shaped: ephemeral agent key + cert chain to the node key,
  scoped and expiring; expiry-as-revocation), designed together with the
  native-wire attach scope — see the convergence note in ADR 0004 §Open
  questions. ACLs stay keyed on stable node identities; attenuation rides the
  cert, so peers never manage ephemeral agent keys.

### Why you can build open but cannot stay open

"Open = anyone who knows your address" is *bearer-capability* semantics: the
iroh address (a 256-bit pubkey) functions like a secret link. That is a
legitimate v0 — but there is a trap specific to *this* project. **The catalog is
an address-distribution machine.** Its entire purpose is to spread, name, and
share addresses. So in open mode, the thing that must stay secret (the address)
is exactly the thing the system is built to pass around: your only protection
and your core feature are in direct conflict, and the catalog erodes its own
protection the more you use it. That is why "security by obscurity" is the
precise phrase, and why the honest sequencing is **build open (Layer 0+1), and
treat Layer 2/3 as a deliberately deferred — but inevitable — second project**,
not an optional nicety.

There is also an irreducible **bootstrap root of trust**: to import your catalog
onto a new device, that device must already trust one address — your home node /
your own identity. One root, everything else hangs off it. That is Plan 9's
auth-server model; we just need to name it so it does not surprise us later.

---

## Confinement contract: BinVerbs (accepted, shipped)

Resources ship their vocabulary: a served resource may expose executable verbs
as plain files in a `bin/` directory beside its tree, and a client that mounts
the resource gains those verbs — confined by default. This subsection is the
pinned contract (the rest of this ADR remains a draft).

**Serving.** `bin/` is HOST-served, never guest-routed: `app serve` exposes
`<app-dir>/bin/*` when the directory exists, and `tool serve --config` takes an
optional per-tool `bin` directory key. The surface is read-only, flat (one
level of plain files), and size-capped (`wanix_fs::VerbBinFs`,
`MAX_VERB_FILE_BYTES`); an over-cap file is refused at open with an honest
error, never streamed. The resource's own root listing shows `bin/`.

**Invocation.** The shell form is `NAME:CMD [args]`: `NAME` must be a current
mount (`/n/NAME` or `/vol/NAME`, tried in that order) and `CMD` must resolve to
exactly one of `bin/CMD.js` or `bin/CMD.wasm` under it (none and both are
honest 127s). There is deliberately **no `$PATH` merging** of mounted `bin/`
directories: a verb only ever runs qualified by the resource it came from, so a
hostile mount can never squat an unqualified command name.

**Confined execution.** A verb runs as an ordinary tier-2 child task
(ADR 0010, driver auto-selected by extension) whose namespace contains
EXACTLY: the resource bound at `res`, the stdio fds the launcher bound, and
the argv/env it passed explicitly. No `#task`, no `#pipe`, no other mounts, no
host directories. The seam is the `#task` ctl verb `confine <mount-path>`
(`Task::confine_to`): the launcher binds fds (resolved against the inherited
namespace), then confines, then starts; a started task can never be confined.
The verb's bytes are read through the mount itself — the program and the
authority it gets arrive together, and that authority is exactly its source.
Widening (granting a verb anything beyond its own resource) is a future
explicit, visible act — an `--allow`-shaped flag — never a default; it is
deliberately unimplemented today.

Proofs: `crates/wanix-wasm/src/driver.rs` (BinVerbs tests: post/probe verbs,
pipeline composition, the confinement probe, squatting safety) and
`crates/wanix-cli/src/app/serve/verb_tests.rs` (a live loopback room: verbs
invoked over a mesh mount, the served `bin/` surface, the size cap on the
wire). Concept walkthrough:
[docs/site/content/concepts/bin-verbs.md](../site/content/concepts/bin-verbs.md).

---

## Worked example: a chatroom

*Implemented:* this example now runs — `wanix app serve` exports
`examples/chatroom` behind the `crates/wanix-appfs` file2chan adapter with
transport-verified attribution, `who` presence, and the never-EOF `stream`
(design: `docs/appfs.md`; proofs: `crates/wanix-cli/src/app/serve/tests.rs`).

A chatroom is the whole trust model in miniature, which is why it is worth
recording here rather than as a standalone feature. It also makes every abstract
layer concrete with semantics nobody has to be taught (*who said that*, *who is
online*, *is this room private*, *can I kick someone*).

**The room is the resource.** Someone runs a chat server that exports a
namespace; you mount it at `/n/general` and find:

```
/n/general/
  post      → write a message (body only)
  stream    → never-EOF read; new messages arrive as lines
  latest    → bounded read, last N messages
  who       → read the current participants (presence)
```

This is `#plumb` plus three additions — history (`latest`), presence (`who`),
and authenticated attribution — so the shape is already proven (`stream` is the
same never-EOF pattern as `#agent/<id>/events`). The host is the serialization
hub: every `post` funnels through it, it assigns order, and it fans out to all
`stream` readers. (Whether the room can outlive the host — a replicated/CRDT log
— is a *different layer*, replication, not the resource contract; out of scope
here.) Note a chatroom is the worst case for the single-frame serve websocket
(you block-read `stream` while writing `post`), but the native mesh path already
handles it by giving every open file its own bidi stream, so a chatroom is
naturally mesh-native.

What it teaches about identity, layer by layer:

- **Attribution and presence are free from Layer 0.** When you write to `post`,
  the server already knows your authenticated ed25519 pubkey from the mesh
  attach. So it **stamps each message with the authenticated principal and
  ignores any client-claimed author** — `post` carries the body only. There is
  no login and no spoofable username field, because identity comes from the
  transport, not the payload. This is Plan 9's "file server tags writes by the
  attached user," except the trusted `uname` string is replaced by a
  cryptographic pubkey — stronger than the model it imitates. `who` is just the
  set of currently-attached (or currently-`stream`-reading) principals.

- **Human names are *not* free — this is the petname problem.** The pubkey is
  unforgeable but unreadable; you want `stream` to say `jesse:`, not
  `ed25519:4f3a…:`. Who vouches that a pubkey is "jesse"? This is Zooko's
  triangle (secure / human-meaningful / global — pick two). The clean answer
  falls out of Layer 1: **the catalog is the petname store.** The same address
  book that names resources names *people* — each viewer sees the names *they*
  chose for the pubkeys they have met. Display names are local, like petnames.

- **Public vs private is Layer 2; moderation is Layer 3.** An open room is the
  bearer-capability v0 (anyone with the address joins — and the
  catalog-spreads-the-address trap is vivid here: one screenshot leaks entry). A
  private room is an allow-list of pubkeys (Layer 2 ACL, keyed on the same
  Layer-0 identity). **Kick** = remove from ACL + clunk their sessions,
  **invite** = add a pubkey, **"ops can invite"** = delegation — Layer 3, with
  semantics that cannot be misread.

**The chatroom is the forcing function for principal-aware resources.** The
native mesh handler already learns the verified peer from iroh and resolves a
per-connection root from it, but the core `FileSystem` trait is intentionally
principal-blind. A chatroom needs the authenticated pubkey to reach the resource
implementation as the acting principal for `post`, `stream`, and `who`. That may
be a principal-scoped wrapper returned at attach time, a device-local session
context, or a small resource-specific extension — but it should be designed as a
native mesh identity seam, not as a client-claimed `uname`. When this plumbing is
built, promote it to its own accepted ADR.

---

## Suggested build order

Front-load visible payoff (a real remote volume in a real shell); run open to
start; defer catalog naming and authorization until the address-based bind path
is solid.

0. **Native volume proof (shipped).** Keep the manual invariant green: one process
   runs `mesh-serve --root DIR --addr 127.0.0.1:PORT --insecure-open`, another uses
   `mount-cat`/`mount-write iroh://...` and the write lands on the served root.
   This proves the production native server and production native client speak
   the same wire.
1. **Persistent mesh mount into a shell/task namespace (shipped).** `qjs-shell
   --mount-mesh iroh://...=/vol` keeps the dialer `IrohMount` alive for the
   session because it owns the runtime handle used by the native import.
2. **Two shells, one open volume (shipped).** Two shell sessions with the same
   `--mount-mesh` volume; one writes `/vol/x`, the other reads it — the
   resource-composition experience before naming exists.
3. **`wanix volume create` and per-resource volume serving (shipped).** Persistent
   local volume dirs under `~/.wanix/volumes/<name>`; `mesh-serve --volume NAME` is
   the single-volume debug shorthand, and `volume serve (--volume ... | --all)`
   runs one native endpoint per volume (distinct identity each) and prints one
   ticket per volume.
4. **Client composition across multiple served resources (shipped).** Repeated
   `--mount-mesh` flags mount two different volume tickets into one shell/task
   namespace, with writes scoped to their target volumes.
5. **The catalog (shipped — as a CLI store, not yet a device).** `wanix
   catalog add/show/rm/ls` keeps one pretty-printed JSON entry per name under
   `~/.wanix/catalog/` (name grammar lowercase `[a-z0-9-]`, never spellable as
   a ticket or path); `catalog ls` probes liveness through the exact mount
   dial path (online/offline/unknown, bounded deadline, chunked fan-out);
   v0 addresses are live `iroh://` tickets, with `cas:`/`local:` reserved and
   refused with a pointer. The `#catalog`-device-backed-by-a-synced-volume
   shape is deliberately ahead (open question 2).
6. **Register entries and mount by name (shipped).** Every resource serve
   takes `--register NAME` (one endpoint registers as NAME, several as
   `NAME-<resource>`; re-announce refreshes the route hint), and a bare name
   resolves through the catalog at LAUNCH time — logged once per name on
   stderr — at every ticket-accepting surface: `--mount-mesh NAME[=GUEST]`
   (sh/wasm/qjs-shell), `mount-ls/cat/write`, recipe binds, and `serve --bind`
   (no entry falls back to a relative dir; `./name` forces the dir). There is
   no separate `wanix mount` verb; mounting by name rides the existing flags.
   `cpu --node` still takes tickets only.
7. **The host wrapper (shipped).** The wrapper device became ToolFS (ADR 0009,
   docs/toolfs.md): `wanix tool serve --tool NAME` runs one native endpoint +
   ticket per tool, every connection bound to a principal-scoped `jobs/` view
   from the verified `remote_id()`, and the wanix-sh `tool` builtin drives it
   through the visible files (the composed walkthrough is recipe 06). The
   fixed-command process runner shipped too: `tool serve --config tools.toml`
   wraps real host programs through the `ProcRunner` fixed-policy inversion
   (recipe 08), request/response only.
8. **Recipes (shipped).** `recipe save NAME --mount ... [--run LINE]` authors
   one TOML file at `~/.wanix/recipes/<name>.recipe` (binds + optional run
   line; no magic capture); `recipe run` resolves binds through the catalog at
   launch with the saved address as a drift-check hint (see open question 6),
   and a run-less recipe opens an interactive `sh` over the mounts.
   Remaining: `recipe ls/show/rm`.
9. **Authorization (Layer 2) and ownership/delegation (Layer 3).** Start with
   per-resource endpoint ACLs. Re-evaluate scoped subresources only if one
   endpoint must intentionally expose multiple independently grantable resources.
10. **Streaming wrappers and cockpit/serve edge concurrency.** The native mesh
   path is stream-per-open-file already; the remaining concurrency problem is the
   9P websocket edge.

---

## Open questions (the discussion below the story)

These are the seams we should talk through before committing contracts.

1. **The authorization model (Layer 2/3) — the deferred-but-inevitable
   project.** Naming (the catalog) does not need this; security-that-is-not-
   obscurity does. What is the per-resource ACL — a file under the resource
   (`#whisper/acl`) listing allowed pubkeys? In the per-resource endpoint model,
   the first ACL protects the endpoint root. Who is the *owner* that may edit it,
   and how is ownership established? Can a grantee *delegate* (re-grant) access,
   and can grants be *revoked*? Where does the optional pairing handshake fit —
   out-of-band ticket exchange, a `#pair` device, or an interactive
   approve-this-key prompt on the host (like the `#agent` approval files)? Note
   that Layer 0 already hands every resource the caller's authenticated pubkey,
   so this is purely a policy/ownership question, not an identity one.

2. **Catalog as a synced volume vs. local-only.** Single-node first is obvious.
   But multi-device sync needs a merge story (last-writer-wins? CRDT? a
   designated home node that is the source of truth?). And the bootstrap
   chicken-and-egg: how does device B first learn device A's address?

3. **Liveness / status.** Catalog entries point at nodes that may be offline,
   unlike CAS/capsule entries which are always loadable. *Partially answered
   (shipped):* `catalog ls` probes each entry through the exact mount dial
   path within the shared deadline and renders online/offline/unknown, where
   "offline" is ADR 0008's outage shape ("no route authenticated within the
   deadline") — which, pre-ACL, is all Layer 1 can know. Still open:
   distinguishing "the host is down" from "you are not granted" in the UX,
   which arrives with Layer 2.

4. **Host-wrapper safety surface.** Fixed program + argv template is the core
   safety property. But what about: environment leakage, working directory,
   filesystem access the wrapped program inherits, resource limits, and
   concurrent sessions? How much of `#cpu`'s `ExportScope` confinement do we
   reuse, and how do we keep the wrapper from drifting back into arbitrary exec?

5. **Streaming over the single-frame serve connection.** Request/response works
   today; live streaming hits the "one frame at a time per connection" limit on
   the 9P websocket edge (a blocking `recv` cannot interleave with a write). The
   native mesh wire (ADR 0004: one stream per call / per open file over iroh)
   makes this a non-issue on the Wanix↔Wanix path by construction. Do we fix
   serve concurrency for cockpit clients, or keep streaming resources mesh-native
   until the cockpit becomes a composing native client?

6. **Naming collisions and scope.** `/n/<peer>` is globally unambiguous (it is a
   pubkey). `/n/whisper` is a *local* alias resolved through *my* catalog.
   *Answered (shipped):* the resolution rule is — names resolve through the
   *resolver's own* catalog at LAUNCH time only (the spec/namespace always
   carries the resolved address; a rebind affects the next launch), each
   resolution is logged once per name on stderr for the audit trail,
   resolution never probes, and names are sugar over keys, never authority.
   Recipes carry **both, address-as-hint**: `recipe save` records the
   resolved address beside the name as a drift-check hint; at `recipe run`
   the catalog wins and drift warns loudly, and a vanished catalog entry
   falls back to the recorded hint. So a shared recipe resolves against the
   recipient's catalog by design — the hint makes drift visible, not
   authoritative.

7. **Address format / portability.** *v0 answered (shipped):* the catalog
   `address` is the existing `iroh://<hex>[?addr=...]` ticket string naming
   one resource root, and the name grammar (lowercase `[a-z0-9-]`) guarantees
   a name can never be mistaken for a ticket or a path at any resolution
   point, so every surface routes on spelling alone. `cas:` and `local:` are
   reserved kinds, refused with a pointer until a loader exists. Still open:
   whether a richer typed value is ever needed, and how `cas:`/`local:`
   entries behave in one resolver when they land.

8. **When to re-evaluate scoped subresources.** Per-resource endpoints are the
   simple default and have a useful bearer-capability property: knowing the
   `notes` ticket does not imply any way to guess or request sibling resources.
   Revisit native subresource selection only after one of these pressures is
   real:

   - endpoint-per-resource overhead is measurably painful (too many sockets,
     runtimes, relays, identities, or printed tickets for ordinary use);
   - users need one stable host identity with many resources addressed beneath it
     for pairing, reputation, petnames, or revocation UX;
   - a resource genuinely has independently grantable children that should share
     one connection/identity (for example a managed project workspace with
     sub-volumes); or
   - catalog/recipe portability clearly benefits from `host + resource-id`
     records more than opaque per-resource tickets.

   If that point comes, do not import 9P's `aname` ceremony blindly. Define a
   native resource selector, decide whether it is in the ticket URL or a
   structured catalog field, and require the server to authorize
   `(verified-peer, selector)` before returning the scoped `FileSystem`.

9. **Display names / petnames (the chatroom surfaces this).** Layer 0 gives
   unforgeable but unreadable pubkeys. Human names are Zooko's triangle. Is the
   answer purely catalog-as-petname-store (each viewer names the pubkeys they
   have met), or do some resources (a chatroom) also want an owner-assigned
   name map? How does a petname assigned in the catalog flow into a rendered
   `stream`/`who`?

10. **Principal-aware resources on the mesh wire.** The chatroom is the first
   consumer that needs the authenticated pubkey to reach the resource
   implementation as the acting principal (so `post` can be attributed and `who`
   can be computed). The native mesh wire (ADR 0004) is the place to build this
   from the verified iroh `remote_id()`, rather than retrofitting 9P's
   client-claimed `uname`. Open: whether this is modeled as a per-connection
   principal-scoped filesystem wrapper, a device-local session context, or a
   narrower resource-specific trait. (ToolFS and AppFS v0 both shipped the
   first shape — a per-connection `open_view(principal)` wrapper bound from
   the verified `remote_id()`; whether that generalizes is still open.)

11. **Where does this become ADRs?** Likely five durable boundaries: the catalog
    format, the resource address model, the host-wrapper service contract, the
    Layer-2/3 authorization model, and the principal-aware resource seam.
    Confirm that split before promoting any of them out of this draft. Already
    promoted since this draft was written: the invocation convention the
    host-wrapper implies is ADR 0009 (the job protocol), and the agent-side
    consumer story this document serves is ADR 0000 (the agent-native
    manifesto) — both should constrain the remaining five.

---

## Consequences (if accepted)

Wanix gains a humane front door: users address resources by name, create new
resources (volumes, services) as first-class catalog entries, and compose
namespaces from their catalog rather than from pasted cryptographic addresses.
The mesh's existing composition machinery is exposed, not extended. Naming
(Layer 0+1) ships first and runs open; the new trust-boundary surface —
authorization (Layer 2/3), host-wrapper confinement, catalog sync — is where the
real design risk concentrates, and it should be recorded as its own accepted
ADR(s) once stabilized, because it changes the mesh trust boundary established by
the `wanix-id` grant model. Building open is acceptable as a v0, but the catalog
is an address-distribution machine, so "open" is self-undermining by design and
Layer 2/3 is inevitable rather than optional.
