---
title: Recipe 06 — Compose a Volume and Two Tools in One Shell Across the Mesh
slug: recipes/06-compose-volume-and-tools
pageType: use-case
oneLiner: Serve a demo-notes volume and the upper/sha256 tools as three mesh resources, mount all three into one wanix-rust sh namespace, pipe a remote file through a remote tool back into the remote volume, then drive the raw job protocol with no sugar.
audience: [developer]
tags: [mesh, cli, tools, volumes, shell, shipped, caveat]
sourceRefs:
  - crates/wanix-cli/src/volume/serve.rs:160-190
  - crates/wanix-cli/src/tool/serve.rs:96-110
  - crates/wanix-cli/src/tool/serve.rs:186
  - crates/wanix-cli/src/sh.rs:73-101
  - crates/wanix-cli/src/mesh/ticket.rs:156-201
  - crates/wanix-cli/src/qjs_args/mod.rs:144-148
  - crates/wanix-sh/src/tool.rs:1-21
seeAlso:
  - learn/compose-volumes-and-tools
  - concepts/jobs-are-files
  - concepts/key-is-the-address
  - concepts/wanix-sh
  - recipes/02-mount-remote-peer
prerequisites:
  - concepts/key-is-the-address
  - concepts/wanix-sh
usedInFlows:
  - {flow: compose-volumes-and-tools, step: 4}
honestLimits:
  - "A remount is a new principal: the dialer identity is ephemeral per dial, so jobs allocated in one sh invocation read as NotFound from the next. Keep multi-step job interactions in one session."
  - "tool serve admits any ticket holder (no allow-list yet); each is confined to its own private jobs/ view."
  - "Only the model/sha256/upper built-in runners are servable; volume state is plain host files, but tool job state lives only while tool serve runs."
canonicalCaveatFor: []
---

# Recipe 06 — Compose a Volume and Two Tools in One Shell Across the Mesh

Serve a `demo-notes` volume and the `upper`/`sha256` tools as three mesh resources, mount all three into one `wanix-rust sh` namespace, pipe a remote file through a remote tool back into the remote volume, then drive the raw job protocol with no sugar.

**What & why.** Three serves, three tickets, one shell line. The point is that "run a remote tool on a remote file and store the result remotely" needs no new verb: a volume is a `FileSystem`, a tool is a `FileSystem`, and the consuming shell just resolves paths. Everything below ran verbatim on one machine over loopback (tickets shortened to a recognizable prefix; yours will differ). The conceptual walkthrough is [Compose volumes and tools](/learn/compose-volumes-and-tools).

## 0. Build the binary

```sh
cargo build --package wanix-cli
alias wanix-rust='./target/debug/wanix-rust'
```

## 1. Create and seed the volume

```sh
wanix-rust volume create demo-notes
# created volume demo-notes at /root/.wanix/volumes/demo-notes
printf 'wanix makes the mesh feel local\n' > ~/.wanix/volumes/demo-notes/hello.txt
```

A volume is a host directory plus its own persisted ed25519 identity (`~/.wanix/volume-identities/demo-notes.key`, created on first serve). The directory is the durable state; the identity is the name.

## 2. Serve the volume (terminal 1)

```sh
wanix-rust volume serve --volume demo-notes --addr 127.0.0.1:0
```

The process parks forever (Ctrl-C to stop) and prints one two-line record per volume on stderr:

```text
demo-notes	iroh://6e8a49fe...?addr=127.0.0.1:38119
# mount with: wanix-rust mount-ls 'iroh://6e8a49fe...?addr=127.0.0.1:38119'
```

`iroh://6e8a49fe...` (64 hex chars in full) is the volume's public key; `?addr=` is a route hint. Stop and restart the serve and the peer half is identical while the port changes — copy the fresh ticket after any restart.

## 3. Serve the tools (terminal 2)

```sh
wanix-rust tool serve --tool upper --tool sha256 --addr 127.0.0.1:0
```

Port `0` is required when serving more than one tool (each gets its own endpoint). One record per tool, same format:

```text
upper	iroh://2bcb5813...?addr=127.0.0.1:52164
# mount with: wanix-rust mount-ls 'iroh://2bcb5813...?addr=127.0.0.1:52164'
sha256	iroh://ae985229...?addr=127.0.0.1:46571
# mount with: wanix-rust mount-ls 'iroh://ae985229...?addr=127.0.0.1:46571'
```

Sanity-check a tool root — the whole API is five files:

```sh
wanix-rust mount-ls 'iroh://2bcb5813...?addr=127.0.0.1:52164'
```

```text
health
jobs
new
spec.json
usage
```

## 4. The composition (terminal 3)

Mount all three resources into one shell namespace and pipe across them. Quote each `--mount-mesh` argument whole — the value splits on the **last** `=`, and the ticket itself contains `=` in `?addr=`:

```sh
VOL='iroh://6e8a49fe...?addr=127.0.0.1:38119'
UP='iroh://2bcb5813...?addr=127.0.0.1:52164'
SHA='iroh://ae985229...?addr=127.0.0.1:46571'

wanix-rust sh -c 'cat < /vol/notes/hello.txt | tool /n/upper > /vol/notes/HELLO.txt' \
  --mount-mesh "$VOL=/vol/notes" --mount-mesh "$UP=/n/upper" --mount-mesh "$SHA=/n/sha256"
# (no output; exit 0)
```

Verify on the serving side — the bytes really landed in the volume's host directory:

```sh
cat ~/.wanix/volumes/demo-notes/HELLO.txt
```

```text
WANIX MAKES THE MESH FEEL LOCAL
```

Hash the new file with the third resource, and cross-check with a host tool:

```sh
wanix-rust sh -c 'cat < /vol/notes/HELLO.txt | tool /n/sha256' \
  --mount-mesh "$VOL=/vol/notes" --mount-mesh "$SHA=/n/sha256"
```

```text
46faf38c2de00014c7d96ee93363da026cab6ca90f12f7e4f43ed49e5c01ee49
```

```sh
sha256sum ~/.wanix/volumes/demo-notes/HELLO.txt
# 46faf38c2de00014c7d96ee93363da026cab6ca90f12f7e4f43ed49e5c01ee49  .../HELLO.txt
```

## 5. The raw job protocol (no `tool` sugar)

The `tool` builtin is convenience over visible files. Drive one job by hand in the interactive REPL (`wanix-rust sh` with no `-c` — the same `--mount-mesh` flags). It must be one session: a new invocation is a new principal and would not see this job.

```text
/ $ cat < /n/upper/new
j77762dbf2540e718
/ $ echo raw protocol, no sugar > /n/upper/jobs/j77762dbf2540e718/in
/ $ cat < /n/upper/jobs/j77762dbf2540e718/status
{"state":"receiving","createdAt":1781049730406,"startedAt":null,"finishedAt":null,"expiresAt":null,"inputBytes":23,"outputBytes":0}
/ $ echo run > /n/upper/jobs/j77762dbf2540e718/ctl
/ $ cat < /n/upper/jobs/j77762dbf2540e718/out
RAW PROTOCOL, NO SUGAR
/ $ cat < /n/upper/jobs/j77762dbf2540e718/result.json
{"state":"done","exitCode":0,"durationMs":0,"inputBytes":23,"outputBytes":23,"error":null,"retryable":true}
/ $ echo close > /n/upper/jobs/j77762dbf2540e718/ctl
/ $ exit
```

`new` allocates, `in` receives bytes, `ctl run` is synchronous (terminal on return), `out` and `result.json` are the answer, `close` releases retention early. See [jobs are files](/concepts/jobs-are-files).

## 6. Two shells are two principals

Identity comes from the QUIC handshake, never from a payload — and the CLI dials each mount with a fresh ephemeral key, so even your own next invocation is a stranger:

```sh
wanix-rust sh -c 'cat < /n/upper/new' --mount-mesh "$UP=/n/upper"
# jc883d86da5693426
wanix-rust sh -c 'cat < /n/upper/jobs/jc883d86da5693426/status' --mount-mesh "$UP=/n/upper"
```

```text
wsh: /n/upper/jobs/jc883d86da5693426/status: No such file or directory (os error 44)
```

Foreign job ids read as `NotFound` — the per-principal `jobs/` confinement working as designed, and the reason step 5 stayed inside one session.

## Troubleshooting (friction actually hit while testing)

- **`resource unreachable: peer 6e8a49fe... did not answer within 5s`** — the serve behind that ticket is down (or the route hint is stale). The full message says it plainly: "the provider is offline or not discoverable from here, and the mount will work again when it returns (a bare iroh://PEER is found by mDNS on the LAN; pass ?addr=IP:PORT as a direct route hint)". Restart the serve and re-copy the ticket — the peer id stays the same, but `--addr 127.0.0.1:0` picks a new port each run.
- **`cat /vol/notes/hello.txt` prints nothing and exits 0.** The shell's `cat` builtin reads stdin only and ignores operands. Use a redirect: `cat < /vol/notes/hello.txt`.
- **Mount flag parses the wrong path.** `--mount-mesh` splits on the *last* `=`, so an unquoted ticket lets your host shell or the parser carve it at `?addr=`. Always quote the whole `TICKET=/guest/path` argument.
- **A job id from a previous invocation reads as `No such file or directory`.** Not a bug — see step 6. Keep allocate → write → run → read inside one `sh` session (the `tool` builtin does all five steps in one line for exactly this reason).
- **`tool serve` / `volume serve` refuses to start without `--addr`.** Default-deny: serving on the public endpoint hands the resource to anyone with the ticket, so it demands either `--addr IP:PORT` or an explicit `--insecure-open`.
- **`tool serve --tool a --tool b --addr 127.0.0.1:5700` is rejected.** A fixed nonzero port cannot back multiple endpoints; use port `0` when serving more than one tool (or volume).

## Cleanup

Ctrl-C both serves; the volume's files stay under `~/.wanix/volumes/demo-notes/`, the identities under `~/.wanix/volume-identities/` and `~/.wanix/tool-identities/`. Tool job state is gone with the serve process.
