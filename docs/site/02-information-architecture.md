# Wanix Site — Information Architecture

# Wanix Documentation — Information Architecture

## 0. Design goals and the central tension

Wanix is a Rust-native, Plan 9-style OS core that runs outside Chrome: everything is a file, every process has its own rearrangeable namespace, and one 9P contract reaches local services and remote machines alike. The audience spans a JS developer who has never heard of Plan 9 (Maya) to a Plan 9 researcher probing whether the capability model is principled (Dr. Aria Vance). A single linear tutorial fails the expert; a flat reference fails the newcomer.

The architecture resolves this with a **dual-mode spine**: every idea exists once as a canonical **Concept page** (reference-grade, deep-linkable, searchable) and is *threaded* by one or more **Learning Flows** (ordered, hand-held, persona-shaped). Flows never duplicate concept content — they sequence, motivate, and link into it. An expert ignores flows entirely and lands on a concept, a device-contract anchor, an ADR, or a crate via search/index/breadcrumb. A newcomer starts a flow and is carried.

The non-negotiable IA rule, drawn straight from the research: **honesty is a first-class content type.** Every concept page and use-case carries a "Status / honest limits" block (FakeEngine vs real codex, `#kv` is in-memory, single-frame serve, exec-is-local-trust, no hard CPU/mem limits, mesh has no ADRs yet). Aria trusts the project *for* its caveats; the IA must surface them, never bury them.

## 1. Top-level sections (global navigation)

The global nav has eight destinations. Five are content modes; three are findability surfaces.

1. **Home** (`/`) — purpose: the 30-second hero (`wanix qjs examples/qjs-demo.js` → "outside Chrome: true"), the one-line thesis ("Plan 9 reincarnated for the age of agents"), and four persona on-ramps ("I want to run JS", "I want to wire machines together", "I'm extending the core", "I want the deep idea"). Audience: all.
2. **Learn** (`/learn`) — purpose: the catalog of guided Flows, grouped by persona/goal, each with an est. time and prerequisite badge. The hand-held mode. Audience: all (newcomers first).
3. **Concepts** (`/concepts`) — purpose: the canonical reference for every idea, one page per concept, each a node in the Concept Graph. The findable mode. Audience: all, but the spine for developers/visionaries.
4. **Devices** (`/devices`) — purpose: the device-contract reference: `#task`, `#term`, `#kv`, `#pipe`, `#plumb`, `#cas`, `#agent`, `#cpu`. Each page is a stable set of deep-link anchors (one per file: `#kv/<key>`, `#agent/<id>/reply`, etc.). This is where Theo and Devraj live. Audience: developer + power user.
5. **Use cases** (`/use-cases`) — purpose: the "why", outcome-first: personal compute mesh, agents on your files, portable worlds, browser cockpit, distributed dev. Audience: visionary + decision-maker.
6. **Reference** (`/reference`) — purpose: ADRs (0001–0005), crate map + dependency direction, `FileSystem`/`File` trait reference, `just` recipes / quality gates, discovery-doc / handoff JSON, CLI command index. Audience: developer (Priya, Theo).
7. **Recipes** (`/recipes`) — purpose: the five hands-on, copy-paste recipes (01 repair, 02 mount peer, 03 capsule, 04 #kv HTTP app, 05 two agents) plus the walkthrough sections. Demos with real transcripts. Audience: user + power user.
8. **Find** (`/find`) — purpose: the findability hub: full-text search, the Concept Index (graph-backed), the A–Z Glossary, and a Tag browser. Audience: all, expert-favoring.

## 2. The dual-mode design

### Concept reference vs guided flow — one source, two doors

Every concept (e.g. "A capability is a bind") has exactly one canonical page under `/concepts/<slug>`. That page is *self-contained reference*: a one-liner at top, a body, a **source-of-truth box** (cite the exact file+lines, e.g. `crates/wanix-vfs/src/subtree.rs:58-176`, linked into the repo), a **Status / honest limits** block, a **See also** rail (graph neighbors), and a **Used in flows** rail (every flow step that threads it).

A flow step is a thin wrapper: a goal sentence, a "do this" action (a command, a click, a file op), an "observe this" expected output, and a **"Read the concept →"** link to the canonical page. Flows own *motivation and order*; concepts own *truth*. This means the maintenance cost of accuracy is paid once.

### Progressive disclosure (three depths on every concept page)

- **Depth 1 — the one-liner + analogy.** Maya reads "#kv/<key> is a file: read returns the value, write commits on close." No Plan 9 history required.
- **Depth 2 — the body + a runnable/clickable example.** The mechanics, the file tree, the demo.
- **Depth 3 — collapsible "Under the hood" + source box + honest-limits.** The fid lifecycle, the `confine_to_prefix` symlink fix, the exact crate lines, the gaps. Collapsed by default so newcomers aren't drowned; one click for Priya/Aria.

Newcomers read top-down and stop when satisfied; experts jump to Depth 3 / the source box immediately.

### How an expert bypasses the hand-holding

- **Search-first**: `/find` is in the global nav and bound to `/` keypress; typing `#agent reply` lands directly on `/devices/agent#reply`.
- **Deep-link anchors**: every device file, trait method, and ADR has a stable `#anchor`. Devraj bookmarks `/devices/cpu#read-only-jail`; Priya bookmarks `/reference/filesystem-trait#content_hash`.
- **Concept Index** (`/find/concepts`): a dense, flow-free A–Z + graph-cluster list; one click to any canonical page.
- **"Expert path" callout** at the top of every flow: "Already know namespaces? Jump to the `#cpu` contract →" so even inside a flow the escape hatch is one click.
- **Crate map as a navigable diagram**: clicking `wanix-kv` in the dependency graph opens the crate's concept + source.

## 3. Concept graph (the backbone)

The graph is the spine of findability: nodes are concepts, typed edges are `prerequisite-of`, `composed-of`, and `related-to`. It powers three UI affordances: (a) "See also" rails (the `related-to` and `composed-of` neighbors), (b) prerequisite badges on flow steps and concept pages (the `prerequisite-of` in-edges, e.g. "Understand *Per-process namespaces* first"), and (c) the visual Concept Map at `/find/concepts` where clusters are colored by audience (newcomer / developer / visionary).

The root of the graph is **Everything is a file** → which is `prerequisite-of` **The FileSystem trait**, **Service devices (#name)**, **Per-process namespaces**, and **Import/export and /n/**. The mesh cluster composes from **RemoteFs**, **9P over iroh QUIC**, **The key is the address**, and **A capability is a bind**. The agent cluster (`#agent`, approvals-as-files, agents-as-operators) hangs off the device + namespace clusters. See the structured `conceptGraph` for the full node/edge set.

## 4. Learning flows (one per major persona)

Each flow is an ordered set of steps; each step names a goal and links into canonical concept/device/recipe pages. Flows are sized for their persona's patience and seeded by the persona journeys in the brief.

- **"JavaScript outside Chrome in 10 minutes" (Maya)** — hero command → "what just happened" (tasks + namespaces, jargon-free) → feed env/cwd/stdin/argv (walkthrough §2) → explicit host mount (§3, "no ambient authority" lands gently) → boot the cockpit with `--wanix-services` → click-through Agent Repair / Duet / HTTP counter / Self-check, each with a "what file ops this really is" reveal → write her own `apps/counter.js` (recipe 04) → honesty callouts (FakeEngine, in-memory `#kv`, freeze with a capsule) → soft hand-off to the mesh.
- **"Wire two machines into a mesh" (Devraj)** — recipe 02 two-terminal transcript (with the `/n/remote` single-slot caveat upfront) → identity primer (`~/.wanix/node.key`, key-is-the-address, why Tauth stays ENOSYS) → devices-for-free (read `/n/A/#kv/<key>`) → `#cpu` exec plane (reverse-export, read-only jail, `--write`, batched-output/no-remote-cancel limits) → trust-boundary deep dive (default-deny GrantTable, capability-as-SubtreeFs, public-endpoint refusals, exec-local-trust) → capsules (freeze to a hash, not-portable list) → multi-agent + `#plumb` (recipe 05, single-frame caveat) → cockpit as dashboard + "not-yet-shipped" page.
- **"Build an HTTP app backed by #kv" (Maya → builder)** — serve with services → understand `/.wanix/app/<name>` route → write `apps/counter.js` doing read-modify-write on `#kv/counter` → curl + cockpit increment one counter → "state survives because the device lives for the serve process; freeze with a capsule to persist" (recipe 04 honesty).
- **"Add a service device" (Theo)** — extension-points landing → `#kv` as the canonical minimal `FileSystem` (`crates/wanix-kv/src/lib.rs`, ~177 lines) → implement open/read_dir/remove, the blocking-stream EOF contract → bind into the served namespace (`roots.rs`) → the payoff: mesh import + 9P export + cockpit inspector for free → device-contract cards + don't-overclaim caveats.
- **"Add a task driver / a transport" (Theo)** — `TaskDriver` check/start, register-from-above, shared `task_wasi_config`, auto-select-by-suffix → transport as a thin adapter over `P9Server`/`RemoteFs`, frame-boundary rule, protocol-vs-server split → honest constraints (Tauth ENOSYS, single-frame serve, local-trust exec) → layering rules + `just check`.
- **"The agent on your files" tour (Kemi)** — agents-as-operators thesis → `#agent` as files / approvals as files (recipe 01) with the cockpit Repair Demo as the live one-click trust-gate proof → reach across the mesh (`/n/<peer>`, `#cpu`, `#plumb`, recipe 05 over QUIC) → capability security she can reason about (SubtreeFs, "no outside to name", default-deny, exec-local-trust) → rooms-not-houses cost story (with the "no hard CPU/mem limits yet" caveat) → provenance roadmap (traceable namespaces, clearly labeled exploratory) → the honest "not yet" page.
- **"The Plan 9 ideas tour" (Aria)** — the thesis as a proof: the missing half of 9P (export vs import, `/n/`, one protocol + one bind = network transparency) → browser-as-host vs Wanix-as-host (with the honest "Go Wanix is still broader" admission) → the mechanism in code (RemoteFs as the photographic negative of `serve_stream`, the five hostile-peer corrections) → the trust boundary as one pure function (`AttachPolicy::evaluate`, default-deny, capability-as-bind, `confine_to_prefix`, key-is-address, Tauth ENOSYS) → the full Plan 9 cast slice by slice (factotum→QUIC, venti→`#cas`, cpu(1)→`#cpu`, plumber→`#plumb`) → the honest gaps page (single-attach-per-connection, the shared-`P9Server.root` seam, no public auth, no ethernet/vnet, `#cpu` cancel doesn't stop remote work, blob plane experimental).
- **"Contribute to the core" (Priya)** — developer landing ("Rust port, not the Go tree; use `just check`, ignore the Go Makefile/CONTRIBUTING") → crate map + dependency direction (async/iroh confined to `wanix-mesh`, no upward deps) → active ADRs 0001–0005 + ADR-vs-commit-message workflow → `FileSystem`/`File` trait reference (defaults, device-aware semantics, which methods default to `NotSupported`) → worked example `#kv` → code-quality guardrails + `just` recipes → queued follow-ups as "good first cleanups" (split `codex.rs`/`exec_server.rs`/`app.rs`, wire the v86-shared-demo stub).

## 5. Findability

- **Search** (`/find`, hotkey `/`): indexes concept pages, device anchors, ADRs, crates, recipes, and glossary terms. Results are typed (Concept / Device file / ADR / Crate / Recipe / Glossary) and show the one-liner. A query like `poll_oneoff` lands on the wasm-linker concept; `EACCES` lands on the grants/capability concept.
- **Tags**: every page carries a controlled tag vocabulary — by audience (`newcomer`, `developer`, `visionary`), by cluster (`namespace`, `mesh`, `task-runtime`, `device`, `security`, `serve`), and by status (`shipped`, `local-trust-only`, `exploratory`, `caveat`). Tag pages aggregate; the Tag browser is the expert's faceted filter.
- **Concept Index** (`/find/concepts`): the graph rendered two ways — an A–Z flat list and the clustered visual map. Each node links to its canonical page; hovering shows in/out edges.
- **A–Z Glossary** (`/find/glossary`): terse definitions for every term (`9P`, `ALPN`, `BindPosition`, `confine_to_prefix`, `EndpointId`, `fid`, `msize`, `NormalizedPath`, `SubtreeFs`, `Tauth`, `venti`, `.wcap`…), each linking to its concept/device page.
- **Deep-link anchors**: device files (`/devices/kv#key`, `/devices/agent#reply`, `/devices/cpu#read-only-jail`), trait methods (`/reference/filesystem-trait#content_hash`), ADR sections, and the five hostile-peer corrections each get stable `#anchors`. This is how Devraj and Aria cite and bookmark.
- **Breadcrumbs**: every page shows `Section › Cluster › Page › #Anchor` so an expert who arrived by search knows where they are and can climb to siblings.

## 6. Cross-linking strategy

The four content types form a cycle, and every page closes the loop:

- **Flow step → Concept**: "Read the concept →" (truth lives in the concept).
- **Concept → Flows**: "Used in flows" rail (how to learn it in context).
- **Concept ↔ Concept**: "See also" rail from the graph's `related-to`/`composed-of` edges; "Prerequisites" from `prerequisite-of` in-edges.
- **Concept → Device / Reference**: a concept like "Service devices (#name)" links to each device page; "The FileSystem trait" links to the trait reference and ADR 0001.
- **Concept → Use case + Demo**: every concept lists "Seen in" (the recipes/demos that exercise it, e.g. "A capability is a bind" → recipe 02 grant transcript + the over-the-wire EACCES demo).
- **Use case → Concepts + Recipe**: each use case ("personal compute mesh") opens with the outcome, links the concepts it rests on, and ends in the runnable recipe.
- **Reference (ADR/crate) → Concept**: ADR 0002 links to the task-runtime concepts; the `wanix-kv` crate links to the `#kv` device page and the "add a service device" flow.

Honesty links are bidirectional too: every "Status / honest limits" block links to the related queued-follow-up so a reader can see whether a caveat is being worked.

## 7. Navigation model — "I'm here, where next"

- **Global nav** (top bar): Home · Learn · Concepts · Devices · Use cases · Reference · Recipes · Find. Persistent.
- **In-section nav** (left rail): contextual. In Concepts, the rail is the graph clusters; in Devices, the device list + per-device anchor TOC; in Reference, ADRs + crates + trait + gates; in Learn, the flow's ordered steps with a progress indicator.
- **In-page**: a right-rail TOC (the `#anchors`), breadcrumbs above the title, and the See-also / Used-in-flows / Seen-in rails below the body.
- **The "where next" affordance**: every concept page ends with a "Next" block driven by the graph — for a newcomer it suggests the next `prerequisite-of` consumer ("Now that you know namespaces, see *Service devices*"); for a flow it's the next step; for an expert the See-also rail is the lateral move. Flows always show step N of M with prev/next and an "exit to reference" link so the hand-holding is never a cage.

This IA lets Maya ride a 10-minute flow to a visible result, lets Devraj wire two nodes from a transcript and bookmark the `#cpu` jail anchor, lets Priya land a clean PR from the crate-map-plus-ADR reference, lets Theo template a new device off `#kv`, and lets Aria audit the trust boundary down to `subtree.rs:58-176` and the honest-gaps page — all from one set of pages, reached two ways.

---

## Appendix A — Top-level sections

| Section | Slug | Audience | Purpose |
|---|---|---|---|
| Home | `home` | all | The 30-second hero (wanix qjs examples/qjs-demo.js -> 'outside Chrome: true'), the one-line thesis ('Plan 9 reincarnated for the age of agents'), and four persona on-ramps that route to the right flow. |
| Learn | `learn` | all, newcomers first | Catalog of guided learning flows grouped by persona/goal, each with an estimated time and prerequisite badges. The hand-held mode; threads concepts in order without duplicating them. |
| Concepts | `concepts` | all; spine for developers and visionaries | Canonical reference for every Wanix idea, one page per concept, each a node in the Concept Graph with progressive disclosure (one-liner / body+example / under-the-hood+source+honest-limits) and See-also / Used-in-flows rails. |
| Devices | `devices` | developer, power user | Device-contract reference for #task, #term, #kv, #pipe, #plumb, #cas, #agent, #cpu. Each page is a stable set of deep-link anchors, one per file, plus the shared blocking-stream EOF contract and don't-overclaim caveats. |
| Use cases | `use-cases` | visionary, decision-maker | Outcome-first 'why' pages: personal compute mesh, AI agents on your files across devices, portable worlds via capsules, browser-native operator cockpit, distributed dev environments. Each links the concepts it rests on and ends in a runnable recipe. |
| Reference | `reference` | developer (core contributor, integrator) | ADRs 0001-0005 + ADR/commit workflow, the crate map and dependency-direction diagram (async/iroh confined to wanix-mesh, no upward deps), the FileSystem/File trait reference, just check quality gates, discovery-doc/handoff JSON, and the CLI command index. |
| Recipes | `recipes` | user, power user | The five hands-on copy-paste recipes (01 agent repair, 02 mount remote peer, 03 freeze to capsule, 04 #kv HTTP app, 05 two agents collaborate) plus the numbered walkthrough sections, with real transcripts and verified screenshots. |
| Find | `find` | all, expert-favoring | The findability hub: full-text typed search (hotkey '/'), the graph-backed Concept Index (A-Z + clustered visual map), the A-Z Glossary, and the Tag browser (by audience/cluster/status). The expert's low-friction entry point. |

## Appendix B — Learning flows

### JavaScript outside Chrome in 10 minutes  `(js-outside-chrome)`

- **Persona:** Maya, the curious app developer
- **Goal:** Run a .js file as a Wanix task, feed it real process context, then boot the cockpit and click through the demos seeing 'everything is a file' as concrete file operations.

1. **The 30-second hero** — Copy 'wanix qjs examples/qjs-demo.js' and see 'outside Chrome: true' — JavaScript running as a Wanix task, not in a browser. Immediate visible result before any theory.  _(`recipes/walkthrough-1-run-js`)_
2. **What just happened** — Read one jargon-free paragraph: a task is a process Wanix owns; a namespace is its private file view. No Plan 9 history.  _(`concepts/everything-is-a-file`)_
3. **Feed env, cwd, stdin, and argv into a script** — Pass --env/--cwd/--stdin and -- argv and watch scriptArgs, std.getenv, fd 0, and #task/self/id flow in as task state — no globalThis.Wanix bridge.  _(`recipes/walkthrough-2-process-context`)_
4. **Mount a host directory explicitly** — Bind a rooted LocalFs with --mount and learn 'host files are not ambient authority' gently — the task sees a Wanix path, the host boundary is enforced.  _(`concepts/host-not-ambient-authority`)_
5. **Boot the browser cockpit** — Run serve --bundle workbench-fs9p --wanix-services (note: --wanix-services is what turns the demos on) and open the printed URL into Code OSS.  _(`use-cases/browser-cockpit`)_
6. **Click through the four cockpit demos** — Run Agent Repair, the qjs->wasm->qjs Duet, the HTTP counter + #kv, and the Self-Check, each with a 'what file ops this really is' reveal.  _(`concepts/service-devices`)_
7. **Build your own #kv-backed counter handler** — Write apps/counter.js doing read-modify-write on #kv/counter so 'everything is a file' clicks for real (recipe 04).  _(`recipes/04-tiny-http-app-with-kv`)_
8. **Honest limits and how to keep state** — Meet the caveats: the served #agent is a deterministic FakeEngine, #kv is in-memory; freeze a world with a capsule (recipe 03) when you ask 'how do I keep it'. Soft hand-off: 'your laptop is a node — reach another machine?'  _(`concepts/fakeengine-vs-codex`)_

### Wire two machines into a mesh  `(wire-a-mesh)`

- **Persona:** Devraj, the personal-compute-mesh power user
- **Goal:** Stand up two nodes with persistent identities, mount one from the other over iroh QUIC, operate its devices as files, send compute to the data, and lock down trust.

1. **Mount a remote peer (the two-terminal hero)** — Follow recipe 02: node B mesh-serves and prints an iroh:// ticket; node A mounts it and ls/cat/writes through /n/remote. Upfront note: mount-* binds at the single slot /n/remote; the peer-id lives in the ticket.  _(`recipes/02-mount-remote-peer`)_
2. **Identity primer: the key is the address** — Understand ~/.wanix/node.key (persisted ed25519, 0600), why the public key is both identity and dialable EndpointId, and why in-band Tauth stays ENOSYS (the QUIC handshake authenticates the peer).  _(`concepts/key-is-the-address`)_
3. **Devices import across the mesh for free** — Read /n/A/#kv/<key> and drive a peer's #cas with one 9P client and zero per-service network code — the payoff of everything-is-a-file.  _(`concepts/devices-import-for-free`)_
4. **Send a #cpu job to the data** — Reverse-export your cwd (default read-only, --write to opt in) and run a task ON the node holding the files. Learn the v1 limits: batched output, no remote cancel.  _(`devices/cpu`)_
5. **Lock down trust** — Configure a default-deny GrantTable keyed on the verified peer key, see capability-as-bind via SubtreeFs, and confirm public-endpoint refusals and exec-stays-local-trust, with the gating tests named.  _(`concepts/capability-is-a-bind`)_
6. **Freeze a world to a capsule** — capsule save a built world to a CAS-backed hash and reconstruct it byte-identical elsewhere from one hash; learn dedup + integrity and the explicit not-portable list.  _(`recipes/03-freeze-world-to-capsule`)_
7. **Multi-agent and #plumb coordination** — Run recipe 05: two agents collaborate via #agent/<id>/reply delegation and a cross-node #plumb handoff, with the single-frame-recv caveat called out.  _(`recipes/05-two-agents-collaborate`)_
8. **Cockpit as operator dashboard + the roadmap** — Use the cockpit over the mesh and read the 'not-yet-shipped' page: mesh panel, browser-as-peer, per-peer /n/<peer-id> mounts.  _(`use-cases/personal-compute-mesh`)_

### Build an HTTP app backed by #kv  `(http-app-with-kv)`

- **Persona:** Maya, the curious app developer (builder mode)
- **Goal:** Write a stateful HTTP handler whose request state lives in a Wanix device file, so 'everything is a file' becomes a thing you build, not just read about.

1. **Serve with services enabled** — Start serve --wanix-services so the /.wanix/app/<name> route and #kv turn available.  _(`reference/serve-and-discovery`)_
2. **Understand the /.wanix/app/<name> route** — See how the route resolves apps/<name>.{js,wasm}, allocates a #task, binds fds to trace files, runs it, and returns stdout with X-Wanix-Task-Id headers (loopback-only, services-gated).  _(`concepts/http-app-route`)_
3. **Write the counter handler against #kv** — Open #kv/counter RDONLY, parse, increment, open WRONLY|CREAT|TRUNC, write back — a stateless handler made stateful by a device file (recipe 04).  _(`devices/kv`)_
4. **Increment one counter from curl and the cockpit** — Confirm curl and the cockpit hit one shared #kv/http-counter and the count survives per-request task teardown.  _(`recipes/04-tiny-http-app-with-kv`)_
5. **Honest limit: state lives only while serve lives** — Learn that #kv is in-memory for the serve process lifetime; to persist, freeze into a capsule.  _(`concepts/kv-smallest-database`)_

### Add a service device  `(add-a-service-device)`

- **Persona:** Theo, the integrator / extension author
- **Goal:** Implement wanix_fs::FileSystem for a new device using #kv as the template, bind it into the served namespace, and get mesh import + 9P export + cockpit inspector for free.

1. **Extending Wanix from the edges** — Understand the three extension points (service device = FileSystem, task driver = TaskDriver, transport = 9P adapter) and that you do not need to fork the core.  _(`reference/extension-points`)_
2. **Study #kv as the canonical minimal FileSystem** — Read crates/wanix-kv/src/lib.rs (~177 lines): parse_path single-segment rule, snapshot-on-open reads, commit-on-close writes, read_dir enumerates keys.  _(`concepts/the-filesystem-trait`)_
3. **Match the device contracts** — Implement the blocking-stream EOF contract (Ok(0) only at true end-of-stream, 50ms re-check never signals EOF) and pick the right shape (allocate-then-operate vs key-as-file) without overclaiming.  _(`concepts/blocking-stream-eof-contract`)_
4. **Bind it into the served namespace** — Add the device to the bind set in serve/roots.rs and the INSPECTABLE_SERVICE_DEVICES source so discovery and the cockpit see it.  _(`reference/serve-and-discovery`)_
5. **Collect the mesh-for-free payoff** — Confirm that because your device is a plain FileSystem, /n/A/#yourdevice imports across the mesh and the cockpit inspector renders it with no device-specific code.  _(`concepts/devices-import-for-free`)_

### Add a task driver or a transport  `(add-driver-or-transport)`

- **Persona:** Theo, the integrator / extension author
- **Goal:** Add a new runtime via TaskDriver registered from above, or a new protocol transport as a thin adapter over P9Server/RemoteFs, while honoring the layering rules.

1. **Implement TaskDriver check/start** — Write check() to auto-select by program suffix and start() to launch, reusing the shared task_wasi_config; register the driver from the orchestration layer so the task core stays engine-free.  _(`concepts/task-drivers`)_
2. **Reuse the shared WASI/fd contract** — Build the live WASI config (namespace, cwd preopen, argv/env, stdio fds) from task_wasi_config and mirror dynamic file fds into the task table — do not reimplement it per runtime.  _(`concepts/shared-wasi-fd-contract`)_
3. **Add a transport as a thin 9P adapter** — Wrap P9Server (export) or RemoteFs (import) preserving frame boundaries; respect the protocol-vs-server split (codecs in wanix-protocol, fid->FileSystem mapping in wanix-9p).  _(`concepts/protocol-vs-server-split`)_
4. **Honor the honest constraints** — Account for Tauth ENOSYS (identity is a transport property), the single-frame-at-a-time serve (blocking recv needs a second connection), and exec-stays-local-trust.  _(`concepts/the-9p-contract`)_
5. **Fit the workspace: layering + just check** — Keep async/iroh out of every crate but wanix-mesh, avoid upward deps, stay under the 250-350 line module limit, and pass just check on the first try.  _(`reference/crate-map-and-layering`)_

### The agent on your files (tour)  `(agent-on-your-files)`

- **Persona:** Kemi Okafor, the AI-agent-infrastructure builder
- **Goal:** Confirm the agent really is files with approvals as files, that agents + compute compose across machines safely today, and find the real trust limits and the provenance roadmap.

1. **Agents are the operators namespaces always needed** — Read the thesis: per-process namespaces and file-shaped services were too fiddly for humans but native to an LLM that reads by cat, mutates by write, lists by ls.  _(`concepts/agents-as-operators`)_
2. **#agent as files, approvals as files** — Walk the device interface (new/prompt/events/reply/pending/ctl/status) and the trust gate: a powerful action parks in pending; nothing runs until a human writes approve <id> to ctl. Recipe 01 + the cockpit Repair Demo are the one-click proof.  _(`devices/agent`)_
3. **Reach: agents + compute across the mesh** — Bind /n/<peer> and the agent's cat/write/ls extend unchanged to a peer's #kv/#cas/files; #cpu sends the agent to the data; #plumb lets two agents on two machines hand off by typed message (recipe 05, over real QUIC).  _(`concepts/send-agent-to-the-data`)_
4. **Capability security you can reason about** — A grant is a re-rooted SubtreeFs ('no outside for the agent to name'), default-deny, keyed by the verified key — blast-radius control — plus the honest line that exec/agent devices stay local-trust until public auth lands.  _(`concepts/capability-is-a-bind`)_
5. **The cost/scale story** — Rooms-not-houses: ~0.25 MB wasm vs ~10 MB process (~40x), engine-per-task — WITH the explicit caveat that hard CPU/memory limits for arbitrary untrusted code are not wired yet.  _(`concepts/rooms-not-houses`)_
6. **The provenance roadmap** — Read the traceable/forkable/rewindable-namespaces vision ('show me what the agent changed; restore to before task 42'), clearly labeled exploratory, with #plumb provenance + the qjs-shell mutation frame as the first shipped slice.  _(`use-cases/traceable-namespaces`)_
7. **The honest 'not yet' page** — Confront the limits: served #agent = FakeEngine (not a live LLM by default), single-frame serve makes live #plumb recv publish-only on one connection, mesh panel / browser-as-peer queued, no public multi-user auth.  _(`concepts/fakeengine-vs-codex`)_

### The Plan 9 ideas tour  `(plan9-ideas-tour)`

- **Persona:** Dr. Aria Vance, the Plan 9 / distributed-systems thinker
- **Goal:** Evaluate whether Wanix recovered the deep idea (per-process namespaces + import/export as the universal mechanism) and whether its capability model is principled, auditing both claims and gaps in source.

1. **The missing half of 9P, stated as a proof** — Export vs import, /n/, and why everything-is-a-file makes one protocol plus one bind buy network transparency across every service at once.  _(`concepts/missing-half-of-9p`)_
2. **Browser-as-host vs Wanix-as-host** — The deliberate host-boundary move out of Chrome, plus the honest admission that Go Wanix is still broader as a browser system (no parity claim).  _(`concepts/wanix-as-host`)_
3. **The mechanism in code** — RemoteFs as the photographic negative of serve_stream, reusing the same codecs, plus the five hardening corrections against a hostile peer (frame-size ceiling, honest seekability, RAII fid guards, bounded read_dir, server-side O_APPEND).  _(`concepts/remotefs-import-half`)_
4. **The trust boundary as one pure function** — AttachPolicy::evaluate -> Option<Authorization>, default-deny GrantTable keyed by the verified key, 'a capability is a bind' via SubtreeFs + the confine_to_prefix symlink fix, the key IS the address, and Tauth stays ENOSYS.  _(`concepts/capability-is-a-bind`)_
5. **The full Plan 9 cast, slice by slice** — factotum->QUIC handshake, venti->#cas, cpu(1)->#cpu (send the job to the data, default read-only jail), plumber->#plumb (typed, broker-less, best-effort) — each one impl FileSystem + one bind.  _(`concepts/devices-import-for-free`)_
6. **The honest gaps page** — Single-attach-per-connection, the 9P session/namespace seam (one shared P9Server.root in the plain path that discards uname/aname), no public multi-user auth, no ethernet/vnet, #cpu cancel doesn't stop remote computation, blob plane upstream-experimental.  _(`concepts/trust-boundary-gaps`)_

### Contribute to the Wanix core  `(contribute-to-core)`

- **Persona:** Priya, the Rust core contributor
- **Goal:** Understand the crate map, dependency rules, ADRs, and quality gates well enough to land a PR cleanly on the first try and pick a good first cleanup.

1. **Developer-track landing** — Orient: this is the Rust port, not the Go tree — ignore the Go Makefile/CONTRIBUTING; use just check.  _(`reference/contributor-landing`)_
2. **Crate map + dependency direction** — Read the layered diagram with async/iroh confined to wanix-mesh and the no-upward-deps rules called out, so new code goes in the right crate.  _(`reference/crate-map-and-layering`)_
3. **The active ADR set and workflow** — Review ADRs 0001-0005 each linked to source and the ADR-vs-commit-message workflow before touching any boundary.  _(`reference/adr-index`)_
4. **The FileSystem/File trait reference** — Have the trait at hand: defaults, device-aware semantics (read_ready/write_ready, is_seekable, content_hash, confine_to_prefix), and which methods default to NotSupported.  _(`reference/filesystem-trait`)_
5. **Worked example: #kv as the canonical minimal FileSystem** — Walk crates/wanix-kv/src/lib.rs (~177 lines) end to end as the template for a clean, small FileSystem implementation.  _(`concepts/the-filesystem-trait`)_
6. **Quality guardrails and the just recipes** — Run fmt, module-lines, clippy -D warnings, test, and the composite just check; stay under the 250-350 line module limit; use explicit newtypes over raw i32 flags.  _(`reference/quality-gates`)_
7. **Pick a good first cleanup** — Choose from queued follow-ups: split an over-limit module (codex.rs ~307, exec_server.rs ~283, app.rs ~273) or port the v86-shared-demo stub.  _(`reference/queued-follow-ups`)_

## Appendix C — Concept graph

**Nodes (76):** `Everything is a file`, `The FileSystem trait`, `NormalizedPath`, `Per-process namespaces`, `Namespace binding (bind/mount)`, `Namespace resolution (longest-prefix, union)`, `Service devices (#name)`, `Tasks own process identity`, `The fd table`, `The #task device`, `Task drivers`, `Shared WASI/fd contract`, `Wasmtime as substrate`, `Wanix-backed WASI (not host WASI)`, `Host files are not ambient authority`, `qjs task (JavaScript outside Chrome)`, `Compiled wasm task driver`, `Command-style wasm linker (poll_oneoff = NOSYS)`, `Two tiers, one substrate`, `Compiled-artifact cache`, `QuickJS snapshots are VM images`, `Bounded execution policy (not a scheduler)`, `#term device`, `qjs-shell (shell as a guest program)`, `Raw vs cooked input & control bytes`, `Resize / winch lifecycle`, `The 9P contract`, `Protocol vs server split`, `RemoteFs (import half of 9P)`, `The missing half of 9P`, `Import/export and /n/`, `9P over iroh QUIC under one ALPN`, `The async/sync bridge confined to one seam`, `StreamingImportFs (one stream per blocking open)`, `Five hostile-peer corrections`, `The key is the address`, `Persisted ed25519 node identity`, `Tauth is ENOSYS (identity is a transport property)`, `A capability is a bind`, `AttachPolicy (trust boundary as one pure function)`, `SubtreeFs / confine_to_prefix`, `Devices import across the mesh for free`, `One identity, two planes (9P control + iroh-blobs data)`, `#kv key/value device`, `#kv as the smallest database`, `#pipe byte channels`, `#plumb plumber bus`, `Best-effort epidemic delivery (not a queue)`, `Blocking-stream EOF contract`, `#cas content-addressed store (venti)`, `End-to-end hash verification`, `Content-addressed data plane (content_hash)`, `wanix capsule (CAS-backed world snapshots)`, `#cpu exec plane (send the agent to the data)`, `Send-agent-to-the-data (mesh reach)`, `#agent device (an LLM you can cat)`, `Approvals as files (the trust gate)`, `Agents are the operators namespaces always needed`, `FakeEngine vs codex (exec local-trust-only)`, `Rooms-not-houses (cheap scalable isolation)`, `Safe-for-untrusted-code is not claimable yet`, `Traceable / forkable namespaces (provenance, rewind)`, `Wanix-as-host, not browser-as-host`, `Crate layering & dependency direction`, `Serve as one local composition surface`, `Discovery document (/.well-known/wanix.json)`, `--wanix-services device set`, `Three serve bundles`, `The browser cockpit (operator surface)`, `Direct-9P operator surface (no side channel)`, `Live-stream vs one-shot 9P access`, `/.wanix/app/<name> HTTP route`, `Loopback-only VM and app handoffs`, `Trust-boundary gaps`, `Code-quality guardrails (just check)`, `ADR index & workflow`

**Edges:**

| From | Relation | To |
|---|---|---|
| `Everything is a file` | composed-of | `The FileSystem trait` |
| `The FileSystem trait` | composed-of | `NormalizedPath` |
| `The FileSystem trait` | composed-of | `Content-addressed data plane (content_hash)` |
| `The FileSystem trait` | composed-of | `Blocking-stream EOF contract` |
| `Everything is a file` | prerequisite-of | `Per-process namespaces` |
| `Everything is a file` | prerequisite-of | `Service devices (#name)` |
| `Per-process namespaces` | composed-of | `Namespace binding (bind/mount)` |
| `Per-process namespaces` | composed-of | `Namespace resolution (longest-prefix, union)` |
| `Namespace binding (bind/mount)` | related-to | `Namespace resolution (longest-prefix, union)` |
| `Per-process namespaces` | related-to | `SubtreeFs / confine_to_prefix` |
| `Service devices (#name)` | composed-of | `The #task device` |
| `Service devices (#name)` | composed-of | `#term device` |
| `Service devices (#name)` | composed-of | `#kv key/value device` |
| `Service devices (#name)` | composed-of | `#pipe byte channels` |
| `Service devices (#name)` | composed-of | `#plumb plumber bus` |
| `Service devices (#name)` | composed-of | `#cas content-addressed store (venti)` |
| `Service devices (#name)` | composed-of | `#agent device (an LLM you can cat)` |
| `The FileSystem trait` | prerequisite-of | `#kv key/value device` |
| `#kv key/value device` | related-to | `#kv as the smallest database` |
| `#pipe byte channels` | prerequisite-of | `Blocking-stream EOF contract` |
| `#plumb plumber bus` | composed-of | `Best-effort epidemic delivery (not a queue)` |
| `#cas content-addressed store (venti)` | composed-of | `End-to-end hash verification` |
| `#cas content-addressed store (venti)` | prerequisite-of | `wanix capsule (CAS-backed world snapshots)` |
| `#cas content-addressed store (venti)` | related-to | `Content-addressed data plane (content_hash)` |
| `Tasks own process identity` | composed-of | `The fd table` |
| `Tasks own process identity` | composed-of | `The #task device` |
| `Tasks own process identity` | composed-of | `Task drivers` |
| `Per-process namespaces` | prerequisite-of | `Tasks own process identity` |
| `Task drivers` | prerequisite-of | `qjs task (JavaScript outside Chrome)` |
| `Task drivers` | prerequisite-of | `Compiled wasm task driver` |
| `Task drivers` | related-to | `Shared WASI/fd contract` |
| `qjs task (JavaScript outside Chrome)` | related-to | `Compiled wasm task driver` |
| `qjs task (JavaScript outside Chrome)` | composed-of | `Two tiers, one substrate` |
| `Compiled wasm task driver` | composed-of | `Two tiers, one substrate` |
| `Compiled wasm task driver` | composed-of | `Command-style wasm linker (poll_oneoff = NOSYS)` |
| `Wasmtime as substrate` | prerequisite-of | `qjs task (JavaScript outside Chrome)` |
| `Wasmtime as substrate` | prerequisite-of | `Compiled wasm task driver` |
| `Wasmtime as substrate` | composed-of | `Wanix-backed WASI (not host WASI)` |
| `Wanix-backed WASI (not host WASI)` | related-to | `Host files are not ambient authority` |
| `Shared WASI/fd contract` | composed-of | `Wanix-backed WASI (not host WASI)` |
| `Shared WASI/fd contract` | related-to | `The fd table` |
| `qjs task (JavaScript outside Chrome)` | related-to | `Compiled-artifact cache` |
| `Compiled wasm task driver` | related-to | `Compiled-artifact cache` |
| `qjs task (JavaScript outside Chrome)` | related-to | `QuickJS snapshots are VM images` |
| `qjs task (JavaScript outside Chrome)` | related-to | `Bounded execution policy (not a scheduler)` |
| `Bounded execution policy (not a scheduler)` | related-to | `Safe-for-untrusted-code is not claimable yet` |
| `Two tiers, one substrate` | related-to | `Rooms-not-houses (cheap scalable isolation)` |
| `Rooms-not-houses (cheap scalable isolation)` | related-to | `Safe-for-untrusted-code is not claimable yet` |
| `#term device` | prerequisite-of | `qjs-shell (shell as a guest program)` |
| `qjs-shell (shell as a guest program)` | composed-of | `Raw vs cooked input & control bytes` |
| `#term device` | composed-of | `Resize / winch lifecycle` |
| `qjs-shell (shell as a guest program)` | related-to | `The #task device` |
| `Everything is a file` | prerequisite-of | `The 9P contract` |
| `The 9P contract` | composed-of | `Protocol vs server split` |
| `The 9P contract` | related-to | `Tauth is ENOSYS (identity is a transport property)` |
| `Import/export and /n/` | related-to | `The missing half of 9P` |
| `The 9P contract` | prerequisite-of | `RemoteFs (import half of 9P)` |
| `RemoteFs (import half of 9P)` | composed-of | `The missing half of 9P` |
| `RemoteFs (import half of 9P)` | prerequisite-of | `Import/export and /n/` |
| `RemoteFs (import half of 9P)` | composed-of | `Five hostile-peer corrections` |
| `RemoteFs (import half of 9P)` | related-to | `Protocol vs server split` |
| `Import/export and /n/` | prerequisite-of | `Devices import across the mesh for free` |
| `Service devices (#name)` | prerequisite-of | `Devices import across the mesh for free` |
| `9P over iroh QUIC under one ALPN` | composed-of | `The async/sync bridge confined to one seam` |
| `9P over iroh QUIC under one ALPN` | composed-of | `StreamingImportFs (one stream per blocking open)` |
| `9P over iroh QUIC under one ALPN` | composed-of | `One identity, two planes (9P control + iroh-blobs data)` |
| `RemoteFs (import half of 9P)` | prerequisite-of | `9P over iroh QUIC under one ALPN` |
| `Persisted ed25519 node identity` | prerequisite-of | `The key is the address` |
| `The key is the address` | prerequisite-of | `9P over iroh QUIC under one ALPN` |
| `The key is the address` | related-to | `Tauth is ENOSYS (identity is a transport property)` |
| `A capability is a bind` | composed-of | `SubtreeFs / confine_to_prefix` |
| `A capability is a bind` | composed-of | `AttachPolicy (trust boundary as one pure function)` |
| `AttachPolicy (trust boundary as one pure function)` | prerequisite-of | `The key is the address` |
| `Namespace binding (bind/mount)` | prerequisite-of | `A capability is a bind` |
| `A capability is a bind` | related-to | `FakeEngine vs codex (exec local-trust-only)` |
| `Devices import across the mesh for free` | prerequisite-of | `#cpu exec plane (send the agent to the data)` |
| `#cpu exec plane (send the agent to the data)` | related-to | `A capability is a bind` |
| `#cpu exec plane (send the agent to the data)` | related-to | `FakeEngine vs codex (exec local-trust-only)` |
| `#cpu exec plane (send the agent to the data)` | composed-of | `Send-agent-to-the-data (mesh reach)` |
| `#agent device (an LLM you can cat)` | composed-of | `Approvals as files (the trust gate)` |
| `#agent device (an LLM you can cat)` | related-to | `Agents are the operators namespaces always needed` |
| `Agents are the operators namespaces always needed` | prerequisite-of | `Per-process namespaces` |
| `#agent device (an LLM you can cat)` | composed-of | `FakeEngine vs codex (exec local-trust-only)` |
| `#agent device (an LLM you can cat)` | related-to | `Send-agent-to-the-data (mesh reach)` |
| `#plumb plumber bus` | related-to | `#agent device (an LLM you can cat)` |
| `wanix capsule (CAS-backed world snapshots)` | related-to | `#kv as the smallest database` |
| `#agent device (an LLM you can cat)` | related-to | `Traceable / forkable namespaces (provenance, rewind)` |
| `#plumb plumber bus` | related-to | `Traceable / forkable namespaces (provenance, rewind)` |
| `Wasmtime as substrate` | related-to | `Wanix-as-host, not browser-as-host` |
| `Crate layering & dependency direction` | related-to | `The async/sync bridge confined to one seam` |
| `Crate layering & dependency direction` | related-to | `ADR index & workflow` |
| `Crate layering & dependency direction` | related-to | `Code-quality guardrails (just check)` |
| `Serve as one local composition surface` | composed-of | `Discovery document (/.well-known/wanix.json)` |
| `Serve as one local composition surface` | composed-of | `--wanix-services device set` |
| `Serve as one local composition surface` | composed-of | `Three serve bundles` |
| `Serve as one local composition surface` | composed-of | `/.wanix/app/<name> HTTP route` |
| `Serve as one local composition surface` | related-to | `Loopback-only VM and app handoffs` |
| `--wanix-services device set` | prerequisite-of | `Service devices (#name)` |
| `Three serve bundles` | prerequisite-of | `The browser cockpit (operator surface)` |
| `The browser cockpit (operator surface)` | composed-of | `Direct-9P operator surface (no side channel)` |
| `Direct-9P operator surface (no side channel)` | composed-of | `Live-stream vs one-shot 9P access` |
| `The browser cockpit (operator surface)` | prerequisite-of | `The 9P contract` |
| `/.wanix/app/<name> HTTP route` | related-to | `#kv key/value device` |
| `Tauth is ENOSYS (identity is a transport property)` | related-to | `Trust-boundary gaps` |
| `AttachPolicy (trust boundary as one pure function)` | related-to | `Trust-boundary gaps` |
| `FakeEngine vs codex (exec local-trust-only)` | related-to | `Trust-boundary gaps` |
| `Best-effort epidemic delivery (not a queue)` | related-to | `Trust-boundary gaps` |
