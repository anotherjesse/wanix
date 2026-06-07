---
title: Browse by tag
slug: find/tags
pageType: reference
oneLiner: The status and cluster tag vocabulary used across the site.
audience: [newcomer, developer, visionary]
tags: [reference, shipped]
sourceRefs: [docs/site/00-master-plan.md:225-238]
seeAlso: [find/index, find/concepts, find/glossary, reference/crate-map-and-layering]
prerequisites: []
usedInFlows: []
honestLimits: ["A tag describes a page's claims, not a runtime guarantee; the source-cited body is always the ground truth.", "Tags are documentation metadata, not a permission system — local-trust-only marks what the prose warns about, it does not enforce anything."]
canonicalCaveatFor: []
---

# Browse by tag

The status and cluster tag vocabulary used across the site.

Every page on this site carries a `tags` list in its frontmatter. Two kinds of tags share that one field: **status** tags say how trustworthy a claim is, and **cluster/audience** tags say what the page is about and who it serves. Search (`/find`) and this browser read those tags so you can ask "show me everything shipped about the mesh" or "everything still marked exploratory." The vocabulary is fixed and small on purpose — the canonical list lives in the frontmatter schema (`docs/site/00-master-plan.md:231`). Below is what each tag means and how to filter with it.

## Status tags

Status tags are the honesty surface in machine-readable form. They line up with the project's stated scope (`docs/site/00-master-plan.md:46-48`, "Honest scope — shipped vs aspirational").

- **`shipped`** — the behaviour exists on this branch and is backed by a source citation in the body. Example: the HTTP-app route `/.wanix/app/<name>` is shipped (loopback-only, services-gated).
- **`local-trust-only`** — the capability works, but only inside the trust boundary. The exec devices (`#task`, `#agent`, `#cpu`) are local-trust only: cheap, scalable isolation, **not** safe for arbitrary untrusted code, with no hard CPU or memory limits yet.
- **`exploratory`** — designed or prototyped but not a shipped guarantee. Example: per-peer `/n/<peer-id>` mounts; the shipped CLI binds a single `/n/remote` slot (`crates/wanix-cli/src/mount.rs:26`).
- **`caveat`** — the page is where a known sharp edge is documented flatly. Example: the served `#agent` is a deterministic `FakeEngine`, not a live LLM; `#kv` is in-memory; `serve` handles one 9P frame at a time per connection.

A page can hold a status tag and still be useful — `caveat` is not a warning to skip the page, it is a promise the page tells you the boundary.

## Cluster and audience tags

These route by topic and reader.

- **`cli`** — the native `wanix-rust` command surface and its subcommands.
- **`mesh`** — the distributed layer: import/export, 9P over iroh QUIC, identity, capability binds.
- **`devices`** — the `#name` service-device contracts (`#task`, `#term`, `#kv`, `#pipe`, `#plumb`, `#cas`, `#agent`, `#cpu`).
- **`newcomer`**, **`developer`**, **`visionary`** — the audience personas (`docs/site/00-master-plan.md:79`); they also appear in the separate `audience` field and drive prerequisite badges.

## How to filter

Type a tag into search at [Find](/find/index) to narrow results to pages carrying it; combine a status tag with a cluster tag — `mesh local-trust-only` — to see exactly the distributed capabilities that stay inside the trust boundary. For a graph-backed browse instead of a tag filter, use the [concept index](/find/concepts); for definitions of the Plan 9 terms themselves, the [glossary](/find/glossary).

## See also

- [Find](/find/index) — the search and findability hub.
- [Concept index](/find/concepts) — graph-backed A–Z of every idea.
- [Glossary](/find/glossary) — definitions of the Plan 9 and Wanix terms.
- [Crate map and layering](/reference/crate-map-and-layering) — how `shipped` and `mesh` map onto the workspace.

## Status / honest limits

A tag describes a page's claims, not a runtime guarantee. `local-trust-only` and `caveat` are documentation metadata — they mark what the prose warns about; they enforce nothing. When a tag and the page body seem to disagree, the source-cited body wins: it carries the `crate/src/file.rs:line` reference, the tag does not.
