---
title: Trust-Boundary Gaps (Do Not Overclaim)
slug: concepts/trust-boundary-gaps
pageType: concept
oneLiner: The honest "not yet" inventory — exec devices are local-trust only, the uname/aname seam is discarded, there is no public multi-user auth and no ethernet/vnet, a #cpu cancel does not stop remote computation, and the blob plane is upstream-experimental.
audience: [developer, visionary]
tags: [mesh, trust-boundary, caveat, local-trust-only, exploratory, shipped]
sourceRefs:
  - AGENTS.md:240-247
  - AGENTS.md:357-380
  - crates/wanix-cli/src/mesh/serve.rs:33-149
  - crates/wanix-9p/src/session.rs:31-74
  - crates/wanix-cpu/src/lib.rs:33-41
  - crates/wanix-cli/src/mount.rs:26-29
seeAlso:
  - concepts/attach-policy
  - concepts/tauth-is-enosys
  - concepts/single-frame-serve-caveat
  - concepts/safe-for-untrusted-not-claimable
  - concepts/fakeengine-vs-codex
prerequisites:
  - concepts/attach-policy
  - concepts/tauth-is-enosys
usedInFlows: []
honestLimits:
  - The mesh/agent layer has no ADRs yet; its contracts are fast-moving, not frozen API.
  - Exec devices (#task/#agent/#cpu) are local-trust only and have no hard CPU/memory limits.
  - There is no public multi-user auth and no ethernet/vnet; Tauth stays ENOSYS.
  - handle_attach discards uname/aname; every fid resolves through one shared P9Server.root.
  - A #cpu Cancel stops the caller draining; it does not abort the remote guest.
  - The shipped CLI mount binds a single slot /n/remote; per-peer /n/<peer> is unshipped.
canonicalCaveatFor: [trust-boundary-gaps]
---

# Trust-Boundary Gaps (Do Not Overclaim)

The honest "not yet" inventory — exec devices are local-trust only, the `uname`/`aname` seam is discarded, there is no public multi-user auth and no ethernet/vnet, a `#cpu` cancel does not stop remote computation, and the blob plane is upstream-experimental.

The rest of the documentation shows you what Wanix *does*. This page is the opposite discipline: a flat catalogue of where the trust boundary is unfinished, written so that a careful reader never mistakes a designed-but-unshipped seam for a shipped guarantee. Every limit here is an engineering boundary the code states about itself, with the line that states it. None of it is an apology — it is the contour of what you can safely build on today.

## The headline: refuse first, then explain

Run a public mesh export with no grants and watch it refuse, not serve:

```sh
cargo build --package wanix-cli
alias wanix-rust='./target/debug/wanix-rust'

wanix-rust mesh-serve --root /tmp/world
# error: mesh-serve on a non-loopback endpoint (public, or a LAN --addr whose
# node id mDNS advertises) with no --peer/--grant exports the entire root
# read-write to anyone who can reach it; pass --peer HEX with --grant to gate
# access, --addr 127.0.0.1:PORT to serve a loopback-only endpoint, or
# --insecure-open to deliberately export it open
```

That refusal is the shape of the whole trust story: an iroh `Endpoint` is reachable by any NodeID that holds the ticket (and a LAN `--addr` is mDNS-discoverable by any LAN host), so default-deny is enforced at parse time (`crates/wanix-cli/src/mesh/serve.rs`). The boundary is real, and so are its holes. Below is the inventory.

## No ethernet/vnet, no public multi-user auth

Two whole categories are explicitly unbuilt. There is no ethernet/vnet plane and no public/multi-user authentication; both are named as "explicitly unimplemented trust-boundary work" (`AGENTS.md:246-247`). The 9P auth message that would carry a session credential — `Tauth` — is wired to refuse: `handle_auth` decodes the frame and replies `ENOSYS` (`crates/wanix-9p/src/session.rs:71-74`). There is no auth handshake at the protocol layer, by design, until that work lands. See [Tauth is ENOSYS](/concepts/tauth-is-enosys) for the full reasoning. The mesh's actual gate is the iroh-verified ed25519 peer identity plus a default-deny grant table — capability, not session credential. See [attach policy](/concepts/attach-policy).

## Exec devices are local-trust only

The most important boundary on this page: the exec devices `#task`, `#agent`, and `#cpu` run guest code, and that capability is **never** handed to an untrusted or public peer. `mesh-serve --wanix-services` binds the exec devices into the served namespace, and the parser refuses it on any non-loopback endpoint outright — even with `--peer`/`--grant`, even with `--insecure-open` — because a grant's backing is the same services namespace, so a granted open serve would still expose `#task` (`crates/wanix-cli/src/mesh/serve.rs`). The only path that allows services export is a loopback `--addr 127.0.0.1:PORT`: a socket no other host can reach (a LAN `--addr` does not qualify — mDNS advertises the NodeID to the local network). And `--insecure-open` is deliberately *not* a backdoor — it exports the host directory read-write only, never the exec devices, and the announcement says so (`crates/wanix-cli/src/mesh/serve.rs`).

State the positive claim precisely. Wanix gives you cheap, scalable isolation for code you already trust; it is **not** "safe for arbitrary untrusted code." There are no hard CPU or memory limits on a running guest yet. See [safe for untrusted is not yet claimable](/concepts/safe-for-untrusted-not-claimable). And on the served path the agent is a deterministic `FakeEngine`, not a live LLM — real codex is the local-trust `wanix agent` CLI path only ([FakeEngine vs codex](/concepts/fakeengine-vs-codex)).

## The session/namespace seam is discarded

A 9P `Tattach` carries `uname` (who is attaching) and `aname` (which tree). Wanix decodes both and, in the plain path, throws them away: `handle_attach` resolves every fid through one shared `P9Server.root` (`crates/wanix-9p/src/session.rs:31-49`). There is no per-principal namespace on a single served endpoint. The mesh path does scope by `aname` — `install_attach_root` consults the attach policy and installs a re-rooted tree when a grant matches, denying with `EACCES` otherwise (`crates/wanix-9p/src/session.rs:58-69`) — but that is the *capability bind* (a grant is a re-rooted `SubtreeFs`, not an ACL), not multi-user identity. A per-principal `NamespaceProvider` is designed but deliberately unshipped: adding the trait as a no-op seam would be a dead abstraction, so it waits for per-fid root storage and a first real consumer (`AGENTS.md:376-380`).

The shipped CLI mount makes the same point at the import half. `wanix-rust mount-*` dials one server and binds the remote at a single fixed slot, `/n/remote` (`crates/wanix-cli/src/mount.rs:26-29`). Per-peer `/n/<peer-id>` addressing is the designed convention you will see in mesh prose, but the shipped binary mounts one slot. Use `/n/<peer>` only as a labelled convention, not as something the CLI produces today.

## Single-frame serve, cancel that does not cancel, experimental blobs

Three more concrete limits round out the inventory.

**Single 9P frame at a time per connection.** The served websocket handles one frame per connection, so a blocking read cannot interleave with a write on the same connection (`AGENTS.md:242-246`). A blocking `#plumb/<topic>/recv` therefore cannot run alongside a publish on that connection — live pub/sub needs a second connection or concurrent frame handling. The cockpit self-check probes only the publish path for exactly this reason (`AGENTS.md:357-362`). Full detail lives in the [single-frame serve caveat](/concepts/single-frame-serve-caveat).

**`#cpu` cancel stops draining, not the guest.** The CPU control protocol carries a `Cancel` event, but the task driver has no abort hook, so a cancel stops the *caller* draining the control stream — it does not stop the remote computation running on the acceptor (`crates/wanix-cpu/src/lib.rs:33-41`). The asymmetry is documented, not faked. v1 also delivers stdout/stderr/exit as a single batch after `start` returns, not incrementally. Real remote cancellation and incremental streaming are named follow-ups.

**Blob plane is upstream-experimental.** The content-addressed data plane (venti, `#cas`) rides `iroh-blobs`, which upstream self-describes as not production quality; the blueprint treats the bulk plane as experimental on that basis (`docs/mesh-blueprint.md:183`). End-to-end hash verification holds; production-grade durability does not yet.

## See also

- [Attach policy](/concepts/attach-policy) — the default-deny grant table that gates a mesh attach by verified peer and `aname`.
- [Tauth is ENOSYS](/concepts/tauth-is-enosys) — why the 9P auth handshake is deliberately absent.
- [Single-frame serve caveat](/concepts/single-frame-serve-caveat) — the one-frame-per-connection limit and what it blocks.
- [Safe for untrusted is not yet claimable](/concepts/safe-for-untrusted-not-claimable) — the precise scope of exec-device isolation.
- [FakeEngine vs codex](/concepts/fakeengine-vs-codex) — why the served `#agent` is deterministic, not a live LLM.

## Status / honest limits

- The mesh/agent layer has **no ADRs yet** (`AGENTS.md` ADR index). Its contracts — 9P-over-QUIC transport, identity, the `#agent` device, the service-device shapes — are fast-moving and not frozen API. Treat everything on this page as current behaviour, not a guarantee.
- Exec devices (`#task`/`#agent`/`#cpu`) are local-trust only and have no hard CPU/memory limits; they are not exposed to untrusted or public peers.
- There is no public multi-user auth and no ethernet/vnet. `Tauth` stays `ENOSYS`.
- `handle_attach` discards `uname`/`aname` in the plain path; per-principal namespaces are unshipped.
- A `#cpu` `Cancel` stops the caller draining; it does not abort the remote guest.
- The shipped CLI mount binds a single slot `/n/remote`; per-peer `/n/<peer>` is a labelled convention only.
