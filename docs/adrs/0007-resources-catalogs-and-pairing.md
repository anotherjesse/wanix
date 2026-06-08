# ADR 0007: Resources, Catalogs, and Pairing (DRAFT / PROPOSED)

## Status

**Proposed — draft for discussion.** This record is broader than a typical
terse ADR on purpose: it carries the *motivation story* for how a user builds
things in Wanix, then proposes the primitives that story implies, then collects
the open questions we still need to answer. Once the contracts here stabilize,
split the durable boundaries (the catalog format, the host-wrapper service
contract, the Layer-2/3 authorization model) into consecutive accepted ADRs and
let this draft retire into commit history.

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
  parked it on a small **volume server** (one Wanix binary that exposes several
  named volumes). Either way, the volume is now a resource, so it goes in the
  catalog.

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

- The mesh imports a remote namespace at `/n/<peer>`, where `<peer>` is the raw
  64-hex ed25519 public key (`crates/wanix-mesh/src/dialer.rs`).
- Every device is a plain `FileSystem`, so `#kv`/`#cas`/`#plumb`/`#agent`/`#cpu`
  import across the mesh for free (`/n/A/#kv/...`).
- Binding a `RemoteFs` into a `Namespace` *is* "compose resources into a
  namespace."
- Streaming files (never-EOF reads such as `#agent/<id>/events`) already get a
  dedicated bidi stream so a blocking read does not freeze the import
  (`StreamingImportFs`, `crates/wanix-mesh/src/dialer.rs`).
- `serve --wanix-services` already exports a namespace of devices over 9P/QUIC.
- Attach is capability-gated, default-deny, with per-peer grants
  (`crates/wanix-cli/src/mesh/grant.rs`, `crates/wanix-id`).

> Wire-protocol direction (ADR 0004): the mesh currently tunnels 9P over iroh,
> but the decided direction is a **native FileSystem-over-iroh wire (`irpc`)**
> for Wanix↔Wanix, with 9P quarantined to the foreign edge (Linux/VM, external
> 9P tools, the current cockpit). The catalog/recipe story below is wire-agnostic
> — it composes `FileSystem`s into namespaces regardless of which encoding
> carries them — so it is unaffected by that change.

**Not yet built (the story's missing half):**

- **There is no persistent registry of resources anywhere.** Identity persists
  at `~/.wanix/node.key`, but addresses are pasted by hand as
  `iroh://<64-hex>[?addr=IP:PORT]`. Even `#plumb` admits its topic list is "a
  best-effort directory, not a global registry" (`crates/wanix-plumb/src/lib.rs`).
- **There is no way to project a single local CLI program onto the mesh
  safely.** `#cpu` exists, but it runs *arbitrary* caller-supplied job specs —
  the wrong direction for "expose exactly this one capability of my machine."
- **There is no named "wire these together" artifact** (a recipe) distinct from
  a state snapshot (a capsule).

So the work is naming, wrapping, and pairing — not composition.

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

- `iroh:<pubkey>[+addrs]` — a **live** node or service. May be down; needs a
  probe.
- `cas:<hash>` — a **static** blob or `.wcap` capsule. Always loadable, never
  offline.
- `local:<path>` — something on this machine.

This live-vs-static split is real and should be explicit from the start: a
capsule is a frozen *world*, a CAS blob is frozen *data*, an iroh entry is a
*live reference* that can fail to attach. The catalog unifies them under
name/tags but they do not behave the same.

### 2. The volume server

"A single Wanix binary that exposes any volumes." This is `serve
--wanix-services` generalized to *N named data volumes*, each registering a
catalog entry on creation. `wanix volume create photos` makes a volume and emits
its catalog entry. Mostly orchestration over existing machinery.

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
exports that one device, default-denies attach, and spawns
`whisper <fixed-flags>` per session, feeding `in`→stdin and stdout→`out`.

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
  allow-lists keyed on the Layer-0 pubkey. The default-deny grant table
  (`AttachPolicy`/`GrantTable` in `crates/wanix-id`, `--grant
  ANAME:PREFIX:RIGHTS --peer <hex>` in `crates/wanix-cli/src/mesh/grant.rs`) is
  *already the enforcement primitive* — it is just statically configured on the
  server today.

- **Layer 3 — Ownership & delegation.** Who may *edit* a resource's ACL, can
  access be delegated, can it be revoked. This is the genuinely unbuilt deep
  end: the current grant model has no owner-principal-who-edits-grants and no
  delegation.

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

## Worked example: a chatroom

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
(you block-read `stream` while writing `post`), but the mesh path already handles
it via `StreamingImportFs` dedicated bidi streams, so a chatroom is naturally
mesh-native.

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

**The chatroom is the forcing function for per-principal identity.** It is the
first feature that genuinely needs the mesh-authenticated pubkey threaded down to
the individual `post` operation as the acting principal. The 9P session seam
today decodes `uname`/`aname` and discards them, resolving everything through one
shared root (tracked as a queued follow-up in `CLAUDE.md`); the deferred
per-principal `NamespaceProvider` was explicitly waiting for a first consumer
rather than landing as a no-op seam. The chatroom is that consumer. When this
plumbing is built, promote it to its own accepted ADR.

---

## Suggested build order

Front-load visible payoff (naming, Layer 0+1); run open to start; defer
authorization (Layer 2/3) until the shape is felt with real resources in hand.

1. **`#catalog` device backed by a volume.** Entries as files; model on
   `#kv`/`#agent`. It is a volume, so it syncs across your devices for free.
2. **Make existing commands register entries.** `mesh-serve` and a new
   `volume create` write a catalog entry on startup. Now the catalog has real
   contents (your own exports) with zero new trust work.
3. **`wanix mount <name>`.** Resolve a catalog name → address → `bind` at
   `/n/<name>` instead of pasting `iroh://`. This alone delivers the headline
   UX: grab a resource by name.
4. **The host wrapper** as an `#agent`-shaped device (Whisper is the perfect
   first target), request/response only.
5. **Recipes.**
6. **Authorization (Layer 2)** — per-resource allow-lists keyed on caller
   pubkey, flipping the default off "open." Then ownership/delegation (Layer 3).
7. **Streaming wrappers** (gated on serve concurrency).

---

## Open questions (the discussion below the story)

These are the seams we should talk through before committing contracts.

1. **The authorization model (Layer 2/3) — the deferred-but-inevitable
   project.** Naming (the catalog) does not need this; security-that-is-not-
   obscurity does. What is the per-resource ACL — a file under the resource
   (`#whisper/acl`) listing allowed pubkeys? Who is the *owner* that may edit it,
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
   unlike CAS/capsule entries which are always loadable. Do we add a `probe` /
   `status` per entry? How do we distinguish "the host is down" from "you are
   not granted" in the UX?

4. **Host-wrapper safety surface.** Fixed program + argv template is the core
   safety property. But what about: environment leakage, working directory,
   filesystem access the wrapped program inherits, resource limits, and
   concurrent sessions? How much of `#cpu`'s `ExportScope` confinement do we
   reuse, and how do we keep the wrapper from drifting back into arbitrary exec?

5. **Streaming over the single-frame serve connection.** Request/response works
   today; live streaming hits the "one frame at a time per connection" limit
   noted in `CLAUDE.md` (a blocking `recv` cannot interleave with a write). The
   native mesh wire (ADR 0004: stream-per-call over iroh) makes this a non-issue
   on the Wanix↔Wanix path by construction; the limitation then lives only on the
   9P serve/websocket *edge* (the cockpit). Do we fix serve concurrency there, or
   route streaming over the native mesh path that does not have the problem?

6. **Naming collisions and scope.** `/n/<peer>` is globally unambiguous (it is a
   pubkey). `/n/whisper` is a *local* alias resolved through *my* catalog. What
   happens when a shared recipe says `bind whisper-stt` and the recipient's
   catalog has a different `whisper-stt`? Do recipes carry addresses inline,
   names only, or both with name-as-hint?

7. **Address format / portability.** Is the catalog `address` the existing
   `iroh://<hex>?addr=...` ticket string, or a richer typed value? How do
   `cas:` and `local:` entries coexist with iroh entries in one resolver?

8. **Display names / petnames (the chatroom surfaces this).** Layer 0 gives
   unforgeable but unreadable pubkeys. Human names are Zooko's triangle. Is the
   answer purely catalog-as-petname-store (each viewer names the pubkeys they
   have met), or do some resources (a chatroom) also want an owner-assigned
   name map? How does a petname assigned in the catalog flow into a rendered
   `stream`/`who`?

9. **Per-principal identity on the mesh wire.** The chatroom is the first
   consumer that needs the authenticated pubkey threaded down to the acting
   operation (so `post` can be attributed and `who` can be computed). The native
   mesh wire (ADR 0004) is the place to build this in from the start — iroh
   already authenticates the connection by pubkey, and the wire carries that
   principal per operation — rather than retrofitting 9P's `attach`/`uname` path
   (which today discards `uname` and resolves through one shared root). Open: the
   exact per-principal session/handle model on the native wire, and whether it
   lands first with the chatroom or with per-user namespaces.

10. **Where does this become ADRs?** Likely four durable boundaries: the catalog
    format, the host-wrapper service contract, the Layer-2/3 authorization model,
    and the per-principal 9P identity seam. Confirm that split before promoting
    any of them out of this draft.

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
