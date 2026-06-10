---
title: Host Files Are Not Ambient Authority
slug: concepts/host-not-ambient-authority
pageType: concept
oneLiner: Host directories enter Wanix only through an explicit rooted LocalFs mount with escape checks; there is no ambient host path in the default namespace.
audience: [developer]
tags: [namespace, trust-boundary, shipped, cli, caveat]
sourceRefs:
  - docs/adrs/0001-rust-native-wasmtime-runtime.md:36-37
  - rust-walkthrough.md:141-178
  - crates/wanix-cli/src/qjs_support/mod.rs:105-125
  - crates/wanix-fs/src/localfs.rs:34-50
  - crates/wanix-fs/src/localfs/paths.rs:9-40
  - crates/wanix-fs/src/traits.rs:207-231
seeAlso:
  - concepts/namespace-binding
  - concepts/wanix-backed-wasi
  - concepts/subtreefs-confine-to-prefix
  - concepts/capability-is-a-bind
prerequisites:
  - concepts/namespace-binding
usedInFlows: []
honestLimits:
  - A capsule snapshot does not store ambient host capabilities; you supply the same --mount again on resume.
  - The escape check rejects symlinks that resolve outside the mounted root, but it does not sandbox the host kernel; LocalFs is a path boundary, not a syscall jail.
---

# Host Files Are Not Ambient Authority

Host directories enter Wanix only through an explicit rooted `LocalFs` mount with escape checks; there is no ambient host path in the default namespace.

A Unix process inherits the whole filesystem the moment it starts: `/`, `/etc`, your home directory, every secret on the machine. That is *ambient authority* — power you hold by default, without anyone granting it. Wanix refuses that inheritance. A task's namespace starts empty of host paths, and a host directory becomes visible only because you named it on the command line. This page shows the mount happening, then names the boundary that makes it safe.

## Show it: one directory, no more

Create a host directory and put a file in it:

```sh
cargo build --package wanix-cli
alias wanix='./target/debug/wanix'

mkdir -p /tmp/wanix-host
printf 'native mount' > /tmp/wanix-host/input.txt
```

Run a JavaScript task that reads and writes through the mount (`rust-walkthrough.md:141-178`):

```sh
wanix qjs --mount /tmp/wanix-host=host examples/qjs-host-mount.js
```

```text
host input: native mount
host output: mounted output for native mount
```

The guest opened `host/input.txt` and wrote `host/output.txt`. Inspect the host side:

```sh
cat /tmp/wanix-host/output.txt   # -> mounted output for native mount
```

The flag is `--mount HOST=GUEST`. `/tmp/wanix-host` is the host directory; `host` is the *only* name the task sees for it. The task never receives a path to `/tmp`, to `/`, or to anything else on your disk. There is no `host/..` that climbs out, no second directory it can reach by guessing. It got exactly one room, because you opened exactly one door.

## Name it: a rooted LocalFs, bound into the namespace

What `--mount` does under the hood is two ordinary Wanix operations. First, `wanix-cli` builds a `LocalFs` rooted at the host directory (`crates/wanix-cli/src/qjs_support/mod.rs:105-125`):

```rust
let local = Arc::new(LocalFs::new(&mount.host_path)?);
task.bind(local, ".", mount.guest_path.as_str(), BindOptions::default())?;
```

`LocalFs::new` canonicalizes the host path and refuses anything that is not a directory (`crates/wanix-fs/src/localfs.rs:34-50`). That canonical path is the *root*, stored once. Every path the task later resolves is interpreted relative to it.

Second, `task.bind` attaches that `LocalFs` into the task's private namespace at `host` — the same binding mechanism every Wanix capability uses (see [namespace binding](/concepts/namespace-binding)). The host directory is not special; it is a `FileSystem` bound at a name, exactly like `#kv` or a remote peer. A mount *is* a [capability that is a bind](/concepts/capability-is-a-bind): the authority is the edge in the namespace graph, not an entry on an access-control list.

This is the rule ADR 0001 fixed at the start: "Host files enter Wanix only through explicit rooted mounts with escape checks; ambient host paths are not part of the default namespace" (`docs/adrs/0001-rust-native-wasmtime-runtime.md:36-37`).

## Guard it: confine_to_prefix rejects escapes

A rooted path string is not enough on its own. A symlink inside the mount could point at `/etc/passwd`, and re-rooting only rewrites the *string* — it would still resolve through the host's shared backing store and escape. So `LocalFs` overrides `confine_to_prefix`, the trait hook that exists precisely for backing stores that resolve symlinks (`crates/wanix-fs/src/traits.rs:207-231`). The default is a no-op `Ok(())` for filesystems like `MemFs` that store link targets opaquely; host-backed filesystems must do real work.

The work is canonicalize-then-compare (`crates/wanix-fs/src/localfs/paths.rs:9-40`):

```rust
let resolved = fs::canonicalize(&host_path)?;
if resolved.starts_with(&*self.root) {
    Ok(resolved)
} else {
    Err(FsError::PermissionDenied)
}
```

After the OS follows every symlink, the result must still start with the canonical root. If it does not, the open fails with `PermissionDenied`. A symlink named `secret-link` that points at a file outside the root is unreadable, and it does not even appear in `ls` of the mount. A broken symlink that *stays* inside the root is treated as in-bounds — the caller's real operation surfaces its own `NotFound`, which is honest rather than a false escape alarm. This is the same confinement primitive a re-rooting export uses; see [SubtreeFs: confine to prefix](/concepts/subtreefs-confine-to-prefix).

## Why this matters for agents and untrusted code

The payoff is leverage over what a task can touch. When you hand a directory to a [Wanix-backed WASI](/concepts/wanix-backed-wasi) guest, an agent, or a `.wasm` program, you are not trusting it to behave — you are arranging that *misbehavior has nowhere to go*. The guest cannot read a path you did not mount, because that path is not in its namespace and there is no ambient root to walk up to. Grant `--mount ~/project/src=src` and the worst a runaway task can do to your disk is scribble inside `~/project/src`. Authority is what you bound, nothing more.

## See also

- [Namespace binding](/concepts/namespace-binding) — the `bind` operation that attaches the `LocalFs` at a name.
- [A capability is a bind](/concepts/capability-is-a-bind) — why the mount edge, not an ACL, is the unit of authority.
- [SubtreeFs: confine to prefix](/concepts/subtreefs-confine-to-prefix) — the same `confine_to_prefix` check on the export side of the mesh.
- [Wanix-backed WASI](/concepts/wanix-backed-wasi) — the guest that sees only the namespace you built for it.
- [Walkthrough: run JS](/recipes/walkthrough-1-run-js) — the host-mount step in a runnable sequence.

## Status / honest limits

- **Mounts are not portable state.** A [capsule](/concepts/wanix-capsule) snapshot freezes a Wanix world's content, not your machine's live directories. It does not store an ambient host capability, so resuming a world supplies the same `--mount` again rather than re-deriving access. This is deliberate: a frozen world should not silently re-acquire host authority on a different machine.
- **LocalFs is a path boundary, not a syscall jail.** The escape check guarantees that path resolution stays inside the mounted root, including through symlinks (`crates/wanix-fs/src/localfs/paths.rs:27-40`). It does not isolate the host kernel, CPU, or memory; a `LocalFs` mount confines *which files* a task reaches, not *how much machine* it consumes. Exec devices remain local-trust only and there are no hard resource limits yet.
