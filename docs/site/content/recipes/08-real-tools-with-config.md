---
title: Recipe 08 — Your Own Programs as Mesh Tools (tools.toml)
slug: recipes/08-real-tools-with-config
pageType: recipe
oneLiner: Write a tools.toml that wraps real host programs (tr, sort, your own script), serve each as its own mesh tool, run jobs from the shell with the tool builtin, watch a job's events stream live with mount-cat --follow, then hit a real timeout and abort a real run.
audience: [developer]
tags: [tools, mesh, cli, shell, shipped, caveat]
sourceRefs:
  - crates/wanix-cli/src/tool/config.rs
  - crates/wanix-cli/src/tool/process.rs
  - crates/wanix-cli/src/tool/serve.rs
  - crates/wanix-jobfs/src/lifecycle.rs:150-178
  - crates/wanix-jobfs/src/files.rs:158-215
  - crates/wanix-sh/src/tool.rs:1-27
  - docs/toolfs.md
seeAlso:
  - recipes/06-compose-volume-and-tools
  - learn/compose-volumes-and-tools
  - concepts/jobs-are-files
  - concepts/key-is-the-address
  - concepts/blocking-stream-eof-contract
prerequisites:
  - recipes/06-compose-volume-and-tools
usedInFlows:
  - {flow: compose-volumes-and-tools, step: 5}
honestLimits:
  - "Tool job state lives only while tool serve runs; retention TTLs bound how long a finished job stays inspectable within one serve lifetime."
  - "tool serve admits any ticket holder (no allow-list yet); each connection is confined to its own private jobs/ view keyed by the verified peer id."
  - "visibility = \"private\" is the only accepted value; local naming shipped (--register NAME writes ~/.wanix/catalog entries, recipe 10) but shared catalogs/discovery for served tools remain ADR 0007 follow-up."
  - "The runner refuses nothing about what the wrapped program does on the host beyond env/cwd/caps: choosing safe commands is the operator's job — the config is the trust decision."
canonicalCaveatFor: []
---

# Recipe 08 — Your Own Programs as Mesh Tools (`tools.toml`)

Write a `tools.toml` that wraps real host programs (`tr`, `sort`, your own script), serve each as its own mesh tool, run jobs from the shell with the `tool` builtin, watch a job's `events` stream live with `mount-cat --follow`, then hit a real timeout and abort a real run.

**What & why.** Recipe 06 served the built-in demo runners. This is the real thing: `tool serve --config tools.toml` wraps *host programs you choose* behind the same job protocol — the ToolFS inversion (`docs/toolfs.md` §Runner Boundary): the operator fixes the executable, its argv, and the input/output mapping; a caller only ever supplies input bytes. No shell anywhere, empty child environment, private per-job temp cwd. Everything below ran verbatim on one machine over loopback (tickets shortened to a recognizable prefix; yours will differ).

## 0. Build the binary

```sh
cargo build --package wanix-cli
alias wanix-rust='./target/debug/wanix-rust'
```

## 1. The config is the trust decision

Four tools: two stock binaries, one program you wrote, one that will blow its own deadline. The optional `limits`/`lifecycle` tables map straight onto the job policy the tool advertises in `spec.json`; unset knobs keep the shipped defaults.

```sh
mkdir -p /tmp/tools-demo
cat > /tmp/tools-demo/summarize <<'EOF'
#!/bin/sh
# An operator-authored program: stdin -> stdout, progress on stderr.
echo "summarize: reading input" >&2
words=$(wc -w)
sleep 2
echo "summarize: counted" >&2
sleep 2
echo "summarize: writing summary" >&2
echo "$words words"
EOF
chmod +x /tmp/tools-demo/summarize

cat > /tmp/tools-demo/tools.toml <<'EOF'
# Each [tools.NAME] is one served tool: one absolute executable, a fixed
# argv, and explicit input/output mapping. No shell, no caller-chosen args.

[tools.rot13]
description = "rot13 text (stdin -> stdout)"
command = "/usr/bin/tr"
args = ["A-Za-z", "N-ZA-Mn-za-m"]

[tools.rot13.limits]
max_input_bytes = 1048576
run_timeout_ms = 5000

[tools.sortlines]
description = "Sort lines (tempfile in/out)"
command = "/usr/bin/sort"
args = ["-o", "{output}", "{input}"]
input = "tempfile"
output = "tempfile"

[tools.summarize]
description = "Slow word count with stderr progress"
command = "/tmp/tools-demo/summarize"

[tools.summarize.limits]
run_timeout_ms = 60000

[tools.slow]
description = "Sleeps longer than its own deadline"
command = "/bin/sleep"
args = ["10"]

[tools.slow.limits]
run_timeout_ms = 2000
EOF
```

`input`/`output` default to `stdin`/`stdout`; `tempfile` switches a side to a private workdir file whose absolute path replaces the `{input}`/`{output}` argv placeholder (required exactly once for each `tempfile` side, forbidden otherwise — `sort -o OUT IN` above is the canonical shape). That the `summarize` "program" is a 9-line shell script is the operator's business: the *runner* never invokes a shell, it execs the one configured path.

## 2. Serve: one endpoint and one persisted identity per tool

```sh
wanix-rust tool serve --config /tmp/tools-demo/tools.toml --listen 127.0.0.1:0
```

```text
rot13	iroh://d3fa691d...?addr=127.0.0.1:32850
# mount with: wanix-rust mount-ls 'iroh://d3fa691d...?addr=127.0.0.1:32850'
slow	iroh://e7a64b50...?addr=127.0.0.1:56418
...
sortlines	iroh://c64da489...?addr=127.0.0.1:55574
...
summarize	iroh://a1b4a6d8...?addr=127.0.0.1:48416
...
```

Each tool keeps its own ed25519 identity under `~/.wanix/tool-identities/<name>.key`, so its ticket's peer half survives restarts. Config names shadow built-ins in the merged registry (`--tool upper` still works alongside `--config`). Add `--register NAME` to also write catalog entries and skip the ticket-pasting below: one served tool registers as `NAME`, several as `NAME-<resource>` (executed: `--tool upper --tool sha256 --register text` wrote `text-upper` and `text-sha256`); a re-announce overwrites in place, so restarts keep the route hint fresh. See [Recipe 10](/recipes/10-name-your-world). The tool advertises exactly what you configured:

```sh
ROT='iroh://d3fa691d...?addr=127.0.0.1:32850'
wanix-rust mount-cat "$ROT" spec.json
```

```text
{"wanix.resource":"v0","kind":"tool","name":"rot13","description":"rot13 text (stdin -> stdout)","input":{"mode":"bytes","contentTypes":["application/octet-stream"],"maxBytes":1048576},...,"limits":{"runTimeoutMs":5000,"maxConcurrentPerPrincipal":2,...},"lifecycle":{"allocatedTtlMs":300000,"retainDoneMs":600000,"retainFailedMs":3600000},"visibility":"private","sideEffects":"none","retryable":true}
```

## 3. Run them from the shell — a config tool is just a tool

Export each ticket from step 2 (`ROT`, `SORT`, `SUM`, `SLOW` below — quote the whole `TICKET=/path` mount argument, the value splits on the last `=`):

```sh
wanix-rust sh -c 'echo the mesh runs my own programs now | tool /n/rot13' --mount-mesh "$ROT=/n/rot13"
```

```text
gur zrfu ehaf zl bja cebtenzf abj
job: /n/rot13/jobs/j46fc075103b33b2f
```

And the `tempfile` mapping is invisible to the caller — bytes in, bytes out (here piping a mounted volume file from recipe 06 through `sortlines`):

```sh
wanix-rust sh -c 'cat /vol/notes/fruit.txt | tool /n/sort' \
  --mount-mesh "$SORT=/n/sort" --mount-mesh "$VOL=/vol/notes"
```

```text
apple
mango
pear
job: /n/sort/jobs/jb5ba1bab9fa49368
```

## 4. Watch a job live: `events` + `mount-cat --follow`

A job directory is `{request, params.json, in, ctl, out, err, status, result.json, events}`. `events` is a never-EOF progress stream fed by the wrapped program's stderr lines as they arrive — so a long job is observable while it runs. Drive `summarize` by hand and park a follower on its events (terminal 2):

```sh
SUM='iroh://a1b4a6d8...?addr=127.0.0.1:48416'
J=$(wanix-rust mount-cat "$SUM" new)
wanix-rust mount-write "$SUM" "jobs/$J/in" 'the quick brown fox jumps over the lazy dog'

# Terminal 2 — prints each event as it arrives, then exits on EOF:
wanix-rust mount-cat "$SUM" "jobs/$J/events" --follow

# Terminal 1:
wanix-rust mount-write "$SUM" "jobs/$J/ctl" run        # blocks ~4 s, terminal on return
```

Terminal 2 prints the three lines as they happen — in this run (timestamping each arrival on the reader side) they landed at 08:45:18, 08:45:20, and 08:45:22, the 2-second gaps being the script's sleeps observed live:

```text
summarize: reading input
summarize: counted
summarize: writing summary
```

The job finishing closes the events stream, so the `--follow` reader exits cleanly on EOF — the [blocking-stream EOF contract](/concepts/blocking-stream-eof-contract) end to end. Then collect the answer:

```sh
wanix-rust mount-cat "$SUM" "jobs/$J/out"
# 9 words
wanix-rust mount-cat "$SUM" "jobs/$J/result.json"
# {"state":"done","exitCode":0,"durationMs":4006,"inputBytes":43,"outputBytes":8,"error":null,"retryable":true}
```

## 5. A real timeout

`slow` sleeps for 10 s but its config says `run_timeout_ms = 2000`. Through the builtin the failure taxonomy lands on stderr and the exit status:

```sh
wanix-rust sh -c 'echo anything | tool /n/slow' --mount-mesh "$SLOW=/n/slow"
```

```text
job: /n/slow/jobs/jaa994e5538ca35db
tool: timeout: run deadline exceeded; process killed
```

(exit status 1; the whole invocation took 2.15 s — the deadline killed the process, not the sleep ending.) The builtin closes only *successful* jobs, so this failed run stays retained behind the `job:` breadcrumb for `retain_failed_ms` — `mount-cat "$SLOW" "jobs/<id>/result.json"` works after the fact. The same retained shape, driven by the raw protocol:

```sh
J=$(wanix-rust mount-cat "$SLOW" new)
wanix-rust mount-write "$SLOW" "jobs/$J/ctl" run      # returns after 2 s, not 10
wanix-rust mount-cat "$SLOW" "jobs/$J/result.json"
```

```text
{"state":"failed","exitCode":null,"durationMs":2014,"inputBytes":0,"outputBytes":0,"error":{"kind":"timeout","message":"run deadline exceeded; process killed"},"retryable":true}
```

## 6. A real abort

`ctl run` is synchronous for the caller, so abort from a *second* client — same principal, same `jobs/` view. Start a `summarize` run in one terminal and write `abort` from another while it sleeps:

```sh
J=$(wanix-rust mount-cat "$SUM" new)
wanix-rust mount-write "$SUM" "jobs/$J/in" 'abort me'
wanix-rust mount-write "$SUM" "jobs/$J/ctl" run &     # parks while the script runs
sleep 1
wanix-rust mount-write "$SUM" "jobs/$J/ctl" abort     # kills the child; run returns
wait
wanix-rust mount-cat "$SUM" "jobs/$J/result.json"; echo
wanix-rust mount-cat "$SUM" "jobs/$J/events"
```

```text
{"state":"aborted","exitCode":null,"durationMs":2004,"inputBytes":8,"outputBytes":0,"error":{"kind":"aborted","message":"aborted by caller"},"retryable":false}

summarize: reading input
```

The abort verdict wins over whatever the dying process manages to exit with, and the events it emitted before death stay retained — a collected `mount-cat` of a *closed* events stream returns the backlog and EOFs immediately.

## 7. Prove the cage

The runner posture is host-fixed: no shell, empty child environment, fresh private cwd per job (mode 0700, removed afterwards). Don't take the doc's word for it — serve two probe tools the same way (`ENV`/`CWD` are their tickets):

```toml
[tools.envprobe]
description = "Print the child environment"
command = "/usr/bin/env"

[tools.cwdprobe]
description = "Print the child working directory"
command = "/bin/pwd"
```

```sh
wanix-rust sh -c 'echo x | tool /n/t' --mount-mesh "$ENV=/n/t"   # prints NOTHING: env is empty
wanix-rust sh -c 'echo x | tool /n/t' --mount-mesh "$CWD=/n/t"
# /tmp/wanix-tool-877669-1-j762d19d4e4fe9af6                     <- private per-job workdir
```

A caller cannot choose the executable, argv, cwd, or env — only input bytes. What the configured program is *allowed to be* is the operator's whole trust decision, made once, in the file.

## Troubleshooting (friction actually hit while testing)

- **`result.json` reads `file does not exist` after a *successful* `tool` builtin run.** The builtin closes a job that succeeded (its `out` already reached your stdout), releasing retention early. Failed, aborted, and timed-out jobs are deliberately left retained behind the `job:` breadcrumb until `retain_failed_ms` expires, so their `result.json`/`err`/`events` stay inspectable.
- **Plain `mount-cat events` on a running job prints nothing.** Same collected-vs-streaming split as everywhere: `events` never EOFs while the job lives, so use `--follow` for the live view; a plain read works once the job is terminal (the stream is closed, so the backlog EOFs).
- **`tool serve --config ... --listen 127.0.0.1:5700` is rejected with more than one tool.** Each tool is its own endpoint; a fixed nonzero port cannot back several. Use port `0`.
- **No `--listen` at all is refused.** Default-deny, verbatim: "tool serve on the public endpoint exposes each tool to anyone with its ticket; pass --listen IP:PORT (use port 0 to serve multiple tools) or --insecure-open to deliberately export to the open internet".
- **A relative `command` or a stray `{input}` placeholder fails at startup, not at run time.** The config is validated when loaded: commands must be absolute, and `{input}`/`{output}` must appear exactly once when (and only when) the matching side is `tempfile`.

## Cleanup

Ctrl-C the serve; job state dies with it. The per-tool identities stay under `~/.wanix/tool-identities/`; delete `/tmp/tools-demo` when done.
