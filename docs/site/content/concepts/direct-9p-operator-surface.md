---
title: Direct-9P Operator Surface (No Side Channel)
slug: concepts/direct-9p-operator-surface
pageType: concept
oneLiner: Every cockpit operation is a 9P file read or write through WanixP9Handle; there is no MessagePort/CBOR bridge and no globalThis.Wanix in the Rust-hosted path.
audience: [developer]
tags: [cockpit, browser, 9p, shipped, caveat]
sourceRefs:
  - workbench/src/wanix/p9.ts:48-441
  - workbench/src/wanix/p9-session.ts:68-272
  - workbench/src/web/extension.ts:2013-2061
  - docs/adrs/0005-serve-and-client-handoffs.md:44-49
seeAlso:
  - concepts/browser-cockpit
  - concepts/live-stream-vs-one-shot
  - concepts/discovery-document
  - concepts/the-9p-contract
prerequisites:
  - concepts/browser-cockpit
  - concepts/the-9p-contract
usedInFlows: []
honestLimits:
  - The legacy MessagePort/CBOR bridge still exists, but it is browser-embedded-Wanix compatibility only, not the Rust-hosted path.
  - readDir/readFile/writeFile drain to EOF or issue a create, so live service streams (#term, #pipe, #plumb, #agent events) take a separate streaming code path.
  - serve handles one 9P frame at a time per connection, so a blocking recv cannot interleave with a write on the same connection.
canonicalCaveatFor: []
---

# Direct-9P Operator Surface (No Side Channel)

Every cockpit operation is a 9P file read or write through `WanixP9Handle`; there is no MessagePort/CBOR bridge and no `globalThis.Wanix` in the Rust-hosted path.

The browser cockpit is a VS Code / Code OSS web extension, and it would be easy to assume it talks to the Rust runtime through some bespoke editor RPC. It does not. When you boot `wanix-rust serve --bundle workbench-fs9p --wanix-services`, the cockpit opens one WebSocket to the served 9P endpoint and does *everything* — browse the tree, edit a file, drive a task, inspect a device — as 9P `walk`/`open`/`read`/`write`/`clunk` exchanges. The rule is worth stating flatly: in the Rust-hosted path there is no second channel. Open a file, read it, write it. That is the whole API surface, and it is the same one a native shell or a remote peer uses.

## Show it: one handle, file verbs all the way down

Every editor gesture lands on a method of `WanixP9Handle` (`workbench/src/wanix/p9.ts:48-441`), and each method is a small 9P script:

- `readDir(name)` walks to the path, `open`s it read-only, and pages through `readdir` until a page comes back empty — that is how the file explorer fills a folder (`p9.ts:83-111`).
- `readFile(name)` walks, opens read-only, and loops `read` at advancing offsets, 64 KiB at a time, until EOF — that is opening a buffer in the editor (`p9.ts:143-173`).
- `writeFile(name, data)` first tries to truncate-and-write an *existing* fid, and falls back to a `Tlcreate` in the parent directory if the file is new — that is saving a buffer (`p9.ts:229-248, 375-386`).
- `makeDir` is a `Tmkdir`, `rename` is a `Trenameat`, `symlink` is a `Tsymlink`, `removeAll` recurses and `unlink`s — folder create, rename, drag, and delete (`p9.ts:113-126, 259-274, 189-197, 323-336`).

`copy` is the tell that there is no side channel: copying a file is literally `writeFile(dst, await readFile(src))`, and copying a directory recurses through `readDir` (`p9.ts:276-315`). The cockpit has no privileged "copy" RPC because it does not need one — it composes the file verbs it already has. *That* composition, after you have seen it work, is the Plan 9 idea: the operator surface and the wire protocol are the same surface.

## The bootstrap hands the route in; the extension never guesses

The cockpit does not hard-code where the 9P endpoint lives. `createWanixHandle` (`workbench/src/web/extension.ts:2013-2061`) waits briefly for an embedder to push a config over a `MessageChannel`; if a direct-9P route arrives (`pendingConfig.p9?.websocket`) it builds the handle from that route, and otherwise it falls back to fetching `/.well-known/wanix.json` and reading `discovery.routes.p9` (`p9.ts:62-77`). Either way the WebSocket URL comes from the serve discovery document, not from a constant baked into the extension. This is the contract ADR 0005 pins down: Rust-hosted workbench integrations "discover serve, browse or mutate `wanix:/` over direct 9P" rather than hard-coding route assumptions (`docs/adrs/0005-serve-and-client-handoffs.md:44-49`).

## Negotiate Google.2 when it is there; degrade when it is not

On connect, the session proposes the richest protocol it knows — `9P2000.L.Google.2` by default — and records whatever the server echoes back (`p9-session.ts:68-78, 236-245`). The payoff is `stat`: when the negotiated version is Google.2 (or any `9P2000.L.Google.N` with `N >= 2`), `walkGetAttrPath` issues a single `Twalkgetattr`, fusing the walk and the attribute fetch into one round trip; otherwise it falls back to a plain `Twalk` followed by a separate `Tgetattr` (`p9-session.ts:98-125, 263-272`). The file explorer gets one RPC per entry instead of two when the server supports it, and stays correct when it does not.

The attributes that come back carry `isSymlink`, and the handle preserves it: `stat` reports `IsSymlink` and `readDir` marks directories with a trailing slash, so the editor can render a symlink as a symlink and a folder as a folder (`p9.ts:213-227, 83-86`). Symlink fidelity survives because it rides the 9P attribute, not an out-of-band hint.

## See also

- [Browser cockpit](/concepts/browser-cockpit) — the operator surface this handle drives.
- [Live stream vs one-shot](/concepts/live-stream-vs-one-shot) — why `#term`/`#pipe`/`#plumb`/`#agent` streams take a different code path than `readFile`/`writeFile`.
- [The discovery document](/concepts/discovery-document) — where the 9P route the bootstrap hands in comes from.
- [The 9P contract](/concepts/the-9p-contract) — the frame and codec contract every method above is built on.

## Status / honest limits

- **The MessagePort/CBOR bridge is compatibility only.** The legacy bridge for browser-*embedded* Wanix still exists, but it is not the Rust-hosted direction; the Rust-hosted cockpit talks direct 9P, full stop (`docs/adrs/0005-serve-and-client-handoffs.md:44-49`).
- **One-shot helpers are not for live streams.** `readFile` drains to EOF and `writeFile` issues a create against allocator-owned files; both would misbehave on a live subscription. The handle routes `#term/*`, `#pipe/*/data`, `#plumb/*/send|recv`, and `#agent/*/events` through a separate streaming path that walks and opens an existing fid and reads/writes at offset 0 (`p9.ts:338-471`). See [live stream vs one-shot](/concepts/live-stream-vs-one-shot).
- **Single-frame serve.** The served WebSocket handles one 9P frame at a time per connection, so a blocking `#plumb/<topic>/recv` cannot interleave with a write on the same connection; live pub/sub needs a second connection. See [the single-frame serve caveat](/concepts/single-frame-serve-caveat).
- **No 9P auth handshake.** Attach sends `uname`/`aname` and no Tauth; authentication over 9P stays ENOSYS (`p9-session.ts:247-254`). The served exec devices are local-trust only.
