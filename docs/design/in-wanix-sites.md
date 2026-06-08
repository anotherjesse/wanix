# In-Wanix Sites: generate, serve, and publish a website from Wanix itself

Status: design + implementation plan (Phases 0–3). Phase 4 is captured as Next
Steps, not yet scheduled.

## North star

A **site is a `FileSystem`**. Wanix already says "everything is a file"; the
corollary is "every website is a filesystem you serve over HTTP." If the HTTP
server reads through the Wanix `FileSystem`/`Namespace` abstraction instead of
the host disk, then *anything that implements `FileSystem` becomes servable with
one code path*:

- an **in-memory FS** a generator just wrote into (no host disk at all),
- a **host directory** (`LocalFs`, what works today),
- a **`#cas` tree** — an immutable, content-addressed published snapshot,
- a **mesh mount** `/n/<peer>/...` — a site that physically lives on another node.

The demo that proves it: a Rust static-site generator compiled to
`wasm32-wasi`, run as a Wanix task, that reads the `docs/site/content/` markdown
corpus from its namespace and writes an HTML tree into a Wanix `FileSystem`,
which `wanix-cli serve` then serves to a browser — Wanix generating and serving
its own documentation, through its own abstractions.

This is implemented entirely inside `wanix-cli serve` (no separate `wanix site`
subcommand): a serve route + a `#sites` service device.

## Three address layers (do not conflate them)

A published site has three independent "addresses" that stack:

1. **Content address** — a `#cas` root hash / `.wcap` capsule. Immutable,
   verifiable bytes. "*This exact site.*"
2. **Identity address** — an **iroh ticket** (ed25519, from `wanix-id`). A
   permanent, dialable name whose content can change and can move between
   machines. "*Whoever serves this site, wherever they are.*" (Phase 4.)
3. **Human address** — a hostname (`blog.localhost`, later `x.example.com`).
   "*The name people type.*" A resolver that points at a site.

Identity (iroh) is the canonical site; a domain is just a binding
`hostname → site`. Browsers cannot dial iroh QUIC, so browser consumption of an
iroh site always goes through an HTTP gateway node (the IPFS-gateway pattern);
that gateway is itself a Wanix node. Phase 4.

## The binding surface is a namespace: `#sites`

Publishing is a filesystem operation. A `#sites` service device maps a host to
an FS source:

```
#sites/blog.localhost   -> live in-memory FS (current generator output)
#sites/docs.localhost   -> #cas/<root-hash>          (immutable snapshot)
#sites/mirror.localhost -> /n/<peer>/sites/blog       (Phase 4: a peer's site)
```

The serve HTTP gateway reads the request `Host` header, looks up
`#sites/<host>`, and serves that `FileSystem`. `*.localhost` resolves to
`127.0.0.1` in browsers with **zero DNS configuration**, so it is the first
supported domain. Bring-your-own wildcard domains and TLS are Phase 4.

---

## Relevant existing code (grounding)

Found during exploration; implementers should re-confirm line numbers.

- `crates/wanix-cli/src/serve/http.rs`
  - `:56-104` static serving: maps URL→path, **reads the host disk via
    `std::fs`** (`read_static_response(&roots.static_root, &path)` ~`:101`),
    does directory-index resolution (`dir → index.html`, `:58-62`).
  - `:192-211` extension→content-type table (`html, js, css, json, wasm, txt`).
    **This is the MIME logic to reuse; it must move to the FS-backed path.**
- `crates/wanix-cli/src/serve/roots.rs` — namespace assembly. Under
  `--wanix-services` the host root is `LocalFs::new(root)` bound at `"."`;
  service devices (`#task #term #kv #pipe #plumb #cas #agent`) are bound. This
  is where a `#sites` device gets bound and where the served `FileSystem` is
  chosen.
- `crates/wanix-cli/src/serve/http/app.rs` — the `/.wanix/app/<name>` route:
  `app_name()` (`:198-216`) requires exactly 3 path components (rejects
  sub-paths → 400), content-type hardcoded `text/plain` (`:20`), spawn-task-
  return-stdout model (`:104-156`). **Not** a static file server; left as-is.
- `crates/wanix-fs/src/traits.rs` — the `FileSystem` trait (open/read/write/
  readdir/stat/create_dir/...). The serve route must depend on this, not
  `std::fs`. `crates/wanix-fs/src/localfs.rs` is the host-backed impl; in-memory
  `MemFs` fixtures also implement it.
- `crates/wanix-vfs` — `Namespace` bind/resolution.
- `crates/wanix-wasm` (`WasmTaskDriver`, `WasiRunner`) + `crates/wanix-wasi*` —
  the wasm task runtime. WASI surface includes `fd_readdir`, `path_open`
  (read+write+create), `path_create_directory`, `fd_write`; `poll_oneoff` is
  NOSYS. A guest opens `#kv/...`, `#cas/...`, and ordinary namespace paths.
  `.wasm` is a first-class task kind (auto-starts via `#task/new`); the driver
  reads the module from the task namespace.
- `crates/wanix-cas` — `ContentStore` trait, `LocalCasStore`, `#cas` device
  (`<hash>` read, `ingest` write→hash, `have/<hash>` probe), `.wcap` capsules.
- `crates/wanix-cli` `capsule save/load` — freeze/load a world via `#cas`.

## Renderer choice

Use **`pulldown-cmark`** for the SSG, not `comrak`. Pure Rust, zero C deps,
compiles to `wasm32-wasi` cleanly. (`comrak`'s default features pull in
`syntect`/`onig` (C), which will not target wasm.) Enable GFM
extensions (tables, strikethrough, footnotes, tasklists) via pulldown-cmark
options.

Prior art to adapt (read-only, in the primary checkout, not this worktree):
`/Users/jesse/lw/wanix/docs/site/preview/src/mdbook_prep.rs` already implements
frontmatter stripping, internal slug-link rewriting, sidebar/nav generation, and
the page→route mapping for this exact corpus. Port that logic; swap the renderer
and write into a `FileSystem` instead of host disk.

---

## Phase 0 — Keystone: serve through the `FileSystem` trait

**Goal.** The serve HTTP static path reads through a `FileSystem`/`Namespace`,
not `std::fs`. Prove it by serving an **in-memory FS** with no host-disk
involvement.

**Work.**
- Introduce a served-`FileSystem` source in the serve config (the static root
  becomes "a `FileSystem` to serve", which `LocalFs` satisfies for the existing
  `--root DIR` path — preserve current behavior).
- Rewrite `read_static_response` to resolve the URL path through the
  `FileSystem` (open/readdir/stat), keeping directory-index (`index.html`) and
  the extension→content-type map. Stream bytes via the FS read API.
- Path safety: reject `..`/escape via the FS path rules; no `std::fs::canonicalize`.

**Proof / tests.**
- Existing `--root DIR` HTTP serving still passes (regression).
- A new test serves a `MemFs` populated in-memory with `index.html` +
  `concepts/foo/index.html` + a `.css`, and asserts GETs return correct bytes,
  correct content-types, and working directory-index resolution — with **no
  files on disk**.

**Done when.** The static server is FS-backed, the in-memory test passes, and
`just check` is green.

## Phase 1 — The SSG, compiled to wasm, run as a Wanix task

**Goal.** A Rust SSG → `wasm32-wasi`, run as a Wanix `.wasm` task, reads the
markdown corpus from its namespace and writes an HTML tree into a Wanix
`FileSystem`. Serve that FS (Phase 0) and browse the result.

**Work.**
- New crate (e.g. `crates/wanix-site-gen` or an example) building to
  `wasm32-wasi`, using `pulldown-cmark`. It: walks an input dir (`fd_readdir`),
  strips frontmatter, renders GFM→HTML, rewrites internal slug links to served
  paths (`/section/page` → `/section/page/` directory-index form), generates a
  nav/index, and writes `<section>/<page>/index.html` via `path_open`
  (`O_CREAT|O_WRONLY`) + `path_create_directory`. Input/output dirs come from
  argv/env.
- Wire it as a Wanix task: the module is bound into a task namespace and started
  via `#task/new` (or the CLI `wasm` path) with the content corpus readable and
  an output `FileSystem` writable.
- A serve-side entry that (a) makes the corpus available to the task and (b)
  serves the generated output FS over HTTP.

**Proof / tests.**
- A test runs the wasm SSG task against the `docs/site/content` corpus (or a
  fixture subset) into an in-memory output FS and asserts page count, that
  frontmatter is stripped, and that an internal link resolves to a generated
  file. Reuse the "0 broken internal links" check from the corpus.
- Manual: `wanix-cli serve ...` then browse `/` and `/concepts/...` in a browser.

**Done when.** Wanix generates its own docs as a wasm task into a Wanix FS and
serves them; tests + `just check` green.

## Phase 2 — `#sites` device + `*.localhost` gateway

**Goal.** Multiple named sites, addressed by hostname, served from
`wanix-cli serve`. `blog.localhost` works with zero DNS config.

**Work.**
- New service device crate `wanix-sites` (a `FileSystem`): `#sites/<host>` binds
  a host to an FS source. Listing enumerates sites; reading a control file shows
  the binding; writing creates/updates a binding. Follow the existing service-
  device shape (`#kv`/`#pipe` style; plain `FileSystem`).
- Bind `#sites` in `serve/roots.rs` under `--wanix-services`; add it to the
  inspectable device set + discovery, consistent with the other devices.
- HTTP gateway: in the serve HTTP entry, read the `Host` header, strip the port,
  look up `#sites/<host>`; if bound, serve that `FileSystem` via the Phase 0
  FS-backed static handler. Fall back to the existing `--root` behavior when no
  site matches (or when `Host` is bare `localhost`/an IP).
- A serve flag/CLI affordance to register a site at startup (e.g. map
  `blog.localhost` → the generated output dir/FS).

**Proof / tests.**
- A test binds two in-memory sites and asserts that requests with different
  `Host` headers are routed to different filesystems with correct content.
- Manual: serve, then visit `http://blog.localhost:PORT/` and a second site.

**Done when.** Host-routed multi-site serving works; `#sites` is inspectable;
`just check` green.

## Phase 3 — Publish = freeze to `#cas`, bind host → hash

**Goal.** Immutable publishing with atomic deploy and instant rollback. "Publish
v2" repoints a name; rollback repoints to the prior hash. No file copying.

**Work.**
- A "freeze" path that ingests a site `FileSystem` tree into `#cas`: each file →
  its content hash, each directory → a manifest (`name → hash`), the tree → one
  **root hash**. Reuse `wanix-cas`/capsule machinery (`ContentStore`, `ingest`,
  `.wcap`) — prefer reusing capsule save over a bespoke encoder.
- A read-only **CAS-backed `FileSystem`** that resolves a path against a root
  hash (walk manifests, read leaves by hash). This composes with Phase 0: serve
  a `#cas` site exactly like any other FS.
- `#sites/<host>` can bind to `#cas/<root-hash>`. Publishing = freeze current
  output → get root hash → bind `#sites/<host> → that hash`. Rollback = bind to
  an earlier hash.

**Proof / tests.**
- Generate a site (Phase 1) → freeze to `#cas` → bind a host to the root hash →
  GET pages and assert byte-identical to the live output. Mutate, publish again,
  assert the new hash serves new content and the old hash still serves the old.
- `just check` green.

**Done when.** A site can be frozen to a content hash, served immutably by hash,
and a host binding can be repointed for atomic deploy/rollback.

---

## Follow-up: file-driven `#sites` registration vs. CLI transport

Site registration is a filesystem operation, not a CLI flag: write a source
descriptor to `#sites/<host>` —

```
echo 'dir /abs/path/to/site'  > '#sites/blog.localhost'   # serve a host directory
echo 'cas <root-hash>'        > '#sites/docs.localhost'   # serve an immutable snapshot
```

The `--site HOST=PATH` flag was **removed**; the `dir <path>` source is its
file-driven replacement (and `cas <hash>` is the publish form). The device
parses/round-trips/serves all three sources (`memory` placeholder, `dir`, `cas`)
and is unit-tested.

**Open transport gap.** Writing `#sites` *live against a running `serve`* needs a
9P client that speaks serve's transport — and serve's 9P rides a **websocket
upgrade**, while the CLI's `mount-write` is **raw-TCP 9P only** (and `p9-listen`,
which is raw 9P, does not expose `--wanix-services`). So today `#sites` is
writable in-browser by the cockpit (over the websocket) or in-process, but not
from the CLI against a live serve. Until that client exists, the CLI demo
(`docs/site/serve-from-wanix.sh`) serves the generated tree via `--root`.

Options to close it (pick one):
- **9P-over-websocket client** in the CLI (`mount-write ws://host:port …`) — the
  most general fix; the CLI then does file operations against any served
  namespace, exactly like the cockpit.
- **Startup namespace/site profile** — `serve` reads a file of bind operations
  (Plan 9 `/lib/namespace` style); file-driven config with no per-site flag.
- **Raw 9P listener** alongside HTTP on the serve endpoint.

## Next steps (Phase 4 — not in this workflow)

- **iroh-addressed sites.** Serve a site identity over iroh QUIC (reuse
  `wanix-mesh`); the HTTP gateway maps `hostname → iroh site → fetch FS (over
  9P-over-iroh) → serve HTTP`. A `#sites/<host>` binding can target
  `/n/<peer>/sites/<name>` — serve a site that lives on another node.
- **Bring-your-own wildcard domains.** User points `*.example.com`
  (A record → gateway IP, or CNAME) and adds a `#sites/foo.example.com` binding.
- **TLS / ACME.** `*.localhost` needs none; real domains need ACME (Let's
  Encrypt) on the gateway, ports 80/443.
- **Mesh distribution of CAS sites.** Because content is hashed, a peer pulling
  a site over iroh fetches only blocks it lacks; "publish to the mesh" = share a
  root hash.
- **Cockpit integration.** A cockpit panel to run the generator, freeze/publish,
  and open the served site — closing the loop in the browser operator surface.

## Guardrails for implementers

- Keep modules within the 250/350-line limits (`just module-lines`). Split
  before growing existing over-limit serve modules.
- `wanix-mesh` is the only async/iroh edge; keep tokio/iroh out of the new
  synchronous crates (`wanix-sites`, the CAS-serve FS). Phase 4 touches the mesh
  edge, not these.
- Do not hold a namespace/filesystem lock while calling into another filesystem.
- Use explicit Rust types for the new contracts (host, site source, root hash) —
  no public raw flags/ints.
- The renderer SSG is `wasm32-wasi`; do not couple Wasmtime into the new
  synchronous service crates.
- Run `just check` after each phase; commit each phase separately.
