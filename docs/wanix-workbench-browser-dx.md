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

The first compiled-WASM path is in the browser too. Right-click a `wanix:`
`.wasm` file in Explorer and choose `Run Current WASM as wasm Task`. The module
runs through `#task/new/wasm`, shares the same namespace as qjs and the shell,
and reports its lifecycle in the Wanix sidebar.

In this smoke test, `guest.wasm` reads `/shared/in.txt`, writes
`/shared/out.txt`, prints its Rust WASI output in the terminal, and exits 0.

![WASM task shown in the Wanix sidebar](assets/wanix-workbench-browser-dx/05-wasm-task-sidebar.png)

The next polish pass removed another bit of demo friction: you no longer need
to hand-copy a WASM fixture into the served root. The Wanix system view has an
`Install JS and WASM Duet Demo` action. It creates:

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
refreshes the sample files, runs qjs producer, runs the WASM transform, runs qjs
verify, and stops if a step exits nonzero. The result is a one-click proof that
the browser workbench can coordinate multiple Wanix runtimes through the same
task and filesystem surface.

![Guided JS and WASM duet run](assets/wanix-workbench-browser-dx/07-guided-duet-run.png)

One more click of polish: after the guided run succeeds, the workbench opens the
generated `duet/shared/out.txt` file. The user sees the actual namespace result,
not just terminal output or a toast.

![Guided duet output file](assets/wanix-workbench-browser-dx/08-guided-duet-output.png)

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

WASM then got the same ergonomic path as qjs. The extension now has a shared
task-runner path for qjs and wasm editor actions, so `.wasm` files can run from
Explorer without trying to open binary modules as text. This is the first
browser proof of the bigger claim: JS and compiled WASM are sibling runtimes on
one Wanix substrate.

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

The HTTP-app route is the first app-platform proof. It is deliberately local
and narrow: `/.wanix/app/<name>` maps to `apps/<name>.js`, requires
`--wanix-services`, rejects non-loopback clients, binds stdout and stderr to
`.wanix/http/<task>.out` and `.wanix/http/<task>.err`, and returns stdout as
`text/plain`. Discovery advertises the contract as `wanix-http-app.v1`.

The browser then got a matching installer command. That command uses the same
direct 9P filesystem surface as the rest of the workbench, so the handler file
is not a fake sample hidden in the extension. It is a normal Wanix file under
`/apps`, ready to edit and serve.

## The Feeling Now

The current loop is:

1. Start `wanix-rust serve`.
2. Open the printed URL.
3. Edit or create a file in `wanix:/`.
4. Run it as a qjs task from the play button.
5. Run a `.wasm` module from Explorer.
6. Watch tasks, terminals, exits, and filesystem refreshes in the Wanix sidebar.
7. See output and exit status in the terminal.
8. Run again without terminal clutter.
9. Install the duet demo and run qjs -> wasm -> qjs as one visible workflow.
10. Or click `Run JS and WASM Duet Demo` and let the workbench drive the
    sequence.
11. Inspect the generated `duet/shared/out.txt` result when the guided run
    opens it.
12. Click `Install HTTP App Demo` to create and open `apps/hello.js`.
13. Open
    `/.wanix/app/hello?from=browser` to get an HTTP response from a Wanix task.

That is a much better base to build on. It makes the Rust Wanix port feel less like a bag of impressive subsystems and more like a small operating environment you can poke at from the browser.

## Good Next Papercuts

The next round should probably focus on making the workbench less demo-only:

- Add a visible `New JS` or `New qjs script` command that creates a starter file and opens it.
- Make shell-created files refresh more precisely than "refresh the root after activity."
- Let the sidebar inspect service files directly, not just extension-observed events.
- Add a small run history or output link per task entry.
- Add a reset button for the duet demo's generated files.
- Add a split in-workbench HTTP preview that can render the app route without
  navigating away from the workbench.
- Add a tiny welcome state when the root is empty, focused on actions rather than marketing copy.

The important thing is that these can now be incremental. The browser loop is alive.
