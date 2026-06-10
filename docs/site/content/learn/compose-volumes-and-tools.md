---
title: Compose volumes and tools in one shell across the mesh
slug: learn/compose-volumes-and-tools
pageType: flow
oneLiner: Serve a data volume and two tools as three mesh resources, mount all three into one shell namespace, and pipe a file through a remote tool back into the remote volume — every step a file operation.
audience: [newcomer, developer]
tags: [mesh, cli, tools, volumes, shell, shipped, caveat]
sourceRefs:
  - docs/site/content/recipes/06-compose-volume-and-tools.md
  - crates/wanix-cli/src/volume.rs:1-9
  - crates/wanix-cli/src/volume/serve.rs:1-10
  - crates/wanix-cli/src/tool/serve.rs:1-11
  - crates/wanix-cli/src/tool/serve.rs:186
  - crates/wanix-cli/src/sh.rs:73-101
  - crates/wanix-cli/src/mesh/mounts.rs:1-10
  - crates/wanix-cli/src/mesh/ticket.rs:156-201
  - crates/wanix-cli/src/qjs_args/mod.rs:144-148
  - crates/wanix-sh/src/tool.rs:1-21
  - crates/wanix-tool/src/service.rs:81
  - docs/adrs/0007-resources-catalogs-and-pairing.md
  - docs/adrs/0009-job-protocol.md
seeAlso:
  - concepts/jobs-are-files
  - concepts/key-is-the-address
  - concepts/namespace-binding
  - concepts/devices-import-for-free
  - concepts/wanix-sh
  - concepts/attach-policy
  - concepts/everything-is-a-file
  - recipes/06-compose-volume-and-tools
  - recipes/08-real-tools-with-config
prerequisites:
  - learn/js-outside-chrome
usedInFlows: []
honestLimits:
  - "The dialer principal is the persisted ~/.wanix/dialer.key (crates/wanix-cli/src/mesh/ticket.rs): every invocation by one user is one principal, so retained jobs survive a remount — and anyone who can read that key file can present it. Distinct users/machines remain distinct principals."
  - "tool serve has no allow-list yet: any holder of a tool's ticket may attach (each bound to its own private job view). Grant lifecycle is ADR 0007 follow-up work."
  - "Command substitution is not yet supported in the shell, so a job id cannot be captured into a variable — use the REPL or the `tool` builtin."
  - "The model/sha256/upper built-ins are deterministic in-process runners; real host programs are served from a tools.toml via tool serve --config (recipe 08)."
canonicalCaveatFor: [ephemeral-dialer-identity]
---

# Compose volumes and tools in one shell across the mesh

Serve a data volume and two tools as three mesh resources, mount all three into one shell namespace, and pipe a file through a remote tool back into the remote volume — every step a file operation.

This flow is the composition the resource model exists for. You will stand up three independent mesh resources — a persistent data volume and two tools — and then compose them in a single shell line on the consuming side, where "call the remote tool on the remote file and store the result remotely" is just a pipeline with redirects. Nothing on the wire is RPC: a volume is a `FileSystem`, a tool is a `FileSystem`, and the shell only ever reads and writes paths. One machine and two terminals are enough; everything runs on loopback. About fifteen minutes.

```sh
cargo build --package wanix-cli
alias wanix-rust='./target/debug/wanix-rust'
```

## 1. A volume is a directory with an identity

A *volume* is a host directory under `~/.wanix/volumes/<name>` plus a persisted ed25519 identity of its own (`~/.wanix/volume-identities/<name>.key`). Create one and seed it:

```sh
wanix-rust volume create demo-notes
# created volume demo-notes at ~/.wanix/volumes/demo-notes
printf 'wanix makes the mesh feel local\n' > ~/.wanix/volumes/demo-notes/hello.txt
```

The identity matters more than the directory. Per ADR 0007 (`docs/adrs/0007-resources-catalogs-and-pairing.md`) **one ticket names exactly one resource root**: when you serve this volume, its ticket is derived from *its* key, not from your node's, and holding that ticket reaches this volume and nothing else. There is no aggregate `/vol/*` root to escape into (`crates/wanix-cli/src/volume/serve.rs`).

## 2. Serve it: one ticket per resource

```sh
# Terminal 1 — parks forever; Ctrl-C to stop.
wanix-rust volume serve --volume demo-notes --addr 127.0.0.1:0
```

Two lines arrive on stderr — the record format every resource server in the CLI shares:

```text
demo-notes	iroh://6e8a49fe...?addr=127.0.0.1:38119
# mount with: wanix-rust mount-ls 'iroh://6e8a49fe...?addr=127.0.0.1:38119'
```

Read the ticket the Plan 9 way: `iroh://PEER` is the *identity* (the volume's ed25519 public key — [the key is the address](/concepts/key-is-the-address)), and `?addr=IP:PORT` is only a route hint. Restart the serve and the peer half stays identical while the hinted port changes — the name survives, the route is disposable. Note the default-deny posture: with no `--addr` at all the command refuses to run unless you pass `--insecure-open`, because a ticket on the open internet is a capability.

## 3. Tools are filesystems too

A *tool* is one host-approved operation served as files — a ToolFS ([jobs are files](/concepts/jobs-are-files)). Serve two of the built-ins; each gets its own endpoint, identity (`~/.wanix/tool-identities/<name>.key`), and ticket, exactly like the volume:

```sh
# Terminal 2 — port 0 is required when serving more than one tool.
wanix-rust tool serve --tool upper --tool sha256 --addr 127.0.0.1:0
```

```text
upper	iroh://2bcb5813...?addr=127.0.0.1:52164
# mount with: wanix-rust mount-ls 'iroh://2bcb5813...?addr=127.0.0.1:52164'
sha256	iroh://ae985229...?addr=127.0.0.1:46571
# mount with: wanix-rust mount-ls 'iroh://ae985229...?addr=127.0.0.1:46571'
```

`mount-ls` on a tool ticket shows the whole API surface — it is five files:

```sh
wanix-rust mount-ls 'iroh://2bcb5813...?addr=127.0.0.1:52164'
# health  jobs  new  spec.json  usage
```

`spec.json` is the self-describing contract (input mode, byte limits, quotas, retention); `new` allocates a job; `jobs/` is where your calls live. Nothing else exists to learn.

## 4. Mounting composes a namespace

Now the consuming side. `wanix-rust sh` builds a per-process namespace ([namespace binding](/concepts/namespace-binding)) from your `--cwd`, the shell's command bin, `#pipe`, `#task` — and any number of `--mount-mesh TICKET=/guest/path` imports, each dialed over the native mesh wire and bound where you say (`crates/wanix-cli/src/mesh/mounts.rs`). The three resources you just served become three directories:

```sh
wanix-rust sh \
  -c 'cat < /vol/notes/hello.txt | tool /n/upper > /vol/notes/HELLO.txt' \
  --mount-mesh 'iroh://6e8a49fe...?addr=127.0.0.1:38119=/vol/notes' \
  --mount-mesh 'iroh://2bcb5813...?addr=127.0.0.1:52164=/n/upper' \
  --mount-mesh 'iroh://ae985229...?addr=127.0.0.1:46571=/n/sha256'
```

Sit with that line. `cat < /vol/notes/hello.txt` reads from one mesh peer; `tool /n/upper` runs a job on a second; `> /vol/notes/HELLO.txt` writes the result back to the first. The shell knows none of this — it resolves paths in its namespace, and the namespace happens to reach across two QUIC connections. Verify on the serving side that the bytes really landed:

```sh
cat ~/.wanix/volumes/demo-notes/HELLO.txt
# WANIX MAKES THE MESH FEEL LOCAL
```

Then hash it on the third resource and cross-check against a host tool:

```sh
wanix-rust sh -c 'cat < /vol/notes/HELLO.txt | tool /n/sha256' \
  --mount-mesh 'iroh://6e8a49fe...=/vol/notes' --mount-mesh 'iroh://ae985229...=/n/sha256'
# 46faf38c2de00014c7d96ee93363da026cab6ca90f12f7e4f43ed49e5c01ee49
sha256sum ~/.wanix/volumes/demo-notes/HELLO.txt   # same digest
```

One syntax fact that will bite you otherwise: the mount value splits on the **last** `=` (`crates/wanix-cli/src/qjs_args/mod.rs:144-148`), so quote the whole `TICKET=/path` argument — the ticket itself contains `=` in `?addr=`. (`cat FILE` and `cat < FILE` both work; the transcripts use the redirect form.)

## 5. A tool call is a job directory

The `tool` builtin (`crates/wanix-sh/src/tool.rs`) is sugar, not a protocol. Underneath, a call is a *job directory* you can drive by hand — ADR 0009's calling convention (`docs/adrs/0009-job-protocol.md`). Run the interactive REPL (`wanix-rust sh` with no `-c`) against the upper mount and do what the builtin does, file by file:

```text
/ $ cat < /n/upper/new
j77762dbf2540e718
/ $ echo raw protocol, no sugar > /n/upper/jobs/j77762dbf2540e718/in
/ $ cat < /n/upper/jobs/j77762dbf2540e718/status
{"state":"receiving","createdAt":1781049730406,...,"inputBytes":23,"outputBytes":0}
/ $ echo run > /n/upper/jobs/j77762dbf2540e718/ctl
/ $ cat < /n/upper/jobs/j77762dbf2540e718/out
RAW PROTOCOL, NO SUGAR
/ $ cat < /n/upper/jobs/j77762dbf2540e718/result.json
{"state":"done","exitCode":0,"durationMs":0,"inputBytes":23,"outputBytes":23,"error":null,"retryable":true}
/ $ echo close > /n/upper/jobs/j77762dbf2540e718/ctl
```

Because the call is files rather than a transcript entry, it has a name, a visible lifecycle (`status` said `receiving` before you ran it), an audit record (`result.json` persists per the spec's retention), and an idempotency key (the job id). A supervisor — or a successor agent after a crash — reads the directory instead of trusting anyone's self-report. The builtin's entire implementation is this dance plus error rendering.

## 6. Identity rides the transport

One more thing the transcript above proves quietly: who you are is never a field in a message. The tool server binds every connection to a principal derived from the QUIC handshake's verified `remote_id()` (`ToolAttachPolicy`, `crates/wanix-cli/src/tool/serve.rs:186`; `ToolService::open_view`, `crates/wanix-tool/src/service.rs:81`), so two distinct callers get **disjoint** `jobs/` views and a guessed foreign job id reads `NotFound`. The CLI presents one *persisted* dialer identity (`~/.wanix/dialer.key`, `crates/wanix-cli/src/mesh/ticket.rs`), so all of your own invocations are one principal — a retained job survives the process that allocated it:

```sh
wanix-rust sh -c 'cat < /n/upper/new' --mount-mesh 'iroh://a2887734...=/n/upper'
# j31770e20a12797a3
wanix-rust sh -c 'cat /n/upper/jobs/j31770e20a12797a3/status' --mount-mesh 'iroh://a2887734...=/n/upper'
# {"state":"allocated","createdAt":1781053502736,"startedAt":null,"finishedAt":null,"expiresAt":null,"inputBytes":0,"outputBytes":0}
```

That durability is ADR 0009's "survives caller death" promise actually crossing the mesh; the disjointness for *distinct* keys is pinned by `two_peers_see_disjoint_jobs` and the continuity by `same_identity_redial_resumes_jobs` (`crates/wanix-cli/src/tool/serve/tests.rs`).

## Where this goes

The full tested transcript — real tickets, real digests, and the failure modes (unreachable peer, a failed job) — is [Recipe 06](/recipes/06-compose-volume-and-tools). The job-directory idea stands alone in [jobs are files](/concepts/jobs-are-files). The catalog/pairing layer that would let you say `notes` instead of pasting a 64-hex ticket is ADR 0007's next slice, not yet shipped.

## Status / honest limits

- **The dialer principal is a key file.** `dial_iroh_remote` presents the persisted `~/.wanix/dialer.key` (`crates/wanix-cli/src/mesh/ticket.rs`), so a remount resumes your jobs — and whoever can read that file can be you to every job-scoped server. Per-principal grants/allow-lists are ADR 0007 follow-up work.
- **No allow-list on `tool serve`.** Any ticket holder attaches (each confined to its own job view); grant lifecycle is ADR 0007 follow-up work.
- **Built-ins are demo runners.** `--tool` accepts `model`, `sha256`, `upper` — deterministic in-process runners. Wrapping *your own host programs* (fixed command/argv, empty env, private per-job cwd, real timeouts and aborts) is `tool serve --config tools.toml` — [Recipe 08](/recipes/08-real-tools-with-config).
- **Shell subset.** No command substitution, so capturing a job id into a variable is not yet expressible — use the REPL or the `tool` builtin.
