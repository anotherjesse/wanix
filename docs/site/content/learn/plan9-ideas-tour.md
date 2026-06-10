---
title: The Plan 9 ideas tour
slug: learn/plan9-ideas-tour
pageType: flow
oneLiner: Walk the thesis as a proof — the missing half of 9P, the mechanism in code, and the trust boundary as one pure function.
audience: [visionary]
tags: [mesh, plan9, shipped, local-trust-only, caveat]
sourceRefs:
  - docs/mesh-the-missing-half-of-9p.md
  - crates/wanix-9p-client/src/remote.rs:4
  - crates/wanix-9p-client/src/remote.rs:42
  - crates/wanix-id/src/policy.rs:10
  - crates/wanix-9p/src/lib.rs:128-135
  - crates/wanix-cli/src/mount.rs:26
seeAlso:
  - concepts/missing-half-of-9p
  - concepts/remotefs-import-half
  - concepts/five-hostile-peer-corrections
  - concepts/capability-is-a-bind
  - concepts/attach-policy
  - concepts/wanix-as-host
  - concepts/tauth-is-enosys
prerequisites:
  - concepts/the-9p-contract
usedInFlows: []
honestLimits:
  - Per-principal namespaces, grant lifecycle, public multi-user auth, and ethernet/vnet are unimplemented trust-boundary work.
  - The shipped CLI mount binds a single slot /n/remote; per-peer /n/<peer-id> is designed but unshipped.
  - Exec devices are local-trust only, with no hard CPU or memory limits yet.
canonicalCaveatFor: []
---

# The Plan 9 ideas tour

Walk the thesis as a proof — the missing half of 9P, the mechanism in code, and the trust boundary as one pure function.

This flow is for the reader who wants to see *why* the three Wanix ideas compose, not just how to type them. It does not teach you a new command; it shows you that the commands you already know are one idea wearing three hats. Everything is a file. Each process composes its own namespace. And importing a *remote* namespace is the same operation as binding a local one. That last equivalence is the whole thesis, and below it is reduced to roughly two hundred lines of Rust and one pure function. Heritage is worn lightly: you do not need to have used Plan 9 to follow it.

## The thesis: the missing half of 9P

Plan 9's deepest idea is not "everything is a file." It is that the namespace is per-process and you can rearrange it. Two operations build a process's view of the tree: **export** hands a subtree to the network as a 9P service, and **import** splices a remote 9P service into *your* namespace at a path you choose. The convention for that path is `/n/` — `cat /n/lab/dev/mouse` reads a mouse on another machine, with no new API and no per-service client library, because one file-transport protocol (9P) plus one placement operation (bind) gives network transparency for free across every service at once (`docs/mesh-the-missing-half-of-9p.md`). (That `/n/lab` spelling is the Plan 9 design being described; the shipped Wanix CLI binds one mount slot at `/n/remote` — the honest-gaps section below owns this.)

For a long time Wanix had only half of that. It could export a namespace over 9P; it could not import one. It had the 9P server and not the 9P client. The mesh is the other half: `RemoteFs`, a synchronous `FileSystem` that speaks 9P to a remote server and presents the remote tree as ordinary local files. Bind it, and a remote namespace — regular files, `#term`, `#task`, `#kv`, all of it — becomes part of yours. Import realized. `/n/` is back. Read the idea in full on [the missing half of 9P](/concepts/missing-half-of-9p).

## Browser-as-host vs Wanix-as-host

The original Wanix ran inside Chrome. The Rust port inverts that: Wanix is the host and the microkernel, Wasmtime is the execution substrate, and the browser becomes a frontend — a [cockpit](/use-cases/browser-cockpit), not the runtime foundation. That is the move that lets a namespace cross *machines* instead of just tabs. See [Wanix as host](/concepts/wanix-as-host) for the boundary in detail.

Stated honestly: the Go Wanix remains broader in places — its in-browser device coverage and v86/editor integrations are more mature than this port's. The Rust thesis is not "we already do more"; it is "we do the load-bearing thing — import/export as one namespace operation, reaching across the open internet — on a substrate that runs outside a browser."

## The mechanism in code: RemoteFs is the photographic negative of serve_stream

Show, then name. (A two-line 9P glossary, since the protocol terms appear below: a client sends **T-messages** (requests) and the server answers with **R-messages**; a **fid** is the client-chosen handle naming a file across requests; a **tag** matches a reply to its request; `Rlerror` is the typed error reply. The full vocabulary lives in [the 9P contract](/concepts/the-9p-contract).) The server's hot loop, `serve_stream`, is strictly serial: read one T-message, dispatch, write one R-message, repeat. The client is that loop developed in reverse. `P9Conn::rpc` encodes one T-message, blocks reading frames until the matching tag returns, and surfaces an `Rlerror` as a typed error. Because the server answers one request at a time, the client keeps exactly one outstanding — `RemoteFs` holds the connection behind an `Arc<Mutex<P9Conn>>` (`crates/wanix-9p-client/src/remote.rs:42`), so concurrent callers serialize on the mutex rather than racing the wire. No pipelining, no tag-match races: the simplest correct thing, matched to the server's own shape (`crates/wanix-9p-client/src/remote.rs:4`). No new wire format, no async runtime, no transport — the client reuses the same `wanix-protocol` codecs the server uses, turned inside out. That is the whole of [the import half](/concepts/remotefs-import-half).

A first-draft 9P client is easy to write and easy to get dangerously wrong, because it imports a peer it does not control. The crate carries five corrections, each an adversarial verdict against the obvious version: a frame-size ceiling checked *before* allocating a body (so a hostile server's claimed 1 GiB frame cannot OOM the importer); honest seekability (a stream device refuses `seek` instead of inventing offsets); RAII fid/tag guards (no leaked fids over a long session); a bounded `read_dir` cookie loop (no infinite spin on a stalled server); and append delegated to the server (no client-side size-probe race). These are the difference between a client that demos and one you would trust against an untrusted node — [the five hostile-peer corrections](/concepts/five-hostile-peer-corrections).

## The trust boundary as one pure function

Import as first built was all-or-nothing: the server handed the entire tree to anyone who opened the socket. A mesh needs the opposite — *who* may import, and *how much*. The whole boundary funnels through one pure function (`crates/wanix-id/src/policy.rs:10`):

```rust
pub trait AttachPolicy: Send + Sync {
    fn evaluate(&self, peer: PeerId, aname: &str) -> Option<Authorization>;
}
```

It takes the verified peer and the requested attach name and returns either an `Authorization { root, rights }` to install or `None` to deny. The backing `GrantTablePolicy` is **default-deny**: a peer with no exactly matching `(peer, aname)` grant is refused; there is no implicit allow. Read it as [the attach policy](/concepts/attach-policy).

The headline is one sentence: a capability is a bind. A grant is not an ACL bolted onto the filesystem — it is a re-rooting of the namespace at a subpath, gated by rights. When a grant matches, it materializes a `SubtreeFs` whose `.` *is* the granted subtree; the peer attaches and stands inside it and cannot name a path outside, because in their namespace there is no outside (`confine_to_prefix` even re-confines symlink dereferences so a planted `../../` link cannot escape). You do not filter a shared tree; you hand out *different trees*, and the namespace does the confinement for free. This is [a capability is a bind](/concepts/capability-is-a-bind).

Three facts make this exact rather than aspirational. The key is the address: a node is named by its ed25519 public key, and the same 32 bytes that authorize it let iroh find it across NATs — "mount this node" and "trust this node" are the same act on the same bytes. `Tauth` (9P's in-band authentication request) stays **ENOSYS** forever ([Tauth is ENOSYS](/concepts/tauth-is-enosys)): the QUIC handshake authenticates the peer's key in the transport, so there is nothing left for an in-band 9P auth message to do; the server never trusts a client-claimed `uname` (the username string a 9P attach carries). And the v1 single-attach simplification is stated in the code itself — the most recent authorized attach defines the connection's root, with per-fid scoping for multiple concurrent attaches deferred to a later slice (`crates/wanix-9p/src/lib.rs:128-135`).

## The full Plan 9 cast, slice by slice

Because every device is a plain `FileSystem`, the rest of Plan 9 imports for free over the same wire:

- **factotum → identity + grants.** Keys held by the node, auth a property of the connection. [Persisted ed25519 identity](/concepts/persisted-ed25519-identity).
- **venti → content-addressed blobs.** Write-once, hash-verified data exposed as files. [`#cas`](/devices/cas).
- **cpu → send the agent to the data.** Run compute on the peer that holds the files, against the caller's reverse-exported namespace. [`#cpu`](/devices/cpu).
- **plumber → gossip.** Pattern-routed messages that span nodes. [`#plumb`](/devices/plumb).

The agent is the operator these mechanisms always wanted. Per-process namespaces were historically *underused* because the ergonomic cost of constantly reshaping a private world fell on a human who wanted their shell to stay put. An LLM has no such reluctance: it reads state by `cat`, mutates by `write`, lists capability by `ls`. The mesh hands that operator the wider world — [agents as operators](/concepts/agents-as-operators).

## The honest gaps

The mesh ships the keystone; it does not yet ship the hardened multi-user edge. Be precise about it: per-principal namespaces, grant lifecycle, public multi-user auth, and ethernet/vnet remain unimplemented trust-boundary work (`docs/mesh-the-missing-half-of-9p.md`). The shipped CLI mount binds a single slot — the constant `MOUNT_POINT` is `n/remote` (`crates/wanix-cli/src/mount.rs:26`); the per-peer `/n/<peer-id>` the design promises is unshipped, so use `/n/<peer>` only as a labelled convention. And the exec devices (`#task`, `#agent`, `#cpu`) are local-trust only — cheap, scalable isolation, *not* a sandbox safe for arbitrary untrusted code, with no hard CPU or memory limits yet.

## See also

- Concepts: [the missing half of 9P](/concepts/missing-half-of-9p) · [the RemoteFs import half](/concepts/remotefs-import-half) · [five hostile-peer corrections](/concepts/five-hostile-peer-corrections) · [a capability is a bind](/concepts/capability-is-a-bind) · [the attach policy](/concepts/attach-policy) · [Tauth is ENOSYS](/concepts/tauth-is-enosys) · [Wanix as host](/concepts/wanix-as-host)
- Devices: [`#cpu`](/devices/cpu) · [`#cas`](/devices/cas) · [`#agent`](/devices/agent) · [`#plumb`](/devices/plumb)
- Flows: [wire a mesh](/learn/wire-a-mesh) · [the agent on your files](/learn/agent-on-your-files) · back to [all flows](/learn/index)
- Reference: [the ADR index](/reference/adr-index) · [the trust-boundary gaps](/concepts/trust-boundary-gaps)

## Status / honest limits

- Per-principal namespaces, grant lifecycle, public multi-user auth, and ethernet/vnet are unimplemented trust-boundary work.
- The shipped CLI mount binds a single slot `/n/remote` (`crates/wanix-cli/src/mount.rs:26`); per-peer `/n/<peer-id>` is designed but unshipped — treat `/n/<peer>` as a labelled convention only.
- The exec devices `#task`/`#agent`/`#cpu` are local-trust only: cheap, scalable isolation, not a sandbox safe for arbitrary untrusted code, with no hard CPU or memory limits yet.
- `Tauth` is ENOSYS — there is no in-band 9P auth handshake; identity comes from the QUIC transport.
