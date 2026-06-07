> **Editorial header (2026-06-06):** This sprint plan predates the `cpu` branch
> mesh slices. Bullets 3 and 4 of the original demo arc — "Linux can mount it"
> and "It can serve apps" as standalone milestones — are now subsumed by the
> mesh work: blobs, cpu exec plane over QUIC, and agents on the mesh already
> deliver network-addressable Wanix objects and remote execution across nodes,
> with Linux/9P participation falling out of the mesh substrate rather than a
> single-host demo. The cockpit described below is therefore best read as the
> **operator UI for the mesh**: the sidebar, activity log, task/terminal/route
> panels, and agent repair flow remain the right surface, but they should
> visualize mesh-wide objects (nodes, remote tasks, mounted blobs, agent
> sessions) rather than a single Rust serve. Treat Linux-mount and HTTP-route
> bullets as cockpit affordances over the mesh, not as the sprint's load-bearing
> proof.

# Wanix OS Cockpit: One-Month Sprint Plan

This sprint should make the Rust-served Wanix workbench feel like an operating-system cockpit, not just a browser editor with a terminal. The browser should expose files, tasks, terminals, services, runtimes, and eventually agents as inspectable Wanix objects.

The month-long demo arc:

```text
This is an OS.
It runs multiple runtimes.
Linux can mount it.
It can serve apps.
Agents can operate it.
```

## Sprint Goal

By the end of the month, a user can open the Rust-served browser workbench and see Wanix as a live system:

- A Wanix sidebar shows drivers, tasks, terminals, namespace mounts, and recent activity.
- JavaScript and compiled WASM programs run from the same browser workbench and share one Wanix namespace.
- Browser file edits, shell commands, and task writes show up as system activity.
- A Linux/v86 or VM demo can read and write the same served Wanix files over 9P.
- A small HTTP route can be backed by a Wanix qjs or wasm file.
- An agent demo can inspect a broken program, run it, edit it, rerun it, and show what it did in the same system panel.

## Non-Goals

This sprint is not trying to finish public auth, multi-tenant hosting, production collaboration, full Linux distro workflows, or a generalized cloud platform. Those need separate trust-boundary work.

Avoid building a marketing surface. The first screen should remain the usable workbench.

Avoid hiding core behavior behind custom magic. The point is to teach the Wanix model by showing real service files, task state, terminal resources, filesystem writes, and runtime exits.

## Week 1: Living System Panel

Theme: make the OS model visible.

### Demo

Open:

```text
http://127.0.0.1:4444/?bundle=workbench-fs9p&term
```

The workbench shows a Wanix activity/sidebar panel:

```text
Wanix
  Drivers
    qjs
    wasm
  Tasks
    1 shell running
  Terminals
    1 attached
  Namespace
    /      local demo root
    #task  task service
    #term  terminal device
```

Click play on `hello.js`.

The panel updates:

```text
Tasks
  1 shell running
  2 qjs hello.js running
  2 qjs hello.js exited 0
```

### Work

- Add a Wanix activity bar container and view in the web extension.
- Build a small state model in the extension for discovery, drivers, task launches, terminal launches, and namespace roots.
- Surface task lifecycle events from direct qjs runs and shell sessions.
- Poll or read `#task` and `#term` service files conservatively enough to stay robust.
- Add a recent activity log: task started, task exited, terminal opened, terminal closed, fs refreshed.
- Keep the view useful without requiring a custom backend protocol.

### Acceptance Criteria

- Browser screenshot clearly shows a Wanix sidebar, not just Explorer and Terminal.
- Running qjs from the editor updates the task list and activity log.
- Closing or replacing qjs task terminals does not leave stale live task state.
- Discovery drivers include `noop`, `qjs`, and `wasm`.
- `npm run compile-web` and `just check` pass.

### Risk Gate

If service-file polling becomes noisy or unreliable, ship the panel with extension-observed events first, then add direct service inspection in Week 2. A partly-live panel is better than a fragile fake-live panel.

## Week 2: JS And WASM Duet

Theme: prove Wanix is a shared runtime substrate.

### Demo

Project files:

```text
producer.js
transform.wasm
verify.js
data/input.txt
```

Flow:

1. Run `producer.js`; it writes `data/input.txt`.
2. Run `transform.wasm`; it reads JS output and writes `data/output.txt`.
3. Run `verify.js`; it validates the wasm result.

Panel:

```text
2 qjs producer.js exited 0
3 wasm transform.wasm exited 0
4 qjs verify.js exited 0
fs write data/input.txt
fs write data/output.txt
```

### Work

- Add `.wasm` run action in the workbench when discovery advertises the wasm driver.
- Add a persistent terminal/output path for wasm task runs, matching the qjs ergonomics.
- Add a demo scaffold under a served sample root or documented fixture path.
- Make Explorer refresh after wasm task exits and observed writes.
- Add a small task-run abstraction in the extension so qjs and wasm share terminal lifecycle code.
- Add focused tests around serve discovery, workbench bootstrap, and wasm driver advertisement.

### Acceptance Criteria

- `.js` and `.wasm` files both get run actions in the browser workbench.
- A wasm task can read a file written by qjs and write a file read by qjs.
- The Wanix sidebar shows qjs and wasm task lifecycle entries.
- Repeated runs do not accumulate stale task terminals.
- `npm run compile-web` and `just check` pass.

### Risk Gate

If generating or shipping a wasm fixture is annoying, use an existing checked-in wasm example first. The demo matters more than perfect project scaffolding in Week 2.

## Week 3: Shared Files With Linux/v86 And HTTP Routes

Theme: Wanix coordinates more than editor tasks.

### Demo A: Linux Guest Shares Files

Split screen:

- Left: Wanix Workbench.
- Right: v86 Linux terminal.

Flow:

1. Edit `shared/message.txt` in the workbench.
2. In Linux: `cat /mnt/wanix/shared/message.txt`.
3. Linux sees the browser edit.
4. In Linux: `echo from-linux > /mnt/wanix/shared/linux.txt`.
5. The file appears in Explorer.
6. Wanix activity shows the write or refresh.

### Demo B: Browser Worker From A Wanix File

Project:

```text
routes/counter.js
data/count.txt
```

Open:

```text
http://127.0.0.1:4444/.wanix/app/counter
```

Each refresh increments `data/count.txt`. The workbench shows the file changing, and the panel shows route task execution.

### Work

- Tighten v86/direct-9P demo handoff enough to present a reliable shared-file round trip.
- Add a minimal route runner on Rust serve:
  - route prefix `/.wanix/app/<name>`
  - maps to `apps/<name>.js` initially
  - runs through Wanix qjs task machinery
  - captures stdout/stderr in `.wanix/http/<task>.out` and `.err`
  - returns stdout as the first response contract
- Start with a deliberately tiny HTTP response API; do not design a full workers platform yet.
- Reflect route execution in discovery and the Wanix sidebar.
- Add loopback-only guardrails for HTTP app routes while auth is not implemented.

### Acceptance Criteria

- A v86/Linux demo can read a workbench-written file and write a file visible to the workbench.
- `/.wanix/app/counter` runs a Wanix-backed route and mutates Wanix state.
- Route task execution appears in the Wanix panel.
- The route contract is documented and has focused tests.
- `just check` passes.

### Risk Gate

If v86 consumes too much time, keep Week 3's required deliverable as HTTP routes plus a documented direct-v86 shared-file status. Do not let VM polish block the app-platform proof.

## Week 4: Agentic Repair Demo And Polish

Theme: agents operate Wanix like a computer.

### Demo

Start with `broken.js`:

```js
std.writeFile("/out/result.txt", value.toUpperCase());
```

Run it. It fails.

Click `Ask Agent To Fix`.

Panel:

```text
Agent
  read broken.js
  ran qjs broken.js
  saw ReferenceError: value is not defined
  edited broken.js
  ran qjs broken.js
  task exited 0
  created out/result.txt
```

The browser shows the edited file and the task output.

### Work

- Define the smallest agent contract that can operate inside Wanix:
  - read file
  - write file
  - run qjs or wasm task
  - observe task exit and output
  - append activity entries
- Use Codex app-server integration only as an engine behind that Wanix-shaped tool contract.
- Build a narrow "Fix current Wanix program" command in the workbench.
- Keep the agent visibly grounded in Wanix actions rather than opaque chat.
- Add failure handling: agent cannot fix, task still fails, dirty file save needed, missing driver.
- Polish screenshots, docs, demo scripts, and a one-page walkthrough.

### Acceptance Criteria

- The broken-program repair demo works from a browser workbench session.
- The agent action log is explicit and maps to Wanix operations.
- The resulting file diff is visible in the workbench.
- The fixed program runs and creates expected output.
- `npm run compile-web` and `just check` pass.

### Risk Gate

If app-server integration is not ready, mock the agent boundary with a deterministic local repair command and keep the command shape identical. The important sprint deliverable is the Wanix tool contract and visible operation log.

## Cross-Cutting Engineering Work

These should happen throughout the month:

- Keep commits small and demo-shaped.
- Run `just check` before cycle commits.
- Keep workbench source readable; avoid hiding the model in one large extension file.
- Keep lower-level Rust crate boundaries intact.
- Add or update ADRs only if a durable API, trust boundary, handoff format, or workflow changes.
- Do not add auth-looking features without deciding the actual trust boundary.
- Prefer observable service files and discovery metadata over bespoke hidden state.

## Suggested Commit Rhythm

Week 1:

- `Add Wanix workbench sidebar`
- `Show task and terminal activity in workbench`
- `Inspect Wanix services from sidebar`

Week 2:

- `Add wasm run action to workbench`
- `Share task terminal lifecycle across qjs and wasm`
- `Add JS and WASM filesystem duet demo`

Week 3:

- `Add Wanix app route handoff`
- `Run qjs route handlers from serve`
- `Document direct-v86 shared namespace demo`

Week 4:

- `Define Wanix agent tool contract`
- `Add fix current Wanix program command`
- `Document OS cockpit demo sequence`

## What "Magical" Means

The sprint succeeds if the user can watch the browser and understand the Wanix model without a lecture:

- Files are a namespace, not just editor tabs.
- Tasks are inspectable objects, not hidden subprocesses.
- Terminals are devices, not just UI panes.
- JS and WASM are runtimes on the same substrate.
- Linux can participate through 9P.
- HTTP routes are Wanix programs.
- Agents use the same system surface a person uses.

That is the unlock. Once people see the system, every next feature has somewhere to live.
