# Wanix Workbench In The Browser: The Small Wins That Made It Usable

This pass was about taking the Rust-served Wanix workbench from "it technically boots" to "I can actually use this from a browser without tripping over the first five papercuts."

The target was intentionally modest: serve the `workbench-fs9p` bundle from Rust, use it in the browser like a real user, fix the smallest annoying thing, verify it, commit it, and repeat. The result is not a full product launch. It is better than that for this stage: it is a working loop.

## What You Can Do Now

Start the Rust server with services enabled:

```sh
cargo run -p wanix-cli -- serve \
  --root /tmp/wanix-web-dx-root \
  --addr 127.0.0.1:4444 \
  --bundle workbench-fs9p \
  --wanix-services
```

The CLI now prints the useful URL:

```text
http://127.0.0.1:4444/?bundle=workbench-fs9p&term
```

That URL opens Code OSS in the browser with the Wanix filesystem, Wanix task service, and an interactive shell ready to go.

![Wanix Workbench with shell](assets/wanix-workbench-browser-dx/01-terminal-ready-workbench.png)

You can also deep-link straight to a file:

```text
http://127.0.0.1:4444/?bundle=workbench-fs9p&term&open=wanix:/hello.js
```

That opens the file in the editor and keeps the shell available below it.

![Deep-linked Wanix JavaScript file](assets/wanix-workbench-browser-dx/02-deep-linked-hello-js.png)

From there, the editor title bar has a play button for JavaScript files. Click it and the current `wanix:` file runs as a QuickJS task through the Rust Wanix task machinery.

![QuickJS task output in the browser](assets/wanix-workbench-browser-dx/03-qjs-run-output.png)

Repeated runs replace the previous qjs task terminal instead of piling up stale tabs.

![Repeated qjs run keeps one task terminal](assets/wanix-workbench-browser-dx/04-repeat-run-single-terminal.png)

The workbench now also has a Wanix activity bar view. It shows the advertised
drivers, observed tasks, terminal resources, namespace roots, and recent
activity. That turns the browser from "an editor attached to Wanix" into a small
cockpit for the system.

Namespace roots are now active inspection points. Click `#task` or `#term` in
the Wanix sidebar and the workbench opens a generated `wanix-inspect:` document
that lists the real service directory without reading files that allocate
resources. That means the browser can show the actual task and terminal service
shape even though those paths are hidden from ordinary root listings.

![Wanix service inspector for #task](assets/wanix-workbench-browser-dx/23-service-inspector.jpg)

Task rows use the same path now. Right-click a task and choose `Open Wanix Path`
to inspect its own `#task/<id>` directory: `cmd`, `ctl`, `dir`, `env`, `exit`,
`fd`, and the rest of the service surface are visible from the task object that
caused them.

![Task row opens its task service directory](assets/wanix-workbench-browser-dx/24-task-service-context.jpg)

The inspector entries are not dead text anymore. Safe metadata files and child
directories are document links: follow `#task/1/kind` and it opens as a normal
Wanix file, while stream, control, and allocator paths stay plain text.

![Service inspector entry opens a Wanix service file](assets/wanix-workbench-browser-dx/25-service-inspector-links.jpg)

The plain-text rows explain themselves now too. `#term/new`, for example, is an
allocator file; reading it creates a terminal. The inspector leaves it unlinked
and says why, so the service surface teaches the trust boundary instead of
surprising the user.

![Unsafe service entries explain why they are not links](assets/wanix-workbench-browser-dx/26-service-inspector-unsafe-entries.jpg)

Task runs now write a second artifact beside the transcript:
`/.wanix/tasks/<task>.metadata.json`. It records the task id, runtime kind,
argv, cwd, env, source path, output path, exit status, and transcript capture
details. The task row context menu has `Open Task Metadata`, so the run has a
machine-readable record without leaving the workbench.

![Task metadata JSON opened from the sidebar](assets/wanix-workbench-browser-dx/27-task-metadata-artifact.jpg)

The first compiled-WASM path is in the browser too. Right-click a `wanix:`
`.wasm` file in Explorer and choose `Run Current WASM as wasm Task`. The module
runs through `#task/new/wasm`, shares the same namespace as qjs and the shell,
and reports its lifecycle in the Wanix sidebar.

In this smoke test, `guest.wasm` reads `/shared/in.txt`, writes
`/shared/out.txt`, prints its Rust WASI output in the terminal, and exits 0.

![WASM task shown in the Wanix sidebar](assets/wanix-workbench-browser-dx/05-wasm-task-sidebar.png)

The standalone WASM path now has the same first-use shape as qjs. `Install WASM
Starter` writes `/wasm/starter.wasm`, `/wasm/README.md`, and
`/wasm/shared/in.txt`. `Run WASM Starter` runs that module as a wasm task and
opens `/wasm/shared/out.txt` when the task exits.

That first run found a useful boundary detail: browser-started tasks preopen
their working directory as the guest root, so the guest's `/shared/in.txt`
appears in Wanix as `/wasm/shared/in.txt`. The README names that explicitly
instead of making the user rediscover it from a missing-file error.

![WASM starter run opens the transformed output](assets/wanix-workbench-browser-dx/30-wasm-starter-run.png)

The System view now has an `Actions` section too. The same useful first moves
are present as rows: create a qjs script, run/install the WASM starter, run the
duet, preview the HTTP app, open the handler, and install the repair demo. That
keeps an empty or unfamiliar root from depending on tiny toolbar icons alone.

![Wanix System action rows](assets/wanix-workbench-browser-dx/31-system-action-rows.png)

Task rows now expand into the things you actually want after a run. For the
WASM starter that means transcript, metadata, terminal, and `#task/<id>` service
links are visible directly under the task. The row itself expands instead of
trying to open the `.wasm` binary as text.

![Wanix task artifact rows](assets/wanix-workbench-browser-dx/32-task-artifact-rows.png)

Repeated demo runs now have a tidy escape hatch too. `Clear Finished Rows`
removes exited tasks and closed terminals from the sidebar while keeping running
sessions, files, transcripts, and service state intact.

![Wanix clear finished rows](assets/wanix-workbench-browser-dx/33-clear-finished-rows.png)

The Linux/v86 proof now has a cockpit entry point as well. Discovery carries the
direct-v86 launch route into the workbench, the System view shows a `direct-v86`
route with boot readiness, and `Open v86 Shared Files Demo` seeds `/shared` with
a README, `message.txt`, and `from-linux.txt`. The README gives the exact
workbench -> Linux -> workbench file loop.

![Wanix v86 shared files demo](assets/wanix-workbench-browser-dx/34-v86-shared-files-demo.png)

The next polish pass removed another bit of demo friction: you no longer need
to hand-copy a WASM fixture into the served root. The Wanix system view has an
`Open JS and WASM Duet Demo` action. It creates missing starter files:

```text
/duet/producer.js
/duet/transform.wasm
/duet/verify.js
/duet/shared/
```

The demo flow is deliberately small and inspectable:

1. `producer.js` runs as qjs and writes `shared/in.txt`.
2. `transform.wasm` runs as compiled WASM and writes `shared/out.txt`.
3. `verify.js` runs as qjs and checks the WASM output.

The sidebar shows all three tasks as one system story.

![JS and WASM duet demo](assets/wanix-workbench-browser-dx/06-js-wasm-duet-demo.png)

The latest pass makes that sequence a button. `Run JS and WASM Duet Demo`
ensures the sample files exist, runs qjs producer, runs the WASM transform, runs
qjs verify, and stops if a step exits nonzero. The result is a one-click proof
that the browser workbench can coordinate multiple Wanix runtimes through the
same task and filesystem surface.

![Guided JS and WASM duet run](assets/wanix-workbench-browser-dx/07-guided-duet-run.png)

One more click of polish: after the guided run succeeds, the workbench opens the
generated `duet/shared/out.txt` file. The user sees the actual namespace result,
not just terminal output or a toast.

![Guided duet output file](assets/wanix-workbench-browser-dx/08-guided-duet-output.png)

The duet run no longer rewrites the sample program every time it starts. It
leaves edited `producer.js` and `verify.js` alone. A separate `Reset JS and WASM
Duet Demo` action is the explicit overwrite path, and it clears the generated
`duet/shared/in.txt` and `duet/shared/out.txt` files so the next run starts
clean.

![Duet reset command keeps reset explicit](assets/wanix-workbench-browser-dx/28-duet-reset-command.png)

The newest slice makes Wanix serve a tiny HTTP program from inside the same
system. With services enabled, `GET /.wanix/app/hello?from=browser` looks for
`apps/hello.js`, starts it as a qjs task, passes request metadata through
environment variables and argv, captures stdout, and returns that stdout as the
HTTP response.

The demo handler is intentionally boring:

```js
import * as std from "qjs:std";

const app = std.getenv("WANIX_HTTP_APP");
const target = std.getenv("WANIX_HTTP_TARGET");
std.out.puts("wanix http app " + app + " saw " + target + "\n");
```

The browser response is now bytes from a Wanix task, not static file content.

![HTTP app route backed by qjs](assets/wanix-workbench-browser-dx/09-http-app-route.png)

That route proof immediately exposed the next browser papercut: nobody should
have to type the handler by hand. The Wanix system view now has an
`Install HTTP App Demo` action. It creates `/apps/hello.js`, refreshes Explorer,
opens the handler in the editor, and leaves the served route ready at:

```text
http://127.0.0.1:4444/.wanix/app/hello
```

![HTTP app demo installer](assets/wanix-workbench-browser-dx/10-http-app-installer.png)

The cockpit now advertises the route contract too. The Wanix sidebar has a
`Routes` section, populated from serve discovery, so the HTTP app path is not
tribal knowledge:

```text
/.wanix/app/<name> -> apps/<name>.js
```

![HTTP route in the Wanix sidebar](assets/wanix-workbench-browser-dx/11-http-route-sidebar.png)

One more pass made the preview stay inside the cockpit. The globe action now
installs the handler if needed, fetches the HTTP app route from the workbench,
writes a small response report to `/apps/hello.response.txt`, and opens that
file in the editor. The route still runs through the Rust serve qjs task path;
the preview just lands the result, URL, status, content type, and timestamp
back in the Wanix namespace.

![HTTP app response report previewed in the workbench](assets/wanix-workbench-browser-dx/13-http-app-preview-report.jpg)

Then the route row itself became clickable. The Wanix System view no longer
just announces `/.wanix/app/<name>` as a passive contract; selecting that route
runs the preview action and opens the response report. That is the small shift
from "this exists" to "this is usable."

![HTTP route row runs the preview report](assets/wanix-workbench-browser-dx/14-http-route-row-preview.jpg)

The next browser pass caught a more subtle but important editor bug. Previewing
the route used to re-install the demo handler before running it, so a user's
edits could be overwritten. It also refreshed the root broadly, which meant an
already-open response report could stay stale. The preview path now only creates
`/apps/hello.js` when it is missing, and it emits precise refreshes for the
handler and response files.

![HTTP preview preserves handler edits and refreshes the open report](assets/wanix-workbench-browser-dx/15-http-preview-preserves-edits.jpg)

After that, handler source got its own action. Preview now means "run whatever
is there," while `Open HTTP App Handler` means "show me the source," and the
old install command is labeled as an explicit reset. That keeps the editing loop
honest: open, edit, preview, reset only when you ask for reset.

![HTTP handler source opened without resetting edits](assets/wanix-workbench-browser-dx/16-http-open-handler.jpg)

The route row now has the same controls in its context menu: preview the route,
open the handler, copy the concrete local URL, or reset the starter handler.
That makes `/.wanix/app/<name>` feel like an inspectable system object rather
than a label that happens to be clickable.

![HTTP route context actions reset the handler intentionally](assets/wanix-workbench-browser-dx/17-http-route-context-actions.jpg)

The latest route polish makes the row remember what happened. After previewing,
the route description includes the last HTTP status, and right-clicking the row
adds `Open Latest HTTP App Preview` so the response report stays attached to the
system object that produced it.

![HTTP route row shows latest status and preview action](assets/wanix-workbench-browser-dx/29-http-route-status-preview.png)

The route execution itself is visible now too. When the workbench previews
`/.wanix/app/hello`, the `Route Runs` section records the run, shows the HTTP
status and URL, and opens the saved response report for that execution.

![HTTP route run row](assets/wanix-workbench-browser-dx/37-http-route-run-row.png)

The route can now prove state too. `Preview HTTP Counter Demo` installs
`/apps/counter.js` and `/apps/counter.count.txt`, calls
`/.wanix/app/counter?from=workbench`, and saves the response report to
`/apps/counter.response.txt`. Run it twice and the second report says
`counter=2` while the `Route Runs` section keeps both HTTP executions.

This small slice caught a real route boundary: app handlers run with `/apps` as
their working directory, so the demo keeps its writable state beside the
handler instead of pretending app routes can freely write broader namespace
state.

![HTTP counter route run](assets/wanix-workbench-browser-dx/38-http-counter-route-run.png)

The first-use loop got a small entry point too: `New qjs Script` creates a
unique `/scratch/qjs-N.js`, opens it, and gives it a tiny program that writes
both terminal output and `last-run.txt`. New users no longer need to know where
to put their first qjs file before trying the runtime.

![New qjs Script creates a runnable scratch file](assets/wanix-workbench-browser-dx/18-new-qjs-script.jpg)

One more loop closed after using that starter in anger. qjs and wasm task rows
now remember the source file that launched them, so clicking a task row reopens
the program and the row context menu exposes `Open Task Source`. That turns the
task list into an inspector for the things you just ran, not just a status log.

![Task rows can reopen their source file](assets/wanix-workbench-browser-dx/19-task-row-open-source.jpg)

The context menu then grew a second operation: `Focus Task Terminal`. Task rows
now offer the two places you most often want after a run: the program that
produced the task and the terminal that captured its output.

![Task rows expose source and terminal actions](assets/wanix-workbench-browser-dx/20-task-row-context-actions.jpg)

The terminal stream is now mirrored into Wanix too. Direct qjs and wasm runs
write a transcript under `/.wanix/tasks`, and `Open Task Output` jumps from the
task row to the saved file. That makes task output inspectable even after focus
moves away from the terminal.

![Task output transcript opened from the sidebar](assets/wanix-workbench-browser-dx/21-task-output-transcript.jpg)

With transcripts in place, the first agent-shaped loop can be deterministic and
still honest. `Install Agent Repair Demo` writes `/agent/broken.js`. `Fix
Current Wanix Program` reads that source, runs it as qjs, observes
`ReferenceError: missingValue is not defined` from the saved transcript, edits
the file, reruns it, and opens `/agent/out/result.txt`.

This is not pretending to be the final app-server integration yet. It is the
risk-gate version of the Wanix agent contract: read file, run task, observe
task output, write file, rerun task, verify filesystem output, and show every
step in the same system panel.

![Deterministic Wanix agent repair demo](assets/wanix-workbench-browser-dx/22-agent-repair-demo.jpg)

The latest pass makes that contract visible as its own system object. The
Wanix sidebar now has an `Agent` section that resets for each install or repair
run and records the concrete steps: install, read, run, capture transcript,
observe the failure, edit source, rerun, capture output, and verify the result.
It sits directly under `Actions`, so the trace is visible while the repair is
running. Rows with filesystem artifacts open those Wanix paths directly.

![Wanix agent action log](assets/wanix-workbench-browser-dx/35-agent-action-log.png)

The repair now leaves a durable before/after trail too. Before editing, the
agent writes a source snapshot under `/agent/out`; after editing, it writes the
repaired snapshot beside it. The `diff broken.js` row reopens a browser-native
diff so the inserted line is visible without leaving the cockpit.

![Wanix agent repair diff](assets/wanix-workbench-browser-dx/36-agent-repair-diff.png)

## Why These Fixes Matter

The browser workbench is interesting because it makes the Rust port tangible. The runtime is no longer hidden behind CLI demos. You can browse a Wanix namespace, edit files, run qjs tasks, and watch the task output in one place.

But the first browser pass had several small breaks in the loop:

- The workbench assets were served from the user root, so disposable roots could fail to boot Code OSS at all.
- Files created from the shell did not show up in Explorer until a manual reload.
- Running a qjs script from the command palette could lose the active editor and then close the terminal too quickly to read output.
- Discovery advertised `noop` and `qjs`, but the served task table also supported `wasm`.
- The browser tab was still branded as generic Code OSS, which is awkward when juggling local servers.
- The obvious served URL did not open the terminal.
- There was no URL-level way to open the file you wanted for a demo.
- Repeated qjs runs left terminal clutter behind.
- The task and terminal model was invisible unless you already knew which
  service files to inspect.
- The WASM driver existed in Rust but did not have a usable browser entrypoint.
- The agent repair loop worked, but its actions were mixed into generic
  activity instead of appearing as a clickable repair trace.
- The repaired source changed in the editor, but there was no durable
  before/after diff artifact to inspect later.
- HTTP route previews updated route status, but the execution itself was not a
  first-class row in the system panel.
- HTTP route previews could prove a qjs handler returned text, but not yet that
  a browser-triggered app could mutate Wanix state.

None of those are huge by themselves. Together, they make the browser experience feel like a prototype you have to babysit. The point of this pass was to remove enough of that babysitting that the system starts to feel direct.

## What Changed

The first fix was asset routing. `workbench-fs9p` now serves `/workbench/...` from the repo's workbench assets instead of looking inside the served user root. That means you can serve a disposable filesystem root and still get a real workbench.

Then the Explorer learned to refresh after terminal activity and task exits. If a shell command writes a file, the browser UI catches up without a page reload.

The qjs run path now works as a real editor action. The extension remembers the last `wanix:` editor, exposes a play icon for `.js` files, saves before running, and keeps the terminal open long enough to show output plus an exit marker.

Discovery now reports the actual task drivers exposed by the served services:

```json
{
  "task": "#task",
  "term": "#term",
  "drivers": ["noop", "qjs", "wasm"]
}
```

The embedded workbench also identifies itself as `Wanix Workbench`, so the browser title is useful:

```text
hello.js - / - Wanix Workbench
```

Finally, the URL became part of the workflow. `open=wanix:/hello.js` lets docs, demos, and tools drop a user straight into the file that matters, while the printed serve URL includes `&term` when `--wanix-services` is on.

The next slice added the Wanix system sidebar and made task runs visible as
operating-system activity. The view is intentionally backed by discovery and
extension-observed task events first, rather than a new hidden backend protocol.
That keeps the UI honest: it shows the same drivers, task starts, terminal
resources, exits, and filesystem refreshes the workbench itself uses.

Service inspection makes that honesty more concrete. Namespace rows can now
open directory snapshots for `#task` and `#term`, so the sidebar is not only an
extension-side activity log. It is a path into the service files Wanix exposes.
Task rows also carry their own service path, which makes the task object a
bridge to `#task/<id>` instead of only a remembered source/output shortcut.
The generated snapshot now links safe files and child directories too, so the
user can walk from task object to service metadata without memorizing paths.
When a service file is deliberately not linked, the row now explains whether it
is an allocator, control file, stream, or unspecified service file.
Finally, each direct qjs/wasm task writes a metadata JSON artifact next to its
transcript, giving agents and humans the same stable handle for cwd, argv,
env, source, output, exit, and capture state.

WASM then got the same ergonomic path as qjs. The extension now has a shared
task-runner path for qjs and wasm editor actions, so `.wasm` files can run from
Explorer without trying to open binary modules as text. This is the first
browser proof of the bigger claim: JS and compiled WASM are sibling runtimes on
one Wanix substrate.

The standalone WASM starter closes the first-use gap. The workbench can install
the compiled Rust fixture, seed its input under the right task-rooted shared
path, run it, and open its output without asking the user to prepare a binary or
remember the cwd-to-guest-root mapping.

Then the browser got a one-click duet installer. It writes the JavaScript,
compiled WASM fixture, verifier, and local shared directory into the Wanix
namespace, opens the producer, and logs the install in the sidebar. The point is
not that the sample is fancy. The point is that a teammate can now open the
served workbench and prove the shared-runtime claim without preparing files by
hand.

The follow-up made the proof runnable as a single guided action. The command
still uses the same task objects and terminal resources as manual runs, but it
awaits each task exit before starting the next step. That gives us a demo button
without inventing a fake demo backend.

The output now opens automatically at the end of that guided run. This makes the
demo land on a concrete artifact in the Wanix namespace, which is a better
teaching moment than ending on a notification alone.

The duet run now follows the same edit-preserving rule as the HTTP demo: run
fills in missing starter files but does not overwrite local edits. Reset is a
separate command, and it owns clearing the generated shared input/output.

The HTTP-app route is the first app-platform proof. It is deliberately local
and narrow: `/.wanix/app/<name>` maps to `apps/<name>.js`, requires
`--wanix-services`, rejects non-loopback clients, binds stdout and stderr to
`.wanix/http/<task>.out` and `.wanix/http/<task>.err`, and returns stdout as
`text/plain`. Discovery advertises the contract as `wanix-http-app.v1`.

The browser then got a matching installer command. That command uses the same
direct 9P filesystem surface as the rest of the workbench, so the handler file
is not a fake sample hidden in the extension. It is a normal Wanix file under
`/apps`, ready to edit and serve.

Finally, the route itself is visible in the Wanix system panel. That matters
because the cockpit should explain the OS shape as you use it: drivers,
services, namespace, and now served programs.

The preview action closes the browser loop: edit a Wanix handler, run it as an
HTTP app, and inspect a status-bearing response report without leaving the
workbench.

The no-clobber preview is the difference between a demo and a usable tool. Once
you edit `apps/hello.js`, previewing the route keeps your handler intact and
updates the already-open report instead of showing stale output.

The handler action separates exploration from mutation. A user can inspect or
edit the qjs-backed HTTP program without accidentally reinstalling the starter
code.

The route context menu is a small but important cockpit pattern: visible system
objects should have operations attached to them. A route is not just something
to read; it is something to run, inspect, copy, and intentionally reset.

The route row now also keeps the latest preview status and saved report path.
That makes the HTTP app route behave more like a live system resource: it shows
last `200 OK` state and exposes the response artifact from the same row.

The route-run row closes the Week 3 loop from another angle: an HTTP request is
now represented in the cockpit as an execution, with the response report and
handler source hanging off the run instead of living only in the editor.

The counter route makes the app-platform proof stateful. It also documents the
current write boundary in practice: keep app-local state under `/apps` unless
the route contract grows broader namespace write semantics on purpose.

The scratch-script action is the same philosophy applied to creation. The
cockpit should not only inspect Wanix; it should help you make the next Wanix
object.

The `Actions` section makes that discoverable as text rows, not only toolbar
icons. It is the tiny welcome state in practice: when the filesystem is empty or
unfamiliar, the System panel still has concrete things to run, create, and
inspect.

Expandable task rows push the sidebar toward the same object model. A task
started from a file should carry enough memory to bring you back to the source,
transcript, metadata, terminal, and service directory without depending on a
right-click menu. For binary WASM, the source link stays out of the way and the
inspectable artifacts remain visible.

The clear-finished action makes the same sidebar usable during a long browser
session. The OS cockpit should accumulate evidence, not make every stale row
compete with the running shell.

The v86 shared-files starter is the same principle applied to the Linux proof.
It does not hide the unprepared-root state; it shows the missing boot markers,
the direct-v86 URL, and the shared files Linux should read and write.

Persisted transcripts make the output handle a real Wanix file. That matters for
the agentic repair work too: a person, a script, or an agent can inspect the
same `/.wanix/tasks/...output.txt` artifact after the run.

The repair demo is deliberately narrow, but it changes the feel of the cockpit:
an automated helper is no longer outside the system. It uses the same qjs task
runner, transcript files, source edits, result files, and Agent action log that
a person can inspect.

## The Feeling Now

The current loop is:

1. Start `wanix-rust serve`.
2. Open the printed URL.
3. Edit or create a file in `wanix:/`.
4. Use the System view's `Actions` rows when you want visible commands instead
   of toolbar icons.
5. Run a qjs task from the play button or `New qjs Script` action.
6. Run a `.wasm` module from Explorer.
7. Watch tasks, terminals, exits, and filesystem refreshes in the Wanix sidebar.
8. Click `#task` or `#term` in the sidebar to inspect real service directories.
9. Right-click a task row and inspect its `#task/<id>` service directory.
10. Follow a safe service entry like `#task/1/kind` into the real Wanix file.
11. See why allocator/control/stream service files stay plain text.
12. See output and exit status in the terminal.
13. Run again without terminal clutter.
14. Click `Install WASM Starter`, then `Run WASM Starter`, and inspect
    `/wasm/shared/out.txt`.
15. Install the duet demo and run qjs -> wasm -> qjs as one visible workflow.
16. Or click `Run JS and WASM Duet Demo` and let the workbench drive the
    sequence.
17. Inspect the generated `duet/shared/out.txt` result when the guided run
    opens it.
18. Edit `duet/producer.js`, run the guided duet without losing the edit, then
    explicitly reset the duet when you want the starter files back.
19. Click `Open v86 Shared Files Demo` to seed `/shared/message.txt` and read
    the direct-v86 launch and 9P mount instructions.
20. Open the `direct-v86` route when a prepared rootfs is available, then have
    Linux read `/shared/message.txt` and write `/shared/from-linux.txt`.
21. Click `Install HTTP App Demo` to create and open `apps/hello.js`.
22. Open
    `/.wanix/app/hello?from=browser` to get an HTTP response from a Wanix task.
23. See the HTTP app route contract in the Wanix sidebar.
24. Click `Preview HTTP App Demo` to fetch the route and open a status-bearing
    `/apps/hello.response.txt` report inside the workbench.
25. Click the `/.wanix/app/<name>` route row itself to run the same preview from
    the visible system contract.
26. Edit `/apps/hello.js`, preview again, and see the changed handler output
    without losing the edit or reopening the report.
27. Open the HTTP handler source directly, and reset the demo only through the
    explicit reset command.
28. Right-click the route row to preview, open source, copy the route URL, or
    intentionally reset the demo handler.
29. After previewing, read the route row's latest HTTP status and reopen the
    saved response report from `Open Latest HTTP App Preview`.
30. Expand `Route Runs` to see the previewed HTTP route execution and reopen
    its response report or handler source.
31. Click `Preview HTTP Counter Demo` twice to run a stateful qjs-backed HTTP
    app, then inspect `apps/counter.response.txt` and `apps/counter.count.txt`.
32. Click `New qjs Script` to create a unique runnable scratch script without
    leaving the workbench.
33. Run that script, then expand its task row to reopen the source, transcript,
    metadata, terminal output, or `#task/<id>` service directory.
34. Open the same task row's metadata JSON to inspect cwd, argv, env, source,
    output, exit, and transcript capture state.
35. Click `Clear Finished Rows` when old exited tasks and closed terminals are
    crowding the sidebar.
36. Click `Install Agent Repair Demo`, then run `Fix Current Wanix Program` on
    `/agent/broken.js`.
37. Watch the `Agent` section list the repair loop: read, run, observe, edit,
    rerun, capture transcripts, and verify `agent/out/result.txt`.
38. Click an Agent row with an artifact to reopen the source, transcript, or
    result file from the same system panel.
39. Click `diff broken.js` to reopen the before/after repair diff from
    `/agent/out`.

That is a much better base to build on. It makes the Rust Wanix port feel less like a bag of impressive subsystems and more like a small operating environment you can poke at from the browser.

## Good Next Papercuts

The next round should probably focus on making the workbench less demo-only:

- Make shell-created files refresh more precisely than "refresh the root after activity."
- Replace the deterministic repair backend with Codex app-server behind the
  same Wanix-shaped command contract.

The important thing is that these can now be incremental. The browser loop is alive.
