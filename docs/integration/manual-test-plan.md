# Manual Test Plan: Workbench in a Browser (cpu branch)

This plan documents how to manually validate the workbench bundle that ships
on the `cpu` branch as of 2026-06-07. The plan distinguishes:

1. What is verifiable today on `cpu` (Wanix-served VS Code workbench shell
   over direct 9P, plus `--wanix-services` allocators) — fully scriptable.
2. What the validation prompt asks for ("cockpit", `wanix-inspect:#agent`,
   agent demo button, sidebar categories, service-inspector) — those belong
   to the cockpit-TS surface described in `docs/integration/plan.md` Slice 1
   and `docs/integration/cockpit-meets-mesh.md`, and are NOT yet on this
   branch.

The automated portion of this plan was already executed once; results are
recorded in the "Observed" section near the end.

## 0. One-time prerequisites

The workbench HTML emitted by `serve --bundle workbench-fs9p` expects the
VS Code Web build to live at `workbench/code/out/...`. That tree is fetched
from a pinned upstream release and is NOT vendored in the repo:

```sh
cd workbench && make build     # downloads vscode-web zip, unpacks to ./code
```

The Makefile uses
`https://github.com/progrium/vscode-web/releases/download/v1/vscode-web-1.108.2.zip`.
If your environment forbids fetching that archive, the workbench shell will
still load (the page returns HTTP 200 and the document title becomes
"Wanix Rust workbench"), but the in-page status banner will show
`Error: failed to load .../workbench/code/out/nls.messages.js` because the
loader cannot find the VS Code web bundle. This is expected and is the
state captured in `docs/integration/screenshots/cockpit-loaded.png`.

If you DO build the workbench assets, the page renders the VS Code-on-Web
workbench against a `wanix:/` `FileSystemProvider` backed by direct 9P.

## 1. Build the wanix binary

From the repo root:

```sh
cargo build -p wanix-cli
ls target/debug/wanix
```

## 2. Start serve

```sh
mkdir -p /tmp/wanix-test-root
echo hello > /tmp/wanix-test-root/hello.txt

target/debug/wanix serve \
  --root /tmp/wanix-test-root \
  --bundle workbench-fs9p \
  --wanix-services \
  --addr 127.0.0.1:17999
```

`--wanix-services` binds `#task` and `#term` into the served namespace so the
qjs-shell websocket and 9P clients can allocate tasks and terminal resources.

## 3. Smoke-check the routes without a browser

```sh
# 1. Discovery JSON: routes, services, drivers
curl -sS http://127.0.0.1:17999/.well-known/wanix.json | jq

# 2. Rootfs handoff (unprepared on a bare /tmp root)
curl -sS http://127.0.0.1:17999/.well-known/rootfs.json | jq

# 3. Bundle page (HTTP 200 even if VS Code assets are absent)
curl -sS -o /tmp/page.html -w 'HTTP %{http_code}\n' \
  http://127.0.0.1:17999/?bundle=workbench-fs9p
grep -i '<title>' /tmp/page.html  # -> "Wanix Rust workbench"
```

Expected: discovery JSON contains
`routes.p9.websocket = ws://127.0.0.1:17999/.well-known/export9p` (protocol
`9P2000.L`, supported `9P2000.L` + `9P2000.L.Google.2`), `routes.qjsShell`
with status `available`, `services.drivers = ["auto","noop","qjs","wasm"]`,
and `bundle = workbench-fs9p`.

## 4. Open the workbench in a browser

```sh
open http://127.0.0.1:17999/?bundle=workbench-fs9p
```

If the VS Code Web bundle is built (`workbench/code/out` populated):

- The page loads VS Code-on-Web with title "Wanix Rust workbench".
- A `wanix:/` workspace is mounted (via `WanixBridge` in
  `workbench/src/web/bridge.ts`). The Explorer should show `hello.txt`.
- Editing/saving a file writes through direct 9P to `/tmp/wanix-test-root`.
- The integrated terminal can open a qjs-shell session via
  `routes.qjsShell.websocket` and run the qjs-shell built-ins
  (`ls`, `cat`, `cd`, ...).
- Symlinks the test setup creates appear with the correct icon (preserved by
  the Google.2 `walkgetattr` extension when negotiated).

If `workbench/code/out` is NOT built, the page still returns HTTP 200 and
title "Wanix Rust workbench", but the in-page red status banner says
`Error: failed to load .../workbench/code/out/nls.messages.js` and the
viewport stays empty. This is the state captured in
`docs/integration/screenshots/cockpit-loaded.png`.

## 5. What the prompt asked for but is NOT on this branch yet

The prompt mentions text "cockpit", a `wanix-inspect:#agent` navigation,
an "agent demo button", and a "service-inspector" port. Those are part of
the proposed integration described in `docs/integration/plan.md` Slice 1
("Restore the cockpit shell") and `docs/integration/cockpit-meets-mesh.md`
("Two spikes, two halves of the same machine"). They were prototyped on the
`origin/rust` branch (`workbench/src/web/system-view.ts`,
`service-inspector.ts`, `extension.ts`, the demo modules) but have NOT
been merged onto `cpu`. None of those TypeScript modules exist on this
branch — `ls workbench/src/web` will not show them.

When Slice 1 lands, the validation should additionally:

- Open the new `Wanix System` activity-bar view and confirm the seven
  device categories (`#task`, `#term`, `#agent`, `#blobs`, `#cas`, `#cpu`,
  `#mesh`) appear, each derived from the discovery JSON's exposed device
  list.
- Click the `Install agent-repair demo` action and observe a `#agent/<id>`
  session being allocated end to end through the cockpit, not via a
  bridge-side fixer.
- Open a `wanix-inspect:#agent` URI and confirm the read-only allocator
  view (`new` is treated as control-only).
- Run `cockpit-self-check` and confirm a green report for the device list
  exposed by `cpu` + `mesh` work.

Those steps cannot be exercised on `cpu` HEAD; do not add them to a CI
gate until the slice merges.

## 6. Tear down

```sh
pkill -f wanix
rm -rf /tmp/wanix-test-root
```

## 7. Observed on 2026-06-07 (cpu @ 667dbc1)

Automated portion of this plan was executed once before writing this file:

- `target/debug/wanix serve --root /tmp/wanix-test-root --bundle
  workbench-fs9p --wanix-services --addr 127.0.0.1:17999` started cleanly
  and logged
  `bundle available at http://127.0.0.1:17999/?bundle=workbench-fs9p`.
- `GET /.well-known/wanix.json` returned the full discovery JSON above
  (p9 websocket, qjs-shell websocket, rootfs unprepared, services with
  drivers `auto/noop/qjs/wasm`, bundle `workbench-fs9p`).
- `GET /?bundle=workbench-fs9p` returned HTTP 200 with title
  `Wanix Rust workbench` and the loader stub referencing
  `/workbench/code/out/vs/workbench/workbench.web.main.css` and
  `/workbench/code/out/nls.messages.js`.
- Those static assets returned 404 because `workbench/code/out` is not
  populated on this branch (vscode-web archive not vendored). The in-page
  status banner reflected `Error: failed to load .../nls.messages.js`.
- Headless Chromium screenshot saved to
  `docs/integration/screenshots/cockpit-loaded.png` (1280x800, fullPage),
  showing the dark background + error banner. The "cockpit" / `Wanix` /
  `agent` keywords described in the prompt are NOT present in the body
  text — both because the VS Code shell did not fully boot and because
  the cockpit/inspector UI from `origin/rust` has not been integrated
  here yet (see Section 5).

The serve plumbing is verifiably correct on `cpu`. The workbench UI on top
of it is gated on (a) vendoring `vscode-web-1.108.2.zip` into
`workbench/code/`, and (b) merging the cockpit-TS slice described in
`docs/integration/plan.md`.
