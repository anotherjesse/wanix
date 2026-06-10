---
title: Bin Verbs
slug: concepts/bin-verbs
pageType: concept
oneLiner: Mounting a resource means gaining its vocabulary — a resource ships executable verbs in bin/, and the shell runs them confined to exactly the resource they came from.
audience: [developer, visionary]
tags: [mesh, shell, apps, trust-boundary, shipped, caveat]
sourceRefs:
  - docs/adrs/0007-resources-catalogs-and-pairing.md
  - crates/wanix-fs/src/verbs.rs
  - crates/wanix-task/src/task/confine.rs
  - crates/wanix-sh/src/verbs.rs
  - crates/wanix-cli/src/verb_bin.rs
  - crates/wanix-cli/src/app/serve/verb_tests.rs
  - examples/chatroom/bin/post.js
seeAlso:
  - concepts/guest-defined-resources
  - concepts/host-not-ambient-authority
  - concepts/capability-is-a-bind
  - concepts/wanix-sh
prerequisites:
  - concepts/guest-defined-resources
  - concepts/wanix-sh
honestLimits:
  - "The confined-child seam is driver-agnostic and both verb kinds launch from `wanix-rust sh` (the CLI shell registers the wasm and qjs task drivers; `.js` and `.wasm` verbs are both proven end to end)."
  - "Widening — granting a verb anything beyond its own resource — is deliberately unimplemented; the future shape is an explicit `--allow`-style flag, never a default."
  - "Verbs must be self-contained: a confined namespace contains only /res, so a .js verb cannot import shared libraries from outside the resource."
---

# Bin Verbs

A resource is not just data — it can ship the commands that know how to use
it. A served resource (an app, a tool) puts executable verbs in a `bin/`
directory beside its tree: the chatroom ships `bin/post.js`, `bin/watch.js`,
and `bin/roster.js`. The directory is host-served, read-only, flat, and
size-capped (`wanix_fs::VerbBinFs`) — never routed through the app's guest.

Mount the room and you have its vocabulary:

```sh
wanix-rust sh --mount-mesh room -c 'room:post hello from the mesh'
```

`NAME:CMD` is the whole grammar. `NAME` must be a current mount (`/n/NAME` or
`/vol/NAME`); `CMD` must resolve to exactly one of `bin/CMD.js` or
`bin/CMD.wasm` under it. There is deliberately **no PATH merging**: a mounted
resource's `bin/` never adds unqualified command names to your shell, so a
hostile mount cannot squat `ls` or `post` — its verbs only ever run spelled
with its own name in front.

## Confined by default

The verb's code comes from someone else's machine, so it runs with authority
over exactly the resource it came from — nothing else:

- The shell launches the verb as an ordinary child task, then writes
  `confine <mount-path>` to the child's `#task` ctl before starting it.
- The child's namespace then contains EXACTLY the resource bound at `/res`,
  plus the stdio fds and argv/env the shell passed explicitly. No `#task`, no
  `#pipe`, no other mounts, no host directories.
- The verb's bytes are read through the mount itself, so the program and the
  authority it gets arrive together: `room:post` literally cannot read your
  home directory, because there is no path in its namespace that names it.

The confinement is the test: in the shipped proof, a probe verb reads
`/res/...` fine and gets an honest not-found for a file the invoking shell
reads freely (`crates/wanix-cli/src/app/serve/verb_tests.rs`).

## Pipelines and conventions

A verb is wired like any external command — pipelines, redirects, `$?`:

```sh
echo hi | room:post        # input convention: no argv -> body from stdin
room:post hello world      # argv joined as the message body
room:roster                # pretty-prints the nick roster
```

This is the self-extending platform move: the platform does not know what
"post" means — the room does, and mounting the room teaches your shell.
