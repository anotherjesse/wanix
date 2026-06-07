# Wanix Website — Master Plan

*Status: planning document. Branch: `cockpit-mesh-integration`. Source of truth for facts: the Rust crates under `/Users/jesse/lw/wanix/crates/`, the recipes under `docs/recipes/`, ADRs `0001`–`0005`, and the verified cockpit screenshots in `docs/integration/screenshots/`.*

---

## 1. Executive summary

We are building the public documentation site for the **Rust-native Wanix port** — a Plan 9-style operating-system core that runs *outside* Chrome, with Wasmtime as the execution substrate, QuickJS/WASI as the first task runtime, and a Plan 9-style mesh that imports remote namespaces as local files over 9P. The site must serve five real, mutually hostile audiences at once: a JavaScript developer who has never heard of Plan 9 (Maya), an infra tinkerer wiring a personal compute mesh (Devraj), a Rust core contributor (Priya), an integrator extending from the edges (Theo), and two visionaries probing whether the deep idea and the agent thesis hold up (Aria, Kemi).

The architecture that resolves this tension is a **dual-mode spine**: every idea exists exactly once as a canonical, deep-linkable, searchable page; persona-shaped **learning flows** *thread* those pages in order without duplicating them. Newcomers are carried; experts bypass the hand-holding by search, deep-link, or the concept graph. The non-negotiable content rule, drawn straight from the source material, is that **honesty is a first-class content type**: every page that can overclaim carries a "Status / honest limits" block (FakeEngine is not a live LLM; `#kv` is in-memory; `serve` handles one 9P frame at a time; exec devices are local-trust-only; the shipped mount lands at `/n/remote`, not `/n/<peer>`; there are no hard CPU/memory limits yet; the mesh layer has no ADRs).

This document ties together the catalogs that already exist (personas, IA narrative, page manifest, demo plan) and **resolves the four adversarial reviews**. The most consequential review findings — three incompatible slug schemes, orphaned flow pages, broken `#kv` newcomer examples, a stale "cpu branch" caveat, a missing `wanix new` on-ramp, a missing glossary, and mis-filed capability contracts — are all addressed here as binding decisions, not open questions. The deliverable is **portable markdown** (no framework lock-in), with a concrete recommendation to render it with **Astro + Starlight**.

What makes this different from a normal docs site: the subject is itself "everything is a file," so the docs lean into it — every device page is a stable set of file-path anchors, every concept cites exact `crate/src/file.rs:line` ranges into the repo, and the site's credibility comes from being *more* accurate than `AGENTS.md` in the places the reviews caught (no `.wcap` file exists; mount lands at `/n/remote`; the HTTP-app route is shipped).

---

## 2. Vision & positioning

### Elevator pitch

> **Wanix is Plan 9 reincarnated for the age of agents.** A Rust-native OS core that runs outside the browser: everything is a file, every process gets its own rearrangeable namespace, and one 9P contract reaches local services and remote machines alike — so files, compute, and AI agents compose across nodes.

### Taglines (use by surface)

- Home hero: **"JavaScript outside Chrome. Files all the way down. Agents that operate your machine."**
- Mesh: **"Mount this node, trust this node — the same bytes."** (the ed25519 key is identity *and* address.)
- Agents: **"An LLM you can `cat`. Approvals are files."**
- Security: **"A capability is a bind. There is no 'outside' for a peer to name."**
- Contributors: **"One trait is the whole universe. Implement `FileSystem` and you're a Wanix capability."**

### The "missing half of 9P" thesis

This is the intellectual spine of the project and the visionary track. Plan 9's deepest idea is not "everything is a file" for its own sake — it is that **the namespace is per-process and rearrangeable**, and that two operations build the view: `export` (serve a subtree as 9P) and `import` (splice a remote 9P service into *your* namespace at `/n/<name>`). Because services are files, *one* file-transport protocol (9P) plus *one* placement operation (bind) buys network transparency for every service at once.

For a long time Wanix could **export** but not **import** — it had half of 9P. The mesh work builds the other half: `wanix-9p-client::RemoteFs` (`crates/wanix-9p-client/src/remote.rs`) is a synchronous `FileSystem` that *is* a 9P client, so binding it makes a remote namespace — regular files **and** `#`-devices alike — part of your local tree. The mesh then re-creates the rest of the Plan 9 cast, each as one slice: **factotum → the QUIC handshake** (`Tauth` stays `ENOSYS` forever), **a capability is a bind** (a grant is a re-rooted `SubtreeFs`, not an ACL), **venti → `#cas`**, **cpu(1) → `#cpu`**, **plumber → `#plumb`**.

### Who it's for

- **Builders (users):** run JS outside Chrome in ten minutes; build an HTTP app backed by `#kv`; click through the cockpit demos.
- **Operators (power users):** wire a laptop + home server + cloud box into one mesh; send compute to the data; freeze worlds to capsules.
- **Engineers (developers):** extend Wanix with a new device, driver, or transport without forking the core; or contribute to the core crates themselves.
- **Visionaries:** evaluate whether the capability model is principled and whether agents-plus-compute really compose across machines safely, today.

### Honest scope — shipped vs aspirational

This is positioning, not fine print. State it plainly on the Home page and in every relevant track:

**Shipped & verified:** JS/wasm tasks on Wasmtime; the seven inspectable service devices + `#cpu`; the 9P server over stdio/TCP/WebSocket/HTTP; `RemoteFs` import; 9P over iroh QUIC with ed25519 identity and default-deny grants; the `#cpu` exec plane; capsules; the browser cockpit (five live-wired features, validated headless at PASS); the `/.wanix/app/<name>` HTTP route (loopback-only, services-gated, **confirmed wired on this branch**).

**Local-trust-only (not exposed publicly):** exec devices (`#task`/`#agent`/`#cpu`); the real codex-backed agent (`wanix agent`); the served `#agent` is a deterministic `FakeEngine`.

**Aspirational / exploratory (label clearly):** per-peer `/n/<peer-id>` mounts (shipped mount lands at the single slot `/n/remote`, `crates/wanix-cli/src/mount.rs:26`); per-principal namespaces and grant lifecycle; public multi-user auth; ethernet/vnet; hard CPU/memory limits ("cheap, scalable isolation," **not** "safe for arbitrary untrusted code"); traceable/forkable/rewindable namespaces; the cockpit mesh panel and browser-as-peer.

---

## 3. Audiences & personas

### Three top-level tracks

The Home page routes to three audience tracks, each with persona on-ramps beneath it:

1. **Users** — "I want to run code / use it." → Maya (curious app dev), Devraj (compute-mesh power user).
2. **Developers** — "I want to extend or contribute to it." → Theo (integrator/extension author), Priya (Rust core contributor).
3. **Visionaries** — "I want to understand and evaluate the deep idea." → Aria (Plan 9 / distributed-systems thinker), Kemi (AI-agent-infrastructure builder).

### Persona summaries

| Persona | Track | Wants | First win |
|---|---|---|---|
| **Maya** — curious app dev, never heard of Plan 9 | Users | Visible results in 10 minutes, no CS-history lecture | `wanix-rust qjs examples/qjs-demo.js` → "outside Chrome: true" |
| **Devraj** — self-hosting infra tinkerer | Users | Make laptop + server + cloud act like one machine; send compute to the data | Two-node mount over iroh QUIC (recipe 02) |
| **Priya** — systems-Rust engineer | Developers | Land a clean core PR first try: crate map, ADRs, `just check` | Crate-map + dependency-direction page; `just check` green |
| **Theo** — integrator / extension author | Developers | Add a device/driver/transport without forking core | "Add a service device" using `#kv` as template |
| **Aria** — Plan 9 / distributed-systems thinker | Visionaries | Confirm the capability model is principled, not aesthetic | "The missing half of 9P" proof; `subtree.rs:58-176` audit |
| **Kemi** — AI-agent-infra builder/architect | Visionaries | Confirm agents + compute compose across machines, safely, today | `#agent` as files / approvals as files (recipe 01) |

The personas drive flow ordering, prerequisite badges, and the audience tag vocabulary (`newcomer`, `developer`, `visionary`). Aria's defining trait is load-bearing for the whole site: **she trusts the project more for its caveats than its claims**, which is why honesty-as-content is an IA rule, not a nicety.

---

## 4. Information architecture

### Sitemap

The global nav has eight destinations. **Decision (resolves IA-coherence review):** there is exactly one canonical slug scheme — `section/page` (single section prefix, no doubling). The page manifest's doubled prefixes (`concepts/concepts/...`) are an artifact of the generator and are flattened to `concepts/...` at build time. Flows live under `/learn/<flow-slug>` and **nowhere else** — no `/flows/` section exists; `/concepts/<flow>` and `/use-cases/<flow>` flow links are banned by the link-checker.

```
/                                   Home — hero + three tracks + honest scope
├── /learn/                         Learn — flow catalog (the guided mode)
│   ├── /learn/js-outside-chrome    (Maya)
│   ├── /learn/wire-a-mesh          (Devraj)
│   ├── /learn/http-app-with-kv     (Maya → builder)
│   ├── /learn/add-a-service-device (Theo)
│   ├── /learn/add-driver-or-transport (Theo)
│   ├── /learn/agent-on-your-files  (Kemi)
│   ├── /learn/plan9-ideas-tour     (Aria)
│   └── /learn/contribute-to-core   (Priya)
│                                   (8 flows — copy says "eight", derived from the list)
├── /concepts/                      Concepts — one canonical page per idea (~78 nodes)
│   └── /concepts/<slug>            everything-is-a-file, capability-is-a-bind, …
├── /devices/                       Devices — file-contract reference + anchor index
│   ├── /devices/                   (index: all 8 devices + per-file anchor tables)
│   ├── /devices/task  /devices/term  /devices/kv  /devices/pipe
│   └── /devices/plumb /devices/cas  /devices/agent /devices/cpu
├── /use-cases/                     Use cases — outcome-first "why" (end in ONE recipe)
│   ├── /use-cases/personal-compute-mesh
│   ├── /use-cases/agents-on-your-files
│   ├── /use-cases/portable-worlds
│   ├── /use-cases/browser-cockpit
│   ├── /use-cases/distributed-dev-environments
│   └── /use-cases/traceable-namespaces
├── /recipes/                       Recipes — copy-paste transcripts (link UP to use-case)
│   ├── /recipes/00-scaffold-a-project   (NEW — wanix new)
│   ├── /recipes/walkthrough-1-run-js
│   ├── /recipes/walkthrough-2-process-context
│   ├── /recipes/01-repair-broken-qjs
│   ├── /recipes/02-mount-remote-peer
│   ├── /recipes/03-freeze-world-to-capsule
│   ├── /recipes/04-tiny-http-app-with-kv
│   └── /recipes/05-two-agents-collaborate
├── /reference/                     Reference — contracts, specs, indices
│   ├── /reference/contributor-landing
│   ├── /reference/crate-map-and-layering
│   ├── /reference/adr-index
│   ├── /reference/filesystem-trait
│   ├── /reference/extension-points          (canonical "add a device" checklist)
│   ├── /reference/attach-and-capability-contract  (NEW — see §10)
│   ├── /reference/guest-sdk                  (NEW — lib/wanix/)
│   ├── /reference/serve-and-discovery
│   ├── /reference/cli-command-index          (includes `new`, `agent`, p9-listen grants)
│   ├── /reference/cli-rootfs-qemu-v86
│   ├── /reference/quality-gates
│   ├── /reference/performance                (NEW — scaling-eli5 + compute_bench)
│   └── /reference/queued-follow-ups
└── /find/                          Find — findability hub
    ├── /find/                      (search, hotkey "/")
    ├── /find/concepts              (graph-backed Concept Index: A–Z + visual map)
    ├── /find/glossary              (NEW — A–Z term lookup, see §10)
    └── /find/tags                  (tag browser)
```

### The dual-mode model

Every concept has exactly one canonical page at `/concepts/<slug>` that is *self-contained reference*: a one-liner, a body, a **source-of-truth box** (exact `file.rs:line` ranges linked into the repo), a **Status / honest limits** block, a **See also** rail (graph neighbors), and a **Used in flows** rail. A flow step is a thin wrapper — a goal sentence, a "do this" action, an "observe this" output, and a "Read the concept →" link. **Flows own motivation and order; canonical pages own truth.** Accuracy is maintained in one place.

### Progressive disclosure (three depths per concept page)

- **Depth 1** — the one-liner + analogy (Maya stops here).
- **Depth 2** — body + a runnable/clickable example.
- **Depth 3** — collapsible "Under the hood" + source box + honest-limits (Priya/Aria jump straight here). Collapsed by default.

### Concept graph backbone

Nodes are concepts; typed edges are `prerequisite-of`, `composed-of`, `related-to`. The graph powers (a) "See also" rails, (b) prerequisite badges, and (c) the visual Concept Map at `/find/concepts`. Root: **Everything is a file** → `prerequisite-of` **The FileSystem trait**, **Service devices (#name)**, **Per-process namespaces**, **Import/export and /n/**. Mesh cluster composes from **RemoteFs**, **9P over iroh QUIC**, **The key is the address**, **A capability is a bind**.

**Orphan-prevention rule (resolves IA-coherence review):** not every concept is threaded by a flow, and the IA must say so honestly: *"Key concepts are threaded by flows; the rest are reference-only, reachable via Find and See-also rails."* The hard guarantee is that **`/find/concepts` is the inbound link for every graph node** — nothing is a true orphan, enforced by the build-time orphan report.

### Findability

- **Search** (hotkey `/`): indexes concept pages, device anchors, ADRs, crates, recipes, glossary. Results are typed. A query like `poll_oneoff` lands on the wasm-linker concept; `EACCES` / `errno 13` lands on the attach-and-capability contract page; `wanix/9p/1` lands on the glossary entry.
- **Glossary** (`/find/glossary`, **new**): A–Z one-line definitions for every term and **typed primitive** an expert searches by exact name — `9P`, `ALPN`, `wanix/9p/1`, `wanix/cpu/1`, `Authorization`, `BindPosition`, `confine_to_prefix`, `ContentHash`, `EACCES`, `EndpointId`, `fid`, `FsError::NotSupported`, `msize`, `NormalizedPath`, `PeerId`, `Rights`, `SubtreeFs`, `Tauth`, `venti` — each deep-linking to its canonical page + source.
- **Concept Index** (`/find/concepts`): the graph rendered A–Z and as a clustered visual map.
- **Deep-link anchors:** device files, trait methods, ADR sections, and the five hostile-peer corrections each get stable `#anchors`.
- **Breadcrumbs:** `Section › Cluster › Page › #Anchor`.

### Cross-linking strategy

The four content types form a closed cycle: **Flow step → Concept** ("Read the concept →"); **Concept → Flows** ("Used in flows"); **Concept ↔ Concept** ("See also" / "Prerequisites" from graph edges); **Concept → Device/Reference**; **Use case → Concepts + exactly one Recipe**; **Recipe → up to its single parent Use case**; **Reference (ADR/crate) → Concept**. Honest-limit blocks link bidirectionally to the matching queued-follow-up so a reader can see whether a caveat is being worked.

**Twin rule (resolves IA-coherence review):** where a concept and a device page both cover one device (e.g. `devices/kv` vs `concepts/kv-smallest-database`), the **Device page owns the file/anchor contract and a one-line caveat that links the concept; the Concept page owns the "why" and the canonical honest-limits block.** Neither restates the other. Each honest-limit caveat (FakeEngine-vs-codex, kv-in-memory, single-frame-serve, exec-local-trust, mount-at-/n/remote) has **one canonical home** and every other mention links it.

---

## 5. Learning flows

Eight flows. Each renders at `/learn/<flow-slug>` with a **flow-progress rail** ("Flow: X — step N of M — Prev/Next") that persists even when a step is a Concept, Device, or Recipe page in another section, so the reader always knows where they are. Each flow opens with a one-time **"Build once" preamble** (resolves newcomer review: `wanix-rust` is not on PATH) and, where relevant, a **prerequisite badge**.

**Build-once preamble (verbatim, top of every flow and the Learn index):**
```
# Build the CLI once, then use it everywhere in this flow:
cargo build --package wanix-cli
alias wanix-rust='./target/debug/wanix-rust'   # or: export WANIX=./target/debug/wanix-rust
# First build compiles 22 crates and is not instant — start the clock after this.
```

1. **JavaScript outside Chrome in 10 minutes (Maya).** Hero command → "what just happened" (tasks + namespaces, jargon-free) → feed env/cwd/stdin/argv (walkthrough §2) → **a short `qjs-shell` step** (write/ls/cat builtins, ~2 min, no extra build — *inserted per newcomer review so the "files all the way down" moment lands before the heavyweight cockpit*) → explicit host mount (§3, **with the prerequisite line** `mkdir -p /tmp/wanix-host && echo 'from host' > /tmp/wanix-host/input.txt`) → boot the cockpit (**prerequisite badge:** `cd workbench && make build`, needs Go, downloads vscode-web; **CLI-only fallback** via `wanix agent --fake` so the agent payoff is reachable without the cockpit build) → click-through Agent Repair / Duet / HTTP counter / Self-check, each with a "what file ops this really is" reveal → write your own `apps/counter.js` (recipe 04) → honesty callouts → soft hand-off to the mesh.

2. **Wire two machines into a mesh (Devraj).** Recipe 02 two-terminal transcript (**`/n/remote` single-slot caveat upfront**) → identity primer (`~/.wanix/node.key` 0600, key-is-the-address, why `Tauth` stays ENOSYS) → devices-for-free (read `/n/remote/#kv/<key>`) → `#cpu` exec plane (reverse-export, read-only jail, `--write`, batched-output/no-remote-cancel limits) → trust-boundary deep dive (default-deny GrantTable, capability-as-SubtreeFs, public-endpoint refusals, **`--insecure-open` as the distinct read-write-no-exec mode**, exec-local-trust) → capsules → multi-agent + `#plumb` (recipe 05, single-frame caveat) → cockpit-as-dashboard + "not-yet-shipped" page.

3. **Build an HTTP app backed by #kv (Maya → builder).** **Step 0: scaffold (`wanix new --js counter`, recipe 00)** → understand the **`/.wanix/app/<name>`** route (loopback-only, services-gated, **shipped on this branch**) → write `apps/counter.js` doing read-modify-write on **`#kv/http-counter`** (the shipped key) → curl + cockpit increment one counter **through the running serve process** (not a bare `wanix qjs` — resolves newcomer review) → "state survives because the device lives for the serve process; `#kv` is in-memory; freeze to a capsule to persist."

4. **Add a service device (Theo).** Extension-points landing → `#kv` as the canonical minimal `FileSystem` (`crates/wanix-kv/src/lib.rs`, ~177 lines) → implement open/read_dir/remove + the blocking-stream EOF contract → **bind into the served namespace** (`crates/wanix-cli/src/serve/roots.rs:119`, the `INSPECTABLE_SERVICE_DEVICES` set) → payoff: mesh import + 9P export + cockpit inspector for free → **a `#pipe` deeper-dive step** (resolves completeness review's orphaned-`#pipe` finding) → device-contract cards + don't-overclaim caveats.

5. **Add a task driver / a transport (Theo).** `TaskDriver` check/start, register-from-above, shared `task_wasi_config`, auto-select-by-suffix → transport as a thin adapter over `P9Server`/`RemoteFs`, frame-boundary rule, protocol-vs-server split → honest constraints (Tauth ENOSYS, single-frame serve, single-attach-per-connection, local-trust exec) → layering rules + `just check`.

6. **The agent on your files (Kemi).** Agents-as-operators thesis → `#agent` as files / approvals as files (recipe 01), cockpit Repair Demo as the live one-click trust-gate proof → reach across the mesh (`/n/remote`, `#cpu`, `#plumb`, recipe 05 over QUIC) → capability security she can reason about → rooms-not-houses cost story (**"no hard CPU/mem limits yet" caveat**) → provenance roadmap (clearly exploratory) → **final step lands on recipe 01 or 05, a runnable artifact** (resolves completeness review: `fakeengine-vs-codex` is the penultimate honest beat, not the terminus).

7. **The Plan 9 ideas tour (Aria).** The thesis as a proof: the missing half of 9P → browser-as-host vs Wanix-as-host (honest "Go Wanix is still broader" admission) → the mechanism in code (RemoteFs as the photographic negative of `serve_stream`, **the five hostile-peer corrections as a real linked pageSlug**) → the trust boundary as one pure function (`AttachPolicy::evaluate(peer, aname)`, default-deny, capability-as-bind, `confine_to_prefix`, key-is-address, Tauth ENOSYS, **the v1 single-attach simplification cited at `crates/wanix-9p/src/lib.rs:128-133`**) → the full Plan 9 cast slice by slice → the honest gaps page.

8. **Contribute to the core (Priya).** Contributor landing ("Rust port, not the Go tree; use `just check`, ignore the Go Makefile/CONTRIBUTING") → crate map + dependency direction (async/iroh confined to `wanix-mesh`, no upward deps) → active ADRs 0001–0005 + ADR-vs-commit-message workflow → `FileSystem`/`File` trait reference → worked example `#kv` → code-quality guardrails + `just` recipes → queued follow-ups as "good first cleanups" (split `codex.rs`/`exec_server.rs`/`app.rs`, port the `v86-shared-demo` stub).

---

## 6. Content plan

### Section-by-section page inventory

| Section | Count | Contains |
|---|---|---|
| **Home** | 1 | Hero (live cast), three audience tracks + persona on-ramps, honest-scope panel |
| **Learn** | 1 index + 8 flow pages | Persona-shaped guided tracks; each a step list threading canonical pages |
| **Concepts** | ~78 | One canonical page per idea; the dual-mode truth source; graph nodes |
| **Devices** | 1 index + 8 | `#task #term #kv #pipe #plumb #cas #agent #cpu`; per-file anchor tables; the index is a one-hop fan-out |
| **Use cases** | 6 | Outcome-first "why"; each ends in exactly one recipe link |
| **Recipes** | 8 | `00-scaffold` + 2 walkthroughs + 5 numbered recipes; copy-paste transcripts |
| **Reference** | 13 | Contributor landing, crate map, ADR index, FileSystem trait, extension points, **attach/capability contract**, **guest SDK**, serve/discovery, CLI index, rootfs/qemu/v86, quality gates, **performance**, queued follow-ups |
| **Find** | 1 index + 3 | Search, Concept Index, **Glossary**, Tag browser |

### Page templates per pageType

Every page carries YAML frontmatter. **Shared frontmatter schema:**

```yaml
title: string                 # H1
slug: string                  # canonical section-relative slug (no doubling)
pageType: home|overview|concept|reference|developer|use-case|flow-step|device|flow|glossary
oneLiner: string              # the Depth-1 sentence; shown in search results
audience: [newcomer|developer|visionary]
tags: [string]                # cluster + status tags (shipped|local-trust-only|exploratory|caveat)
sourceRefs: [string]          # exact crate/src/file.rs:line ranges
seeAlso: [slug]               # graph related-to / composed-of
prerequisites: [slug]         # graph prerequisite-of in-edges
usedInFlows: [{flow: slug, step: int}]   # drives the flow-progress rail
honestLimits: [string]        # required wherever overclaim is possible
canonicalCaveatFor: [string]  # if this page is the single home of a caveat
```

**`concept` template:** frontmatter → H1 + oneLiner → Depth-1 analogy → Depth-2 body + runnable example → collapsible Depth-3 "Under the hood" → **Source-of-truth box** → **Status / honest limits** block → See also / Prerequisites / Used in flows / Seen in rails.

**`device` template:** frontmatter → H1 + oneLiner → one H2/H3 **per file path** with a slugified anchor matching the file (`new` → `#new`, `<id>/data` → `#data`, `<key>` → `#kv-key`) → an inline 3–5 line "file-ops reveal" micro-demo → a one-line caveat that **links** the canonical concept (does not restate) → anchor table at top.

**`reference` template:** flat spec — signatures, defaults, tables, source anchors. No narrative. (The attach/capability contract page is the canonical example: signature, default-deny rule, `SubtreeFs`/`Rights`, verified-PeerId keying, `Tauth=ENOSYS`, the v1 single-attach note.)

**`flow` template:** frontmatter → goal → prerequisite badges → "Build once" preamble → ordered step list, each step a thin wrapper (goal / do / observe / "Read the concept →") with the flow-progress rail → terminal step links a runnable recipe.

**`flow-step` / `recipe`:** verbatim transcript with one-click copy, expected output, a "what file ops this really is" reveal, the flow-progress rail when arrived-at via a flow, and an UP-link to the single parent use case.

**`use-case`:** outcome statement → the concepts it rests on → exactly one "Run it: Recipe NN" link.

**`glossary`:** A–Z; each term one line + deep link to canonical page + source.

### Voice & style guide

- **Show, then name.** Lead with the command and the visible result; introduce the Plan 9 word *after* the reader has seen the effect. (Maya rule.)
- **Cite source, always.** Every claim that touches behavior carries a `crate/src/file.rs:line` reference. Credibility is the product.
- **Honesty is content, not apology.** Caveats are stated flatly and once, in their canonical home, and linked elsewhere. Frame them as engineering boundaries ("local-trust only until public auth lands"), never as excuses.
- **Never overclaim the five landmines:** FakeEngine is not a live LLM; `#kv` is in-memory; `serve` is one-frame-at-a-time; exec is local-trust-only; mount lands at `/n/remote` (use `/n/<peer>` only as a labeled *convention*); "cheap, scalable isolation" not "safe for arbitrary untrusted code."
- **Active voice, terse, no emoji.** Match the repo's own commit/ADR tone.
- **One invocation convention:** `wanix-rust` (via the alias) everywhere a flow shows commands; the Home hero may use `cargo run` but bridges to the alias visibly.

---

## 7. Demos & interactivity

### Media tiers

1. **Live in-browser asciinema casts (HIGH confidence, the workhorse).** Every single-process CLI flow is deterministic and produces clean transcripts: record as `.cast`, embed with the asciinema player (scrub/copy/replay). It feels live with no backend. Covers Home hero, most of Learn/Recipes/Concepts.
2. **Recorded screenshots / short screencasts (HIGH confidence, already captured).** The cockpit needs a running `serve` + vendored vscode-web, so it is *never* embedded live. Use the verified PNGs in `docs/integration/screenshots/` (captured 2026-06-07 at PASS): `wanix-cockpit.png`, `cockpit-loaded-v3.png`, `inspect-kv.png`, `inspect-agent.png`, `inspect-task.png`, `inspect-root.png`, `agent-repair.png`, `duet.png`, `http-counter.png`, `self-check.png`.
3. **Genuinely live embedded cockpit (LOW confidence — DO NOT SHIP).** Exec devices are local-trust-only; never host a shared public cockpit. Offer a "run this locally in 60s" copy-paste block instead.
4. **Copy-paste recipe blocks (load-bearing).** Every recipe is verbatim shell with one-click copy and a "what file ops this really is" reveal.

### Live vs recorded — the decision per demo

| Demo | Media | Why |
|---|---|---|
| Home hero (`qjs examples/qjs-demo.js`) | **LIVE cast** | Single-process, deterministic, "outside Chrome: true" |
| R01 Agent repair | **Recorded** `agent-repair.png` + LIVE CLI cast of the `#agent` file loop | Cockpit needs vscode-web; CLI loop is deterministic (FakeEngine) |
| R02 Mount peer + `#cpu` | **Recorded** dual-pane cast | Two nodes can't be one live cast; no screenshot exists |
| R03 Capsule round-trip | **LIVE cast** | Recipe §6 is a paste-into-shell smoke test ending `round-trip ok` |
| R04 HTTP counter + `#kv` | **Recorded** `http-counter.png` + LIVE cast of the served route | Must run through `serve`, not a bare qjs (the `#kv` binding) |
| R05 Two agents | **LIVE cast** (single-node FakeEngine) | Cross-node `#plumb` variant recorded (single-frame caveat) |
| C1–C4 Cockpit features | **Recorded** PNGs | Verified PASS captures |
| L1–L4, L6, L8 CLI/engine | **LIVE casts** | Single-process, deterministic |
| L7 Capability grant over wire | **Recorded/annotated transcript** | Needs two endpoints; allow-side proven at loopback only (no `--aname` flag yet) |
| G1 Concept graph | **LIVE interactive** (client-side widget) | Docs-native, no backend |

### Asset list

- **Existing (use as-is):** the 10 verified PNGs above.
- **To record (LIVE casts):** Home hero; L1 process-context; L2 host-mount (with the prereq setup line baked into the recording dir); L3 child task via `#task`; L4 `qjs-shell`; R03 capsule round-trip; R04 served counter; R05 single-node two-agents; L6 snapshot/restore; L8 discovery-doc `curl | jq`; the inline micro-demos M1–M6 (rendered code blocks, not players).
- **To record (screencasts):** R02 dual-pane two-node; the "run the cockpit locally in 60s" boot.
- **Never a demo:** `v86-shared-demo` (still a `// STUB:` no-op); a live public cockpit; a "real LLM in the browser"; live cross-node `#plumb` recv over one connection; "Wanix boots/manages a VM" (direct-v86 is a handoff).

---

## 8. Tech stack recommendation

**Recommendation: Astro + Starlight**, content authored as portable Markdown/MDX under `docs/site/content/` (the `docs/site/` directory already exists in the repo).

### Rationale

- **Markdown-first, low lock-in.** Content is plain `.md`/`.mdx` with the frontmatter schema in §6; if Astro is ever dropped, the corpus survives. Starlight gives the sidebar, breadcrumbs, search, and dark mode for free, which maps directly onto the IA's global nav + in-section left rail + right-rail TOC.
- **Content collections enforce the schema.** Astro's `content.config.ts` with a Zod schema makes the §6 frontmatter (including `honestLimits`, `sourceRefs`, `usedInFlows`, `canonicalCaveatFor`) a build-time contract — a page missing a required `honestLimits` block fails the build. This is how "honesty is a first-class content type" becomes mechanically enforced.
- **The slug discipline the reviews demand is native.** A single content collection per section with `slug` = file path gives the `section/page` URLs uniformly. A custom **remark/rehype link-checker plugin** fails the build on any `href` not in the page set and bans the strings `../flows/`, `/concepts/<flow>`, `/use-cases/<flow>` — directly resolving the IA-coherence review's broken-link findings.

### Search

**Pagefind** (Starlight's default static, fully client-side full-text search). No backend, indexes the built HTML, supports typed result filtering via data attributes. Typed results (Concept / Device file / ADR / Crate / Recipe / Glossary) come from the `pageType` frontmatter rendered into a `data-pagefind-filter`. Exact-name queries like `poll_oneoff`, `EACCES`, `wanix/9p/1` resolve because the glossary and concept bodies contain those literal strings.

### Concept-graph rendering

A small **client-side graph widget** at `/find/concepts` driven by a generated `concept-graph.json` (nodes + typed edges derived from each concept page's `seeAlso`/`prerequisites` frontmatter at build time). Use **Cytoscape.js** or a lightweight D3 force layout, clusters colored by `audience`. The same JSON feeds the "See also"/"Prerequisites" rails (rendered server-side from frontmatter, so they work without JS). The graph is a navigation surface, not a Wanix runtime demo.

### Code blocks & asciinema

- **Expressive Code** (Starlight's built-in) for syntax-highlighted, copy-button code blocks and the "what file ops this really is" reveal (use frames/captions).
- **asciinema-player** mounted via a thin Astro component (`<AsciinemaCast src="...">`) loading `.cast` files from `public/casts/`. The player is the standard embed; it is client-only and lazy-loaded.
- **Screenshots** from `docs/integration/screenshots/` are copied into the site's asset pipeline (Astro `<Image>` for responsive variants).

### Ingesting `docs/site/content/*`

Astro content collections read the markdown tree directly:

```ts
// docs/site/src/content.config.ts
import { defineCollection, z } from 'astro:content';
import { docsLoader } from '@astrojs/starlight/loaders';
import { docsSchema } from '@astrojs/starlight/schema';

const wanixSchema = z.object({
  pageType: z.enum(['home','overview','concept','reference','developer',
                    'use-case','flow-step','device','flow','glossary']),
  oneLiner: z.string(),
  audience: z.array(z.enum(['newcomer','developer','visionary'])).default([]),
  sourceRefs: z.array(z.string()).default([]),
  seeAlso: z.array(z.string()).default([]),
  prerequisites: z.array(z.string()).default([]),
  usedInFlows: z.array(z.object({ flow: z.string(), step: z.number() })).default([]),
  honestLimits: z.array(z.string()).default([]),
  canonicalCaveatFor: z.array(z.string()).default([]),
});

export const collections = {
  docs: defineCollection({
    loader: docsLoader(),
    schema: docsSchema({ extend: wanixSchema }),
  }),
};
```

### Runnable getting-started sketch

```sh
# from repo root
mkdir -p docs/site && cd docs/site
npm create astro@latest -- --template starlight --yes .
npm i @astrojs/starlight asciinema-player cytoscape
# author content under docs/site/src/content/docs/{concepts,devices,learn,...}/*.md
# add src/content.config.ts (above) + a remark link-checker plugin
npm run dev          # local preview at http://localhost:4321
npm run build        # static output to docs/site/dist/ — fails on broken links / missing honestLimits
```

Sourcing existing material: the five recipes (`docs/recipes/01–05.md`) and the two walkthrough sections become the Recipes section verbatim; the verified screenshots are the cockpit proof; `docs/scaling-eli5.md` and `docs/performance.md` source `/reference/performance`; ADRs `0001–0005` source `/reference/adr-index`. A small build script can stamp `sourceRefs` line ranges, but the canonical authoring is hand-written markdown.

---

## 9. Build roadmap

### Phase 0 — MVP spine (definition of done: a newcomer reaches a working result)

**Deliverables:** Astro+Starlight scaffold under `docs/site/`; the content schema + link-checker + honestLimits-required build gate; Home page (live hero cast); the `js-outside-chrome` flow end-to-end with the **Build-once preamble**, the `qjs-shell` step, the host-mount prereq line, and the cockpit prereq badge + `agent --fake` CLI fallback; canonical concept pages for the Depth-1 cluster (everything-is-a-file, per-process-namespaces, service-devices, qjs-task, the-#task-device); the `#kv`/`#agent`/`#task` device pages; recipe 00 (`wanix new`) + recipe 01.
**DoD:** Maya can go Home → flow → working `outside Chrome: true` and a `agent --fake` repair **without ever pasting a command that isn't on PATH or a `#kv` op that crashes.** Every link resolves; the build fails if any doesn't.

### Phase 1 — full corpus (DoD: every IA page exists and is cross-linked)

**Deliverables:** all ~78 concept pages; all 8 device pages + the Devices index/anchor convention; all 6 use cases (each ending in one recipe) + all 8 recipes; the full Reference section including the **new** attach-and-capability contract, guest-SDK, performance, and CLI-index-with-`new`/`agent`/`p9-listen` pages; the Find hub including the **new glossary**; the remaining 7 flows.
**DoD:** the orphan report is clean (every concept has an inbound `/find/concepts` link); the twin rule and canonical-caveat rule hold (no duplicated honest-limits blocks); `just`-style link/schema check is green in CI.

### Phase 2 — interactive demos (DoD: the demo plan is realized)

**Deliverables:** all LIVE asciinema casts recorded and embedded (Home hero, L1–L4, L6, L8, R03, R04, R05); the recorded dual-pane R02 screencast; the cockpit PNGs placed on cockpit pages and the js-outside-chrome flow; the inline micro-demos M1–M6; the interactive Concept Graph (G1).
**DoD:** every demo carries its honest-limit; no excluded demo (v86-shared-demo, live public cockpit) ships; casts replay without a backend.

### Phase 3 — polish (DoD: the site works for both newcomers and experts, per §12)

**Deliverables:** Pagefind typed search tuned (exact-name primitives resolve); tag browser; breadcrumbs + flow-progress rail on every threaded page; accessibility/dark-mode pass; first-run compile-cost note on the Maya badge; copy review against the voice guide and the five landmines.
**DoD:** the success metrics in §12 are measurable and met on a manual persona-walk.

---

## 10. Review resolutions

How each top issue from the four adversarial reviews is resolved in this plan.

**Technical accuracy**
- *Stale "cpu branch" caveat (HIGH).* Removed everywhere. The `/.wanix/app/<name>` route is **shipped on this branch** (verified: `crates/wanix-cli/src/serve/http/app.rs:40,114`; commit `f860b75`). Pages state it as shipped, loopback-only, services-gated. Driving the handler via the CLI is framed as a recording choice, not route absence.
- *Wrong route name `GET /apps/<name>` (HIGH).* Corrected to **`/.wanix/app/<name>`** (method-agnostic; source dir is `apps/`). No HTTP-method implication.
- *`/n/<peer>` presented as shipped (MEDIUM).* `missing-half-of-9p`, `import-export-and-n`, and `devices-import-for-free` each gain a one-line honest-limit: the shipped CLI mount binds the single slot **`/n/remote`** (`crates/wanix-cli/src/mount.rs:26`); per-peer `/n/<peer-id>` is designed-but-unshipped. `/n/<peer>` stays as a labeled *convention*.
- *FakeEngine `approve:` conflation (LOW).* Clarified: a **prompt** prefixed `approve:` parks an approval request (`req-<turn>`); the operator resolves it by writing the bare verb `approve <req>` to `#agent/<id>/ctl`. Two files, two tokens.
- *Discovery route paths (LOW).* Pages show the doc is at `/.well-known/wanix.json` while its routes point at `/.well-known/export9p` (p9), `/.well-known/rootfs.json`, `/.well-known/ethernet` (not-implemented).
- *Missing single-attach note.* The attach-and-capability contract page and `remotefs-import-half` cite the concrete v1 limit at `crates/wanix-9p/src/lib.rs:128-133` ("the most recent authorized Tattach wins; `default_root` only until the first attach"). One sentence notes that the **mesh path uses `aname`** in `AttachPolicy::evaluate(peer, aname)` (`session.rs:62`) while the **plain serve path discards it** — so the two are not contradictory.

**Newcomer onboarding**
- *Broken `#kv` examples (HIGH).* Every beginner `#kv` example runs **inside the served namespace** (the `/.wanix/app/<name>` route under `serve --wanix-services`), never a bare `wanix qjs` where `#kv` is unbound. Recipe 04 §3b is rewritten to exercise the counter through the running serve process; the key is **`#kv/http-counter`** (the shipped key in `data/apps/counter.js:7`).
- *`wanix-rust` not on PATH (HIGH).* One **Build-once preamble** at the top of the Learn index and every flow; `wanix-rust` (aliased) used consistently thereafter.
- *Undocumented cockpit prereq (HIGH).* A prerequisite badge before the cockpit step (`cd workbench && make build`, needs Go) plus a CLI-only `wanix agent --fake` fallback so the agent payoff is reachable without the vscode-web build.
- *Recipe slug mismatches (MEDIUM).* Normalized on numbered recipe slugs under `/recipes/`; flows under `/learn/`; link-checker enforces.
- *Host-mount missing setup (MEDIUM).* The `mkdir -p /tmp/wanix-host && echo …` line is in the step with expected output.
- *Counter key drift (MEDIUM).* Single key `#kv/http-counter` everywhere; the file lives at `data/apps/counter.js` relative to `--root`.
- *Cockpit-before-shell ordering / `--root` inconsistency (LOW).* A `qjs-shell` step precedes the cockpit; `--root` documented once as optional ("disposable root; nothing persists").

**Expert findability**
- *Capability contracts mis-filed as concepts (HIGH).* **New `/reference/attach-and-capability-contract`** — a flat spec: `evaluate(peer, aname) -> Option<Authorization>` (`crates/wanix-id/src/policy.rs:14`), default-deny GrantTable (`grant.rs`), `SubtreeFs`/`Rights`, verified-PeerId keying, `Tauth=ENOSYS`, the v1 single-attach note. Concept pages keep the "why"; both cross-link; search indexes `attach`/`capability`/`grant`/`EACCES`/`errno 13`.
- *Promised glossary absent (HIGH).* **New `/find/glossary`** A–Z with the typed primitives listed in §4.
- *"Add a service device" scattered (MEDIUM).* `/reference/extension-points` becomes the **single canonical contract** with the full checklist (implement `FileSystem` → add to `serve/roots.rs:119` bind set + `INSPECTABLE_SERVICE_DEVICES` → mesh import for free) and `#kv` as the worked template; the flow stays as the hand-held route.
- *Broken cross-link slugs (MEDIUM).* One canonical scheme + CI link-checker (see §8).
- *No device anchor convention / index (MEDIUM).* The Devices index lists all 8 devices with per-file anchor tables; convention is one H2/H3 per file path, slugified to match.
- *CLI index buried (LOW).* Added to the "I'm extending the core" Home on-ramp and the `cli` tag.

**Completeness & gaps**
- *`wanix new` absent (HIGH).* **New recipe 00** + listed in the CLI index; step 0 of the http-app flow. Flags `--js`/`--rust`, `wasm32-wasip1` target, generated SDK/tsconfig (verified `crates/wanix-cli/src/new.rs`).
- *Guest SDK undocumented (HIGH).* **New `/reference/guest-sdk`** for `examples/lib/wanix/` (`fs.js`, `process.js`, `task.js`, `bytes.js`, `index.js`, `.d.ts`) — the positive counterpart to `guest-js-guardrails`.
- *`#pipe` orphaned (MEDIUM).* A `#pipe` step added to the add-a-service-device flow; `devices/pipe` "Used in flows" rail is non-empty.
- *`POST /agent` + `--insecure-open` page-less (MEDIUM).* `POST /agent` documented on `devices/agent`/`http-app-route`; `--insecure-open` (read-write data, **no exec**, `help.rs:43`) added to the capability contract / trust-boundary-gaps as a distinct grant shape.
- *`p9-listen` grant grammar undocumented (MEDIUM).* `ANAME:PREFIX:RIGHTS` + `--peer/--grant/--once` documented in the CLI index, cross-linked from the capability contract.
- *Scaling under-surfaced (MEDIUM).* **New `/reference/performance`** sourced from `docs/scaling-eli5.md` + the `compute_bench` example, backing the rooms-not-houses claim.
- *Kemi's flow dead-ends on a concept (LOW).* Re-ordered to land on recipe 01/05; `fakeengine-vs-codex` is the penultimate honest beat.
- *`wanix agent --world` undocumented (LOW).* Documented in the CLI index (`--world` is the confinement boundary; `--fake` the override; verified `crates/wanix-cli/src/agent.rs:42,50`).

**IA coherence**
- *Three slug schemes (HIGH).* One canonical `section/page` scheme; manifest doubling flattened; link-checker enforces.
- *Orphaned flow pages (HIGH).* Each of the 8 flows is a real page at `/learn/<flow-slug>`; Home and flow-to-flow links target them.
- *learn/index mis-filed + bad relative links (HIGH).* `section: learn`; absolute canonical links; reference names reconciled (`adrs`→`adr-index`, `architecture`→`crate-map-and-layering`, `ninep`→`the-9p-contract`); `mesh-blueprint` linked as an external source ref, not a site page.
- *Recipes vs use-cases blur (HIGH).* One-directional split: use case = why, ends in exactly one recipe link; recipe = how, links up to one use case; recipes carry `user`/`power-user` tags.
- *Phantom `/flows/` section (MEDIUM).* Banned; flows live only under `/learn/`.
- *Concept-orphan overclaim (MEDIUM).* IA claim softened to "key concepts threaded; rest reference-only," with `/find/concepts` as the guaranteed inbound link for every node.
- *EOF-contract canonical home (MEDIUM).* Lives on `blocking-stream-eof-contract`; device pages link it.
- *Flow-progress breadcrumb undesigned (MEDIUM).* Specified: a "Flow X — step N of M — Prev/Next" rail bound to the flow definition, shown only when arrived-at via a flow.
- *"nine flows" count drift (LOW).* Copy says **eight**, derived from the flow list.
- *Twin/caveat duplication (LOW).* Twin rule + single canonical home per caveat (§4).

---

## 11. First-PR slice

The smallest valuable first commit: **the MVP spine of the `js-outside-chrome` flow, scaffolded and link-checked.**

Concretely, one PR that lands:
1. `docs/site/` Astro+Starlight scaffold with the content schema (§8) and the **link-checker + honestLimits-required build gate**.
2. The **Home page** with the live "outside Chrome: true" hero cast (recorded from `wanix qjs examples/qjs-demo.js`).
3. The **`/learn/js-outside-chrome` flow page** with the Build-once preamble, the `qjs-shell` step, and the cockpit prereq badge + `agent --fake` fallback.
4. Four canonical concept pages — `everything-is-a-file`, `per-process-namespaces`, `service-devices`, `qjs-task` — each with a source-of-truth box and a honest-limits block.
5. `recipes/walkthrough-1-run-js` (the verbatim hero transcript) and one device page (`devices/task`).

**Why this slice:** it proves the entire architecture in miniature — dual-mode (concept + flow), the slug scheme, the link-checker, the honest-limits gate, a live cast, and a real device anchor page — and it delivers a complete newcomer first-win path that the newcomer review confirmed actually works once the PATH/`#kv` fixes are in. It is independently shippable and reviewable, and it sets the patterns every later page follows.

---

## 12. Success metrics

The site works when it serves both ends of the audience without compromise. Measure on a manual persona-walk plus instrumentation:

**Newcomer (Maya / Kemi entry):**
- **Time-to-first-win < 10 min** from the flow start (clock starts after the Build-once step, per the honest compile-cost note).
- **Zero dead commands:** every code block a newcomer pastes runs as written (CI link-check + a "snippet smoke test" that runs the LIVE-cast commands).
- **No crash on a taught operation:** every `#kv` example runs end-to-end inside `serve --wanix-services`.
- **Flow completion rate:** a reader who starts `js-outside-chrome` reaches the "write your own `apps/counter.js`" step.

**Expert (Aria / Priya / Theo / Devraj entry):**
- **≤ 2 clicks to any contract:** search `evaluate` → attach-and-capability contract; search `#agent reply` → `devices/agent#reply`; search `wanix/9p/1` → glossary. (Measured by a fixed query set.)
- **Deep-link stability:** every device-file anchor and trait-method anchor resolves; no broken anchors (CI).
- **Source-traceability:** every behavioral claim links to a `crate/src/file.rs:line` that exists (a build check resolves `sourceRefs` against the repo).
- **Honesty coverage:** every page where overclaim is possible has a `honestLimits` block (enforced by the schema); each canonical caveat has exactly one home and N links, zero duplicate blocks.

**Structural health (both audiences):**
- **Zero orphans:** every concept-graph node has an inbound link from `/find/concepts` (orphan report green).
- **Zero broken links:** the build fails on any `href` outside the page set, and on the banned `/flows/`, `/concepts/<flow>`, `/use-cases/<flow>` strings.
- **No framework lock-in:** the content tree is portable markdown; a `grep` confirms no behavior is encoded only in Astro components.

The single sharpest signal that the site is doing its job: **Aria reads the honest-gaps page and the self-check `warn` and trusts the project *more*** — because the docs are demonstrably more accurate than the upstream `AGENTS.md` in every place the reviews caught.
