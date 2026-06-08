# In-Wanix Sites — cross-phase implementation map (Phases 0–3)

This is the file-level blueprint that later agents implement strictly against.
It re-confirms current line references (worktree `in-wanix-sites`, June 2026) and
fixes the exact files, functions, types, and tests for each phase. Read it
alongside `in-wanix-sites.md` (architecture) before touching code.

Implement in order; each phase must keep the workspace compiling and `just
check` green, and each phase is its own commit. `just check` =
`fmt module-lines clippy test`; `test` runs `cargo test --workspace --locked`,
so **any new crate / new dep means a plain `cargo build` to refresh
`Cargo.lock`, committed in the same change**.

---

## Ground truth re-confirmed (current code)

- `crates/wanix-cli/src/serve/http.rs` (211 lines):
  - `read_static_response(static_root: &Path, relative_path: &Path) -> StaticResponse`
    at `:56-80` — the **std::fs** path: `static_root.join`, `is_dir() ->
    index.html` (`:58-62`), `fs::canonicalize` + `starts_with(static_root)`
    escape guard (`:64-69`), `fs::read` (`:71`).
  - `http_response(...)` at `:92-104` — POST `/agent` branch, otherwise
    `routes::http_route_response(...).unwrap_or_else(|| read_static_response(
    &roots.static_root, &path))` at `:100-101`. **This `unwrap_or_else` fallback
    is the single static-serve entry point** for Phases 0/2.
  - `content_type(path: &Path) -> &'static str` at `:192-211` — the MIME table
    (`html, js, mjs, css, json, wasm, txt`). Phase 0 reuses this; it keys off the
    path extension only, so it works unchanged on a virtual path.
  - `serve_http_connection` / `http_response` take `roots: &ServeRoots`.
- `crates/wanix-cli/src/serve/roots.rs` (210 lines):
  - `ServeRoots { static_root: PathBuf, p9_root: Arc<dyn FileSystem>,
    driver_kinds, local_addr, bundle, wanix_services }` at `:20-31`.
  - `ServeRoots::new(root_path, local_addr, bundle, wanix_services)` at `:33-56`
    — canonicalizes `static_root`, builds `p9_root` via `serve_p9_root`.
  - `bind_host_and_terminal` at `:122-172` binds `#term #pipe #kv #plumb #cas
    #agent`. `INSPECTABLE_SERVICE_DEVICES` const at `:119-120`.
  - `serve_task_table()` at `:187-193`.
- `crates/wanix-cli/src/serve.rs`:
  - `serve_roots_for_listener` at `:72-86` calls `ServeRoots::new(&command
    .root_path, local_addr, command.bundle.clone(), command.wanix_services)`.
  - module consts `FS9P_BUNDLE`, `WORKBENCH_FS9P_BUNDLE` at `:26-27`.
- `crates/wanix-cli/src/serve/command.rs` + `command/options.rs`:
  - `ServeCommand { root_path, addr, bundle, wanix_services, once }`.
  - flag/value option enums `ServeValueOption {Root,Addr,Listen,Bundle}`,
    `ServeFlagOption {Once,WanixServices}`, parsed via `SERVE_VALUE_OPTIONS` /
    `SERVE_FLAG_OPTIONS` tables and applied in `ServeCommandParts`.
- `crates/wanix-cli/src/serve/http/routes.rs` (224) — `http_route_response`
  chains `well_known_response`, `bundle_response`, `app_route_response`,
  `workbench_asset_response`, `direct_v86_asset_response`. Returns
  `Option<StaticResponse>`.
- `crates/wanix-cli/src/serve/http/response.rs` — `StaticResponse {status,
  content_type:&'static str, headers, body}`, `::plain`, `::with_header`,
  `::encode`; `HttpStatus` enum (Ok/BadRequest/Conflict/Forbidden/NotFound/
  MethodNotAllowed/NotImplemented/InternalServerError).
- `crates/wanix-cli/src/serve/discovery.rs` (188) — `serve_services_json` at
  `:154-171` emits `devices` from `INSPECTABLE_SERVICE_DEVICES`.
- `crates/wanix-fs/src/traits.rs` — `FileSystem` trait: `open(&NormalizedPath,
  OpenOptions)`, `metadata`, `read_dir`, `create_dir`, `content_hash` (default
  `Ok(None)`). `File::read(&mut [u8]) -> FsResult<usize>`. `OpenOptions::read()`
  / `read_write()`. `NormalizedPath::new(&str)` rejects `..`/absolute/empty —
  this **is** the Phase 0 escape guard.
- `crates/wanix-fs/src/memfs/fixture.rs` — `MemFs::create_dir_all`,
  `write_file`, `read_file` test helpers (used by Phase 0/1/3 tests).
- `crates/wanix-cas/src/lib.rs` exports `ContentStore`, `LocalCasStore`,
  `CasDevice`, `CasFs`, `WorldManifest`, `Capsule`, `ManifestEntry`,
  `hash_bytes`, `MAX_BLOB_SIZE`, `verify_hash`, `ContentHash` (re-exported from
  wanix-fs).
  - `ContentStore::{put(&[u8])->ContentHash, get(&ContentHash)->Vec<u8>,
    has(&ContentHash)->bool}`.
  - `Capsule::freeze(store, root: &Path)` and `WorldManifest::materialize(store,
    target: &Path)` are **host-disk (`std::fs`) only today** — Phase 3 adds
    FS-backed equivalents (do not change these).
  - `ManifestEntry {path:String, hash:ContentHash}`; `WorldManifest::{to_blob,
    from_blob, entries}` is the durable manifest wire form to reuse.
  - `ContentHash::{to_hex, from_hex}`.
- `crates/wanix-wasm`: `WasmTaskDriver` (`check` matches `.wasm`, `start` reads
  module from task namespace, builds WASI from task ns/cwd/env/argv/fds),
  `WasiRunner::{from_bytes, from_bytes_cached, run(WasiConfig), run_in_dir}`.
  **wasm fixtures are checked-in `.wasm` built from a nested standalone Cargo
  workspace** (`crates/wanix-wasm/fixtures/guest-src/`, its own `[workspace]`,
  `edition=2021`, `[[bin]]`, profile `opt-level="s"`/`strip`), with the built
  artifact copied to `fixtures/rust-guest.wasm` and `include_bytes!`'d in tests.
  This is the exact pattern the Phase 1 SSG fixture follows.

---

## New crates / new public types (summary)

| Crate (new) | Kind | Phase | Depends on |
|---|---|---|---|
| `wanix-site-fs` | sync lib | 0 | `wanix-fs` |
| `wanix-site-gen` (host lib) | sync lib | 1 | `wanix-fs`, `pulldown-cmark` |
| `wanix-site-gen/wasm-src` | standalone wasm bin | 1 | own workspace, `pulldown-cmark` |
| `wanix-sites` | sync device fs | 2 | `wanix-fs`, `wanix-site-fs` |
| `wanix-site-cas` | sync lib | 3 | `wanix-fs`, `wanix-cas` |

All five are **synchronous, no tokio/iroh, no Wasmtime**. They all build for the
host target (so `cargo test --workspace` works). Only the wasm SSG artifact
targets `wasm32-wasip1`, and it lives in a **nested standalone workspace** so the
parent workspace never tries to host-build it (exactly like
`fixtures/guest-src`).

New public newtypes (no raw int/flag public APIs):

- `wanix-site-fs`: `struct SiteRequestPath(NormalizedPath)` (URL→FS resolution
  result; see Phase 0).
- `wanix-sites`: `struct Host(String)` (normalized, lowercased, port-stripped
  hostname), `enum SiteSource { Memory(Arc<dyn FileSystem>), Cas(CasRootHash) }`,
  `struct CasRootHash(ContentHash)`.
- `wanix-site-cas`: `struct CasRootHash(ContentHash)` (re-exported / shared with
  `wanix-sites`), and `CasSiteFs` implementing `FileSystem`.

---

## Phase 0 — serve the static path through `FileSystem`

**Goal.** `read_static_response` resolves through a `FileSystem`, not `std::fs`.
Keep `--root DIR` behavior identical; prove with an in-memory `MemFs`.

### New crate: `crates/wanix-site-fs`

`Cargo.toml`: `name = "wanix-site-fs"`, workspace package settings, `[dependencies]
wanix-fs = { path = "../wanix-fs" }`. Add to root `Cargo.toml` `members` and to
the `fmt` package list in `justfile:4`.

`src/lib.rs` — the FS-backed static handler core (keep < 250 lines):

```rust
use std::sync::Arc;
use wanix_fs::{FileSystem, FileType, FsError, NormalizedPath, OpenOptions};

/// A request path already resolved to a safe in-FS path (directory-index
/// applied). Never holds `..`/absolute components — `NormalizedPath` enforces it.
pub struct SiteRequestPath(NormalizedPath);

/// Outcome of resolving a URL path against a site filesystem.
pub enum SiteFile {
    /// A regular file: its bytes and the extension used for content typing.
    Found { bytes: Vec<u8>, extension: Option<String> },
    /// No file (or directory had no index.html).
    NotFound,
    /// Path escaped the root / was malformed.
    Forbidden,
}

/// Resolve `url_relative` (the percent-decoded path with no leading `/`,
/// possibly empty for `/`) against `fs`, applying directory-index resolution
/// and returning the file bytes. Pure FS access — no host disk.
pub fn read_site_file(fs: &dyn FileSystem, url_relative: &str) -> SiteFile { ... }
```

Resolution algorithm (mirrors current `read_static_response` semantics):

1. Normalize: empty path → `"."`. Build `NormalizedPath::new(path)`; on `Err`
   return `Forbidden` (this replaces `canonicalize` + `starts_with`; `..` and
   absolute paths are rejected at construction, so there is **no host
   canonicalize and no symlink escape** to reason about — `MemFs`/`LocalFs`
   confine within their own root).
2. `fs.metadata(&p)`: if it is a `Directory`, retarget to `p.join("index.html")`
   (re-validate via `NormalizedPath`). If `NotFound` at this stage → `NotFound`.
3. `fs.open(&p, OpenOptions::read())`, loop `File::read` into a `Vec<u8>` (64 KiB
   chunks), return `Found { bytes, extension: p extension }`.
4. Map `FsError::NotFound` → `NotFound`; `FsError::PermissionDenied` →
   `Forbidden`; other errors → `NotFound` (matches current "best-effort 404").

A small `fn split_extension(path: &NormalizedPath) -> Option<String>` returns the
lowercased trailing `ext` so the caller can reuse the existing MIME table.

> Module-line note: keep `read_site_file` + helpers in `lib.rs`; if it nears 250,
> split resolution into `src/resolve.rs`.

### Change: `crates/wanix-cli/src/serve/roots.rs`

Add a served-FS field so the static path no longer needs `static_root` for I/O,
while preserving `static_root` for diagnostics/discovery/handoffs that still use
it (`serve.rs:99` startup line, rootfs/v86 routes that legitimately read disk).

- Add field `pub(super) site_root: Arc<dyn FileSystem>` to `ServeRoots`
  (`:20-31`).
- In `ServeRoots::new` (`:33-56`): after computing `static_root`, build
  `site_root = open_host_p9_root(root_path)?` (a `LocalFs` over the same dir).
  Reuse the existing `open_host_p9_root` helper (`:69-79`) — **this is how
  `--root DIR` keeps identical behavior**: the default site root is a `LocalFs`,
  so byte content, directory-index, and MIME are unchanged; only the I/O path
  moves from `std::fs` to `LocalFs::open/metadata`.
  - Note: `LocalFs` resolves symlinks against the host root and confines within
    it (`confine_to_prefix`), so the prior `canonicalize + starts_with` guard is
    satisfied by `LocalFs` itself for the `--root` path.

### Change: `crates/wanix-cli/src/serve/http.rs`

- Replace the body of `read_static_response`. New signature (the static path is
  driven by the FS, not a `&Path`):

  ```rust
  pub(super) fn read_static_response(
      fs: &dyn wanix_fs::FileSystem,
      url_relative: &str,
  ) -> StaticResponse
  ```

  Body: call `wanix_site_fs::read_site_file(fs, url_relative)` and map:
  - `Found { bytes, extension }` → `StaticResponse { status: Ok, content_type:
    content_type_for_extension(extension.as_deref()), headers: vec![], body:
    bytes }`.
  - `NotFound` → `StaticResponse::plain(NotFound, "not found")`.
  - `Forbidden` → `StaticResponse::plain(Forbidden, "forbidden")`.
- Refactor `content_type(&Path)` into `content_type_for_extension(Option<&str>)
  -> &'static str` (same table at `:192-211`), so it keys on a borrowed
  extension string instead of a `Path`. Keep a thin `content_type(&Path)` wrapper
  **only if** another caller needs it (currently only `read_static_response` and
  `workbench_asset_response` use it — see below).
- `http_response` (`:100-101`): change the fallback to
  `read_static_response(roots.site_root.as_ref(), &url_relative_string)`. The
  `relative_path: &Path` produced by `parse_http_request` must become a
  `/`-joined relative string; add `fn path_to_url_relative(p: &Path) -> String`
  (join `Component::Normal` parts with `/`; this is already implicitly safe
  because `parse_http_request` rejected traversal). Keep passing the `Path` form
  to `routes::http_route_response` unchanged.

### Change: `crates/wanix-cli/src/serve/http/routes.rs`

`workbench_asset_response` (`:32-42`) calls `read_static_response(&asset_root,
asset_path)` against a host dir under `workbench/`. Two clean options — pick (a):

- (a) Keep workbench assets on `std::fs`: rename the old disk reader to
  `read_disk_static_response(static_root: &Path, relative_path: &Path)` (verbatim
  old body, kept private to `http.rs`), and have `workbench_asset_response` call
  it. The new FS-backed `read_static_response` is used only by the `--root`/site
  path. This keeps the workbench asset root (a real host dir outside the served
  root) on the proven disk path and avoids constructing a `LocalFs` per request.

> This split (disk reader retained for workbench assets, FS reader for the site)
> is the explicit answer to "replace std::fs WITHOUT breaking existing behavior."

### Tests (Phase 0)

- `crates/wanix-site-fs/src/tests.rs` (`#[cfg(test)] mod tests;`):
  - `serves_index_for_root`: `MemFs` with `index.html`; `read_site_file(fs, "")`
    and `read_site_file(fs, ".")` both return the index bytes.
  - `serves_nested_directory_index`: `concepts/foo/index.html`;
    `read_site_file(fs, "concepts/foo")` returns it (directory-index).
  - `serves_css_with_extension`: returns `.css` bytes + `extension=="css"`.
  - `rejects_parent_traversal`: `read_site_file(fs, "../etc/passwd")` →
    `Forbidden`.
  - `missing_is_not_found`.
- `crates/wanix-cli/src/serve/tests.rs` (extend): a regression test that the
  existing `--root DIR` HTTP GET still returns the on-disk file (already covered;
  ensure it still passes through the new `site_root` `LocalFs`). Add an
  in-memory-FS HTTP test only if a `ServeRoots` test constructor is reachable;
  otherwise the `wanix-site-fs` unit tests carry the "no disk" proof and the
  existing serve integration tests carry the regression.

### Module-line risk (Phase 0)

`http.rs` is 211 lines and gains `path_to_url_relative` + the mapping; if it
crosses 250, move static-serving helpers (`read_static_response`,
`read_disk_static_response`, `content_type*`) into a new
`crates/wanix-cli/src/serve/http/static_serve.rs` (`pub(super)` re-exports from
`http.rs`). Plan for this split as part of Phase 0 if the count exceeds 250.

---

## Phase 1 — the SSG, compiled to wasm, run as a Wanix task

**Goal.** A `pulldown-cmark` SSG → `wasm32-wasip1`, run as a `.wasm` Wanix task,
reads the markdown corpus from its namespace and writes an HTML tree into a Wanix
`FileSystem`. Serve that FS (Phase 0).

### Crate structure (host + wasm from one logic core)

Port `docs/site/preview/src/mdbook_prep.rs` logic but (1) swap renderer to
`pulldown-cmark`, (2) target `index.html` directory-form output (not `.html`),
(3) read/write through generic byte I/O so the same code runs on host `std::fs`
**and** in the wasm guest via libstd (`std::fs` on `wasm32-wasip1` maps to WASI
`path_open`/`fd_readdir`/`path_create_directory` against the task's preopened
namespace).

Layout:

```
crates/wanix-site-gen/                # host library crate (in the workspace)
  Cargo.toml                          # wanix-fs (dev only), pulldown-cmark
  src/lib.rs                          # pub fn generate_site(input,&dyn Fs-or-bytes) — pure logic
  src/page.rs                         # Page, load_page, out paths (ported)
  src/links.rs                        # route map + slug-link rewriting (ported)
  src/nav.rs                          # nav/index HTML generation (replaces SUMMARY.md)
  src/render.rs                       # pulldown-cmark GFM render + frontmatter strip
  wasm-src/                           # NESTED standalone workspace (not a member)
    Cargo.toml                        # [workspace], edition 2021, [[bin]] ssg, depends on ../ (path) + pulldown-cmark
    src/main.rs                       # reads argv input/output dirs, walks std::fs, calls wanix_site_gen logic
  fixtures/
    site-gen.wasm                     # checked-in built artifact (include_bytes! in tests)
    corpus/                           # small fixture subset of docs/site/content for the test
```

**Why this builds for both targets.** The pure logic (`wanix-site-gen` lib)
operates over a small filesystem abstraction it defines itself — a trait like:

```rust
pub trait SiteIo {
    fn read_dir(&self, dir: &str) -> std::io::Result<Vec<SiteEntry>>;
    fn read_to_string(&self, path: &str) -> std::io::Result<String>;
    fn create_dir_all(&self, path: &str) -> std::io::Result<()>;
    fn write(&self, path: &str, bytes: &[u8]) -> std::io::Result<()>;
}
```

with a host `std::fs` impl (`StdFsIo`, used in host tests) and the wasm `main.rs`
also providing a `std::fs`-backed impl (libstd → WASI). `generate_site(io:&dyn
SiteIo, input:&str, output:&str)` contains the ported corpus logic and is
target-agnostic (no `std::path::Path::is_dir` reliance beyond what WASI supports;
use `read_dir` entry file-types). This keeps the corpus logic **identical**
across host and wasm and lets host tests exercise it directly without wasm.

`render.rs` uses `pulldown-cmark` with `Options::ENABLE_TABLES |
ENABLE_STRIKETHROUGH | ENABLE_FOOTNOTES | ENABLE_TASKLISTS | ENABLE_GFM` and
wraps body HTML in a minimal page template (`<!doctype html>…<main>{body}</main>`
+ the generated nav). Frontmatter strip + `frontmatter_title`/`first_h1` ported
verbatim from `mdbook_prep.rs:202-238`.

`links.rs`: port `build_route_map` (`:90-106`) and `rewrite_links`/
`rewrite_target` (`:108-151`) but the route map values become **served directory
paths** (`/section/page/`) instead of `*.html`, because Phase 0 serves
`index.html` from a directory. `out_html_path` becomes `out_dir_path` →
`<section>/<page>/index.html`; home → `index.html`; `<section>/index.md` →
`<section>/index.html`.

### Producing the `.wasm` artifact

`wasm-src/` is a standalone `[workspace]` (parent never host-builds it). Build &
vendor command (documented in `wasm-src/README.md` and in the crate doc):

```sh
rustup target add wasm32-wasip1   # once
cd crates/wanix-site-gen/wasm-src
cargo build --release --target wasm32-wasip1
cp target/wasm32-wasip1/release/ssg.wasm ../fixtures/site-gen.wasm
```

Commit `fixtures/site-gen.wasm`. The host build and `--locked` tests never
require the wasm toolchain (the artifact is checked in), matching the existing
`rust-guest.wasm` precedent. `wasm-src/Cargo.lock` is committed (its own
workspace); the artifact is `.gitignore`d under `wasm-src/target` only.

### Wiring the SSG as a Wanix task (serve side, Phase 1 minimal)

Add a host-side runner used by tests and (later) the `#sites`/serve flow:

- A function (in `wanix-site-gen` or a thin `crates/wanix-cli/src/serve/site.rs`)
  that: builds a `Namespace` binding (a) the corpus `FileSystem` read-only at an
  input subdir and (b) an output `MemFs` (or `LocalFs`) writable at an output
  subdir; allocates a `wasm` task via `TaskTable` (`serve_task_table()` already
  registers `wasm`), seeds the module bytes (`include_bytes!`/the served
  `apps/`), sets `cmd = "site-gen.wasm <input> <output>"`, and starts it.
- For the **test**, follow `wanix-wasm/src/driver.rs:99-215` exactly:
  `MemFs` with `site-gen.wasm` seeded, corpus seeded under `content/`, run via
  `WasmTaskDriver` + `TaskTable::start` (auto) or `WasiRunner::run` with a
  `WasiConfig` over the shared namespace, assert output files appear in the
  shared `MemFs`.

### Tests (Phase 1)

- Host unit tests in `wanix-site-gen` (no wasm): `generate_site(StdFsIo|MemIo,
  fixtures/corpus, out)`:
  - `page_count_matches_corpus` (count `index.html` outputs).
  - `frontmatter_is_stripped` (no leading `---` in rendered body).
  - `internal_slug_link_rewritten`: a `/concepts/foo` link becomes
    `/concepts/foo/` and resolves to a generated file (port the corpus's "0
    broken internal links" check: every rewritten `/`-link target maps to an
    emitted output path).
- One wasm integration test (in `wanix-site-gen/tests/` or under `wanix-cli`):
  `include_bytes!("fixtures/site-gen.wasm")`, run it as a Wanix `wasm` task over
  a `MemFs` seeded with the fixture corpus, assert the same page count + a
  rewritten link in the generated `MemFs` — proving the wasm artifact runs
  through `WasmTaskDriver` into a Wanix FS.
- (Optional, gated) a `cargo test` that GETs `/` and `/concepts/...` via the
  Phase 0 FS handler over the generated `MemFs`.

### Module-line risk (Phase 1)

The ported logic is ~275 lines combined; **split it across `page.rs/links.rs/
nav.rs/render.rs`** from the start (do not let `lib.rs` accrete the whole port).

---

## Phase 2 — `#sites` device + `*.localhost` gateway

**Goal.** Host-routed multi-site serving from `wanix-cli serve`.

### New crate: `crates/wanix-sites`

A plain `FileSystem` service device, modeled on `wanix-kv` (`crates/wanix-kv/
src/lib.rs` 177 lines + `files.rs` 76). `Cargo.toml` deps: `wanix-fs`,
`wanix-site-fs` (not strictly needed for the device, but shares the `Host`
newtype if placed here). Add to workspace `members` + `justfile` fmt list.

Public types:

```rust
/// A normalized site host: lowercased, port stripped.
pub struct Host(String);
impl Host { pub fn parse(raw: &str) -> Option<Host>; pub fn as_str(&self)->&str; }

/// What a host binding points at.
pub enum SiteSource {
    /// A live filesystem (in-memory generator output, a LocalFs, etc.).
    Memory(Arc<dyn FileSystem>),
    /// An immutable CAS snapshot (Phase 3 fills this in).
    Cas(/* wanix-site-cas::CasRootHash */ String),
}

/// The `#sites` device.
pub struct SitesDevice { /* RwLock<BTreeMap<Host, SiteBinding>> */ }
impl SitesDevice {
    pub fn new() -> Self;
    /// Register/replace a binding programmatically (serve startup, tests).
    pub fn bind_site(&self, host: Host, source: SiteSource);
    /// Resolve a host to the filesystem to serve, applying CAS sources.
    pub fn resolve(&self, host: &Host) -> Option<Arc<dyn FileSystem>>;
}
```

Device FS shape (`#sites` as files, kv-style):

- `read_dir("#sites" i.e. ".")` → one entry per bound host (the host name).
- `open("#sites/<host>", read)` → a control file whose bytes describe the
  binding, e.g. `memory\n` or `cas <root-hash>\n` (a stable, greppable form).
- `open("#sites/<host>", write)` + write `cas <hash>` / `memory` → create/update
  the binding's *source descriptor*. (A `Memory(Arc<dyn FileSystem>)` source
  cannot be created over 9P by value; writing only sets/repoints to a `cas`
  source or marks a placeholder. Live in-memory sources are registered in-process
  via `bind_site`.) Reuse the kv "ensure key exists on open-for-write so a stat
  between open and commit succeeds" pattern (`wanix-kv/src/lib.rs:90-99`).
- `remove_file("#sites/<host>")` → unbind.

`resolve()` for `SiteSource::Cas` constructs a `wanix-site-cas::CasSiteFs`
(Phase 3); until Phase 3, `Cas` resolve returns `None`/`NotFound`. **Do not** put
a `ContentStore` dependency in `wanix-sites`; instead `SitesDevice::new_with_cas(
store: Arc<dyn ContentStore>)` (Phase 3) or hand `resolve` a store. Cleanest:
`wanix-sites` depends on `wanix-site-cas` (sync, no cas-device coupling) and
takes the store at construction in Phase 3.

> Lock discipline: `resolve()` must clone the `Arc<dyn FileSystem>` (or copy the
> `CasRootHash`) **out of the map lock**, then build/serve outside the lock — do
> not hold the `#sites` `RwLock` while reading the served FS.

### Bind `#sites` in serve

- `crates/wanix-cli/src/serve/roots.rs`: in `bind_host_and_terminal` (`:122`),
  bind `Arc::new(SitesDevice::new())` at `"#sites"` alongside the other devices.
  Store the `Arc<SitesDevice>` on `ServeRoots` (new field `pub(super) sites:
  Arc<SitesDevice>`) so the HTTP gateway can call `resolve()` without re-walking
  the namespace. Add `"#sites"` to `INSPECTABLE_SERVICE_DEVICES` (`:119-120`) —
  this automatically flows into discovery (`discovery.rs:159`) and the
  `serve_wanix_services_*` tests' device set.

### Host-header routing integration point

The gateway hook goes in **`crates/wanix-cli/src/serve/http.rs` `http_response`
at `:99-103`**, before the static fallback:

1. Parse the `Host` header from the request (add `fn request_host(request:
   &[u8]) -> Option<Host>` near the existing header parsers `:106-141`; reuse
   `request_header_str`). Strip `:port`, lowercase → `Host::parse`.
2. If `roots.wanix_services` and `roots.sites.resolve(&host)` yields a
   `FileSystem`, serve via the Phase 0 handler:
   `read_static_response(site_fs.as_ref(), &url_relative)`.
3. Otherwise fall through to the existing `routes::http_route_response(...)
   .unwrap_or_else(|| read_static_response(roots.site_root.as_ref(), ...))`.

Fallback rules (per design): a bare `localhost`, an IP literal, or an unbound
host → existing `--root` behavior. So only a **bound, non-bare** host short-
circuits to a site FS; everything else is unchanged. Bundle/well-known/app routes
keep priority (they run in `http_route_response`); the site lookup is the new
branch of the static fallback only.

### Serve flag to register a site at startup

Add `--site HOST=PATH` (repeatable) to register `Host` → `SiteSource::Memory(
LocalFs::new(PATH))` (or, with Phase 1, a freshly generated `MemFs`):

- `command/options.rs`: add `ServeValueOption::Site` to `SERVE_VALUE_OPTIONS`
  (`"--site"`), label `"serve --site"`, value name `"HOST=PATH"`.
- `command.rs`: `ServeCommand` gains `sites: Vec<(String,PathBuf)>`;
  `ServeCommandParts::set_site` parses `HOST=PATH`. `parse_serve_command` test
  coverage in `command.rs`/`tests`.
- `serve.rs:79-84`: pass `command.sites` into `ServeRoots::new`, which calls
  `sites.bind_site(...)` after building the device.

### Tests (Phase 2)

- `wanix-sites` unit tests: bind two hosts to two `MemFs` sources; `resolve`
  returns the right FS; control-file read shows the binding; write repoints;
  remove unbinds; `Host::parse` strips port + lowercases; bare `localhost`
  handling lives in the gateway test, not the device.
- serve integration test (`serve/tests.rs`): start serve with two in-memory
  sites bound (via a test `ServeRoots`/`SitesDevice::bind_site`), issue two GETs
  with different `Host:` headers, assert different bytes; a GET with `Host:
  localhost` (bare) falls back to `--root`.
- Discovery test: `#sites` appears in `services.devices` (extend the existing
  discovery JSON test).

### Module-line risk (Phase 2)

- `roots.rs` (210) gains a `#sites` bind + a struct field → still < 250, fine.
- `http.rs` gains `request_host` + the site branch; combined with Phase 0 this
  likely crosses 250 → execute the `static_serve.rs` split planned in Phase 0,
  and put `request_host` with the other request parsers (or in `request.rs`).
- `command.rs`/`options.rs` grow modestly; watch `options.rs`.

---

## Phase 3 — publish = freeze to `#cas`, bind host → root hash

**Goal.** Immutable publishing: freeze a site FS to a CAS root hash, serve it
read-only by hash, repoint a host for atomic deploy/rollback.

### New crate: `crates/wanix-site-cas`

Deps: `wanix-fs`, `wanix-cas`. Sync, no iroh/tokio. Two pieces:

**(1) FS-backed freeze** — `wanix-cas`'s `Capsule::freeze`/`WorldManifest::
materialize` are `std::fs`-only (`capsule.rs:244,202`). Add a parallel pair that
reads from a `FileSystem` (so an in-memory generated site freezes with **no host
disk**), reusing the `WorldManifest` wire form and `ContentStore`:

```rust
pub struct CasRootHash(ContentHash);
impl CasRootHash { pub fn hash(&self)->ContentHash; pub fn to_hex(&self)->String;
                   pub fn from_hex(s:&str)->Option<Self>; }

/// Walk `fs` from `root` (read_dir/open), put each file blob, build the sorted
/// WorldManifest, put the manifest blob; its hash is the CasRootHash.
pub fn freeze_fs(store:&dyn ContentStore, fs:&dyn FileSystem, root:&str)
    -> Result<CasRootHash, FreezeError>;
```

`freeze_fs` reuses `WorldManifest`/`ManifestEntry`/`to_blob` from `wanix-cas`
verbatim (the manifest is the durable format); it just sources bytes via
`FileSystem::{read_dir, open, metadata}` instead of `std::fs`, and enforces the
same `MAX_MANIFEST_ENTRIES` / `MAX_BLOB_SIZE` caps. Symlinks skipped (as
`collect_files` does). Do **not** hold any namespace lock while reading.

**(2) Read-only CAS-backed `FileSystem`** — `CasSiteFs`:

```rust
pub struct CasSiteFs { store: Arc<dyn ContentStore>, manifest: WorldManifest }
impl CasSiteFs {
    pub fn open_root(store: Arc<dyn ContentStore>, root: CasRootHash)
        -> Result<Self, LoadError>;   // fetch+parse manifest blob via Capsule::load-style path
}
impl FileSystem for CasSiteFs {
    fn open(&self,path,opts)->...     // read-only; reject write → PermissionDenied
    fn metadata(&self,path)->...      // file from manifest entry (len = blob len, cached);
                                      // a path that is a strict prefix of entries → Directory
    fn read_dir(&self,path)->...      // synthesize entries from manifest path prefixes
    fn content_hash(&self,path)->Ok(Some(entry.hash))  // exact CAS offload hook
    // mutators → default Err(NotSupported)/PermissionDenied
}
```

The manifest (`path -> hash`, sorted) is enough to synthesize the directory tree:
`read_dir("concepts")` returns the distinct next path segments of entries under
`concepts/`; `open("concepts/foo/index.html")` does `store.get(entry.hash)` and
serves an in-memory cursor `File`. A tiny `struct CasBytesFile { bytes: Vec<u8>,
pos: usize }` implementing `File::read` (read-only) backs `open`. This composes
with Phase 0 directly: `read_site_file(&cas_site_fs, "concepts/foo")` works
unchanged because directory-index + read go through the trait.

> Keep `CasSiteFs` < 250 lines; split the directory synthesis into
> `src/tree.rs` and the `File` impl into `src/file.rs` if needed.

### Wire freeze/publish into `#sites`

- `wanix-sites` gains a CAS-aware constructor: `SitesDevice::with_store(store:
  Arc<dyn ContentStore>)`. `resolve()` for `SiteSource::Cas(root)` builds
  `CasSiteFs::open_root(store.clone(), root)` (outside the map lock) and returns
  it as `Arc<dyn FileSystem>`.
- `roots.rs`: build the `SitesDevice` with the **same** `LocalCasStore` instance
  used for `#cas` (currently `CasDevice::new(Arc::new(LocalCasStore::
  open_default()))` at `:158`) — hoist that `Arc<LocalCasStore>` to a local and
  share it with both `#cas` and `#sites`, so a blob ingested via `#cas` (or a
  freeze) is readable by a site by hash.
- Publish flow (a serve-side helper, e.g. in `serve/site.rs`): given a generated
  site FS, `freeze_fs(store, &site_fs, ".")` → `CasRootHash`; then
  `sites.bind_site(host, SiteSource::Cas(root.to_hex()))`. Rollback = bind an
  earlier hash. This is the "no file copying / atomic repoint" of the design.
- (Optional CLI affordance) `--publish HOST` or writing `cas <hash>` to
  `#sites/<host>` over 9P; both end at `bind_site`.

### Tests (Phase 3)

- `wanix-site-cas`:
  - `freeze_then_cas_fs_roundtrip`: build a `MemFs` site, `freeze_fs` →
    `CasRootHash`, `CasSiteFs::open_root`, assert `read_site_file` returns
    byte-identical content for several paths incl. directory-index; `read_dir`
    synthesizes the right tree; writes are rejected.
  - `freeze_is_deterministic`: same tree → same root hash (manifest is sorted).
  - `content_hash_offload`: `CasSiteFs::content_hash` returns the entry hash.
- End-to-end (in `wanix-cli` or `wanix-sites`): generate (Phase 1) → `freeze_fs`
  → bind host → CasRootHash → GET pages == live output bytes; mutate the source,
  freeze again (new hash), assert the **new** hash serves new content and the
  **old** hash still serves old content (immutability + rollback proof).

### Module-line risk (Phase 3)

- `wanix-site-cas` split into `lib.rs` (types + `freeze_fs`), `casfs.rs`
  (`CasSiteFs`), `tree.rs`, `file.rs` from the start.
- `roots.rs` hoisting the shared store + the `#sites` store wiring stays small;
  recheck against 250.

---

## Cross-cutting checklist for every phase

- New crate → add to root `Cargo.toml` `members` **and** the `justfile:4` `fmt`
  `--package` list (fmt --check only covers listed packages); run plain `cargo
  build` and commit the refreshed `Cargo.lock`.
- New crate must build for the host target; only the SSG `wasm-src` targets
  `wasm32-wasip1` and lives in its own nested workspace with a checked-in
  artifact.
- No tokio/iroh/Wasmtime in `wanix-site-fs`, `wanix-sites`, `wanix-site-cas`,
  `wanix-site-gen` (host lib). The SSG runs *as* a wasm guest under the existing
  `wanix-wasm` runtime; these crates do not link Wasmtime.
- Public contracts use newtypes (`Host`, `SiteSource`, `CasRootHash`,
  `SiteRequestPath`) — no public raw ints/flags.
- Never hold a namespace/device lock while calling into another filesystem
  (`resolve()` clones out of the lock; `freeze_fs` does not hold the `#sites`
  lock).
- Run `just check` and commit per phase. `cargo test --workspace --locked` is the
  gate — the checked-in `.wasm` artifact keeps it toolchain-free.
