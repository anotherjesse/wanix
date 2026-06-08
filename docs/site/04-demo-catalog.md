# Wanix Site — Demo & Interactive Plan

% Wanix Docs — Demo & Interactive Plan

This plan enumerates every demoable / interactive / recorded element the documentation site should feature, derived from the real recipes (`docs/recipes/01–05`), the verified cockpit screenshots (`docs/integration/screenshots/`), the in-tree examples (`examples/`), and the serve/cockpit research. Each element states what it shows, the persona it serves, the concept(s) it maps to, whether it can be live-interactive vs recorded, and the exact CLI command or cockpit action that produces it.

The over-arching rule from the IA: **honesty is a first-class content type.** Every demo below carries the honest-limit it must not overclaim (FakeEngine-not-LLM, `#kv` in-memory, single-frame `#plumb` recv, exec-is-local-trust, `--bundle cockpit` not yet renamed, vscode-web archive not vendored). The plan flags which demos are *safely* live, which must be recorded because they depend on un-vendored assets or two machines, and which are publish-path-only.

---

## 1. The three media tiers (what "interactive" can mean here)

The site has three realistic interactivity tiers, in descending order of confidence:

1. **Live in-browser asciinema / cast players (HIGH confidence).** Every single-process CLI flow (`wanix qjs …`, `wanix capsule …`, `wanix qjs-shell`) is deterministic, single-binary, and produces clean terminal transcripts. These should be recorded as asciinema `.cast` files and embedded with the asciinema player (scrub/copy/replay). They feel "live" without needing a backend. This is the workhorse tier for Home, Learn, Recipes, and Concepts.
2. **Recorded screenshots / short screencasts (HIGH confidence — already captured).** The browser cockpit is the one surface that genuinely needs a running `serve` + a vendored vscode-web build (`workbench/code/out`, fetched via `cd workbench && make build` — see `docs/integration/manual-test-plan.md:17-37`). The site must NOT try to embed a live cockpit; instead use the eight verified PNGs in `docs/integration/screenshots/` (captured 2026-06-07 at PASS state) plus optional short screencasts. These are the proof for the cockpit and agent flows.
3. **Genuinely live embedded cockpit (LOW confidence — DO NOT SHIP as the default).** A hosted, multi-tenant live cockpit would require running `serve --wanix-services` per visitor with exec devices (`#task`/`#agent`) exposed — and exec devices are explicitly **local-trust only** (`mesh-serve --wanix-services` is refused on a public endpoint; research GAPS). Offer at most a "run this locally in 60s" copy-paste block that boots the operator's own cockpit; never a shared public one.

A fourth tier — **copy-paste recipe blocks** — is not "interactive" but is the most load-bearing: every recipe (`docs/recipes/01–05`) is already written as verbatim shell. The site renders these with one-click copy and a "what file ops this really is" reveal.

---

## 2. Home — the 30-second hero

**H1. "Outside Chrome: true" hero cast (LIVE asciinema).**
- **Shows:** `wanix qjs examples/qjs-demo.js` printing `outside Chrome: true`, then `task id: 1`, the runtime line, and the round-tripped `hello.txt` it wrote into a Wanix namespace. Source verified: `examples/qjs-demo.js` reads `main.js`, writes/reads `hello.txt`, and reads `#task/self/id`.
- **Persona:** Maya (curious app dev); all.
- **Concepts:** Everything is a file; qjs task (JavaScript outside Chrome); Tasks own process identity; The #task device.
- **Media:** LIVE asciinema cast (single-process, deterministic, ~6 lines of output). The single best "feels live, no backend" artifact on the whole site.
- **Recipe/source:** `examples/qjs-demo.js`; `wanix qjs examples/qjs-demo.js`. (Note the script reads `main.js`, so the cast cwd must contain a `main.js` containing `std.loadFile` for `outside Chrome: true` — pin the recording dir.)
- **Honest limit:** none material; this is the founding proof.

**H2. Four persona on-ramp tiles (static, links).**
- **Shows:** "I want to run JS" → Learn/js-outside-chrome; "I want to wire machines together" → Learn/wire-a-mesh; "I'm extending the core" → Learn/add-a-service-device; "I want the deep idea" → Learn/plan9-ideas-tour.
- **Media:** static cards. Not a demo; the router for the demos.

---

## 3. The five canonical recipe demos (Recipes section + threaded into Learn)

These are the spine. Each is already a verbatim transcript in `docs/recipes/`.

**R01. Agent Repair (one-click in cockpit; copy-paste over 9P).**
- **Shows:** A broken qjs file (`ReferenceError: missingValue`) handed to `#agent`; agent proposes a patch, parks an `approval.needed`, a human writes `approve req-1` to `#agent/<id>/ctl`, the patch applies, the rerun writes `result.txt` = "FIXED BY WANIX AGENT". The cockpit automates the exact 9P file ops.
- **Persona:** Kemi (AI-infra), Maya, decision-makers.
- **Concepts:** #agent device (an LLM you can cat); Approvals as files (the trust gate); Agents are the operators namespaces always needed; FakeEngine vs codex.
- **Media:** RECORDED screenshot `agent-repair.png` (verified: shows repair report + "Wanix agent repair wrote result.txt: FIXED BY WANIX AGENT" toast) for the cockpit click; LIVE asciinema for the bare-CLI equivalent (`wanix qjs broken.js` → fails; the `mount-cat #agent/new` / `mount-write …/prompt` / `mount-write …/ctl approve` loop from recipe 01 §4).
- **Recipe/source:** `docs/recipes/01-repair-broken-qjs.md`; cockpit "Run Agent Repair Demo" (`workbench/src/web/agent-repair-demo.ts`); CLI loop in recipe §4.
- **Honest limit:** served `#agent` is the deterministic **FakeEngine**, not a live LLM; the `approve:`-prefix is the fake's approval contract. Real codex is `wanix agent` CLI-only. Must be stated on the page.

**R02. Mount a remote peer + send a #cpu job (two-terminal hero).**
- **Shows:** Node B `mesh-serve` prints an `iroh://<peer>?addr=…` ticket; node A runs `mount-ls`/`mount-cat` to read bytes that never existed on A's disk; then `wanix cpu --node "$NODE_B" -- qjs /work/build.js` runs the build *on B against B's files*.
- **Persona:** Devraj (compute-mesh power user); Aria.
- **Concepts:** Import/export and /n/; RemoteFs (import half of 9P); The key is the address; #cpu exec plane (send the agent to the data); 9P over iroh QUIC under one ALPN.
- **Media:** RECORDED dual-pane asciinema (two casts side-by-side, B's stderr + A's stdout) — a single live cast cannot capture two nodes cleanly, and there is no screenshot for this flow. Could be a short screencast of two terminals.
- **Recipe/source:** `docs/recipes/02-mount-remote-peer.md`; `wanix mesh-serve`, `wanix mount-ls/mount-cat`, `wanix cpu`.
- **Honest limit:** shipped `mount-*` always binds at the single slot `/n/remote` (not `/n/<peer-id>`); the peer-id lives in the ticket. `#cpu` v1 returns output batched after the task completes, no remote cancel, reverse-export read-only by default. `--wanix-services` is local-trust only (refused on public endpoint).

**R03. Freeze a world to a capsule (round-trip).**
- **Shows:** `wanix capsule save DIR` prints a 64-hex capsule id; `wanix capsule load <id> DIR2` rebuilds a byte-identical tree (`diff -r` empty); blobs in the CAS store are content-addressed and dedup across worlds.
- **Persona:** Devraj, all.
- **Concepts:** wanix capsule (CAS-backed world snapshots); #cas content-addressed store (venti); End-to-end hash verification; Content-addressed data plane.
- **Media:** LIVE asciinema (single-process, fully deterministic; recipe §6 is literally a paste-into-shell smoke test that ends with `round-trip ok: $ID`). Excellent live candidate.
- **Recipe/source:** `docs/recipes/03-freeze-world-to-capsule.md`; `wanix capsule save … --store`, `wanix capsule load … --store`.
- **Honest limit:** there is **no `.wcap` archive file** in the shipped CLI — a capsule is the manifest blob + referenced blobs in the CAS store. Not portable: live mesh peers/tickets, ephemeral fds, live `#kv` state, symlinks/perms/xattrs.

**R04. Tiny HTTP app backed by #kv (counter).**
- **Shows:** A stateless qjs handler (`apps/counter.js`) does read-modify-write on `#kv/counter`; repeated invocations increment the same key because `KvDevice` lives for the serve process; the cockpit and curl hit one shared `#kv/http-counter`.
- **Persona:** Maya (builder mode).
- **Concepts:** #kv key/value device; #kv as the smallest database; /.wanix/app/<name> HTTP route; #kv-backed HTTP app state.
- **Media:** RECORDED screenshot `http-counter.png` (verified: shows `/.wanix/app/counter` response and the counter state) for the cockpit/route view; LIVE asciinema for the honest CLI loop (`wanix qjs ./project-root/apps/counter.js` three times → `counter 1/2/3`).
- **Recipe/source:** `docs/recipes/04-tiny-http-app-with-kv.md`; cockpit `http-app-demo.ts` + Rust `/.wanix/app/<name>` (`serve/http/app.rs`); CLI `wanix qjs apps/counter.js`.
- **Honest limit (branch-specific):** recipe 04 itself notes the `GET /apps/<name>` route is *not wired on the cpu branch* — only `POST /agent` exists there. The cockpit `http-counter.png` is from the integration branch where `/.wanix/app/<name>` IS wired. State the route's loopback-only + services-gated nature; `#kv` is in-memory (gone on serve restart; freeze to a capsule to persist).

**R05. Two agents collaborate via #agent/<id>/reply.**
- **Shows:** Open `#agent/new` twice (A and B), write B a sub-goal, A blocks on `cat #agent/B/reply` (single-read final message + EOF) while the operator approves B's parked actions via `pending`+`ctl`, then `close` B.
- **Persona:** Kemi; Devraj (cross-node variant).
- **Concepts:** #agent device file interface; Approvals as files; Best-effort epidemic delivery (#plumb, in the cross-node variant); Send-agent-to-the-data.
- **Media:** LIVE asciinema for the single-node FakeEngine file-flow (deterministic). The cross-node `#plumb` handoff variant is RECORDED (two nodes + the single-frame-recv caveat).
- **Recipe/source:** `docs/recipes/05-two-agents-collaborate.md`; `cat #agent/new`, `…/prompt`, `cat #agent/$B/reply`, `…/ctl`.
- **Honest limit:** served engine is FakeEngine; cross-node live `#plumb` recv is publish-path-only over one serve connection (a blocking recv would deadlock the single-frame-at-a-time serve) — needs a second 9P connection.

---

## 4. The cockpit operator-surface demos (recorded; the verified screenshot set)

The cockpit is `serve --bundle workbench-fs9p --wanix-services` (note: the bundle is **still named `workbench-fs9p`**, not `cockpit` — Slice 8 not done). All eight screenshots are verified PASS-state captures. These belong to `use-cases/browser-cockpit`, `concepts/the-browser-cockpit`, and the cockpit step of the js-outside-chrome flow.

**C1. The cockpit shell loaded (WANIX: SYSTEM tree + qjs-shell terminal).**
- **Shows:** Code OSS in the browser with the activity-bar tree (Actions, Tour, Reports, Data Stores, Checks, Activity, Routes, Route Runs, Agent, Tasks, Terminals, Namespace, Drivers) and an integrated `qjs-shell` terminal showing `shell task: 1` and a `$` prompt.
- **Persona:** all; the operator surface.
- **Concepts:** The browser cockpit (operator surface); Direct-9P operator surface (no side channel); qjs-shell (shell as a guest program).
- **Media:** RECORDED `wanix-cockpit.png` (verified) and `cockpit-loaded-v3.png`.
- **Source/action:** `serve --bundle workbench-fs9p --wanix-services`; open `/?bundle=workbench-fs9p`.
- **Honest limit:** requires vendored vscode-web (`workbench/code/out`); without it the page is HTTP 200 but an error banner (`cockpit-loaded.png`). The bundle name is not yet `cockpit`.

**C2. Service inspector — read-only device directory inspection.**
- **Shows:** `wanix-inspect:` rendering of `#kv` (empty dir, the allocator/metadata/control/stream "Note" block), `#agent` (`new` flagged as an allocator file), `#task`, and the served root (`/hello.js`). Clicking a link never accidentally allocates a session/blob.
- **Persona:** Theo (integrator), Devraj.
- **Concepts:** Service devices (#name); Allocator/metadata/control/stream classifier; #kv key/value device; #agent device.
- **Media:** RECORDED `inspect-kv.png`, `inspect-agent.png`, `inspect-task.png`, `inspect-root.png` (all verified). The `inspect-root` and `inspect-task` PNGs are the two new ones in `git status`.
- **Source/action:** cockpit Namespace tree → click a device → `service-inspector.ts` (`workbench/src/web/service-inspector.ts`).
- **Honest limit:** read-only inspection; allocator/ctl/stream files stay plain text by design.

**C3. qjs→wasm→qjs Duet on one shared FS.**
- **Shows:** qjs producer writes `shared/in.txt`; `rust-guest.wasm` transform reads it, writes `shared/out.txt`; qjs verifier confirms `out.txt` = "rust-wasm saw: hello from qjs duet" — three tasks, one Wanix namespace, two WASI tiers.
- **Persona:** Maya, Theo, Priya.
- **Concepts:** Two tiers, one substrate; Compiled wasm task driver; Shared WASI/fd contract; Wasmtime as substrate.
- **Media:** RECORDED `duet.png` (verified: shows `out.txt`, the verify terminal "verified shared/out.txt … [wanix task exited 0]", and the Activity feed of task filesystem refreshes).
- **Source/action:** cockpit "Run JS and WASM Duet Demo" (`workbench/src/web/duet-demo.ts`); also reproducible headless via `crates/wanix-cli/tests/shared_vfs_differential.rs` and the `compute_bench`/`shared_vfs` examples.
- **Honest limit:** compiled wasm is command-style WASI only (`poll_oneoff` = NOSYS); no readiness/async I/O.

**C4. Cockpit Self-Check report.**
- **Shows:** A device-set probe (drivers, `#task`/`#term`, qjs-shell route, http-app route, direct-v86 route, report storage, `#agent`/`#kv`/`#pipe`/`#cas`/`#plumb`, mesh peers) writing `/.wanix/cockpit-check.{json,md}` with a green/warn summary.
- **Persona:** all; the "is this real?" honesty proof.
- **Concepts:** Cockpit self-check; --wanix-services device set; Trust-boundary gaps; Blocking-stream EOF contract.
- **Media:** RECORDED `self-check.png` (verified: shows the self-check markdown report with summary/checks and a "Wanix cockpit self check completed with warn" toast — the warn is the honest `#plumb` publish-only result).
- **Source/action:** cockpit "Run Cockpit Self Check" (`workbench/src/web/cockpit-self-check.ts`).
- **Honest limit:** the `#plumb` probe verifies the **publish path only** — a blocking recv would deadlock the single-frame-at-a-time serve. This is the canonical "honesty as content" artifact; show the warn, explain why.

---

## 5. CLI / engine demos (live asciinema; Concepts + Reference + Learn)

These are single-process and deterministic, so all are HIGH-confidence LIVE asciinema candidates. They thread `concepts/*` pages and the js-outside-chrome / contributor flows.

**L1. Pass process context into a task (env/cwd/stdin/argv).**
- **Shows:** `wanix qjs --env … --cwd … --stdin … -- argv` flowing into `scriptArgs`, `std.getenv`, fd 0, and `#task/self/id` — no `globalThis.Wanix`.
- **Persona:** Maya. **Concepts:** Tasks own process identity; The fd table; Wanix-backed WASI (not host WASI); Guest-JS guardrails.
- **Media:** LIVE asciinema. **Source:** `rust-walkthrough.md:91-139`; `examples/qjs-task-context.js`, `qjs-stdin-demo.js`.
- **Honest limit:** WASI is a Wanix boundary, not a host escape.

**L2. Explicit host mount (host files are not ambient authority).**
- **Shows:** `wanix qjs --mount /tmp/wanix-host=host main.js` binds a rooted `LocalFs`; the task sees a Wanix path; the host root boundary is enforced.
- **Persona:** Maya, Theo. **Concepts:** Host files are not ambient authority; Namespace binding (bind/mount); NormalizedPath.
- **Media:** LIVE asciinema. **Source:** `rust-walkthrough.md:141-178`; `examples/qjs-host-mount.js`.

**L3. Create & control a child task via #task.**
- **Shows:** Read `#task/new/qjs` for an id, write `cmd`/`env`/`dir`, bind fds with `ctl`, `start`, read `#task/<id>/exit`.
- **Persona:** Theo, Priya, Maya. **Concepts:** The #task device; Task drivers; The fd table.
- **Media:** LIVE asciinema (or recorded shell). **Source:** `rust-walkthrough.md:211-246`; `examples/qjs-task-spawn.js`.

**L4. qjs-shell — the shell is a guest program.**
- **Shows:** An interactive `wanix qjs-shell` session: builtins `ls/cat/cd/mkdir/write/ps/env`, child `qjs` launches with `< > 2>` redirection, raw-mode echo/Ctrl-C/Ctrl-D, resize via `#term/<id>/winch`.
- **Persona:** Maya, power users, Theo. **Concepts:** #term device; qjs-shell (shell as a guest program); Raw vs cooked input & control bytes; Resize/winch lifecycle.
- **Media:** LIVE asciinema (interactive shells record beautifully). **Source:** `wanix qjs-shell`; `examples/qjs-term-shell-demo.js`; `rust-walkthrough.md:318-385`.
- **Honest limit:** no pipes `|`, no job control, no signals; `#term/<id>/ctl` supports only `close`.

**L5. Two WASI tiers on one shared FS (the differential, headless).**
- **Shows:** A compiled wasm task and a qjs task run against one `MemFs` and observe identical filesystem state.
- **Persona:** Maya, Priya, Theo. **Concepts:** Two tiers, one substrate; Compiled wasm task driver; Shared WASI/fd contract.
- **Media:** LIVE asciinema of `cargo test … shared_vfs_differential` OR the cockpit Duet (C3) as its visual counterpart. **Source:** `crates/wanix-cli/tests/shared_vfs_differential.rs`; `compute_bench`/`shared_vfs` examples.

**L6. Snapshot & restore a qjs VM (engine resumability).**
- **Shows:** Snapshot the QuickJS/Wasm VM image and resume it in a new task with host resources (namespace, stdio, env) explicitly reattached; task id changes.
- **Persona:** Priya, developers. **Concepts:** QuickJS snapshots are VM images; Bounded execution policy.
- **Media:** LIVE asciinema (the `qjs-snapshot-*` / `qjs-restore-*` example pairs). **Source:** `rust-walkthrough.md:481-575`; `examples/qjs-snapshot-before.js` / `qjs-restore-context-after.js` etc.
- **Honest limit:** snapshots are VM memory images, not whole-task checkpoints; namespace/fds/cwd/env must be reattached.

**L7. Capability grant enforced over the wire.**
- **Shows:** `serve --p9 HOST:PORT --peer HEX --grant ANAME:PREFIX:RIGHTS` installs a default-deny GrantTable on the raw-9P TCP door; an authorized peer attaches a `SubtreeFs`-scoped read-write root; an unmatched attach gets EACCES (errno 13).
- **Persona:** Aria, Devraj, Priya. **Concepts:** A capability is a bind; AttachPolicy; SubtreeFs/confine_to_prefix; Persisted ed25519 node identity.
- **Media:** RECORDED asciinema/screencast (needs two endpoints; partly proven only at serve_duplex-level loopback per research, so frame as a recorded/annotated transcript, not a "paste this" live cast). **Source:** `docs/mesh-the-missing-half-of-9p.md:712-851`; `crates/wanix-id/src/grant.rs`; `serve --p9` (the raw-9P door over the one session core, ADR 0006).
- **Honest limit:** the allow-side over plain TCP is proven by loopback tests, not a two-process copy-paste (no `--aname` flag yet). Tauth stays ENOSYS; exec stays local-trust.

**L8. Discovery doc walkthrough (serve contract).**
- **Shows:** `curl -sS http://127.0.0.1:7654/.well-known/wanix.json | jq` — routes (p9 websocket, rootfs, qjsShell, httpApp, ethernet=not-implemented), the v86 block, `services.devices`, `services.drivers`, selected bundle.
- **Persona:** Theo, Priya, integrators. **Concepts:** Discovery document; Serve as one local composition surface; --wanix-services device set; Three serve bundles.
- **Media:** LIVE asciinema (curl+jq against a local serve; deterministic JSON shape). **Source:** `docs/integration/manual-test-plan.md:64-83`; `crates/wanix-cli/src/serve/discovery.rs`.

---

## 6. Concept-page inline micro-demos (small, code-snippet-as-demo)

Each device-contract page and several concept pages get a short inline "file-ops reveal" — a 3-5 line transcript, not a full cast, showing the device IS files. These are RECORDED snippets (rendered code blocks with copy), not players.

**M1. #kv round trip** — `echo 3 > #kv/counter; cat #kv/counter; ls #kv`. Device page `devices/kv`; concepts `#kv as the smallest database`.
**M2. #cas write-then-read-hash** — write to `#cas/ingest`, read `#cas/ingest` for the hash, `cat #cas/<hash>`, `cat #cas/have/<hash>` → `1`. Device page `devices/cas`.
**M3. #pipe unix-pipe composition** — `cat #pipe/new`; open `#pipe/<id>/data` WRONLY in one task, RDONLY in another; EOF only after last writer drops. Device page `devices/pipe`; concept `Blocking-stream EOF contract`.
**M4. #plumb send/recv envelope** — `echo '{"kind":"build.done"}' > #plumb/build/send`; `cat #plumb/build/recv`. Device page `devices/plumb`. **Honest limit inline:** best-effort, not a durable queue; recv-since-open; single-frame serve caveat.
**M5. #agent device tree** — the `new/prompt/events/reply/pending/ctl/status` ASCII tree from recipe 05. Device page `devices/agent`.
**M6. #term cross-feed** — write to `data`, read from `program` (with `\n`→`\r\n`); `winch` broadcasts `cols rows`. Device page `devices/term`.

All six map to: Service devices (#name); Devices import across the mesh for free; Blocking-stream EOF contract. Persona: developer/power user.

---

## 7. Concept-graph visual (Find section)

**G1. Interactive Concept Graph map.**
- **Shows:** The ~78 Concept Graph nodes as a clustered, navigable graph (See-also rails materialized as edges); A-Z index; tag browser by audience/cluster/status.
- **Persona:** Aria, experts. **Concepts:** spans the whole graph (a navigation surface, not a single concept).
- **Media:** LIVE interactive (client-side graph widget; no backend). This is the "expert's low-friction entry point" from the IA Find section.
- **Source:** the Concept Graph node list + See-also/Used-in-flows rails defined by the IA. Not a Wanix runtime demo — a docs-native interactive.

---

## 8. What must NOT be a demo (honest exclusions)

- **v86-shared-demo** — still a `// STUB:` no-op in `workbench/src/web/extension.ts`. Do not feature; mention only on the "not yet" roadmap page.
- **A live public cockpit** — exec devices are local-trust-only; never host a shared one. Offer a local-boot recipe instead.
- **direct-v86 boot** — a validated *handoff*, not a Wanix-supervised VM. Can show the generated page (recorded screenshot) but must not claim Wanix boots/manages the VM.
- **Live cross-node `#plumb` recv over one serve connection** — publish-path only; show it as a self-check warn (C4), not as a working live subscription.
- **A "real LLM in the browser"** — the served `#agent` is FakeEngine; never imply a live model in the cockpit demos.

---

## 9. Persona × demo coverage matrix (quick check)

- **Maya (app dev):** H1, R04, C1, C3, L1, L2, L3, L4 — fully covered (js-outside-chrome + http-app flows).
- **Devraj (mesh power user):** R02, R03, C2, C4, L7, L8 — covered (wire-a-mesh flow).
- **Theo / Priya (integrator / contributor):** C2, C3, L3, L5, L6, L8, M1-M6, G1 — covered (add-device / add-driver / contribute flows).
- **Kemi (AI-infra):** R01, R05, C3, M5 — covered (agent-on-your-files flow).
- **Aria (Plan 9 thinker):** R02, L7, G1, M3/M4 + the honesty blocks — covered (plan9-ideas-tour). Aria's trust comes from the caveats (C4 warn, the "not a demo" exclusions), which the IA mandates as first-class content.


---

## Demo index (20)

| Demo | Type | Interactive | Persona | Concepts | Description |
|---|---|---|---|---|---|
| Hero: outside Chrome: true | cli | yes | Maya (curious app dev) / all | Everything is a file, qjs task (JavaScript outside Chrome), Tasks own process identity, The #task device | LIVE asciinema of `wanix qjs examples/qjs-demo.js` printing 'outside Chrome: true', 'task id: 1', the runtime line, and the round-tripped hello.txt written into a Wanix namespace. The single best feels-live-no-backend artifact. Recording dir must contain a main.js with std.loadFile so the boolean reads true. |
| Recipe 01: Agent Repair | recipe | yes | Kemi (AI-infra) / Maya / decision-maker | #agent device (an LLM you can cat), Approvals as files (the trust gate), Agents are the operators namespaces always needed, FakeEngine vs codex (exec local-trust-only) | Broken qjs (ReferenceError) handed to #agent; agent parks approval.needed; human writes 'approve req-1' to ctl; patch applies; rerun writes result.txt = FIXED BY WANIX AGENT. Cockpit one-click is RECORDED (agent-repair.png, verified). Bare-CLI #agent file-op loop is LIVE asciinema. HONEST: served #agent is the deterministic FakeEngine, not a live LLM; the approve: prefix is the fake's approval contract. |
| Recipe 02: Mount remote peer + #cpu job | cli | no | Devraj (mesh power user) / Aria | Import/export and /n/, RemoteFs (import half of 9P), The key is the address, #cpu exec plane (send the agent to the data), 9P over iroh QUIC under one ALPN | RECORDED dual-pane asciinema: node B mesh-serve prints an iroh:// ticket; node A mount-ls/mount-cat reads bytes never on A's disk; `wanix cpu --node B -- qjs /work/build.js` runs the build ON B against B's files. Two nodes => recorded, not paste-live. HONEST: mount-* binds single slot /n/remote not /n/<peer-id>; #cpu v1 batches output, no remote cancel, reverse-export read-only by default; --wanix-services is local-trust-only. |
| Recipe 03: Freeze world to capsule | cli | yes | Devraj / all | wanix capsule (CAS-backed world snapshots), #cas content-addressed store (venti), End-to-end hash verification, Content-addressed data plane (content_hash) | LIVE asciinema (fully deterministic single-process): `wanix capsule save DIR --store` prints a 64-hex id; `wanix capsule load <id> DIR2 --store` rebuilds byte-identical (diff -r empty); recipe §6 ends with 'round-trip ok: $ID'. HONEST: no .wcap archive file exists; capsule = manifest blob + referenced blobs in CAS; not portable: live peers/tickets, ephemeral fds, live #kv state, symlinks/perms. |
| Recipe 04: HTTP app backed by #kv | recipe | yes | Maya (builder mode) | #kv key/value device, #kv as the smallest database, /.wanix/app/<name> HTTP route, #kv-backed HTTP app state | Stateless qjs handler apps/counter.js does read-modify-write on #kv/counter; repeated runs increment the same key because KvDevice lives for the serve process. Cockpit/route view RECORDED (http-counter.png, verified). Honest CLI loop is LIVE asciinema: `wanix qjs apps/counter.js` x3 => counter 1/2/3. HONEST: GET /apps/<name> route is NOT on the cpu branch (only POST /agent there); route is loopback-only + services-gated; #kv is in-memory (gone on restart; persist via capsule). |
| Recipe 05: Two agents collaborate | recipe | yes | Kemi / Devraj (cross-node variant) | #agent device file interface, Approvals as files (trust gate), Best-effort epidemic delivery (not a queue), Send-agent-to-the-data (mesh reach) | Open #agent/new twice; write B a sub-goal; A blocks on `cat #agent/B/reply` (single-read final message + EOF) while operator approves B via pending+ctl, then closes B. Single-node FakeEngine file-flow is LIVE asciinema; cross-node #plumb handoff variant is RECORDED. HONEST: served engine is FakeEngine; cross-node live #plumb recv is publish-path-only over one serve connection (blocking recv deadlocks single-frame serve). |
| Cockpit shell loaded (WANIX: SYSTEM + qjs-shell) | cockpit | no | all | The browser cockpit (operator surface), Direct-9P operator surface (no side channel), qjs-shell (shell as a guest program), Three serve bundles | RECORDED wanix-cockpit.png / cockpit-loaded-v3.png (verified): Code OSS in-browser with the activity-bar tree (Actions, Tour, Reports, Data Stores, Checks, Activity, Routes, Route Runs, Agent, Tasks, Terminals, Namespace, Drivers) and an integrated qjs-shell terminal showing 'shell task: 1' and a $ prompt. Action: serve --bundle workbench-fs9p --wanix-services, open /?bundle=workbench-fs9p. HONEST: requires vendored vscode-web (workbench/code/out via `cd workbench && make build`); bundle still named workbench-fs9p, not cockpit. |
| Service inspector (read-only device dirs) | cockpit | no | Theo (integrator) / Devraj | Service devices (#name), Allocator/metadata/control/stream classifier, #kv key/value device, #agent device (an LLM you can cat) | RECORDED inspect-kv.png, inspect-agent.png, inspect-task.png, inspect-root.png (all verified). wanix-inspect: rendering of #kv (empty dir + allocator/metadata/control/stream Note block), #agent (new flagged as allocator file), #task, and the served root (/hello.js). Clicking a link never accidentally allocates a session/blob. inspect-root.png and inspect-task.png are the two new PNGs in git status. |
| qjs->wasm->qjs Duet on one shared FS | cockpit | no | Maya / Theo / Priya | Two tiers, one substrate, Compiled wasm task driver, Shared WASI/fd contract, Wasmtime as substrate | RECORDED duet.png (verified): qjs producer writes shared/in.txt; rust-guest.wasm transform writes shared/out.txt = 'rust-wasm saw: hello from qjs duet'; qjs verifier confirms it; verify terminal shows '[wanix task exited 0]'. Action: cockpit 'Run JS and WASM Duet Demo'. Also reproducible headless via the shared_vfs_differential test. HONEST: compiled wasm is command-style WASI only (poll_oneoff=NOSYS). |
| Cockpit Self-Check report | cockpit | no | all (the honesty proof) | Cockpit self-check, --wanix-services device set, Trust-boundary gaps, Blocking-stream EOF contract | RECORDED self-check.png (verified): device-set probe (drivers, #task/#term, qjs-shell route, http-app route, direct-v86 route, report storage, #agent/#kv/#pipe/#cas/#plumb, mesh peers) writing /.wanix/cockpit-check.{json,md} with a green/warn summary; toast reads 'completed with warn'. Action: cockpit 'Run Cockpit Self Check'. HONEST: the warn is real — the #plumb probe verifies the PUBLISH path only because a blocking recv would deadlock the single-frame-at-a-time serve. The canonical honesty-as-content artifact. |
| Pass process context into a task | cli | yes | Maya | Tasks own process identity, The fd table, Wanix-backed WASI (not host WASI), Guest-JS guardrails | LIVE asciinema of `wanix qjs --env/--cwd/--stdin -- argv` flowing into scriptArgs, std.getenv, fd 0, and #task/self/id with no globalThis.Wanix. |
| Explicit host mount | cli | yes | Maya / Theo | Host files are not ambient authority, Namespace binding (bind/mount), NormalizedPath | LIVE asciinema of `wanix qjs --mount /tmp/wanix-host=host main.js` binding a rooted LocalFs; the task sees a Wanix path while the host root boundary is enforced — host files are not ambient authority. |
| Create & control a child task via #task | cli | yes | Theo / Priya / Maya | The #task device, Task drivers, The fd table | LIVE/recorded transcript: read #task/new/qjs for an id, write cmd/env/dir, bind fds via ctl, start, read #task/<id>/exit. |
| qjs-shell interactive session | interactive | yes | Maya / power user / Theo | #term device, qjs-shell (shell as a guest program), Raw vs cooked input & control bytes, Resize / winch lifecycle | LIVE asciinema of an interactive `wanix qjs-shell`: builtins ls/cat/cd/mkdir/write/ps/env, child qjs launches with < > 2> redirection, raw-mode echo/Ctrl-C/Ctrl-D, resize via #term/<id>/winch. The shell is a guest JS program, not a separate process model. HONEST: no pipes, no job control, no signals; #term ctl supports only close. |
| Two WASI tiers on one shared FS (differential) | recorded | no | Maya / Priya / Theo | Two tiers, one substrate, Compiled wasm task driver, Shared WASI/fd contract | LIVE asciinema of `cargo test shared_vfs_differential` (or the cockpit Duet as its visual counterpart): a compiled wasm task and a qjs task run against one MemFs and observe identical filesystem state. |
| Snapshot & restore a qjs VM | cli | yes | Priya / developers | QuickJS snapshots are VM images, Bounded execution policy (not a scheduler) | LIVE asciinema of the qjs-snapshot/qjs-restore example pairs: snapshot the QuickJS/Wasm VM image, resume it in a new task with namespace/stdio/env explicitly reattached; task id changes. HONEST: snapshots are VM memory images, not whole-task checkpoints. |
| Capability grant enforced over the wire | recorded | no | Aria / Devraj / Priya | A capability is a bind, AttachPolicy (trust boundary as one pure function), SubtreeFs / confine_to_prefix, Persisted ed25519 node identity | RECORDED/annotated transcript: serve --p9 HOST:PORT --peer HEX --grant ANAME:PREFIX:RIGHTS installs a default-deny GrantTable on the raw-9P TCP door; an authorized peer attaches a SubtreeFs-scoped read-write root; an unmatched attach gets EACCES (errno 13). HONEST: the allow-side over plain TCP is proven by loopback tests, not a two-process copy-paste (no --aname flag yet); Tauth stays ENOSYS; exec stays local-trust. |
| Discovery doc walkthrough | cli | yes | Theo / Priya / integrators | Discovery document (/.well-known/wanix.json), Serve as one local composition surface, --wanix-services device set, Three serve bundles | LIVE asciinema of `curl -sS http://127.0.0.1:7654/.well-known/wanix.json \| jq` showing routes (p9 ws, rootfs, qjsShell, httpApp, ethernet=not-implemented), the v86 block, services.devices, services.drivers (auto/noop/qjs/wasm), and the selected bundle. |
| Device file-ops micro-demos (#kv/#cas/#pipe/#plumb/#agent/#term) | recipe | no | developer / power user | Service devices (#name), Devices import across the mesh for free, Blocking-stream EOF contract, #cas content-addressed store (venti), #pipe byte channels, #plumb plumber bus | RECORDED 3-5 line transcript snippets per device page proving the device IS files: #kv round trip (echo>#kv/k; cat; ls), #cas write-then-read-hash (ingest -> hash -> #cas/<hash> -> have), #pipe unidirectional EOF-on-last-writer-drop, #plumb send/recv envelope, #agent device tree, #term data<->program cross-feed with winch. Inline honest limits per device (e.g. #plumb best-effort + single-frame-serve recv caveat). |
| Interactive Concept Graph map | interactive | yes | Aria / experts / all | Crate layering & dependency direction, The 9P contract, A capability is a bind, Everything is a file | LIVE client-side interactive graph widget rendering the ~78 Concept Graph nodes as a clustered navigable map (See-also rails as edges) plus A-Z index and tag browser by audience/cluster/status. A docs-native interactive (no Wanix backend); the expert's low-friction entry point in the Find section. |
