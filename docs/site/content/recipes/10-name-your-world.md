---
title: Recipe 10 — Name Your World (catalog, names, recipes)
slug: recipes/10-name-your-world
pageType: use-case
oneLiner: Register a volume, a tool, and the chatroom in the catalog with --register, watch liveness in catalog ls, mount everything by bare name, run the room's verbs, save the whole desk as a recipe, and rebuild it on a fresh machine from two copied directories.
audience: [newcomer, developer]
tags: [mesh, cli, catalog, recipes, names, shell, shipped, caveat]
sourceRefs:
  - crates/wanix-cli/src/catalog.rs
  - crates/wanix-cli/src/catalog/tests.rs
  - crates/wanix-cli/src/recipe.rs
  - crates/wanix-cli/src/recipe/tests.rs
  - crates/wanix-cli/src/mesh/resource.rs
  - crates/wanix-cli/src/mesh/ticket.rs
  - crates/wanix-cli/src/qjs_args/mod.rs:144-167
  - examples/chatroom/bin/post.js
  - examples/chatroom/bin/watch.js
  - docs/adrs/0007-resources-catalogs-and-pairing.md
seeAlso:
  - learn/name-your-world
  - concepts/bin-verbs
  - concepts/key-is-the-address
  - recipes/06-compose-volume-and-tools
  - recipes/07-chatroom-over-the-mesh
  - recipes/02-mount-remote-peer
prerequisites:
  - concepts/key-is-the-address
usedInFlows:
  - {flow: name-your-world, step: 2}
honestLimits:
  - "Name resolution is strictly launch-time and never probes: a stale catalog address surfaces as the ordinary ADR 0008 \"resource unreachable\" dial error at first use, not at resolve time."
  - "Names work where mounts are taken (--mount-mesh, mount-ls/cat/write, recipe binds, serve --bind); `cpu --node` still wants a ticket (\"mesh address must start with iroh://: nodeb\" — executed)."
  - "catalog ls liveness is one bounded dial per entry: offline means \"nothing answered within the deadline\", which pre-ACL is all Layer 1 can know."
  - "A captured `sh -c 'room:watch'` collects output and prints it when the verb exits (at stream EOF); live line-by-line rendering needs an interactive terminal. After abrupt serve death the parked read waits for QUIC liveness (tens of seconds) before the EOF."
  - "The recipe file and the catalog are plain local files; copying them moves names and addresses, NOT identity — a fresh machine dials as a fresh principal (~/.wanix/dialer.key is created on first dial, never copied by this recipe)."
canonicalCaveatFor: []
---

# Recipe 10 — Name Your World (catalog, names, recipes)

Register a volume, a tool, and the chatroom in the catalog with `--register`, watch liveness in `catalog ls`, mount everything by bare name, run the room's verbs, save the whole desk as a recipe, and rebuild it on a fresh machine from two copied directories.

**What & why.** Recipes 06 and 07 work, but every transcript line carries a pasted 64-hex ticket. This recipe replaces the pasting with *your* names: the catalog (`~/.wanix/catalog`, ADR 0007 Layer 1) is a local address book binding a humane `NAME` to an `iroh://` ticket, every serve can self-register, and a saved recipe rebuilds a whole multi-resource namespace anywhere the catalog files travel. Everything below ran verbatim on one machine over loopback (tickets shortened to a recognizable prefix; yours will differ). The conceptual walkthrough is [Name your world](/learn/name-your-world).

A name is spelled `lowercase [a-z0-9-]`, first and last character alphanumeric — so a name can never be mistaken for a ticket (no scheme) or a path (no slash, no dot), and every resolution point routes on spelling alone.

## 0. Build the binary

```sh
cargo build --locked --package wanix-cli       # see /reference/build-and-install
alias wanix-rust='./target/debug/wanix-rust'
```

## 1. Serve three resources, registering each by name

Each serve below takes `--register NAME`: alongside the usual ticket record it writes (or overwrites — a re-announce updates the route hint) one catalog entry. One terminal per serve; each parks until Ctrl-C.

A volume named `notes`:

```sh
wanix-rust volume create notes
# created volume notes at ~/.wanix/volumes/notes
printf 'welcome to the writing desk\n' > ~/.wanix/volumes/notes/welcome.txt
printf 'meet the mesh at noon\n'       > ~/.wanix/volumes/notes/draft.txt
wanix-rust volume serve --volume notes --listen 127.0.0.1:0 --register notes
```

```text
notes	iroh://b070895d...?addr=127.0.0.1:51277
# mount with: wanix-rust mount-ls 'iroh://b070895d...?addr=127.0.0.1:51277'
# registered catalog entry notes -> iroh://b070895d...?addr=127.0.0.1:51277 (/root/.wanix/catalog/notes.json)
```

The built-in `upper` tool, named `upper`:

```sh
wanix-rust tool serve --tool upper --listen 127.0.0.1:0 --register upper
# upper	iroh://8c7a7009...?addr=127.0.0.1:49531
# ...
# registered catalog entry upper -> iroh://8c7a7009...?addr=127.0.0.1:49531 (...)
```

And the bundled chatroom ([Recipe 07](/recipes/07-chatroom-over-the-mesh)), named `room`:

```sh
wanix-rust app serve --app examples/chatroom --state /tmp/room \
    --listen 127.0.0.1:0 --register room
# chatroom	iroh://e1de10e6...?addr=127.0.0.1:51637
# ...
# registered catalog entry room -> iroh://e1de10e6...?addr=127.0.0.1:51637 (...)
```

(One endpoint registers as `NAME`; a serve announcing several — `tool serve --tool upper --tool sha256 --register text` — registers each as `NAME-<resource>`: executed, that run wrote `text-upper` and `text-sha256`.)

An entry is one pretty-printed JSON file, no daemon anywhere:

```sh
cat ~/.wanix/catalog/notes.json
```

```json
{
  "name": "notes",
  "description": "volume notes (registered at serve time)",
  "tags": [
    "volume"
  ],
  "address": "iroh://b070895d...?addr=127.0.0.1:51277"
}
```

## 2. `catalog ls` is names plus liveness

`catalog ls` probes every entry concurrently with one bounded dial each (the shared 5 s mount deadline) and renders `NAME  STATUS  ADDRESS`:

```sh
wanix-rust catalog ls
```

```text
notes	online	iroh://b070895d...?addr=127.0.0.1:51277
room	online	iroh://e1de10e6...?addr=127.0.0.1:51637
upper	online	iroh://8c7a7009...?addr=127.0.0.1:49531
```

Kill the `upper` serve (Ctrl-C its terminal) and list again — the *name* stays, the resource is honestly offline:

```text
notes	online	iroh://b070895d...?addr=127.0.0.1:51277
room	online	iroh://e1de10e6...?addr=127.0.0.1:51637
upper	offline	iroh://8c7a7009...?addr=127.0.0.1:49531
```

Restart it with the same `--register upper` and the entry is overwritten in place: same name, same `8c7a7009...` identity (the key *is* the resource), fresh `?addr=` route hint for the new port:

```text
upper	online	iroh://8c7a7009...?addr=127.0.0.1:54872
```

`catalog ls --no-probe` lists instantly with status `-`. `catalog add NAME IROH_URL`, `show NAME`, and `rm NAME` are the manual verbs; `add` refuses to overwrite without `--force`.

## 3. Mount by bare name

Anywhere a mount target is taken, a bare name now resolves through the catalog *at launch*, and each resolution is logged once on stderr — the audit trail that names are for humans while the dialed capability is the address:

```sh
wanix-rust sh --mount-mesh notes --mount-mesh room -c 'cat < /n/notes/welcome.txt'
```

```text
wanix-rust: name 'notes' -> iroh://b070895d...?addr=127.0.0.1:51277 (resolved through the catalog at launch)
wanix-rust: name 'room' -> iroh://e1de10e6...?addr=127.0.0.1:51637 (resolved through the catalog at launch)
welcome to the writing desk
```

A bare `NAME` mounts at the conventional `/n/NAME`; `--mount-mesh notes=/vol/notes` picks the guest path explicitly. The same spelling works on `wasm` and `qjs-shell`, and on the one-shot verbs:

```sh
wanix-rust mount-ls notes            # -> draft.txt  welcome.txt
wanix-rust mount-cat notes welcome.txt
wanix-rust mount-write notes todo.txt 'rotate the keys'
```

An unknown name fails with the fix spelled out (executed verbatim):

```text
$ wanix-rust mount-ls whiteboard
no catalog entry "whiteboard" (catalog /root/.wanix/catalog); add one with `wanix-rust catalog add whiteboard IROH_URL` or serve the resource with `--register whiteboard`
```

## 4. The room's verbs, by name

Mounting the room brings [its vocabulary](/concepts/bin-verbs) along — the chatroom ships `bin/post.js`, `bin/watch.js`, `bin/roster.js`, runnable as `room:CMD` and confined to exactly the room:

```sh
wanix-rust sh --mount-mesh room -c 'room:post hello from a verb'
wanix-rust sh --mount-mesh room -c 'echo piped through stdin | room:post'
wanix-rust mount-cat room latest
```

```text
{"at":1781105183456,"from":"desk (0f285641)","body":"hello from a verb"}
{"at":1781105202183,"from":"desk (0f285641)","body":"piped through stdin\n"}
```

(`desk` is a self-claimed nick set with `mount-write room nick desk`; the short hex is the verified principal — display sugar over key truth, exactly recipe 07's contract.)

`room:watch` tails the room's never-EOF `stream` file. On an interactive terminal it renders live; in a *captured* `sh -c` run the shell collects output and prints it when the command exits — which happens at stream EOF, i.e. when the room goes away. Executed: a watcher was started, two posts landed from other invocations, then the room serve was killed —

```sh
wanix-rust sh --mount-mesh room -c 'room:watch'    # parks, collecting
# ... meanwhile: room:post line one for the watcher
# ...            room:post line two for the watcher
# ... then the room serve is killed; the watcher exits 0 and prints:
```

```text
{"at":1781105714138,"from":"iroh:0f285641...","body":"line one for the watcher"}
{"at":1781105714383,"from":"iroh:0f285641...","body":"line two for the watcher"}
```

Both lines were delivered to the verb live (their timestamps are the post times); the *rendering* waited for the collected run to finish. `stream` carries the raw `iroh:<hex>` principal; `latest` renders the nick form. For a live-rendering tail from the host CLI, `wanix-rust mount-cat room stream --follow` is the streaming consumer.

## 5. Save the desk as a recipe

(Restart the room serve from step 4's kill first — same `--register room`, same `e1de10e6...` identity, fresh port; with `--state` kept, the history above survives the restart.)

A recipe is a saved mount+run composition — authored explicitly, never captured. `NAME` targets must resolve at save time; the resolved address is recorded as a drift-check `hint`:

```sh
wanix-rust recipe save writing-desk \
  --description 'notes + upper + room on one desk' \
  --mount notes --mount upper --mount room \
  --run 'tool /n/upper < /n/notes/draft.txt | room:post'
```

```text
saved recipe writing-desk -> /root/.wanix/recipes/writing-desk.recipe
bind notes -> /n/notes (hint iroh://b070895d...?addr=127.0.0.1:51277)
bind upper -> /n/upper (hint iroh://8c7a7009...?addr=127.0.0.1:54872)
bind room -> /n/room (hint iroh://e1de10e6...?addr=127.0.0.1:34876)
```

The file is small TOML you could have written by hand:

```toml
name = "writing-desk"
description = "notes + upper + room on one desk"
run = "tool /n/upper < /n/notes/draft.txt | room:post"

[[binds]]
target = "notes"
path = "n/notes"
hint = "iroh://b070895d...?addr=127.0.0.1:51277"
# ... [[binds]] for upper and room ...
```

`recipe run` resolves every bind through the catalog *now*, mounts them, and runs the line through `sh -c` — remote file through remote tool into the room, one line:

```sh
wanix-rust recipe run writing-desk
```

```text
wanix-rust: name 'notes' -> iroh://b070895d...?addr=127.0.0.1:51277 (resolved through the catalog at launch)
wanix-rust: name 'upper' -> iroh://8c7a7009...?addr=127.0.0.1:54872 (resolved through the catalog at launch)
wanix-rust: name 'room' -> iroh://e1de10e6...?addr=127.0.0.1:34876 (resolved through the catalog at launch)
job: /n/upper/jobs/j573273e80834a2ac
```

```sh
wanix-rust mount-cat room latest | tail -1
# {"at":1781105905105,"from":"desk (0f285641)","body":"MEET THE MESH AT NOON\n"}
```

A recipe with no `--run` line opens an interactive shell over its mounts, mirroring `sh`: on a tty that is a live session; on a piped or redirected stdin it executes the input line by line and exits at EOF. Extra words after `recipe run NAME --` are appended to the run line.

## 6. Drift warns loudly, then the catalog wins

Restart the `upper` serve (new port, entry overwritten) and rerun — resolution is launch-time, so the recipe follows the catalog and says so:

```text
wanix-rust: recipe "writing-desk" bind 'upper' DRIFTED since save: saved hint iroh://8c7a7009...?addr=127.0.0.1:54872, catalog now iroh://8c7a7009...?addr=127.0.0.1:40066 — using the catalog address
job: /n/upper/jobs/j2dfdaad6a69e467b
```

(A bind whose entry has *vanished* from the catalog falls back to the saved hint, also loudly.)

## 7. The fresh machine

Simulated with an alternate `$HOME` — an empty home with no Wanix state at all. First, the honest failure:

```text
$ HOME=/tmp/fresh wanix-rust recipe run writing-desk
no recipe "writing-desk" (recipes /tmp/fresh/.wanix/recipes); save one with `wanix-rust recipe save writing-desk --mount NAME[=PATH] ... [--run LINE]`
```

Names and compositions are plain files, so moving them is `cp`:

```sh
mkdir -p /tmp/fresh/.wanix
cp -r ~/.wanix/catalog  /tmp/fresh/.wanix/catalog
cp -r ~/.wanix/recipes  /tmp/fresh/.wanix/recipes
HOME=/tmp/fresh wanix-rust recipe run writing-desk
```

```text
wanix-rust: name 'notes' -> iroh://b070895d...?addr=127.0.0.1:51277 (resolved through the catalog at launch)
wanix-rust: name 'upper' -> iroh://8c7a7009...?addr=127.0.0.1:40066 (resolved through the catalog at launch)
wanix-rust: recipe "writing-desk" bind 'upper' DRIFTED since save: ... — using the catalog address
wanix-rust: name 'room' -> iroh://e1de10e6...?addr=127.0.0.1:34876 (resolved through the catalog at launch)
job: /n/upper/jobs/j83c1f9270c576f27
```

The whole namespace rebuilt from two copied directories. Be precise about what moved:

- **Transferred:** the names (catalog entries) and the composition (the `.recipe` file). Both are addresses and structure — public-ish facts.
- **Not transferred — identity.** The fresh home dialed as a *fresh principal*: `~/.wanix/dialer.key` was created on first dial, and the room attributes the new post to `(3c362be5)`, not `desk (0f285641)` (executed; both lines visible in `latest`). Nicks, retained tool jobs, and any per-principal state belong to the key, and the key deliberately does not ride along.
- **Not transferred — the data.** `/tmp/fresh/.wanix` has no `volumes/`: `notes` still lives on the serving machine and is reached over the mesh, which is the point.

## Troubleshooting

- **`no catalog entry "NAME"`** — exactly what it says; `catalog add NAME IROH_URL` or serve with `--register NAME`. Spelling decides routing: anything with a scheme, slash, or dot is never treated as a name.
- **`resource unreachable: peer ... did not answer within 5s`** — resolution never probes, so a name that resolves can still point at a dead serve; this is the ordinary ADR 0008 outage error at dial time. Check `catalog ls`.
- **`catalog add` refuses `cas:`/`local:` addresses** — v0 names address live `iroh://` resources only; other address kinds are reserved future work (the error says so).

## Cleanup

Ctrl-C the serves. Names and recipes persist under `~/.wanix/catalog/` and `~/.wanix/recipes/` (delete with `catalog rm NAME` / `rm ~/.wanix/recipes/NAME.recipe`); volume data under `~/.wanix/volumes/`.

## See also

- [Name your world](/learn/name-your-world) — the flow this recipe proves, including the verb-confinement transcript.
- [Bin verbs](/concepts/bin-verbs) — vocabulary arrives with the mount, confined by default.
- [Recipe 06](/recipes/06-compose-volume-and-tools) · [Recipe 07](/recipes/07-chatroom-over-the-mesh) — the same resources by pasted ticket (the no-catalog fallback).
- [Recipe 02](/recipes/02-mount-remote-peer) — naming a whole peer with `catalog add`.
