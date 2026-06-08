# Wanix Site

The plan **and** the full written content for the public documentation site of the
**Rust-native Wanix port** — a Plan 9-style OS core that runs outside Chrome,
with Wasmtime as the substrate, QuickJS/WASI as the first task runtime, and a
mesh that imports remote namespaces as local files over 9P.

> Scope: the Rust-native port only. The deprecated Go runtime (`wanix.run`) has
> its own site and is out of scope here.

This site is **portable markdown** — no framework lock-in. The master plan
recommends rendering it with Astro + Starlight (see `00-master-plan.md` §8), but
the content tree under `content/` stands on its own.

## What's here

### The plan (`00`–`07`)

| File | What it is |
|---|---|
| [`00-master-plan.md`](00-master-plan.md) | The decision-bearing master plan: vision, audiences, IA, content plan, demos, tech-stack recommendation, build roadmap, review resolutions, first-PR slice, success metrics. **Start here.** |
| [`01-personas.md`](01-personas.md) | Six personas across three audience tracks (users / developers / visionaries). |
| [`02-information-architecture.md`](02-information-architecture.md) | The dual-mode IA (guided flows + canonical reference), the sitemap, the 77-node concept graph, the 8 learning flows. |
| [`03-content-catalog.md`](03-content-catalog.md) | The page catalog / manifest grouped by section. |
| [`04-demo-catalog.md`](04-demo-catalog.md) | The demo & interactivity plan (live casts vs recorded screenshots), 20 demos. |
| [`05-voice-and-style.md`](05-voice-and-style.md) | Voice & style guide. |
| [`06-research-briefs.md`](06-research-briefs.md) | The six grounding research briefs (with honest caveats). |
| [`07-review-notes.md`](07-review-notes.md) | The five adversarial reviews that the master plan resolves. |

Machine-readable provenance: [`page-manifest.json`](page-manifest.json) (the
researched catalog) and [`write-list.json`](write-list.json) (the final
reconciled corpus list).

### The content (`content/`)

The complete written corpus — **121 pages, ~150k words**, every page link-checked
(0 broken internal links) and carrying a `Status / honest limits` block wherever
it makes a behavioural claim.

| Section | Pages | Words | What it holds |
|---|---:|---:|---|
| `home` | 1 | ~1.2k | Hero + three audience tracks + honest-scope panel |
| `learn/` | 9 | ~12.5k | Index + 8 persona-shaped guided flows |
| `concepts/` | 67 | ~83k | One canonical page per idea — the dual-mode truth source |
| `devices/` | 9 | ~12k | `#task #term #kv #pipe #plumb #cas #agent #cpu` file contracts + index |
| `use-cases/` | 7 | ~9k | Outcome-first "why", each ending in one recipe |
| `recipes/` | 9 | ~10k | Copy-paste transcripts (incl. `wanix new` scaffold) |
| `reference/` | 14 | ~18k | Contracts, specs, indices (crate map, ADRs, attach/capability contract, guest SDK, performance, CLI index, …) |
| `find/` | 4 | ~4k | Search hub, concept index, glossary, tag browser |

## Conventions (binding)

Authored into every page; enforced at build time if/when the Astro project lands:

- **Canonical slugs:** `section/page`, single prefix, no doubling. Flows live
  only under `/learn/<flow>`. There is no `/flows/` section.
- **Dual mode:** every concept has exactly one canonical page; flows *thread*
  those pages in order rather than duplicating them.
- **Honesty is content:** caveats are stated flatly, once, in a canonical home,
  and linked elsewhere. Never overclaim the five landmines — FakeEngine is not a
  live LLM; `#kv` is in-memory; `serve` is one-9P-frame-at-a-time; exec devices
  are local-trust-only; the shipped mount lands at `/n/remote`.
- **Cite source:** behavioural claims carry `crate/src/file.rs:line` references.
- **Frontmatter schema:** see `00-master-plan.md` §6 (also the Zod schema sketch
  in §8 for the Astro content collection).

## Turning this into a running site

Per `00-master-plan.md` §8 (Astro + Starlight):

```sh
cd docs/site
npm create astro@latest -- --template starlight --yes .
npm i @astrojs/starlight asciinema-player cytoscape
# wire src/content.config.ts (Zod schema in §8) + a remark link-checker plugin
# point the docs collection at content/  (or move content/ under src/content/docs/)
npm run dev      # http://localhost:4321
npm run build    # static output; should fail on broken links / missing honestLimits
```

Search → Pagefind (static, client-side). Concept graph → a client-side widget at
`/find/concepts` fed by a generated `concept-graph.json`. Demos → asciinema casts
for deterministic CLI flows + the verified screenshots in
`../integration/screenshots/`.

## Known follow-ups

These are tracked, non-blocking polish items (the corpus is link-clean and
complete as-is):

- **`usedInFlows` frontmatter normalization.** Some pages carry flow ids without
  the `learn/` prefix (e.g. `js-outside-chrome` vs `learn/js-outside-chrome`).
  This is metadata, not a body link (all body links resolve), and only matters
  once the Astro flow-stitching/progress-rail step is built — normalize the flow
  ids then.
- **Concept-graph JSON generation.** `02-information-architecture.md` Appendix C
  has the nodes/edges; a small build step should emit `concept-graph.json` from
  each page's `seeAlso`/`prerequisites` frontmatter.
- **Demo assets.** The casts listed in `04-demo-catalog.md` still need recording;
  the cockpit screenshots already exist under `../integration/screenshots/`.

## How this was produced

Two multi-agent workflows: a **plan** pass (research → personas → IA → catalogs →
exemplars → adversarial review → synthesis) and a **content** pass (write every
page from its spec + real source files → normalize exemplars → per-section
cross-link/honesty audit). The plan is grounded in the crates under `../../crates`,
the recipes under `../recipes`, ADRs `0001`–`0005`, and the cockpit screenshots
under `../integration/screenshots`.
