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

That sidebar now checks the service files too. A conservative browser poll reads
`#task` and `#term` directory state, then reads only safe task metadata files
like `kind`, `cmd`, and `exit`. Rows confirmed from the service surface are
marked with `· #task` or `· #term`, so a reload or shell startup can still show
live Wanix objects instead of only extension-remembered events.

![Service-backed task row in the Wanix sidebar](assets/wanix-workbench-browser-dx/47-service-backed-task-sidebar.png)

The served shell now identifies itself through the qjs-shell websocket contract.
Before terminal bytes arrive, Rust sends a `wanix-qjs-shell.v1` session frame
with the shell task id, terminal id, and cwd. The workbench records that as the
real `#task` and `#term` objects, so the System Journal can show `1 shell -
running` and `1 Shell - attached` without scraping `shell task: 1` out of the
prompt text.

![Shell session metadata in the System Journal](assets/wanix-workbench-browser-dx/66-shell-session-metadata.png)

The same qjs-shell contract now reports confirmed filesystem mutations. After
completed shell input, Rust compares the served root against its last snapshot
and sends a `mutation` frame containing the changed Wanix paths. The workbench
uses that frame to refresh Explorer and record Activity, so a shell write shows
up as `shell changed /made.txt` because the server observed the change, not
because the browser guessed from the typed command.

![Confirmed shell mutation in the System Journal](assets/wanix-workbench-browser-dx/67-shell-mutation-event.png)

The next slice made those mutation frames more expressive. They still carry the
changed `paths`, but now also include an `operations` array with fields such as
`kind`, `target`, and `status`. That lets the browser show `shell write
/opmade.txt` from server-authored metadata instead of collapsing everything into
`shell changed ...`.

![Shell operation metadata in the System Journal](assets/wanix-workbench-browser-dx/68-shell-operation-metadata.png)

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

That loop is now live instead of manual. Opening the v86 shared-files demo arms
a `/shared` watcher in the System view. When a Linux guest writes
`/shared/from-linux.txt` through the shared 9P filesystem, the workbench refreshes
the changed path and records it in Activity, so the browser side becomes a
dashboard for guest-visible filesystem traffic instead of a page you remember to
refresh. Activity now sits directly under Actions, because the newest feedback
from the system should be visible before routes, tasks, terminals, and namespace
details.

![Wanix Activity near Actions](assets/wanix-workbench-browser-dx/49-activity-near-actions.png)

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

The app surface is discoverable now too. `Open HTTP App Catalog` scans the real
Wanix `/apps` directory for `.js` and `.wasm` handlers, publishes concrete
route rows such as `/.wanix/app/custom`, and writes
`/.wanix/http-apps.md` plus `/.wanix/http-apps.json` with schema
`wanix.http-apps.v1`. Click a discovered app row and it runs through the same
preview path as the demos, saving `/apps/<name>.response.txt` and recording the
route run in the System view.

![Wanix HTTP app catalog](assets/wanix-workbench-browser-dx/63-http-app-catalog.png)

The next pass made creation first-class too. `New HTTP App` chooses a free
`/apps/app.js`-style path, writes a tiny qjs handler, opens the source, and
publishes the HTTP app catalog immediately. The new route row is already live:
click `/.wanix/app/app` and the workbench runs it as a Wanix HTTP task, opens
`/apps/app.response.txt`, and leaves the route run plus task trace in the
sidebar.

![New Wanix HTTP app in the catalog](assets/wanix-workbench-browser-dx/64-new-http-app.png)

The follow-up made the edit loop feel less ceremonial. When a handler under
`/apps` is active, `Preview Current HTTP App` saves the editor, resolves the app
name from `/apps/<name>.js` or `/apps/<name>.wasm`, runs the matching
`/.wanix/app/<name>` route, and opens the response report. If that response
report is already active, the same command can infer the source handler and run
it again.

![Preview current HTTP app response report](assets/wanix-workbench-browser-dx/65-preview-current-http-app.png)

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

The route run now carries the state file as an artifact too. Expand the counter
run and the same system object exposes `Response Report`, `Handler Source`,
`State File`, and `URL`. That makes the mutation inspectable from the execution
row instead of asking the user to remember which file changed.

![HTTP counter state artifact](assets/wanix-workbench-browser-dx/39-http-counter-state-artifact.png)

That state file is now a first-class data store object. `Open Data Store Index`
discovers existing app state such as `/apps/counter.count.txt`, publishes it in
the sidebar under `Data Stores`, and writes both `/.wanix/data-stores.md` and
`/.wanix/data-stores.json`. The JSON sidecar uses the explicit
`wanix.data-stores.v1` schema, so a future agent or HTTP-program manager can
ask "what mutable app state exists here?" without scraping route-run history.

![Wanix data store index](assets/wanix-workbench-browser-dx/62-data-store-index.png)

The route run also exposes the task traces that the Rust HTTP app runner writes
under `/.wanix/http`. The local route response carries the task id plus stdout
and stderr paths, the response report records them, and the `Route Runs` tree
hangs `Task Stdout` and `Task Stderr` under the exact HTTP execution. That makes
debugging a qjs-backed route feel like inspecting a process, not spelunking a
server log.

![HTTP route task trace artifacts](assets/wanix-workbench-browser-dx/40-http-route-trace-artifacts.png)

HTTP app routes are no longer only qjs-shaped. The Rust route runner now looks
for `apps/<name>.js` first and then `apps/<name>.wasm`, starts the matching
Wanix task kind, binds stdout/stderr to the same `/.wanix/http` trace files, and
returns stdout as the response. The workbench has a `Preview HTTP WASM Demo`
action that installs a tiny WASI module at `/apps/wasm.wasm`, calls
`/.wanix/app/wasm?from=workbench`, and opens the response report. In the
sidebar, that same request appears as both a route run and a `wasm` task.

![HTTP WASM route run](assets/wanix-workbench-browser-dx/50-http-wasm-route-run.png)

Using that screenshot exposed another small annoyance: the run was useful, but
buried below every other live-system section. The cockpit now puts `Routes` and
`Route Runs` directly under `Actions`, and tree rows have stable IDs plus
tooltips that include their full backing paths. After previewing a route, the
system object and the execution log are back in view with much less hunting.

![Route runs moved near the top](assets/wanix-workbench-browser-dx/41-route-runs-near-top.png)

The sidebar now stays there after the preview too. `Preview HTTP App Demo` and
`Preview HTTP Counter Demo` still open the saved response report in the editor,
but the left pane returns to Wanix System instead of Explorer, so the new route
run is visible at the same time as the response body.

![HTTP preview keeps Wanix focus](assets/wanix-workbench-browser-dx/42-http-preview-keeps-wanix-focus.png)

The execution labels are concrete now as well. The `Route Runs` list says
`/.wanix/app/hello` and `/.wanix/app/counter`, not only the template
`/.wanix/app/<name>`, so mixed route runs read like a log of what actually ran.

Shell filesystem activity got the same treatment first through a browser-side
parser: common mutating commands like `write`, `mkdir`, `rm`, `mv`, `cp`,
symbolic-link creation, and stdout/stderr redirection refreshed the touched path
and its parent instead of only refreshing the root. That made shell writes
visible, but it was still a guess made from terminal input.

![Shell write creates a concrete activity row](assets/wanix-workbench-browser-dx/43-shell-write-activity.png)

The served qjs-shell path now uses confirmed mutation frames instead. A command
like `write made.txt confirmed-event` creates `/made.txt`, Rust observes the
changed path, and the browser records `shell changed /made.txt` in the System
Journal.

![Confirmed shell mutation in the System Journal](assets/wanix-workbench-browser-dx/67-shell-mutation-event.png)

That frame now includes operation metadata too. The same server-observed
mutation carries a `write` operation with `status: changed` and a concrete
target path. In the journal that means the activity row becomes `shell write
/opmade.txt`, which is specific enough to read as an action, not merely a file
diff.

![Shell operation metadata in the System Journal](assets/wanix-workbench-browser-dx/68-shell-operation-metadata.png)

The shell now reports useful misses too. A failed command like
`rm absent.txt` does not mutate the served root, so Rust sends a mutation frame
with `paths: []` and an operation marked `status: unchanged`. The browser can
record `shell rm /absent.txt no change` without pretending a file changed, and
the frame still points at the attempted target for debugging.

![Shell no-change operation frame](assets/wanix-workbench-browser-dx/70-shell-unchanged-operation.png)

The newest pass threads the observed shell error into that same operation. A
failed `rm definitely-missing.txt` now carries `diagnostic:
rm: definitely-missing.txt: errno -44`, and the Activity row shows that detail
without making the main label noisy. That matters because "no change" is not
the same as "nothing happened": the attempted operation is still system
activity, and now it has a reason attached.

![Shell no-change diagnostic in Activity](assets/wanix-workbench-browser-dx/71-shell-unchanged-diagnostic.png)

That error reason now has a stable shape too. Each operation carries the
original shell `command` and an `outcome` object, for example
`{"status":"error","changed":false,"diagnostic":"rm: definitely-missing.txt: errno -44"}`.
The browser still keeps the row readable, but tools and future agents no
longer need to infer "failed command" from a free-form activity description.

![Shell structured command outcome](assets/wanix-workbench-browser-dx/72-shell-structured-outcome.png)

The outcome now carries command evidence as well. Rust strips the command echo
and prompt from the terminal bytes, keeps a compact `terminalOutput` snippet,
and extracts `exitCode` when the shell reports a foreground qjs child exit.
That lets a redirected qjs task say both things that are true: stdout/stderr
files changed, and the command exited 6.

![Shell qjs exit outcome](assets/wanix-workbench-browser-dx/73-shell-exit-outcome.png)

The newest shell slice moves from terminal-output heuristics to shell-authored
command records. Served browser qjs-shell sessions opt into a hidden
`wanix.qjs-shell.command.v1` record after each completed command. Rust strips
that record before forwarding terminal bytes, matches it back to the operation
with the same command text, and publishes `evidence:
qjs-shell-command-record` in the mutation frame.

That also fixes a mixed-command edge case: a batch can now report changed
operations and failed no-change operations in the same mutation message. In the
browser, the failed Activity row stays readable, but its description says the
result was `recorded by shell`.

![Shell command-record evidence](assets/wanix-workbench-browser-dx/74-shell-command-record-evidence.png)

The record is durable now too. Each served qjs-shell command writes
`/.wanix/qjs-shell/commands.jsonl`, `/.wanix/qjs-shell/latest.json`, and
`/.wanix/qjs-shell/latest.md`. Mutation frames carry those paths as
`historyPaths`, the System view has an `Open Shell Command History` action, and
the report inventory includes the shell history once it exists.

That matters for agents: they can inspect recent shell outcomes without
subscribing to terminal bytes or scraping Activity labels. The terminal remains
a terminal, while the command history becomes a Wanix artifact.

![Shell command history artifact](assets/wanix-workbench-browser-dx/75-shell-command-history-artifact.png)

The history is searchable from the cockpit now. `Search Shell Command History`
reads `commands.jsonl`, opens a filterable picker over recent commands, and
writes the chosen record to `/.wanix/qjs-shell/selected.md` so the exact command,
outcome, terminal snippet, operation, and raw JSON are visible in the editor.
There is also a guarded `Clear Shell Command History` action for long-lived
browser sessions where the generated history should start fresh.

![Shell command history search controls](assets/wanix-workbench-browser-dx/76-shell-history-search-controls.png)

Long-lived sessions now get a grouped summary too. `Open Shell History Summary`
generates `/.wanix/qjs-shell/summary.md` from the append log, grouping commands
by outcome, changed status, hour, cwd, terminal id, and task id. It is the
human-scale view: use search when you need one command, and summary when you
need to understand the shape of the whole session.

![Shell command history summary](assets/wanix-workbench-browser-dx/77-shell-history-summary.png)

The summary rows are navigable now. Regenerating the summary writes one evidence
file per command under `/.wanix/qjs-shell/commands/`, and the summary links each
command label to its evidence file while linking touched paths back to
`wanix:/...`. That gives humans and agents a stable way to jump from a grouped
session view to the exact raw JSON, terminal snippet, operation metadata, and
file path behind one command.

![Shell command history linked evidence](assets/wanix-workbench-browser-dx/78-shell-history-linked-evidence.png)

The history can be compacted from the cockpit too. `Compact Shell Command
History` offers count and age retention choices, confirms before it removes
records, rewrites `commands.jsonl`, refreshes `latest.md` and `summary.md`,
relinks `commands/*.md`, and drops stale `selected.md` evidence. That turns a
long-lived browser session from "clear everything or live with the pile" into a
deliberate retention workflow.

![Shell command history compaction controls](assets/wanix-workbench-browser-dx/79-shell-history-compaction-controls.png)

That row is not just a log line anymore. Activity entries can now carry their
Wanix path, show it as row context, and open it directly. Click
`shell write jumpable` and the workbench jumps to `/jumpable`, with the shell
command, activity trail, and file contents all visible in one cockpit view.

![Activity row opens the changed Wanix path](assets/wanix-workbench-browser-dx/44-activity-row-opens-path.png)

Multi-path shell activity now has a better shape too. A command like
`mv movefrom moveto` expands into the source path and the resulting target path.
The target child is marked `open target` and opens the moved file, while the
source remains visible as the touched path. That keeps history honest without
making the user click a path that no longer exists.

![Moved-file activity exposes source and target paths](assets/wanix-workbench-browser-dx/45-activity-mv-target-child.png)

Browser file operations now feed the same activity stream. When the workbench's
`wanix:` file provider creates a directory, writes a file, copies, renames, or
deletes through editor/Explorer UI, the System view records the mutation with a
clickable Wanix path. In this proof, the editor's `Create File` path created
`/apps` and saved `/apps/counter.response.txt`, and both operations appeared as
Activity rows without using the shell.

![Browser file provider writes appear as Activity](assets/wanix-workbench-browser-dx/46-browser-provider-save-activity.png)

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

The next pass turns the repair trace into a durable Wanix object. After a
repair, the agent writes `/agent/out/broken.repair-report.md` with the target,
backend, operation list, task ids, transcript paths, before/after snapshots,
result file, and status. The System actions now include `Fix Current Wanix
Program`, so install and repair both live in the cockpit instead of depending on
the editor title bar alone.

![Wanix agent repair report](assets/wanix-workbench-browser-dx/51-agent-repair-report.png)

The report now has a machine-readable sibling too:
`/agent/out/broken.repair-report.json`. It uses schema
`wanix.agent-repair.v1` and records the same repair as structured data:
backend, contract, target, result, ordered operations, task ids, transcript
paths, metadata paths, snapshots, and artifacts. That gives a future Codex
app-server backend a concrete run record to produce and inspect instead of
asking it to scrape Markdown.

![Structured Wanix agent repair JSON](assets/wanix-workbench-browser-dx/69-agent-repair-json-report.png)

The demo path is now one click. `Run Agent Repair Demo` installs the broken
program and immediately repairs `/agent/broken.js` as an explicit target, so an
empty workbench can show the whole agent loop without relying on the active
editor. `Fix Current Wanix Program` stays next to it for the more general case:
repair the qjs file the user is already looking at.

![One-click Wanix agent repair demo](assets/wanix-workbench-browser-dx/52-one-click-agent-repair.png)

The agent tool surface is now a Wanix artifact too. `Open Agent Tool Contract`
writes `/.wanix/agent-tools.md` and `/.wanix/agent-tools.json`, opens the
markdown contract, and records the write in both the Agent section and Activity.
The JSON uses the explicit `wanix.agent-tools.v1` schema and names the current
repair backend, service roots, policies, repair-demo loop, and tools such as
`readFile`, `writeFile`, `runTask`, `observeTask`, `previewHttpRoute`, and
`writeRepairReport`. This is the shape a Codex app-server engine can attach to
without changing the Wanix-facing contract.

![Wanix agent tool contract](assets/wanix-workbench-browser-dx/57-agent-tool-contract.png)

The cockpit now has a capstone button too. `Run OS Cockpit Tour` runs the story
as one visible system operation: it seeds the v86 shared-files workspace, runs
the qjs -> WASM -> qjs duet, previews a stateful qjs HTTP route, previews a
WASM HTTP route, runs the agent repair, and writes
`/.wanix/cockpit-tour.md`. The report is not a marketing page; it is a Wanix
file that points at the exact artifacts the tour created.

![Wanix OS cockpit tour report](assets/wanix-workbench-browser-dx/53-os-cockpit-tour.png)

The tour is live in the sidebar now too. The `Tour` section resets when a tour
starts, shows each step as running/ok/failed, and expands into the Wanix files
that step created or used. That means the capstone demo is inspectable while it
runs, not just after the report opens.

![Live Wanix tour section](assets/wanix-workbench-browser-dx/54-live-tour-section.png)

The system panel can write its own state now as well. `Open System Journal`
generates `/.wanix/system-journal.md` for people and
`/.wanix/system-state.json` for agents/tools, refreshes the namespace, opens the
markdown file, and records both writes in Activity. It captures drivers,
namespace roots, observed tasks, terminals, routes, route runs, tour state,
agent steps, and recent activity. That gives people and future agents normal
Wanix artifacts to read instead of scraping transient UI state.

![Wanix system journal](assets/wanix-workbench-browser-dx/55-system-journal.png)

The JSON sidecar uses the explicit `wanix.system-state.v1` schema. It carries
the same cockpit model as structured data: driver names, namespace entries,
task status, service paths, route previews, route-run artifacts, tour steps,
agent steps, and recent activity. The Activity rows make both files reopenable
from the sidebar.

![Wanix system state JSON](assets/wanix-workbench-browser-dx/56-system-state-json.png)

## Why These Fixes Matter

The browser workbench is interesting because it makes the Rust port tangible. The runtime is no longer hidden behind CLI demos. You can browse a Wanix namespace, edit files, run qjs tasks, and watch the task output in one place.

But the first browser pass had several small breaks in the loop:

- The workbench assets were served from the user root, so disposable roots could fail to boot Code OSS at all.
- Files created from the shell did not show up in Explorer until a manual reload.
- Shell-triggered filesystem refreshes were generic, so the sidebar could say
  something happened but not which path changed.
- Failed or no-op shell mutations disappeared from the activity model, so the
  cockpit could not say "the user tried this and nothing changed."
- Browser-side file provider writes were not visible as system activity, so an
  editor or Explorer mutation could feel disconnected from the cockpit.
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
- The repair report was human-readable, but did not yet have a structured JSON
  run record for agents or tooling.
- HTTP route previews updated route status, but the execution itself was not a
  first-class row in the system panel.
- HTTP route previews could prove a qjs handler returned text, but not yet that
  a browser-triggered app could mutate Wanix state.
- Stateful route files were inspectable only from the route execution that
  happened to create them, not from a durable data-store inventory.
- HTTP apps were still mostly demo-command-shaped; a user-created
  `/apps/custom.js` did not become a concrete cockpit route until somebody knew
  what command to run.
- Creating a fresh HTTP app still required knowing the `/apps/<name>.js`
  convention before the cockpit could help.
- After creation, rerunning the app still meant finding the route row or a demo
  button instead of previewing the handler already open in the editor.
- The complete OS story existed, but only if the user knew which demo buttons
  to press and in what order.

None of those are huge by themselves. Together, they make the browser experience feel like a prototype you have to babysit. The point of this pass was to remove enough of that babysitting that the system starts to feel direct.

## What Changed

The first fix was asset routing. `workbench-fs9p` now serves `/workbench/...` from the repo's workbench assets instead of looking inside the served user root. That means you can serve a disposable filesystem root and still get a real workbench.

Then the Explorer learned to refresh after terminal activity and task exits. If a shell command writes a file, the browser UI catches up without a page reload.

That refresh became path-targeted for common shell mutations. The direct
terminal input path still tracks the shell's current working directory,
recognizes simple built-ins and redirections, refreshes the touched Wanix paths
plus their parents, and logs a concrete Activity label such as `shell write
shellmade`. Unknown newline commands still fall back to the older broad
refresh.

The qjs-shell websocket path has moved past that guesswork. Rust now advertises
a `wanix-qjs-shell.v1` `mutation` message, snapshots the served root, and sends
changed Wanix paths after completed shell input. The browser consumes those
server-observed paths for refreshes and Activity, which is why the fresh
verification journal says `shell changed /made.txt`.

The frame now includes operation metadata as well, so the browser can prefer a
server-authored label like `shell write /opmade.txt` when the completed command
and changed path line up.

Activity records now preserve that path context too. When a filesystem event
has a path, the row becomes an `Open Wanix Path` target; multi-path events keep
their touched paths as row children, target children open the resulting object,
and destructive commands open the nearest stable parent. That makes Activity a
navigation surface instead of only a timestamp-free feed.

The workbench file provider now reports its own mutations into that same model.
Editor saves and Explorer creates, copies, renames, deletes, and directory
creates emit bridge-level events. The extension formats those into rows like
`saved counter.response.txt` or `created directory apps`, with paths attached
for direct navigation.

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
The sidebar also polls those service directories carefully. It does not touch
allocator, control, or stream files; it only reads task directory names and safe
metadata fields, then annotates confirmed rows with `· #task` or `· #term`.
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
and narrow: `/.wanix/app/<name>` maps to `apps/<name>.js` or
`apps/<name>.wasm`, requires
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

The app catalog makes that route model file-driven. Instead of only showing the
advertised template route and the canned demo buttons, the workbench can scan
`/apps`, publish `wanix.http-apps.v1` markdown/JSON, and add concrete route
rows for the handlers it found. The same row can then run the app, open the
response report, and leave a route-run/task trace.

The new-app command closes the other side of that loop. The user can create a
handler from the cockpit, get the source opened, get the catalog refreshed, and
run the resulting route row without ever typing the route path by hand.

The current-app preview closes the editor loop. When `/apps/app.js` is open,
the user can click one command to save, run `/.wanix/app/app`, open
`/apps/app.response.txt`, refresh the catalog's latest-preview pointer, and
leave another task trace. Running it again from the response report works too.

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

The cleanup pass made route-run artifacts generic. Counter state is the first
use, but the tree can now hang other run outputs under the same execution row
without adding another route-specific sidebar path.

The data-store index applies that object-model rule to mutable app state. If a
route writes durable state under the Wanix namespace, the cockpit can publish
it as a data-store row and as markdown/JSON artifacts under `/.wanix`, instead
of leaving it as an incidental file someone has to rediscover from a prior run.

HTTP app responses now carry local Wanix trace headers for the task id, stdout
trace, and stderr trace. The workbench records those paths in the saved response
report and exposes the trace files under the route run, so failures have a
natural place to attach process output.

The System view order now keeps route work close to the actions that create it:
`Routes` and `Route Runs` sit near the top instead of below every driver, task,
terminal, and namespace row. Stable tree item IDs and explicit tooltips also
make refreshes and truncated rows less slippery.

HTTP preview commands now restore focus to the Wanix System view after opening
the response report. That keeps the editor output and the live execution log on
screen together instead of sending the user back through the Explorer tab.

Route-run labels now use the concrete app path too, so the log distinguishes
`/.wanix/app/hello` from `/.wanix/app/counter` at a glance.

Shell activity now follows the same concrete-label rule. A file created through
the terminal is no longer only a filesystem refresh; it is a named mutation in
the sidebar, with enough path information to refresh Explorer precisely.

Activity rows now use that same path information for navigation. The row itself
opens the changed file or stable parent, so a shell write can take you from the
history line back to the Wanix object it changed.

For multi-path operations, Activity expands into a tiny path trace. The source
path stays visible as context, while the target path becomes the openable row.

Browser file-provider actions now join the stream too. A file created or saved
from the editor is visible beside shell and task activity instead of being an
invisible editor-only operation.

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

Service-backed task rows are the same idea applied to reloads and shell startup.
When the browser can see a task through `#task`, the row says so. The cockpit is
not merely remembering that it launched something; it is checking Wanix's own
object table.

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

The repair report makes that loop durable. It is useful today for humans
reviewing what happened, and it gives the future Codex app-server backend a
stable artifact shape to fill in without changing the browser story.
The structured JSON sidecar makes that artifact shape explicit:
`wanix.agent-repair.v1` records the repair contract, operations, task ids,
transcripts, metadata, snapshots, result, and status as data.

The one-click action matters for demos because it removes the last bit of
coordination: no command palette, no title-bar hunting, no active-editor
assumption. The system view can bootstrap the broken program, operate it, and
leave the report behind.
The agent tool contract makes that operation boundary explicit before a repair
runs, so an external engine can use the same Wanix-shaped tools instead of
learning behavior from UI affordances.

The cockpit tour does the same thing for the whole sprint arc. It makes the
system prove itself in one pass, then leaves a report that tells people where to
inspect the actual files, task transcripts, route traces, and agent artifacts.
The live Tour section keeps those same steps visible in the system panel, with
artifact children that open directly into the Wanix namespace.
The system journal and JSON sidecar make the current cockpit state inspectable
through the same file model, which is the shape agents can build on later.
The shell session frame makes that snapshot more trustworthy: task id, terminal
id, and cwd now come from a served `wanix-qjs-shell.v1` message before terminal
output, not from prompt text scraping.
The shell mutation frame extends that trust boundary from startup metadata to
filesystem change notification: the browser no longer needs to parse qjs-shell
commands to know a served-root path actually changed.
The operation metadata makes the visible activity less generic without giving
that trust boundary back to the browser.
The no-change operation frame closes the other side of that loop: the browser
can show a failed or unchanged shell action as explicit system activity while
keeping `paths` reserved for files Rust actually observed changing.
The diagnostic field keeps that explicit miss useful for people and agents:
the cockpit can record why the operation did not change the namespace without
loosening the meaning of `paths`.
The command outcome object gives that diagnostic a durable shape: `changed`
means observed filesystem effect, `status` means command outcome, and
`diagnostic` remains the human-facing reason when the shell knows one.
The newest pass also includes `terminalOutput` and qjs child `exitCode`, so a
future agent can see the command evidence that produced an Activity row without
parsing terminal scrollback.
The command-record pass closes the last obvious gap in that chain: served
browser qjs-shell sessions emit a hidden structured command record, Rust strips
it from the visible terminal stream, matches it by command text, and marks the
operation with `evidence: qjs-shell-command-record`. Mixed batches can now show
both changed operations and failed no-change operations in one frame.
The follow-up pass persists the same command records under
`/.wanix/qjs-shell`, advertises the paths in discovery and mutation frames, and
adds the workbench action/report hook that opens the latest Markdown view.

The newest slice gives the cockpit a way to diagnose itself before anyone asks
an agent to build on top of it. `Run Cockpit Self Check` verifies the advertised
qjs/wasm drivers, `#task` and `#term` service roots, shell route, HTTP app
route, direct-v86 discovery, and the ability to write/read under `/.wanix`. It
then writes `/.wanix/cockpit-check.md`, `/.wanix/cockpit-check.json`, and a
small `/.wanix/checks/probe.txt` file.

Warnings are not hidden as failures. If the session has not published
`/.wanix/system-state.json` or the agent tool contract yet, the report says so
and points at the action that creates them. That makes the browser cockpit feel
less like a demo launcher and more like a tiny operator console: it can tell you
what is ready, what is optional, and what artifact to inspect next.

![Wanix cockpit self-check](assets/wanix-workbench-browser-dx/58-cockpit-self-check.png)

That immediately exposed the next small paper cut: a good diagnostic is useful,
but a diagnostic that points at two setup actions still makes the operator do
coordination work. The System view now has `Prepare Cockpit Reports`. It writes
the agent tool contract, writes the system journal and JSON state sidecar, runs
the cockpit self-check, and then writes the system snapshot again so the final
state file includes the check result.

On a fresh served root, that turns the missing report artifacts from warnings
into ok checks. The remaining warning in this proof is the real environment
state: direct-v86 is advertised, but this root has not been prepared with
`/boot/bzImage` and executable `/bin/init`. That is a better warning. It is not
"you forgot to run another cockpit command"; it is "this VM boot root is not
ready yet."

![Wanix cockpit prepare reports](assets/wanix-workbench-browser-dx/59-cockpit-prepare-reports.png)

The report inventory now has its own cockpit object model too. Preparing the
cockpit writes `/.wanix/cockpit-reports.md` and
`/.wanix/cockpit-reports.json`, then publishes a `Reports` section in the
System view. That section indexes the report manifest, self-check, system
journal, and agent tool contract as openable Wanix objects. The JSON sidecar
uses `wanix.cockpit-reports.v1`, so an agent does not have to scrape the UI or
guess which `.wanix` files matter after a preparation pass.

This is the same principle as the task and route rows: if the cockpit creates
an important artifact, that artifact becomes part of the system model. Reports
are not just files in a hidden folder; they are inspectable OS objects.

![Wanix cockpit report inventory](assets/wanix-workbench-browser-dx/60-cockpit-report-inventory.png)

The inventory is live now, not just a static bundle from the preparation pass.
After another operation writes a report, `Open Report Inventory` republishes
`/.wanix/cockpit-reports.md` and `/.wanix/cockpit-reports.json` from the
current Reports model. In this proof, I prepared the cockpit, ran the one-click
agent repair demo, then refreshed the inventory. The markdown report gained an
`agent` section with `/agent/out/broken.repair-report.md`, and the JSON sidecar
listed all five current report rows plus the repair artifacts.

![Live Wanix report inventory after agent repair](assets/wanix-workbench-browser-dx/61-live-report-inventory.png)

## The Feeling Now

The current loop is:

1. Start `wanix-rust serve`.
2. Open the printed URL.
3. Click `Run OS Cockpit Tour` when you want the whole story at once: v86
   shared files, JS/WASM duet, qjs HTTP route, WASM HTTP route, agent repair,
   and `/.wanix/cockpit-tour.md`.
4. Expand `Tour` to inspect each completed step and open the exact artifacts it
   created.
5. Click `Open System Journal` to write `/.wanix/system-journal.md` and
   `/.wanix/system-state.json` from the live sidebar state.
6. Confirm the shell row has a real `#task` id and the terminal row has a real
   `#term` id from the structured qjs-shell session frame.
7. Click `Run Cockpit Self Check` to verify drivers, service roots, routes, and
   report storage, then inspect `/.wanix/cockpit-check.md` or
   `/.wanix/cockpit-check.json`.
8. Click `Prepare Cockpit Reports` to publish the agent tool contract, system
   journal, system-state JSON, self-check report, and probe file in one pass.
9. Expand `Reports` or open `/.wanix/cockpit-reports.md` to inspect the report
   manifest, self-check, system journal, and agent tool contract from one
   inventory.
10. After later runs such as `Run Agent Repair Demo`, click
   `Open Report Inventory` to refresh `/.wanix/cockpit-reports.md` and
   `/.wanix/cockpit-reports.json` from the live Reports rows.
11. Edit or create a file in `wanix:/`.
12. Create a file from the served qjs-shell, for example
   `write made.txt confirmed-event`, and see Explorer plus the System Journal
   update from the confirmed `mutation` frame for `/made.txt`.
13. Run another served-shell write and see the row use operation metadata, such
    as `shell write /opmade.txt`, rather than a generic changed-path label.
14. Try `rm absent.txt` in the served qjs-shell and see the no-change operation
    frame report `status: unchanged` with `paths: []` and a `diagnostic`, so the
    cockpit can say the attempted mutation did not change the namespace and why.
    The same operation now includes the original `command` and structured
    `outcome`, so future tools can consume it without scraping the Activity row.
15. Run a redirected qjs child that exits nonzero and see the operation keep
    `changed: true` for the redirected files while reporting `outcome.status:
    error`, `exitCode`, and a compact `terminalOutput` snippet.
16. Run a mixed shell batch like `write browser-evidence.txt ok` followed by
    `rm browser-missing.txt` and see changed and failed no-change operations
    reported together. The failed row says `recorded by shell` because its
    outcome came from the shell's hidden command record, not browser parsing.
    Then click `Open Shell Command History` to inspect the durable
    `/.wanix/qjs-shell/latest.md` artifact produced from those same records, or
    use `Search Shell Command History` to filter the append log and open a
    focused `/.wanix/qjs-shell/selected.md` selection artifact.
17. Click `Open Shell History Summary` to generate the grouped
    `/.wanix/qjs-shell/summary.md` session view. Command rows link to
    `/.wanix/qjs-shell/commands/<n>.md`, and touched paths link back to
    `wanix:/...`. Then use `Compact Shell Command History` to keep the latest
    count or age window and regenerate the retained evidence set.
18. Click a changed Activity row to reopen the Wanix file it touched.
19. Move a file from the shell, expand the `shell mv ...` Activity row, and open
   the `open target` child.
20. Create or save a `wanix:` file from the browser editor/Explorer and see the
   file-provider mutation appear in Activity.
21. Use the System view's `Actions` rows when you want visible commands instead
   of toolbar icons.
22. Run a qjs task from the play button or `New qjs Script` action.
23. Run a `.wasm` module from Explorer.
24. Watch tasks, terminals, exits, and filesystem refreshes in the Wanix sidebar;
    rows marked `· #task` have been confirmed from Wanix service files.
25. Click `#task` or `#term` in the sidebar to inspect real service directories.
26. Right-click a task row and inspect its `#task/<id>` service directory.
27. Follow a safe service entry like `#task/1/kind` into the real Wanix file.
28. See why allocator/control/stream service files stay plain text.
29. See output and exit status in the terminal.
30. Run again without terminal clutter.
31. Click `Install WASM Starter`, then `Run WASM Starter`, and inspect
    `/wasm/shared/out.txt`.
32. Install the duet demo and run qjs -> wasm -> qjs as one visible workflow.
33. Or click `Run JS and WASM Duet Demo` and let the workbench drive the
    sequence.
34. Inspect the generated `duet/shared/out.txt` result when the guided run
    opens it.
35. Edit `duet/producer.js`, run the guided duet without losing the edit, then
    explicitly reset the duet when you want the starter files back.
36. Click `Open v86 Shared Files Demo` to seed `/shared/message.txt`, arm the
    `/shared` watcher, and read the direct-v86 launch and 9P mount instructions.
37. Open the `direct-v86` route when a prepared rootfs is available, then have
    Linux read `/shared/message.txt` and write `/shared/from-linux.txt`; the
    changed file appears in Activity without a manual Explorer refresh.
38. Click `Install HTTP App Demo` to create and open `apps/hello.js`.
39. Open
    `/.wanix/app/hello?from=browser` to get an HTTP response from a Wanix task.
40. See the HTTP app route contract in the Wanix sidebar.
41. Create or edit a handler under `/apps`, then click `Open HTTP App Catalog`
    to publish `/.wanix/http-apps.md` and `/.wanix/http-apps.json`.
42. Or click `New HTTP App` to create `/apps/app.js`, open the source, and
    publish the catalog in one step.
43. Click a discovered route row such as `/.wanix/app/custom` or
    `/.wanix/app/app` to run that handler and open its
    `/apps/<name>.response.txt` report.
44. With `/apps/app.js` active, click `Preview Current HTTP App` to save and
    run the current handler without finding its route row.
45. With `/apps/app.response.txt` active, click the same command again to rerun
    the source handler inferred from the response report.
46. Click `Preview HTTP App Demo` to fetch the route and open a status-bearing
    `/apps/hello.response.txt` report inside the workbench.
47. Click the `/.wanix/app/<name>` route row itself to run the same preview from
    the visible system contract.
48. Edit `/apps/hello.js`, preview again, and see the changed handler output
    without losing the edit or reopening the report.
49. Open the HTTP handler source directly, and reset the demo only through the
    explicit reset command.
50. Right-click the route row to preview, open source, copy the route URL, or
    intentionally reset the demo handler.
51. Click `Preview HTTP WASM Demo` to install `/apps/wasm.wasm`, run it through
    `/.wanix/app/wasm`, and inspect the route response plus wasm task trace.
52. After previewing, read the route row's latest HTTP status and reopen the
    saved response report from `Open Latest HTTP App Preview`.
53. Expand `Route Runs` to see the previewed HTTP route execution and reopen
    its response report or handler source.
54. Click `Preview HTTP Counter Demo` twice to run a stateful qjs-backed HTTP
    app, then inspect `apps/counter.response.txt` and `apps/counter.count.txt`.
55. Expand the counter route run and open the `State File` child directly from
    the execution row.
56. Click `Open Data Store Index` to publish `/.wanix/data-stores.md` and
    `/.wanix/data-stores.json`, then inspect `/apps/counter.count.txt` as a
    stateful Wanix data store.
57. Expand the same route run and inspect `Task Stdout` or `Task Stderr` from
    the exact HTTP execution that produced the response.
58. Notice that `Routes` and `Route Runs` stay near the top of the System view,
    and hover truncated rows to see their full backing paths.
59. Run either HTTP preview action again and see the response report open while
    the sidebar stays on Wanix System.
60. Click `New qjs Script` to create a unique runnable scratch script without
    leaving the workbench.
61. Run that script, then expand its task row to reopen the source, transcript,
    metadata, terminal output, or `#task/<id>` service directory.
62. Open the same task row's metadata JSON to inspect cwd, argv, env, source,
    output, exit, and transcript capture state.
63. Click `Clear Finished Rows` when old exited tasks and closed terminals are
    crowding the sidebar.
64. Click `Open Agent Tool Contract` to write `/.wanix/agent-tools.md` and
    `/.wanix/agent-tools.json`.
65. Click `Run Agent Repair Demo` to install `/agent/broken.js`, run it, repair
    it, rerun it, and open the generated report.
66. Watch the `Agent` section list the repair loop: read, run, observe, edit,
    rerun, capture transcripts, and verify `agent/out/result.txt`.
67. Click an Agent row with an artifact to reopen the source, transcript, or
    result file from the same system panel.
68. Click `diff broken.js` to reopen the before/after repair diff from
    `/agent/out`.
69. Click `write repair report` or open
    `/agent/out/broken.repair-report.md` to inspect the complete repair trace:
    operation list, task ids, transcript paths, snapshots, result, and status.
70. Open `/agent/out/broken.repair-report.json` to inspect the same repair as
    `wanix.agent-repair.v1`: structured operations, artifacts, result, and
    backend contract.
71. Use the System action row `Fix Current Wanix Program` when the current
    editor is a qjs file and you want to apply the same repair contract outside
    the canned demo.

That is a much better base to build on. It makes the Rust Wanix port feel less like a bag of impressive subsystems and more like a small operating environment you can poke at from the browser.

## Good Next Papercuts

The next round should probably focus on making the workbench less demo-only:

- Move the direct terminal path onto the same server-authored event model as
  qjs-shell, so all shell-like activity uses one mutation contract.
- Add an explicit shell-history export/archive action before compaction when a
  session needs an audit artifact outside the live `.wanix/qjs-shell` surface.
- Replace the deterministic repair backend with Codex app-server behind the
  same Wanix-shaped command contract, producing the same
  `wanix.agent-repair.v1` run record.

The important thing is that these can now be incremental. The browser loop is alive.
