---
title: Name your world — the self-extending platform
slug: learn/name-your-world
pageType: flow
oneLiner: Names are for humans and resolve at launch from YOUR catalog; verbs are vocabulary that arrives with a mount and runs confined to the resource it came from — so the platform extends itself one served resource at a time.
audience: [newcomer, developer, visionary]
tags: [mesh, cli, catalog, names, shell, trust-boundary, shipped, caveat]
sourceRefs:
  - crates/wanix-cli/src/catalog.rs
  - crates/wanix-cli/src/recipe.rs
  - crates/wanix-cli/src/verb_bin.rs
  - crates/wanix-cli/src/app/serve/verb_tests.rs
  - crates/wanix-cli/src/sh/both_kinds_tests.rs
  - crates/wanix-fs/src/verbs.rs
  - crates/wanix-task/src/task/confine.rs
  - examples/chatroom/bin/post.js
  - examples/chatroom/bin/watch.js
  - docs/adrs/0007-resources-catalogs-and-pairing.md
seeAlso:
  - recipes/10-name-your-world
  - concepts/bin-verbs
  - concepts/key-is-the-address
  - concepts/capability-is-a-bind
  - concepts/guest-defined-resources
  - concepts/wanix-sh
  - learn/compose-volumes-and-tools
  - learn/build-a-chatroom
prerequisites:
  - concepts/key-is-the-address
  - concepts/bin-verbs
usedInFlows: []
honestLimits:
  - "The catalog is Layer 1 naming only (ADR 0007): no ACLs, no shared registry, no petname exchange yet — your names are private to your ~/.wanix, and giving a friend a name still means sending them the ticket."
  - "Resolution never probes; a renamed/rebound entry affects the NEXT launch (specs carry resolved addresses), and a stale address fails at dial time with the ADR 0008 outage error."
  - "Verb widening (--allow) is deliberately unimplemented: a verb's world is exactly its resource at /res, full stop."
  - "Verbs must be self-contained: the confined namespace contains only /res, so a .js verb cannot import libraries from outside its resource."
canonicalCaveatFor: []
---

# Name your world — the self-extending platform

Names are for humans and resolve at launch from *your* catalog; verbs are vocabulary that arrives with a mount and runs confined to the resource it came from — so the platform extends itself one served resource at a time.

Two mechanisms make a mesh of machines feel like one livable computer, and this flow shows both running: **names** (the catalog) and **vocabulary** (bin verbs). Neither adds a daemon, a registry service, or an RPC scheme — a name is a JSON file in `~/.wanix/catalog/`, and a verb is a script in the served resource's `bin/`. One machine, loopback, about twenty minutes. Every transcript below ran verbatim.

```sh
cargo build --locked --package wanix-cli
alias wanix='./target/debug/wanix'
```

## 1. Names are for humans; resolution is launch-time

The catalog is *your address book*, not a directory service: `wanix catalog add NAME IROH_URL` (or `--register NAME` on any resource serve) binds a humane name to a dialable ticket, locally, under your `~/.wanix`. Nobody else sees your names; two people can call the same room different things — which is exactly the petname-system shape ADR 0007 grows toward (today: Layer 1, your private edge names; later: exchanging and delegating them).

Three rules carry the design:

- **Spelling is routing.** A name is `lowercase [a-z0-9-]`, alphanumeric at both ends. A ticket has a scheme, a path has a slash or dot — so a bare name can never be confused with either, and every mount surface (`--mount-mesh`, `mount-ls/cat/write`, recipe binds, `serve --bind`) routes on spelling alone.
- **Resolution is launch-time, audited.** A name resolves through the catalog when the command starts — never silently in the background — and each resolution prints once on stderr:

  ```text
  wanix: name 'notes' -> iroh://b070895d...?addr=127.0.0.1:51277 (resolved through the catalog at launch)
  ```

  Rebind a name and the *next* launch follows it; running namespaces carry the address they resolved. (`recipe run` makes drift loud: "bind 'upper' DRIFTED since save ... — using the catalog address".)
- **The name is sugar; the key is the address.** Whatever the catalog says, the dial still authenticates the peer's ed25519 key — a wrong or stale entry can misroute your *attempt*, never your *trust* ([the key is the address](/concepts/key-is-the-address)).

The full transcript — `--register` from three serves, `catalog ls` liveness, mount-by-name, recipes, and the fresh-machine rebuild — is [Recipe 10](/recipes/10-name-your-world).

## 2. Vocabulary arrives with the mount

A served resource can ship the commands that know how to use it: executable verbs in a `bin/` directory beside its tree ([bin verbs](/concepts/bin-verbs)). Mount the chatroom and your shell can speak chatroom:

```sh
wanix sh --mount-mesh room -c 'room:post hello from a verb'
wanix sh --mount-mesh room -c 'echo piped through stdin | room:post'
```

`room:post` is `bin/post.js` *served by the room itself* — the platform does not know what "post" means; the room does, and mounting the room teaches your shell. The input convention composes: argv joined is the body, no argv reads stdin.

Now the self-extending move. The room is just files, so teaching it a new verb is dropping a file in `bin/`. Copy the bundled room and add `peek`:

```sh
cp -r examples/chatroom /tmp/myroom
cat > /tmp/myroom/bin/peek.js <<'EOF'
// room verb: read a path from inside the verb's own confined world.
import * as std from "qjs:std";
const path = scriptArgs[1] ?? "/res/latest";
const text = std.loadFile(path);
if (text === null) {
  std.err.puts(`peek: ${path}: not found in this verb's world\n`);
  std.exit(1);
}
std.out.puts(text);
EOF
wanix app serve --app /tmp/myroom --state /tmp/room --listen 127.0.0.1:0 --register room
```

Everyone who mounts your room now has `room:peek` — no client update, no plugin install, no version negotiation. The verb travels with the resource because it *is* part of the resource:

```sh
wanix mount-ls room bin
# peek.js  post.js  roster.js  watch.js
wanix sh --mount-mesh room -c 'room:peek /res/latest'
# {"at":1781105714138,"from":"desk (0f285641)","body":"line one for the watcher"}
# ...
```

## 3. Confinement is the trust story

That convenience would be horrifying if a mounted verb ran with your authority — the code came from someone else's machine. It does not. The shell launches a verb as a child task and writes `confine <mount-path>` to its `#task` ctl before starting it: the child's namespace contains EXACTLY the resource at `/res`, plus stdio and the argv/env you passed. Not your home directory, not your other mounts, not `#task` — those paths do not exist in its world ([a capability is a bind](/concepts/capability-is-a-bind), not a check).

`peek` is the proof, executed. The invoking shell reads a local file freely; the room's own verb, asked for the same file, has no path that names it:

```sh
ls
# secret.txt
wanix sh --mount-mesh room -c 'cat secret.txt'
# meeting notes: the demo is at noon
wanix sh --mount-mesh room -c 'room:peek secret.txt'
# peek: secret.txt: not found in this verb's world      <- stderr, exit status 1
```

Same shell, same mount, same invocation shape — the only difference is *whose code* runs, and the code that arrived with the mount gets the mount and nothing else. There is also deliberately **no PATH merging**: a hostile room cannot squat `ls`; its verbs only ever run spelled `room:...`. And widening is not a flag you can pass today — the future shape is an explicit `--allow`, never a default.

## 4. Both kinds, one shell

Verbs and externals come in two kinds — interpreted `.js` and compiled `.wasm` — and both are first-class from inside the shell (ADR 0002), down to mixed pipelines. Executed, from a directory holding a three-line qjs producer:

```sh
cat > gen.js <<'EOF'
import * as std from "qjs:std";
std.out.puts("[1,2,3]\n");
EOF
wanix sh -c "gen.js | jaq 'map(.+1)'"
# [2,3,4]
```

A `.js` stage feeding a `.wasm` stage through a real `#pipe`: same resolution, same `#task` dispatch, same fd contract for both task kinds. The room's verbs above are `.js`; the test fixtures prove the `.wasm` half (`crates/wanix-cli/src/sh/both_kinds_tests.rs`).

## 5. Where this is going

Put the pieces together and the trajectory is visible: resources serve themselves under *your* names, arrive with their own vocabulary, run that vocabulary confined by construction, and compose into saved recipes that rebuild a whole working namespace from two copied directories ([Recipe 10 §7](/recipes/10-name-your-world)). What's deliberately not here yet — the honest edge: name *exchange* (petnames between people), ACLs above Layer 1, verb widening, and per-user web principals at the gateway. Each is recorded follow-up work, not an implied capability.

## See also

- [Recipe 10 — name your world](/recipes/10-name-your-world) — the full executed transcript.
- [Bin verbs](/concepts/bin-verbs) · [Guest-defined resources](/concepts/guest-defined-resources) — the vocabulary mechanism and the resource model it rides.
- [The key is the address](/concepts/key-is-the-address) · [A capability is a bind](/concepts/capability-is-a-bind) — why names stay sugar and confinement stays structural.
- [Compose volumes and tools](/learn/compose-volumes-and-tools) · [Build a chatroom](/learn/build-a-chatroom) — the resources this flow names.
