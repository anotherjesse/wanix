---
title: Find — Search, Concept Index, Glossary, Tags
slug: find/index
pageType: overview
oneLiner: The findability hub — full-text typed search on the '/' hotkey, the graph-backed Concept Index, the A-Z Glossary, and the Tag browser.
audience: [newcomer, developer, visionary]
tags: [navigation, findability, shipped]
sourceRefs: [docs/site/02-information-architecture.md, docs/site/00-master-plan.md, AGENTS.md]
seeAlso: [find/concepts, find/glossary, find/tags, learn/index]
prerequisites: []
usedInFlows: []
honestLimits: ["This is a navigation surface, not new content — every fact lives once on its canonical concept, device, or reference page.", "Search indexes the built site, not the running Wanix runtime; it does not query a live mesh or serve process."]
---

# Find — Search, Concept Index, Glossary, Tags

The findability hub — full-text typed search on the '/' hotkey, the graph-backed Concept Index, the A-Z Glossary, and the Tag browser.

This site has roughly seventy-eight concept pages, eight device contracts, five ADRs, a crate map, six use cases, and a stack of recipes. A newcomer wants a guided route; an expert wants to skip straight to `subtree.rs` or the `#agent/<id>/reply` anchor and bookmark it. **Find** is the door for the second reader. It is four navigation surfaces, not a fifth body of facts: every claim still lives exactly once on its canonical page, and Find only points at it.

## Search — type `/` and go

Press `/` anywhere to focus the search box. Search indexes concept bodies, every device-file anchor, the ADRs, the crate map, the recipes, and the glossary, and the results are **typed**: each hit is labelled Concept, Device file, ADR, Crate, Recipe, or Glossary, and shows its one-liner so you can pick without clicking.

Exact-name queries resolve because the literal string appears in the page body. Search `poll_oneoff` and you land on the [command-style wasm linker](/concepts/command-style-wasm-linker) (it is `NOSYS`). Search `EACCES` and you land on the capability contract, because a denied attach is a default-deny grant decision, not a bug. Search `#agent reply` and you go to the [`#agent` device](/devices/agent). Type a Plan 9 primitive — `fid`, `msize`, `Tauth`, `venti` — and the [glossary](/find/glossary) entry comes back with a deep link onward.

## The Concept Index — A-Z and the visual map

[The Concept Index](/find/concepts) renders the concept graph two ways. The flat **A-Z list** is the dense, flow-free directory an expert scans top to bottom. The **clustered visual map** draws the same graph with nodes colored by audience — newcomer, developer, visionary — and edges typed `prerequisite-of`, `composed-of`, and `related-to`, so you can see how [everything is a file](/concepts/everything-is-a-file) sits at the root and how the mesh cluster composes from [RemoteFs](/concepts/remotefs-import-half), [9P over iroh QUIC](/concepts/9p-over-iroh-quic), [the key is the address](/concepts/key-is-the-address), and [a capability is a bind](/concepts/capability-is-a-bind).

The Concept Index carries one hard guarantee: it is the inbound link for **every** graph node. Not every concept is threaded by a [learning flow](/learn/index) — many are reference-only — but nothing is a true orphan, because the Index reaches all of them.

## The Glossary — terse definitions, deep links

[The A-Z Glossary](/find/glossary) is one line per term and typed primitive an expert searches by exact name: `9P`, `ALPN`, `BindPosition`, `confine_to_prefix`, `ContentHash`, `EndpointId`, `fid`, `msize`, `NormalizedPath`, `SubtreeFs`, `Tauth`, `venti`, `.wcap`. Each entry deep-links to its canonical concept or device page and its source. The definitions are generated from the concept one-liners, so the glossary and the page can never drift.

## The Tag browser — faceted by audience, cluster, status

[The Tag browser](/find/tags) is the expert's facet filter. Every page carries a controlled vocabulary on three axes: **audience** (`newcomer`, `developer`, `visionary`), **cluster** (`namespace`, `mesh`, `task-runtime`, `device`, `security`, `serve`), and **status** (`shipped`, `local-trust-only`, `exploratory`, `caveat`). The status axis is the honest one: filter on `local-trust-only` to see exactly which surfaces — the exec devices, the codex-backed [agent](/devices/agent) — are not exposed to untrusted peers, or on `exploratory` to see what is designed but unshipped.

## How experts enter low-friction

Land wherever you already think. Search a contract and bookmark its anchor. Jump from the Concept Index to a node like [the missing half of 9P](/concepts/missing-half-of-9p) or [agents as operators](/concepts/agents-as-operators). Walk the [crate map](/reference/crate-map-and-layering) and click a crate into its concept and source. Open the [ADR index](/reference/adr-index) and read the boundary directly. No flow required.

## See also

- [The Concept Index](/find/concepts) · [The Glossary](/find/glossary) · [The Tag browser](/find/tags)
- [Learn — guided flows](/learn/index) for the hand-held mode instead
- Root concepts: [everything is a file](/concepts/everything-is-a-file) · [a capability is a bind](/concepts/capability-is-a-bind) · [the missing half of 9P](/concepts/missing-half-of-9p)
- [Crate map and layering](/reference/crate-map-and-layering) · [ADR index](/reference/adr-index)

## Status / honest limits

- Find is a **navigation surface, not new content.** Every fact lives once on its canonical concept, device, or reference page; Find only routes you there, so accuracy is maintained in exactly one place.
- Search indexes the **built site**, not the running Wanix runtime. It is client-side full-text over the pages; it does not query a live `serve` process or a mesh peer.
- The Concept Index guarantees an inbound link for every concept-graph node, but **not** that every concept is threaded by a flow — many concepts are reference-only and reachable only through Find and the See-also rails.
