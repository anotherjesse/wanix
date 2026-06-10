---
title: Recipe 06 — Compose a Volume and Two Tools in One Shell Across the Mesh
slug: recipes/06-compose-volume-and-tools
pageType: use-case
oneLiner: Serve a demo-notes volume and the upper/sha256 tools as three mesh resources, mount all three into one wanix sh namespace, pipe a remote file through a remote tool back into the remote volume, then drive the raw job protocol with no sugar.
audience: [developer]
tags: [mesh, cli, tools, volumes, shell, shipped, caveat]
sourceRefs:
  - crates/wanix-cli/src/volume/serve.rs:160-190
  - crates/wanix-cli/src/tool/serve.rs:96-110
  - crates/wanix-cli/src/tool/serve.rs:186
  - crates/wanix-cli/src/sh.rs:73-101
  - crates/wanix-cli/src/mesh/ticket.rs:156-260
  - crates/wanix-cli/src/qjs_args/mod.rs:148-152
  - crates/wanix-sh/src/tool.rs:1-21
seeAlso:
  - learn/compose-volumes-and-tools
  - concepts/jobs-are-files
  - concepts/key-is-the-address
  - concepts/wanix-sh
  - recipes/02-mount-remote-peer
  - recipes/08-real-tools-with-config
  - recipes/10-name-your-world
prerequisites:
  - concepts/key-is-the-address
  - concepts/wanix-sh
usedInFlows:
  - {flow: compose-volumes-and-tools, step: 4}
honestLimits:
  - "The dialer principal is the persisted ~/.wanix/dialer.key: every invocation by one user is one principal, so anyone who can read that key file can present it. Distinct users/machines remain distinct principals."
  - "tool serve admits any ticket holder (no allow-list yet); each is confined to its own private jobs/ view."
  - "Volume state is plain host files, but tool job state lives only while tool serve runs."
canonicalCaveatFor: []
---

# Recipe 06 — Compose a Volume and Two Tools in One Shell Across the Mesh

Serve a `demo-notes` volume and the `upper`/`sha256` tools as three mesh resources, mount all three into one `wanix sh` namespace, pipe a remote file through a remote tool back into the remote volume, then drive the raw job protocol with no sugar.

**What & why.** Three serves, three tickets, one shell line. The point is that "run a remote tool on a remote file and store the result remotely" needs no new verb: a volume is a `FileSystem`, a tool is a `FileSystem`, and the consuming shell just resolves paths. Everything below ran verbatim on one machine over loopback (tickets shortened to a recognizable prefix; yours will differ). The conceptual walkthrough is [Compose volumes and tools](/learn/compose-volumes-and-tools).

## 0. Build the binary

```sh
cargo build --package wanix-cli
alias wanix='./target/debug/wanix'
```

## 1. Create and seed the volume

```sh
wanix volume create demo-notes
# created volume demo-notes at /root/.wanix/volumes/demo-notes
printf 'wanix makes the mesh feel local\n' > ~/.wanix/volumes/demo-notes/hello.txt
```

A volume is a host directory plus its own persisted ed25519 identity (`~/.wanix/volume-identities/demo-notes.key`, created on first serve). The directory is the durable state; the identity is the name.

## 2. Serve the volume (terminal 1)

```sh
wanix volume serve --volume demo-notes --addr 127.0.0.1:0
```

The process parks forever (Ctrl-C to stop) and prints one two-line record per volume on stderr:

```text
demo-notes	iroh://6e8a49fe...?addr=127.0.0.1:38119
# mount with: wanix mount-ls 'iroh://6e8a49fe...?addr=127.0.0.1:38119'
```

`iroh://6e8a49fe...` (64 hex chars in full) is the volume's public key; `?addr=` is a route hint. Stop and restart the serve and the peer half is identical while the port changes — copy the fresh ticket after any restart.

## 3. Serve the tools (terminal 2)

```sh
wanix tool serve --tool upper --tool sha256 --listen 127.0.0.1:0
```

`upper` and `sha256` are built-in runners; serving *your own host programs* from a `tools.toml` is [Recipe 08](/recipes/08-real-tools-with-config). Port `0` is required when serving more than one tool (each gets its own endpoint). One record per tool, same format:

```text
upper	iroh://2bcb5813...?addr=127.0.0.1:52164
# mount with: wanix mount-ls 'iroh://2bcb5813...?addr=127.0.0.1:52164'
sha256	iroh://ae985229...?addr=127.0.0.1:46571
# mount with: wanix mount-ls 'iroh://ae985229...?addr=127.0.0.1:46571'
```

Sanity-check a tool root — the whole API is five files:

```sh
wanix mount-ls 'iroh://2bcb5813...?addr=127.0.0.1:52164'
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

wanix sh -c 'cat /vol/notes/hello.txt | tool /n/upper > /vol/notes/HELLO.txt' \
  --mount-mesh "$VOL=/vol/notes" --mount-mesh "$UP=/n/upper" --mount-mesh "$SHA=/n/sha256"
# job: /n/upper/jobs/j6ad9f09ad9bc2437      <- stderr; exit 0
```

The `job:` line on stderr is the builtin announcing the job path as soon as it is allocated — the crash-resume handle: a successor of a crashed caller can find the retained job directory and resume from its `status`/`result.json`.

Verify on the serving side — the bytes really landed in the volume's host directory:

```sh
cat ~/.wanix/volumes/demo-notes/HELLO.txt
```

```text
WANIX MAKES THE MESH FEEL LOCAL
```

Hash the new file with the third resource, and cross-check with a host tool:

```sh
wanix sh -c 'cat /vol/notes/HELLO.txt | tool /n/sha256' \
  --mount-mesh "$VOL=/vol/notes" --mount-mesh "$SHA=/n/sha256"
```

```text
46faf38c2de00014c7d96ee93363da026cab6ca90f12f7e4f43ed49e5c01ee49
job: /n/sha256/jobs/jadf6716e90d09c55
```

```sh
sha256sum ~/.wanix/volumes/demo-notes/HELLO.txt
# 46faf38c2de00014c7d96ee93363da026cab6ca90f12f7e4f43ed49e5c01ee49  .../HELLO.txt
```

### Or: skip the pasting and use names

The ticket dance above is the no-catalog fallback. Add `--register NAME` to each serve (`volume serve --volume demo-notes ... --register demo-notes`, `tool serve --tool upper ... --register upper`) and the composition line shrinks to names that resolve through `~/.wanix/catalog` at launch — executed verbatim:

```sh
wanix sh -c 'cat /vol/notes/hello.txt | tool /n/upper > /vol/notes/HELLO.txt' \
  --mount-mesh demo-notes=/vol/notes --mount-mesh upper=/n/upper
```

```text
wanix: name 'demo-notes' -> iroh://a4166b05...?addr=127.0.0.1:40213 (resolved through the catalog at launch)
wanix: name 'upper' -> iroh://8c7a7009...?addr=127.0.0.1:54872 (resolved through the catalog at launch)
job: /n/upper/jobs/j5fc61d9fe59708e6
```

Same result, no hex; the stderr audit lines record what each name resolved to. The full naming story — `catalog ls` liveness, recipes, the fresh-machine rebuild — is [Recipe 10](/recipes/10-name-your-world).

## 5. The raw job protocol (no `tool` sugar)

The `tool` builtin is convenience over visible files. Drive one job by hand in the interactive REPL (`wanix sh` with no `-c` — the same `--mount-mesh` flags). One session is convenient, not required: the CLI dials every mount with your persisted dialer identity, so a later invocation is the same principal and still sees this job (step 6).

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

`new` allocates, `in` receives bytes, `ctl run` is synchronous (terminal on return), `out` and `result.json` are the answer, `close` releases retention early. The full job directory is `{request, params.json, in, ctl, out, err, status, result.json, events}` — `err` is the runner's diagnostics and `events` is a never-EOF progress stream you can watch live with `mount-cat --follow` ([Recipe 08](/recipes/08-real-tools-with-config) does, plus timeout and abort). See [jobs are files](/concepts/jobs-are-files).

When a job fails, the taxonomy stays visible through the builtin too — pipe invalid UTF-8 into `upper` and stderr carries the `result.json` error plus the job's `err` diagnostics, with a nonzero exit:

```sh
wanix sh -c 'cat /vol/notes/blob.bin | tool /n/upper' \
  --mount-mesh "$VOL=/vol/notes" --mount-mesh "$UP=/n/upper"
```

```text
job: /n/upper/jobs/j48f273aaf171eeb4
tool: invalid_input: input is not valid UTF-8
input is not valid UTF-8
```

(exit status 1; `blob.bin` here was three bytes of `\xff\xfe\xfd`.)

## 6. Your invocations are one principal; strangers stay strangers

Identity comes from the QUIC handshake, never from a payload. The CLI dials every mount with one persisted key (`~/.wanix/dialer.key`, created on first dial), so a job allocated in one invocation is still yours from the next:

```sh
wanix sh -c 'cat < /n/upper/new' --mount-mesh "$UP=/n/upper"
# j31770e20a12797a3
wanix sh -c 'cat /n/upper/jobs/j31770e20a12797a3/status' --mount-mesh "$UP=/n/upper"
```

```text
{"state":"allocated","createdAt":1781053502736,"startedAt":null,"finishedAt":null,"expiresAt":null,"inputBytes":0,"outputBytes":0}
```

That durability is what makes a retained job a real audit/recovery record (ADR 0009's "survives caller death") instead of state orphaned behind a throwaway key. The privacy boundary is unchanged: a *different* dialer key is a different principal, its `jobs/` view is disjoint, and a guessed foreign job id reads as `NotFound` — never `PermissionDenied` (pinned by `two_peers_see_disjoint_jobs` and `same_identity_redial_resumes_jobs` in `crates/wanix-cli/src/tool/serve/tests.rs`).

## Troubleshooting (friction actually hit while testing)

- **`resource unreachable: peer 6e8a49fe... did not answer within 5s`** — the serve behind that ticket is down (or the route hint is stale). The full message says it plainly: "the provider is offline or not discoverable from here, and the mount will work again when it returns (a bare iroh://PEER is found by mDNS on the LAN; pass ?addr=IP:PORT as a direct route hint)". Restart the serve and re-copy the ticket — the peer id stays the same, but `--addr 127.0.0.1:0` picks a new port each run.
- **Mount flag parses the wrong path.** `--mount-mesh` splits on the *last* `=`, so an unquoted ticket lets your host shell or the parser carve it at `?addr=`. Always quote the whole `TICKET=/guest/path` argument.
- **A job id reads as `No such file or directory` from another machine or user.** Not a bug — see step 6: `jobs/` is per-principal, and the principal is your `~/.wanix/dialer.key`. A different key (different user, different machine, a deleted key file) is a stranger to your jobs.
- **`tool serve` / `volume serve` refuses to start without a listen address.** Default-deny: serving on the public endpoint hands the resource to anyone with the ticket, so it demands an explicit address (verbatim: "pass --listen IP:PORT (use port 0 to serve multiple tools) or --insecure-open to deliberately export to the open internet"). `--addr` still parses as an alias for `--listen`.
- **`tool serve --tool a --tool b --listen 127.0.0.1:5700` is rejected.** A fixed nonzero port cannot back multiple endpoints; use port `0` when serving more than one tool (or volume).

## Cleanup

Ctrl-C both serves; the volume's files stay under `~/.wanix/volumes/demo-notes/`, the identities under `~/.wanix/volume-identities/` and `~/.wanix/tool-identities/`, and your dialer principal at `~/.wanix/dialer.key`. Tool job state is gone with the serve process.
