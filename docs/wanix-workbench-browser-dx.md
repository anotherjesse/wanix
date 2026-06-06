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

## The Feeling Now

The current loop is:

1. Start `wanix-rust serve`.
2. Open the printed URL.
3. Edit or create a file in `wanix:/`.
4. Run it as a qjs task from the play button.
5. See output and exit status in the terminal.
6. Run again without terminal clutter.

That is a much better base to build on. It makes the Rust Wanix port feel less like a bag of impressive subsystems and more like a small operating environment you can poke at from the browser.

## Good Next Papercuts

The next round should probably focus on making the workbench less demo-only:

- Add a visible `New JS` or `New qjs script` command that creates a starter file and opens it.
- Add a `.wasm` run path now that discovery advertises the wasm driver.
- Surface task status in a small workbench panel instead of relying only on terminal output.
- Make shell-created files refresh more precisely than "refresh the root after activity."
- Add a tiny welcome state when the root is empty, focused on actions rather than marketing copy.

The important thing is that these can now be incremental. The browser loop is alive.
