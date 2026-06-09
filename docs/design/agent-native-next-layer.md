# The Agent-Native Next Layer: Four Open Questions

Status: **draft thinking note — written by a subagent for discussion; not a
contract.**

Inputs: ADR 0002 (task runtime boundary, snapshots), ADR 0004 (native mesh
wire), ADR 0007 (resources/catalogs/pairing), ADR 0008 (liveness),
[toolfs.md](../toolfs.md), [appfs.md](../appfs.md), and the recent working-session
direction: the job protocol as a platform-wide call convention, three execution
tiers (turn-based resident / blocking POSIX-ish / full guest), CAS-pinned app
provenance with mount-approval as the deploy gate, delegation certs heading
toward UCAN-shaped with namespace attenuation as Layer-3-lite today, and the
product framing of personal/home agentic computing.

Each section takes a position, names the tradeoffs, sketches the contract, and
ends with a recommendation plus what evidence would change it.

---

## 1. Is a task snapshot a capsule?

### Position: one artifact *family*, three artifact *kinds*. Unify the envelope and the loader; do not unify the payload.

Today's `.wcap` capsule is already the right skeleton: every blob in CAS, a
deterministic sorted manifest, **the manifest blob's hash is the artifact id**
(`crates/wanix-cas/src/capsule.rs`). An app package (appfs.md) is a manifest
that CAS-pins code. A tier-1 task snapshot is linear memory + globals + host
handle-table records + task metadata. All three are naturally CAS-rooted. The
temptation is to call them one thing; the trap is pretending a memory image is
a file tree.

They differ in exactly two ways that matter:

- **Payload semantics.** A world materializes to paths (path-safety is the
  trust boundary). A memory image restores into a wasm instance (version/ABI
  compatibility is the trust boundary). Loaders share fetch/verify; they do not
  share the last step.
- **Liveness of references.** A world capsule is closed: every ref is a CAS
  hash, always loadable. A task snapshot is *open*: its mounts are live
  `iroh://PEER` references that are re-dialed, not embedded — exactly the
  recipe shape from ADR 0007, and exactly ADR 0008's identity-vs-hint rule
  (peer id is authority, `addr=` is hint, never serialized as truth).

So: one envelope, one loader front-end, a `kind` tag, three back-ends.

```json
{
  "wanixArtifact": "v1",
  "kind": "world" | "app" | "task-snapshot",
  "refs":   { "<role>": "cas:<hash>", ... },
  "mounts": [ { "path": "/state", "resource": "<catalog-name or iroh://PEER>",
                "hint": "addr=...", "rights": "rw" } ],
  "meta":   { ... kind-specific ... }
}
```

- `world`: `refs` is the existing path→hash manifest; `mounts` empty. The
  current `.wcap` is this kind with a trivial envelope — migration is additive.
- `app`: `refs.code = cas:<hash>` (the provenance pin from the working-session
  decision), `mounts` is the app's entire authority — the three lines a human
  reviews at deploy. The deploy gate reviews the *envelope*, never the code.
- `task-snapshot`: `refs.app = <app capsule id>` (a snapshot names its package,
  it does not copy it), `refs.memory = [page blobs]`, `refs.globals`,
  `refs.handles`, plus `mounts` copied from the running task's namespace —
  recorded as stable identities, restored by re-dial.

### The detail that makes this more than taxonomy: page-chunked memory

Store the linear memory not as one blob but as fixed-size page blobs (64 KiB —
the wasm page — is the obvious unit) listed in the manifest. Turn N and turn
N+1 of a resident task share almost every page, so **per-turn snapshots
deduplicate to near-zero marginal cost in CAS**. This is the enabling
economics for §3's time-travel debugger; without it, "snapshot every turn" is
a storage policy fight. It also means two forks of one session share their
common prefix for free. The capsule machinery (caps, defensive materialize)
applies unchanged; only the blob granularity differs.

### Handle records: reopen instructions, not resurrected state

ADR 0002 already says open dynamic descriptors block snapshot. Keep that
strictness, but the tier-1 turn model relaxes it honestly: at a turn boundary
the wasm stack is empty and every host handle is one of:

- **reopenable** — a namespace file open: record `(mount, path, offset, mode)`;
  restore replays the open. ADR 0008 already says the recovery move for a dead
  handle is "open the path again" — restore is the same move.
- **resubscribable** — a stream (`#plumb` recv, `#agent` events): record the
  subscription; restore resubscribes and *may miss messages*. That gap is the
  resource's problem to make survivable (the `latest` + `stream` pattern from
  the chatroom design exists for exactly this), not the snapshot's problem to
  hide.
- **neither** — an in-flight job stream, a half-written `in`: snapshot is
  refused until the turn quiesces. The job protocol rescues this: a pending
  *job* is not a handle, it is a retained directory with an id. Close the
  stream, keep the job id in task metadata, re-poll `status` after restore.
  **The job id as idempotency key is what makes tier-1 tasks migratable while
  work is outstanding.** This is the strongest single argument for the job
  protocol being platform-wide.

### Tradeoffs

- One envelope means one loader attack surface and one place to do defensive
  decode caps — good. It also means `kind` dispatch in the loader forever —
  acceptable; three kinds is a closed set until proven otherwise.
- Deliberately *not* unified: a `world` is portable to any node; a
  `task-snapshot` is portable only to a node that has the wasm runtime version
  envelope (Wasmtime already embeds and rejects engine mismatches in the module
  cache — reuse that posture: restore-or-refuse, never restore-and-corrupt).
- Recipes (ADR 0007 §4) collapse into this family too: a recipe is an `app`
  envelope with no code ref. Do not keep a fourth artifact format alive.

### Recommendation

Adopt the three-kind family under one CAS envelope. Implement in this order:
(1) wrap the existing `.wcap` in the v1 envelope as `kind: world`; (2) define
`app` with `refs.code` + `mounts` and make mount-approval read the envelope;
(3) `task-snapshot` with page-chunked memory and reopen/resubscribe handle
records, gated on the tier-1 turn runtime existing.

**What would change my mind:** if handle restore turns out to need arbitrary
per-resource custom logic (not just reopen/resubscribe/job-repoll), the
snapshot becomes coupled to app versions and the "one loader" claim breaks —
at that point split task-snapshot into its own format owned by the task
runtime, and keep only world/app unified. Also: if page-level dedup measures
poorly on real resident tasks (e.g. a JS heap that touches every page every
turn), per-turn snapshots need a different lever (delta encoding or
snapshot-every-N), and the time-travel economics in §3 get re-costed.

---

## 2. Who names things for the family?

### Position: a household catalog is one served volume on the home node, with names scoped by *which catalog you mounted* — never a merge. Resolution is launch-time, not run-time.

The per-viewer petname store (ADR 0007 §1, §9) is right for people you meet on
the open mesh. A household is different: `front-door` must mean the same lock
to mom, kid, and the night-watch agent, and "whose catalog wins" is not
allowed to be ambiguous when the answer actuates hardware.

Three candidate shapes:

1. **Per-person catalogs + CRDT/overlay merge.** Reject for v0. Merge
   semantics on a name that controls a door lock is precisely where eventual
   consistency is a bug. A conflict on `vacation-photos` is annoying; a
   conflict on `front-door` is a security incident.
2. **Naming as a resource with its own ACL.** Right, but it is not a new
   mechanism — it is shape 3 plus write-ACLs on entry files.
3. **A designated home-node catalog volume everyone mounts.** Adopt. ADR 0007
   already concedes the bootstrap root of trust is "one address you already
   trust — your home node." The same node that anchors trust anchors names.
   This is Plan 9's auth-server instinct applied to naming, and it matches the
   product story (the catalog as the house's address book).

The key move that dissolves "how do agents resolve when catalogs disagree":
**catalogs never disagree, because names are scoped by catalog and catalogs
are just mounts.** Composition is the namespace's job; reuse it:

```text
/cat/home/        <- the household catalog volume (served by the home node)
  front-door      { name, tags, address: iroh://LOCK..., ... }
  thermostat
  printer
/cat/me/          <- my personal petname catalog (local volume)
  jesses-laptop
  notes
```

A bare name like `front-door` has no global meaning; `home/front-door` does.
An agent launched for household work gets `/cat/home` bound (read-only); a
personal agent gets both. If a launcher wants union semantics it binds a union
directory and the bind order *is* the documented, inspectable resolution
policy — the same answer Plan 9 gives for `$PATH`.

### Resolution is launch-time

Agents should not resolve names mid-task as an ambient operation. The
launcher/recipe resolves `(catalog, name) → address` at mount time, and the
agent's namespace contains *resolved mounts*. Names are for humans and
recipes; capabilities are for runtime. This makes agent behavior deterministic
under catalog edits (a rebind affects the next launch, not a running task —
mirroring ADR 0008's "the mount survives; operations fail" temporal honesty),
and it makes §3's audit trail meaningful: the turn log records which address a
name resolved to, at which time, through which catalog.

Late binding ("find me a printer") is still possible — but it is an explicit,
auditable read of the mounted catalog followed by an explicit mount request
through the launcher, not a hidden resolver. Appetite control (§4) and the
mount-approval gate both depend on mounting being an *event*, not an ambient
power.

### Recipes pin the triple

A recipe entry should carry `(catalog-id, name, address-hint)`:

```text
bind home/front-door -> /dev/lock     # catalog-id: <home catalog volume id>
                                      # hint: iroh://LOCK...?addr=10.0.0.7:4433
```

Name is the authority *within that catalog*; the hint makes the recipe work
offline-ish and detect drift (if the catalog now says a different address,
that is a visible event — warn or re-approve, do not silently follow for
actuating resources). This is ADR 0008's identity/hint split lifted one level:
peer-id over addr at the transport, name-in-catalog over address at the
naming layer. The same pattern at every layer is a feature; keep it.

### Who may rebind `front-door`?

Write-ACL on the entry file, keyed on Layer-0 pubkeys, enforced by the home
node serving the catalog volume. The chatroom analysis (ADR 0007) applies
verbatim: attribution is free from the transport, rebind rights are Layer 2,
"parents can rebind, kids can read, agents can only read" is a three-line
policy. Every rebind is attributed and logged (the catalog should keep an
append-only `log` file per entry — naming changes for actuators are
audit-worthy events). Do not build a general delegation system here; the
household catalog needs exactly owner-writes/member-reads to ship.

### Tradeoffs

- Single naming authority means the home node down ⇒ no *fresh* resolution.
  Acceptable: launchers cache the catalog (it is a volume — snapshot it), and
  running tasks hold resolved mounts that survive per ADR 0008. A household
  that cannot tolerate this needs a replicated home node, which is a
  replication problem, not a naming-model problem.
- Scoped names push a small cost onto humans ("home/front-door" not
  "front-door"). Worth it; bare-name ambiguity is where the lock opens the
  wrong door. Shells can default a scope (`WANIX_CATALOG=/cat/home`).

### Recommendation

Ship the household catalog as one home-node-served volume; names scoped by
catalog mount path; per-entry write-ACLs + append-only rebind log; recipes pin
`(catalog-id, name, hint)`; resolution at launch, late binding as an explicit
auditable act. Personal petname catalogs stay local and separate.

**What would change my mind:** real households without a plausible
always-on-ish home node (then signed catalog snapshots + offline-first cached
resolution become the primary path, and the design shifts toward
per-entry-signed records with the home key, SSB/petname-cert style). Or:
multi-home people (two houses, shared custody) making catalog *identity*
itself ambiguous — then catalog-ids need human-grade petnames too, and the
scoping story needs one more level sooner than I'd like.

---

## 3. What does "inspect a running agent" concretely mean?

### Position: the debugger is `ls`. Three planes — authority (what it *can* do), activity (what it *did*), and state (what it *is*) — each already file-shaped. The turn is the unit of everything.

This is plausibly Wanix's most differentiated agent feature, because the
substrate makes the hard part free: an agent's tools are mounts, its effects
are jobs, its session is files. No tracing SDK, no instrumentation protocol —
inspection is reading a tree that exists because the work happened through it.

Proposed tree (extends the existing `#agent/<id>/` contract):

```text
#agent/<id>/
  prompt  events  pending  reply  ctl  status     # existing
  ns/                  # AUTHORITY: read-only view of the agent's namespace —
                       #   the answer to "what can it touch" is `ls -R ns/`
  turns/
    <n>/
      input            # what entered the turn: prompt | event | job completion
      output           # what the turn produced: message, tool calls, mount requests
      trace            # model interaction record(s): request, response, token counts
                       #   (when the LLM is a mount, these are links to its job dirs)
      ops              # namespace operations performed during the turn, JSONL:
                       #   {op, path, bytes, result, jobId?} — the syscall trace
      usage            # budget deltas for the turn (§4 meters, sampled)
      snapshot         # cas:<hash> of the post-turn task-snapshot artifact (§1)
  fork                 # write "turn <n>" → allocates a new session continuing
                       #   from turn <n>'s snapshot; read returns new session id
```

What each question maps to:

- **"What are you doing and why?"** `status` + `turns/<latest>/input` +
  `turns/<latest>/ops` (which jobs are in flight, with links to their retained
  job dirs) + `ns/` (the complete authority surface). A supervising agent
  answers this with four reads. The *why* chain is the input lineage: each
  turn's `input` names what triggered it, walking back to the originating
  prompt.
- **Replay / time-travel.** `turns/` is the timeline; `snapshot` per turn plus
  §1's page dedup makes keeping every turn affordable. Reading history is
  cheap now; *resuming* history needs the tier-1 turn model.
- **Diff two turns.** Diff at the *semantic* layer: `input`/`output`/`trace`/
  `ops`/`usage` are all small text/JSONL — ordinary diff tools work. Memory
  snapshots diff at the page level (CAS makes "which pages changed" free), but
  a memory diff is for machines (fork-point selection, "did state change at
  all"), not for humans. Do not build a memory-semantics differ; the
  meaningful story of a turn is its trace.
- **Fork at turn N.** Write `turn 12` to `fork`: host restores the snapshot
  artifact into a fresh session — with the same namespace, or an *attenuated*
  one (this composes with §4: fork the agent into a sandbox with read-only
  mounts and a tiny budget to safely try a risky continuation — that
  combination is the genuinely new capability here). Then write a different
  `prompt`. The forked session shares the CAS prefix with its parent.

### Honest v0 vs the turn model

Cheap now (no tier-1 runtime required):

- `ns/` — namespaces exist; bind a read-only view.
- `turns/<n>/{input,output,trace}` — the engine already normalizes events to
  JSONL (`events`); this is the same data structured by turn instead of as one
  stream. Mostly a re-projection of existing state.
- `ops` — an **audit decorator**: a plain `FileSystem` wrapper that logs
  operations and delegates. It composes today (devices import across the mesh
  because they are plain FileSystems; a logger wraps for the same reason), and
  it is the same decorator seam §4's meters need — build it once.
- The job-dir audit trail — falls out of the job protocol wherever it lands.
- **Fork-as-transcript-replay**: today's `#agent` session state is
  approximately its transcript, so v0 `fork` = new session seeded with the
  transcript prefix through turn N. Honest caveat, stated in the file's docs:
  this forks the *conversation*, not the computation — an agent with
  out-of-band state (files it wrote, jobs it ran) is not rolled back, and a
  re-run against a real LLM is not deterministic. Still useful ("try a
  different instruction from turn 8"), and it pins the `fork` contract so the
  real implementation slots in.

Needs the turn model (v1): `snapshot` per turn, true memory fork, and the
guarantee that `turns/<n>` boundaries are quiescent points (empty stack, all
handles classified per §1). Note the dependency direction: §1's artifact
family is the storage substrate for this section; this section is the reason
§1's page-chunking matters.

One non-feature to defend: **no live mid-turn introspection** (peeking at a
turn while the guest is executing). The single-actor turn model is what makes
everything above coherent; a mid-turn debugger reintroduces concurrent access
to guest state. If mid-turn visibility is ever needed, it is the host's edge
that reports (jobs started, streams opened), never the guest's memory.

### Recommendation

v0: add `ns/` and `turns/<n>/{input,output,trace,ops}` to `#agent`, built on
the audit decorator and the existing event stream; ship transcript-prefix
`fork` with documented limits. v1 (with tier-1): per-turn `snapshot` + real
fork + attenuated-namespace fork. Treat `turns/` as the durable contract and
promote it to the agent-as-device ADR when it stabilizes.

**What would change my mind:** if the audit decorator's overhead on hot paths
(a build writing thousands of files through a logged mount) is material, `ops`
needs sampling/aggregation modes and stops being a complete record — weakening
the "the audit trail is total" claim to "the audit trail is total for jobs,
sampled for file ops." If per-turn snapshot cost stays high even page-chunked
(§1 caveat), `snapshot` becomes every-N-turns + replay-from-nearest, which
complicates fork but preserves the contract.

---

## 4. What bounds an agent's appetite?

### Position: budgets are capabilities, expressed as *meter decorators* bound at mount time, with provider-side per-principal quotas as the hard floor. No central accountant, no universal currency.

Two enforcement points exist in this architecture, and both are needed:

1. **The provider edge (hard).** ToolFS quotas live where they cannot be
   bypassed: the resource's owner enforces per-principal limits and reports
   `usage`. The elegant special case is the anchor: **mount the LLM as a
   ToolFS-shaped resource** (`/n/anthropic`: `spec.json`, `usage`, `new`,
   `jobs/<id>/...`) and token budgets are just per-principal quotas on that
   mount — plus every model call becomes a retained job dir, which §3's
   `trace` links to for free. This single move buys budget enforcement,
   audit, idempotent retry, and abort for the most expensive resource in the
   system. Do it early.
2. **The grantor edge (attenuating).** A provider quota is per-*principal*; it
   cannot slice one principal's allowance among that principal's sub-agents.
   The grantor (launcher, parent agent) can: wrap each mount in a **meter
   filesystem** — a decorator that counts (bytes, jobs, native units), refuses
   past its limit, and exposes itself as files. This is sound *because the
   namespace is the authority*: the guest holds only the wrapped mount, never
   the underlying ticket, so there is no path around the meter from inside.
   Sub-agent budgeting is then exactly Layer-3-lite namespace attenuation:
   bind the same resources, wrapped with smaller meters.

```text
agent namespace:
  /n/anthropic/        # metered mount of the LLM resource
  /vol/photos/         # metered mount, read-mostly
  #budget/
    n/anthropic        # {"unit":"tokens","limit":200000,"used":58231,"remaining":...}
    vol/photos         # {"unit":"bytes","limit":1073741824,...}
    jobs               # {"unit":"jobs","limit":50,...}    # cross-mount job count
```

`#budget/` is read-only to the agent (an agent that can raise its own limit
has no limit) and read-write to the supervisor through the launcher's side of
the namespace. An agent can — and should — *read* its remaining budget and
plan; budget pressure becoming visible to the planner is a feature, not a
leak.

### On exceed: fail like a tool fails

A metered mount past its limit returns the job protocol's `quota_exceeded` —
the shared error taxonomy from the working-session direction. No new signal,
no kill-on-overdraft by default: the agent's next effectful call fails with a
structured, retryable=false error it already knows how to surface, and the
supervisor sees it in `turns/<n>/usage` and `pending`. (A hard-kill policy can
exist as launcher policy — `ctl` is right there — but the default should be
"the world stops saying yes," which is how budgets feel to people too.)

### No universal currency

Resist a cross-resource "money" unit in the enforcement path. Each meter
counts its resource's native unit: tokens, bytes, jobs, cents (a paid endpoint
declares `unit: "cents"` in its spec), eventually watts for a metered plug.
Cross-unit rollup ("what did this agent cost today") is a *reporting* read
over `#budget/` and the retained job dirs — a supervisor's join, not an
enforcement mechanism. A universal unit in the enforcement path requires an
exchange-rate oracle inside the trust boundary; do not put one there.

### Across the mesh: budgets meet delegation certs

The local meter is sound locally but invisible to remote providers — two
sub-agents holding mounts to the same provider share that provider's
per-principal quota unless the principal distinguishes them. This is where the
UCAN/biscuit direction lands naturally: a delegated agent key whose cert chain
carries **budget caveats** ("may spend ≤ 50k tokens at this resource, expires
in 1h"), enforced by the provider, attenuated at each delegation hop. The
meter-decorator and the cert-caveat are the same idea at two trust radii —
design the meter's vocabulary (unit, limit, window, expiry) so it can be
serialized into a caveat later, and the local mechanism becomes the wire
mechanism without a second model.

Known gap to name honestly: until delegation certs exist, sub-agents of one
node share one transport principal at remote providers, so the *hard* floor is
per-node and only the *soft* slice is per-agent. For the home (every resource
provider is in the house and mostly mediated through the local namespace),
soft slicing plus per-node hard quotas is adequate. For paid third-party
endpoints, it is the first real forcing function for the cert work.

### Recommendation

Build the meter decorator (the same `FileSystem`-wrapper seam as §3's audit
log — one crate, two decorators); expose `#budget/` read-only in agent
namespaces; adopt `quota_exceeded` from the shared taxonomy as the exceed
behavior; mount the LLM as a ToolFS-shaped resource so token budgets are mount
quotas; keep units native per resource with rollup as reporting; design meter
vocabulary to serialize into delegation-cert caveats later.

**What would change my mind:** if agents legitimately need many short-lived
mounts (late binding everywhere), per-mount meters fragment and a per-*task*
budget object that mounts attach to becomes the better primitive (the meter
moves from "wraps a mount" to "is referenced by mounts"). And if shared-mount
routing-around (two agents granted the same unmetered mount by different
launchers) shows up in practice, provider-side per-delegation accounting moves
from "later, with certs" to "now."

---

## Cross-cutting observations

**1. Every answer is the same answer.** Snapshot = files in CAS. Names = files
in a volume. Debugging = reading the tree the work already flowed through.
Budgets = a wrapper that is also files. None of the four questions needed a
new kind of thing — they needed the existing things (CAS manifests, volumes,
decorators, job dirs) pointed at a new layer. That is evidence the substrate
is right, and it should raise the bar for any future proposal that *does*
introduce a non-file mechanism.

**2. The FileSystem decorator is the recurring mechanism and deserves to be
first-class.** §3's audit log, §4's meter, read-only attenuation, and
principal-scoped views (ToolFS) are all "wrap a `FileSystem`, intercept,
delegate." Build one small decorator crate with shared tests (metadata
passthrough, error transparency, mesh-import composition) instead of four
ad-hoc wrappers.

**3. The job protocol is the spine, not a convention.** §1 needs job ids to
make in-flight work survivable across snapshot/migrate; §3's audit trail and
LLM traces *are* retained job dirs; §4's exceed behavior is the job taxonomy's
`quota_exceeded`; the LLM-as-mount move depends on the ToolFS shape. The job
protocol should be promoted to an ADR *first*, because the other three cite
it.

**4. Identity-over-hint at every layer.** Peer id over `addr=` (ADR 0008),
catalog name over address hint (§2), capsule id over fetch location (§1),
snapshot mounts by stable identity (§1). One pattern, four layers. Worth one
paragraph in an ADR so future layers copy it on purpose.

**5. The turn is becoming a load-bearing word.** Snapshot boundary (§1),
inspection unit (§3), usage sampling point (§4), and the single-actor
execution quantum (appfs.md). When the tier-1 runtime lands, "what exactly is
a turn boundary" (stack empty, handles classified, budgets sampled, snapshot
eligible) should be pinned in the task-runtime ADR rather than implied by four
documents.

**Eventual ADRs**, in dependency order:

1. **The job protocol** — job-dir call convention, lifecycle states, shared
   error taxonomy, job id as idempotency key (promotes from toolfs.md;
   workspace-wide).
2. **The artifact family** — the v1 CAS envelope; world/app/task-snapshot
   kinds; page-chunked memory; handle reopen/resubscribe records (subsumes the
   capsule format and the recipe format from ADR 0007).
3. **Household naming** — the home-node catalog volume, scoped names,
   launch-time resolution, rebind ACLs + log, the `(catalog-id, name, hint)`
   recipe pin (splits out of ADR 0007).
4. **Agent inspection** — the `#agent` `turns/`/`ns/`/`fork` contract (joins
   the agent-as-device ADR when that one is promoted).
5. **Budgets and attenuation** — meter decorators, `#budget/`, native units,
   the caveat-serialization bridge to delegation certs (lands beside, or
   inside, the Layer-2/3 authorization ADR).
