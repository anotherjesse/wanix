# Cockpit + Mesh Integration Status

Branch: `cockpit-mesh-integration` (off `cpu`). No commits yet — workflow left
everything staged/unstaged in the working tree for you to review and split.

## 1. TL;DR

- The cockpit shell, sidebar, demos, and probes from `origin/rust` are
  re-landed in `workbench/src/web/`, retargeted at cpu's device model on top
  of `WanixP9Handle` / `WanixHandle`.
- `cargo check --workspace` is **green** across all 27 crates (10.45s).
- `npm run compile-web` is **red**: `tsc` reports 15 errors and short-circuits
  before esbuild runs. No `extension.js` bundle was produced.
- Headless Chromium loaded the served bundle (
  `http://127.0.0.1:17999/?bundle=workbench-fs9p`) but, lacking the compiled
  `extension.js` and the upstream vscode-web zip, it renders the "Wanix Rust
  workbench" shell with a banner error. Screenshot:
  `docs/integration/screenshots/cockpit-loaded.png` (18 KB).
- Plan, recipes, and restored design docs are all in place
  (`docs/integration/plan.md`, `docs/recipes/01..05`, `docs/wanix-*.md`,
  `docs/traceable-dynamic-namespaces.md`).
- **Review first:** `workbench/src/web/extension.ts` (now 2,786 lines, +2,367
  from the cpu stub) and the new TS modules — they carry the merge surface.
- **Fix first:** the 15 tsc errors gating `npm run compile-web`; until then
  the browser cannot render the cockpit even though the Rust side is ready.

## 2. Branch

```text
Current: cockpit-mesh-integration
Base:    cpu @ 667dbc1 (docs(mesh): agent-mesh — what/why/how + live transcript)
Commits ahead of cpu: 0
```

All changes are uncommitted so you can choose the commit split yourself.

## 3. Files changed

`git status --short`:

```text
A  workbench/media/rust-guest.wasm
A  workbench/media/wanix.svg
M  workbench/src/web/bridge.ts
MM workbench/src/web/extension.ts
A  workbench/src/web/workbench-assets.ts
?? docs/integration/
?? docs/recipes/
?? docs/traceable-dynamic-namespaces.md
?? docs/wanix-os-cockpit-month-sprint.md
?? docs/wanix-workbench-browser-dx.md
?? workbench/src/web/agent-repair-demo.ts
?? workbench/src/web/agent-tool-contract.ts
?? workbench/src/web/cockpit-self-check.ts
?? workbench/src/web/qjs-starter.ts
?? workbench/src/web/service-inspector.ts
?? workbench/src/web/system-view.ts
?? workbench/src/web/wasm-starter.ts
```

`git diff --stat HEAD` (tracked-only):

```text
 workbench/media/rust-guest.wasm       |  Bin 0 -> 128565 bytes
 workbench/media/wanix.svg             |    7 +
 workbench/src/web/bridge.ts           |   41 +
 workbench/src/web/extension.ts        | 2443 ++++++++++++++++++++++++++++++++-
 workbench/src/web/workbench-assets.ts |   28 +
 5 files changed, 2443 insertions(+), 76 deletions(-)
```

Untracked TS modules (line counts):

```text
agent-repair-demo.ts     236
agent-tool-contract.ts   211
cockpit-self-check.ts    597
qjs-starter.ts            59
service-inspector.ts     421
system-view.ts            54
wasm-starter.ts          112
```

Untracked docs:

```text
docs/integration/plan.md                  (10-slice integration plan)
docs/integration/cockpit-meets-mesh.md    (narrative synthesis)
docs/integration/manual-test-plan.md      (manual validation steps)
docs/integration/screenshots/cockpit-loaded.png
docs/recipes/01-repair-broken-qjs.md
docs/recipes/02-mount-remote-peer.md
docs/recipes/03-freeze-world-to-capsule.md
docs/recipes/04-tiny-http-app-with-kv.md
docs/recipes/05-two-agents-collaborate.md
docs/traceable-dynamic-namespaces.md
docs/wanix-os-cockpit-month-sprint.md
docs/wanix-workbench-browser-dx.md
```

## 4. Slice-by-slice status

From `docs/integration/plan.md`:

| # | Slice | Status | Notes |
|---|-------|--------|-------|
| 1 | Cockpit bones: bridge + system-view skeleton | **partial** (build green; serve route missing) | `bridge.ts`, `system-view.ts`, `service-inspector.ts`, `workbench-assets.ts` ported; `extension.ts` wired; tsc + esbuild now green (pass 2). Browser still fails — `serve` does not route `/workbench/code/...` to the vendored vscode-web tree. |
| 2 | Device-aware system view (#agent/#kv/#pipe/#plumb/#cas/#cpu) | **partial** | Categories present in `system-view.ts`; `device-discovery.ts` module not yet split out; discovery JSON not extended on Rust side. |
| 3 | Mesh panel: peers + /n/<node> mounts | **queued** | No `mesh-panel.ts`, no `wanix-cli/src/serve/discovery.rs` extension, no `wanix.mesh.*` commands yet. |
| 4 | Demos retargeted: qjs-starter, wasm-starter, duet over #task + #cas | **partial** | `qjs-starter.ts`, `wasm-starter.ts`, `rust-guest.wasm` landed; **`duet-demo.ts` not yet present**. |
| 5 | Agent repair demo on #agent | **partial** (build green; needs browser run) | `agent-repair-demo.ts` ported; tsc now green so the `#agent` wiring is type-checked. Browser exercise is blocked by the missing `/workbench/code/` serve route. |
| 6 | Cockpit self-check + tool contract | **partial** (build green; needs browser run) | `cockpit-self-check.ts` (597 lines) and `agent-tool-contract.ts` (211) ported; tsc green; discovery shape-pin test not added; browser exercise blocked by the missing static route. |
| 7 | HTTP app demo + route orchestration over #task + #plumb | **queued** | No `http-app-demo.ts`, no `/.wanix/app/<name>` route. |
| 8 | Cockpit bundle: rename, default route | **queued** | `--bundle cockpit` not yet accepted; embedded HTML still references `workbench/code/out/...` vscode-web layout. Vendor is now on disk (pass 2); the missing piece is the serve static route, then the rename. |
| 9 | Browser-as-peer over WebSocket | **queued** | No `workbench/src/wanix/peer.ts`; ws session-token gate not added. |
| 10 | Retire & document | **partial** | Restored docs landed under `docs/`; `docs/integration/retired.md` not yet written. |

_Tracking note (pass 2):_ no slices moved to **done** yet. Slices 1, 5, 6 are
now build-green but remain **partial** because browser validation still
fails. Slice 1 will only flip to **done** when the headless probe loads the
cockpit shell without the `nls.messages.js` banner.

## 5. Open questions for you

1. **Commit split.** The cpu-stub `extension.ts` (76 lines) grew to 2,786 lines
   in a single working-tree change. Do you want me to break this into the 10
   plan slices as separate commits, or land it as one large "restore cockpit"
   commit plus follow-ups? See Section 6.
2. **tsc errors.** The 15 errors are the gating issue. Want me to push through
   and fix them in a follow-on pass, or do you want to triage them yourself
   first? The error summary lives in the build-check artifact noted in the
   workflow output (`ts_errors_summary`).
3. **vscode-web vendor.** The cockpit page still expects
   `workbench/code/out/nls.messages.js`. Slice 8 plans to retire that path by
   loading `workbench/dist/web/extension.js` directly. Do you want that done
   before the first commit, or kept as a deliberate next slice?
4. **`duet-demo.ts`.** Slice 4 calls for it; it was not ported in this pass.
   Should it land before the first commit (so Slice 4 is complete) or as a
   follow-up?
5. **Mesh panel scope.** Slice 3 needs both a TS `mesh-panel.ts` and Rust
   discovery/serve changes. Do you want it built as one cross-stack commit, or
   land the discovery extension first and the TS surface after?

## 6. Recommended commit strategy

**Suggested: slice-by-slice, but coarse.** Six commits, not ten:

1. **chore(docs): land integration plan, restored design docs, recipes.**
   All of `docs/integration/`, `docs/recipes/`, `docs/wanix-*.md`,
   `docs/traceable-dynamic-namespaces.md`. Zero code risk; reviewable on its
   own.
2. **feat(workbench): cockpit bones — bridge + system-view + inspector
   (Slices 1+2).** `bridge.ts`, `system-view.ts`, `service-inspector.ts`,
   `workbench-assets.ts`, `extension.ts` activation/commands for these
   surfaces, plus `workbench/media/wanix.svg`.
3. **feat(workbench): qjs + wasm starters and demo fixture (Slice 4
   partial).** `qjs-starter.ts`, `wasm-starter.ts`,
   `workbench/media/rust-guest.wasm`, and the matching `extension.ts`
   commands.
4. **feat(workbench): agent repair demo + tool contract (Slices 5+6).**
   `agent-repair-demo.ts`, `agent-tool-contract.ts`,
   `cockpit-self-check.ts`, and the matching `extension.ts` commands.
5. **fix(workbench): green tsc + esbuild.** Whatever it takes to get
   `npm run compile-web` clean. Touches only the modules above.
6. **chore(integration): STATUS + next-step queue.** This file plus any
   retired.md when Slice 10 lands.

Avoid one mega-commit: the 2,443-line `extension.ts` diff is already at the
limit of what is reviewable in isolation.

## 7. Suggested next moves

- ~~**First fix:** get `npm run compile-web` green. The 15 tsc errors are the
  single thing blocking browser validation.~~ **DONE 2026-06-07** — 3/3 TS
  fixes applied; `check-types` and `compile-web` both pass;
  `workbench/dist/web/extension.js` is 302 KB and present.
- ~~**vscode-web vendor.**~~ **DONE 2026-06-07** — `make code` vendored 93 MB
  into `workbench/code/`; `nls.messages.js` (790 KB) and
  `workbench.web.main.css` (854 KB) both present on disk.
- **NEW FIRST FIX:** add a static-asset route for `workbench/code/**` to the
  `workbench-fs9p` bundle handler in `crates/wanix-cli/src/serve/`. The
  vendored files are on disk but `serve` returns 404 for
  `GET /workbench/code/out/nls.messages.js`. This is the only thing keeping
  the cockpit from rendering; the TS bundle and the asset tree are both
  ready. Verified by `docs/integration/screenshots/cockpit-loaded-v2.png`,
  which shows the identical pass-1 banner error despite green build + green
  vendor.
- **Next TS module to port:** `duet-demo.ts` (Slice 4). It is the only
  origin/rust demo not yet present, and it is what `cockpit-self-check.ts`
  expects to find when probing `#cas` + `#plumb` round-trip.
- **First recipe to try once the static route is in:**
  `docs/recipes/01-repair-broken-qjs.md` — it exercises Slices 1, 2, 4, 5
  end-to-end and is the shortest path to a visible win.
- **Slice 8 (rename `--bundle cockpit`, retire `workbench/code/` dependency)
  is no longer the gating Rust-side item** — it should still land, but the
  cheap static route above unblocks browser validation today; Slice 8 is the
  durable cleanup that follows.
- **Typed-discovery cleanup:** raise the urgency on the AGENTS Queued
  Follow-up for typed `discovery.rs` structs. Slices 2, 3, 6, 7 all extend
  the JSON, and doing it once up front avoids `format!()` accretion.

## 8. Validation evidence

- **Cargo:** `cargo check --workspace` →
  `Finished dev profile [unoptimized + debuginfo] target(s) in 10.45s`.
  All 27 crates compile, including `wanix-mesh`, `wanix-cpu`, `wanix-agent`,
  `wanix-plumb`, `wanix-cli`.
- **Workbench TS:** `npm install` succeeded (31 packages, ~2s, offline
  cache). `npm run compile-web` chained `npm run check-types` which reported
  15 errors and exited; esbuild never ran. No `workbench/dist/web/` output.
- **Browser (headless):** Playwright 1.60
  (chromium\_headless\_shell-1223) drove a Node script against a freshly
  started `target/debug/wanix serve --root /tmp/wanix-test-root
  --bundle workbench-fs9p --wanix-services --addr 127.0.0.1:17999`.
  Probed `/.well-known/wanix.json`, `/.well-known/rootfs.json`, and the
  bundle HTML with curl. Captured screenshot, console messages, request
  failures, body text, and document HTML.
- **Screenshot:**
  `docs/integration/screenshots/cockpit-loaded.png` (full page, shows the
  "Wanix Rust workbench" shell with the expected `nls.messages.js` banner
  error — the same state documented in
  `docs/integration/manual-test-plan.md`).
- **Manual test plan:** `docs/integration/manual-test-plan.md`.
- **Narrative:** `docs/integration/cockpit-meets-mesh.md`.

## Update 2026-06-07: build + browser validation pass 2

### TS fixes applied (3/3)

Three TypeScript fixes were applied to clear the 15 `tsc` errors that were
blocking `npm run compile-web`. With those in, `check-types` passes,
`compile-web` produces `workbench/dist/web/extension.js` (302,413 bytes plus a
209,544-byte sourcemap), and the bundler no longer short-circuits before
esbuild.

### vscode-web vendor: success

`make code` from `/Users/jesse/lw/wanix/workbench/Makefile` downloaded
`vscode-web-1.108.2.zip` from `https://github.com/progrium/vscode-web/releases`,
unzipped it into `dist/vscode/`, and moved the tree to `workbench/code/` (93M
total). Both load-bearing files are present on disk:

```text
workbench/code/out/nls.messages.js                       790094 bytes
workbench/code/out/vs/workbench/workbench.web.main.css   854038 bytes
```

No sandbox override was needed — `curl`/`unzip` ran under default permissions.

### Build: green

- `npm run check-types` — pass.
- `npm run compile-web` — pass; `workbench/dist/web/extension.js` is present
  and non-empty.

### Browser validation: **FAIL**

Headless Chromium against `wanix serve --bundle workbench-fs9p` on
`127.0.0.1:17999` still shows the same banner as pass 1, even though the
vscode-web files are now on disk:

> `Error: failed to load http://127.0.0.1:17999/workbench/code/out/nls.messages.js`
> `at script.onerror (http://127.0.0.1:17999/?bundle=workbench-fs9p&term:62:39)`

The bundle bootstrap HTML loads (document title is `"Wanix Rust workbench"`),
but vscode-web never initializes because the `wanix serve` binary does
not route `/workbench/code/...` to the vendored `workbench/code/` tree. The
file exists on disk; serve just does not have a static route for it. The
banner is identical to pass 1; the screen behind it stays black because the
loader bails on the nls error.

**Screenshot:**
`/Users/jesse/lw/wanix/docs/integration/screenshots/cockpit-loaded-v2.png`
(20,628 bytes, full page, dark; banner top-left).

### Fresh repo state

`git status --short`:

```text
A  workbench/media/rust-guest.wasm
A  workbench/media/wanix.svg
M  workbench/src/web/bridge.ts
MM workbench/src/web/extension.ts
A  workbench/src/web/workbench-assets.ts
?? docs/integration/
?? docs/recipes/
?? docs/traceable-dynamic-namespaces.md
?? docs/wanix-os-cockpit-month-sprint.md
?? docs/wanix-workbench-browser-dx.md
?? workbench/src/web/agent-repair-demo.ts
?? workbench/src/web/agent-tool-contract.ts
?? workbench/src/web/cockpit-self-check.ts
?? workbench/src/web/qjs-starter.ts
?? workbench/src/web/service-inspector.ts
?? workbench/src/web/system-view.ts
?? workbench/src/web/wasm-starter.ts
```

`git diff --stat HEAD` (tracked-only):

```text
 workbench/media/rust-guest.wasm       |  Bin 0 -> 128565 bytes
 workbench/media/wanix.svg             |    7 +
 workbench/src/web/bridge.ts           |   41 +
 workbench/src/web/extension.ts        | 2443 ++++++++++++++++++++++++++++++++-
 workbench/src/web/workbench-assets.ts |   28 +
 5 files changed, 2443 insertions(+), 76 deletions(-)
```

The TS fixes landed inside the already-modified `extension.ts`/`bridge.ts`
working-tree diff, so `git diff --stat` is structurally unchanged from pass 1.
The new artifacts on disk (`workbench/code/**`, `workbench/dist/web/**`,
`docs/integration/screenshots/cockpit-loaded-v2.png`) are gitignored or
untracked build output and so do not appear above.

### Updated recommended next moves

The single root cause of the FAIL is now clear: **`wanix serve` does not
serve the `workbench/code/` tree.** The TS toolchain and the vendored assets
are both ready; the gap is a static route in the Rust binary. The next move
must be:

1. **Add a static-asset route for `workbench/code/`** to the
   `workbench-fs9p` bundle handler in `crates/wanix-cli/src/serve/` (likely
   `bundle.rs` or the workbench-specific handler). The route should map
   `GET /workbench/code/<path>` to `workbench/code/<path>` on disk, with the
   normal path-traversal guard. Until this exists, the cockpit cannot render
   regardless of TS/vendor state.
2. **Re-run the headless probe** and confirm `nls.messages.js` and
   `workbench.web.main.css` both return 200 and the banner disappears.
3. Only then is Slice 8 (rename to `--bundle cockpit`, retire `workbench/code/`
   dependency) the right move; doing Slice 8 first would skip the cheapest
   currently-blocking fix.

## Update 2026-06-07: browser validation pass 3 — **PASS**

The cockpit now renders fully against the cpu mesh backend. Two root causes
were found and fixed, and the result was verified end-to-end in headless
Chromium.

### Root cause 1 — missing static-asset route (Rust)

The cpu branch had dropped `workbench_asset_response` from
`crates/wanix-cli/src/serve/http/routes.rs` when it deleted the cockpit work,
so `GET /workbench/code/...` 404'd and vscode-web never booted. Restored the
route (and its `workbench_asset_path` / `workbench_asset_root` helpers) from the
origin/rust design, wired into the `http_route_response` chain ahead of the
direct-v86 fallback. Also restored `StaticResponse`'s `headers` field +
`with_header()` so the asset route can send `Cache-Control: no-store` (prevents
the browser serving a stale `extension.js` across rebuilds).

- `crates/wanix-cli/src/serve/http/routes.rs` — route + helpers + `no-store`.
- `crates/wanix-cli/src/serve/http/response.rs` — `headers` field, `with_header`.
- `http.rs`, `discovery.rs` (x2), `direct_v86.rs`, `http/agent.rs` — each
  `StaticResponse { .. }` literal gained `headers: Vec::new()`.
- `crates/wanix-cli/src/serve/tests.rs` — new
  `serve_once_returns_workbench_assets_outside_served_root` test: asserts
  `/workbench/package.json` serves the repo manifest (not a served-root decoy)
  with `Cache-Control: no-store`.

### Root cause 2 — stubbed `system-view.ts` + minimal manifest (TS)

After the asset route landed, the workbench shell rendered but showed
"There is no data provider registered that can provide view data." Two reasons:

1. `workbench/src/web/system-view.ts` on the branch was a 97-line **stub** (vs
   origin/rust's 1905 lines); its `register()` was empty, so no
   `TreeDataProvider` was registered for the `wanix.system` view. Restored the
   real implementation from origin/rust. `check-types` stays green (the real
   file already exports `WanixSystemConfig` and the `checkStarted/...` API the
   expanded `cockpit-self-check.ts` calls).
2. `workbench/package.json` was the minimal cpu manifest (53 lines, 2 commands,
   no views). Restored origin/rust's full manifest (545 lines): the `wanix`
   activity-bar container, the `wanix.system` view, 40 commands, and menus.
   Scripts and dependencies are identical to the minimal version, so the build
   is unaffected.

### Build + checks: green

- `cargo check --workspace` — clean.
- `cargo clippy -p wanix-cli --all-targets` — clean.
- `cargo test -p wanix-cli --lib serve` — 75 passed (incl. the new asset test).
- `just module-lines` — ok (the two `wanix-agent` warnings are pre-existing and
  under the hard limit; my edits added no over-limit module).
- `npm run check-types` — pass; `npm run compile-web` — `extension.js` 357,872
  bytes (grew because the real 1905-line system-view is now compiled in).

### Browser validation: **PASS**

Headless Chromium (Playwright 1.53, cached chromium 1223) against
`wanix serve --root /tmp/wanix-test-root3 --bundle workbench-fs9p
--wanix-services --addr 127.0.0.1:18021`:

- Code OSS workbench renders — no banner, no failed requests, no page errors.
- EXPLORER shows the Wanix 9P filesystem (`/` → `hello.js`).
- TERMINAL shows the live qjs-shell: `shell task: 1` with a `$` prompt.
- The **Wanix** activity-bar entry is present; opening it shows the full
  **WANIX: SYSTEM** tree — 39 rows across Actions (15 commands), Tour, Reports,
  Data Stores, Checks, Activity, Routes, Route Runs, Agent, Tasks, Terminals.
- Live over 9P: Activity shows "service task 1 observed" / "qjs shell route
  discovered"; **Tasks shows "1 shell — running · #task"** (read from the `#task`
  service, not extension memory).

**Screenshots (current):**
- `docs/integration/screenshots/cockpit-loaded-v3.png` — workbench + 9P explorer
  + live shell.
- `docs/integration/screenshots/wanix-cockpit.png` — the WANIX: SYSTEM tree
  fully populated.

### Known remaining gap (not blocking render)

Three large demo modules are still stubbed inline in `extension.ts` with
rest-param no-ops (documented with `// STUB:` comments): `duet-demo.ts`,
`http-app-demo.ts`, `v86-shared-demo.ts`. Their rows appear in the sidebar but
their actions do nothing yet. The device-adapted modules
(`service-inspector.ts`, `agent-repair-demo.ts`, `cockpit-self-check.ts`) are
compiled and wired but their `#kv`/`#agent`/`#cas`/mesh-probe behavior is
runtime-unverified — exercising them against the live devices is the next step.

### Suggested next moves (revised)

1. Exercise the cockpit actions against the live mesh devices: click
   `Run Cockpit Self Check` (mesh probes), `Run Agent Repair Demo` (`#agent`
   flow), inspect `#kv`/`#agent` via the service inspector. Capture screenshots.
2. Port the three stubbed demos (`duet`, `http-app`, `v86`) — http-app should
   drive `#kv` for counter state per the plan; duet is subsumed by the existing
   `shared_vfs_differential` test and may just need re-wiring.
3. Then Slice 8 (`--bundle cockpit` rename + retire the `workbench/code/` vendor
   dependency).

## Update 2026-06-07: cockpit features wired to the mesh devices — **working**

Each cockpit feature below was made to actually drive the cpu mesh/agent
devices over direct 9P and verified end-to-end in headless Chromium (and via
curl for the HTTP route). Committed as focused slices.

1. **service-inspector** — the Namespace tree now lists every bound service
   device (discovery `services.devices`, sourced from
   `roots::INSPECTABLE_SERVICE_DEVICES`); opening one renders the live
   directory over 9P with allocator/metadata/stream annotations. Verified:
   `#kv` (empty), `#agent` (`new`→allocator), `#cas` (`have/` + `ingest`).
   Screenshots: `inspect-kv.png`, `inspect-agent.png`.

2. **agent-repair-demo** — `repairViaAgent` opens a `#agent` session, submits
   the patch plan as an `approve:` prompt, polls `pending`, resolves via `ctl`,
   then applies the approved patch. Verified: one click fixes
   `/agent/broken.js`, `result.txt` reads "FIXED BY WANIX AGENT", and the
   report logs the `#agent` session/approval. Screenshot: `agent-repair.png`.

3. **cockpit-self-check** — drivers now flow through to the config; `#pipe` and
   `#plumb` no longer hang (bridge live-stream fix + `O_WRONLY` write ends).
   Verified: the check completes and writes `/.wanix/cockpit-check.{md,json}`;
   drivers/services/shell/report-storage and `#agent`/`#kv`/`#pipe`/`#cas`/
   `#plumb` pass. Screenshot: `self-check.png`.

4. **duet** — un-stubbed; qjs producer → wasm transform → qjs verify on one
   shared FS. Verified: `shared/out.txt` = "rust-wasm saw: hello from qjs
   duet". Screenshot: `duet.png`.

5. **http-app + #kv** — restored the Rust `/.wanix/app/<name>` route, made
   `#kv` (and the other `#` devices) reachable from WASI guests, and rebacked
   the counter demo on `#kv/http-counter`. Verified: curl + cockpit increment
   one shared counter; "HTTP Counter State" indexed as a data store.
   Screenshot: `http-counter.png`.

Build status: `cargo check --workspace` clean; `cargo test -p wanix-cli --lib
serve` 76 pass; `cargo test -p wanix-wasi` 59 pass; `npm run compile-web`
clean.

Still stubbed (documented `// STUB:` in extension.ts): `v86-shared-demo`.
Known constraint: `#plumb` live receive needs a second 9P connection (the
single-connection serve handles one frame at a time), so the self-check probes
the publish path only.
