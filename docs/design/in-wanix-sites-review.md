# In-Wanix Sites — implementation review

Reviewer pass over branch `in-wanix-sites` vs base `d8a5dfa` (the commit before
the design doc). Four feature commits:

- `19cafa1` Phase 0 — serve through the `FileSystem` trait
- `2aaf1a5` Phase 1 — pulldown-cmark SSG to wasm, run as a Wanix task
- `207ad17` Phase 2 — `#sites` device + `*.localhost` gateway
- `bb19774` Phase 3 — publish = freeze to `#cas`, bind host -> root hash

## Quality gate

`just check` (fmt --check, module-lines, clippy -D warnings, test --workspace
--locked): **GREEN** (exit 0), re-run during this review.

- Module lines: hard limit 350 satisfied; only the three pre-existing
  over-250 warns (`wanix-agent/codex.rs`, `wanix-agent/exec_server.rs`,
  `wanix-cli/serve/http/app.rs`). `wanix-sites/src/lib.rs` is 372 *total* lines
  but under 350 non-test (the `#[cfg(test)] mod tests` body lives in
  `tests.rs`), so it passes.
- `Cargo.lock` and `wasm-src/Cargo.lock` are committed and clean; a
  `cargo build --locked` of the four new crates leaves the lock untouched.
- No `tokio` / `iroh` / `wasmtime` in any of the four new sync crates
  (`wanix-site-fs`, `wanix-site-gen`, `wanix-sites`, `wanix-site-cas`).
- The wasm SSG artifact (`fixtures/site-gen.wasm`) is byte-reproducible from
  `wasm-src/` (`cargo build --release --target wasm32-wasip1`, identical
  371424-byte output); `wasm-src` is a nested standalone workspace, not a
  parent member, so `--locked` host tests never need the wasm toolchain.

## Per-phase status

### Phase 0 — FS-backed serve — GREEN

- `wanix-site-fs::read_site_file` resolves a URL through the `FileSystem` trait:
  directory-index, MIME-by-extension, 64 KiB-chunk drain. Path safety is the
  `NormalizedPath::new` constructor (rejects `..`/absolute/empty) — no
  `std::fs::canonicalize`, no symlink-escape window.
- `static_serve.rs` cleanly splits the FS-backed `read_static_response` (site /
  `--root`) from the retained disk `read_disk_static_response` (workbench
  assets only). The only `std::fs` left in HTTP routing is the legitimate
  workbench asset-root discovery (`routes.rs:49`).
- `ServeRoots.site_root` is a `LocalFs` over the same dir for `--root`, so
  existing behavior is preserved; an in-memory `MemFs` test proves the
  no-disk path. Verified the site path no longer touches `std::fs`.

### Phase 1 — wasm SSG as a Wanix task — GREEN, with one correctness bug

- Clean host/wasm split via the `SiteIo` trait; the same `generate_site` logic
  runs in host tests and in the `wasm32-wasip1` guest (libstd -> WASI). The
  wasm task test (`tests/wasm_task.rs`) runs the real checked-in artifact
  through `WasmTaskDriver` into a shared `MemFs` and asserts page count,
  frontmatter stripping, and a rewritten internal link. pulldown-cmark with GFM
  options, as specified (no comrak/C deps).
- **BUG (must fix): `links.rs::rewrite_links` mangles all non-ASCII text.**
  The fallback branch does `out.push(bytes[i] as char)`, casting a raw UTF-8
  byte to `char`. For any multi-byte character every byte becomes a separate
  Latin-1 `char`, so `café — naïve` renders as `cafÃ© â\u{80}\u{94} naïve`.
  All 120 corpus pages contain non-ASCII (em-dashes are pervasive), so the
  real generated site is mojibake in every page body. The existing link tests
  use ASCII-only fixtures and miss it. Reproduced standalone with the exact
  loop. Fix: iterate over `char`s (or copy the byte run between link matches as
  a `&str` slice) instead of `as char`; then rebuild and re-vendor
  `fixtures/site-gen.wasm`. Add a non-ASCII assertion to the link tests so the
  regression is caught. This is the single blocking issue for Phase 1 to be
  "truly done"; structurally the phase is otherwise complete and correct.

### Phase 2 — `#sites` device + gateway — GREEN

- `SitesDevice` is a plain `FileSystem` (kv-style): `read_dir` lists hosts,
  read shows the `memory`/`cas <hash>` descriptor, write commits-on-drop,
  `remove_file` unbinds, with the kv ensure-on-open-for-write pattern. `Host`
  newtype lowercases + strips port and rejects bare `localhost`/IP in `parse`
  (gateway path) while `registered` keeps them (programmatic path). `SiteSource`
  is a proper enum; no raw int/flag public APIs.
- Gateway hook (`http.rs::site_gateway_response`) reads the `Host` header,
  only short-circuits for a bound non-bare host under `--wanix-services`, and
  otherwise falls through to `--root` — matches the design. `#sites` is bound
  in `roots.rs`, added to `INSPECTABLE_SERVICE_DEVICES`, and flows into the
  discovery JSON (test updated). `--site HOST=PATH` is parsed and registered as
  a `LocalFs` `Memory` source at startup.
- Lock discipline correct: `resolve()` clones the source out of the `RwLock`
  read guard before building/serving any filesystem.
- Minor: the serve gateway integration test proves two distinct hosts route to
  distinct filesystems but does not assert the bare-`localhost` -> `--root`
  fallback end to end (only `Host::parse` unit-covers it). Test gap, not a bug.
- Minor wart: an opened-for-write-then-never-committed binding leaves the
  `ensure_host` placeholder `SiteSource::Cas("")` in the table, which lists as
  `cas \n` and resolves to `None`. Harmless but slightly untidy.

### Phase 3 — freeze to `#cas` + rollback — GREEN

- `wanix-site-cas::freeze_fs` walks the site through the `FileSystem` trait
  (no `std::fs`), reuses the `wanix-cas` `WorldManifest` wire form verbatim,
  enforces `MAX_MANIFEST_ENTRIES`/`MAX_BLOB_SIZE`, skips symlinks, and is
  deterministic (sorted `BTreeMap`). `CasRootHash` is a newtype with hex
  round-trip. `CasSiteFs` is a read-only `FileSystem` that synthesizes the
  directory tree from manifest path prefixes, rejects writes, and offloads
  `content_hash` to the entry hash. Split across `lib.rs`/`casfs.rs`/`tree.rs`/
  `file.rs`, all under limit.
- `SitesDevice::publish` freezes (outside the binding lock) and repoints; the
  same `LocalCasStore` instance backs both `#cas` and `#sites` in `roots.rs`,
  so a freeze is a pure name repoint with no copying. The end-to-end test
  (`tests/publish.rs`) generates v1 via the wasm task, freezes, serves by hash
  byte-identical to live, regenerates v2 (distinct hash), serves new content,
  and rolls back to v1's hash with the old bytes intact — the full
  immutability + rollback proof, all in-memory.
- `manifest_from_entries` falls back to `WorldManifest::default()` if
  `from_blob` ever rejects the freshly serialized blob; argued unreachable
  given prior cap check + `NormalizedPath`-safe paths. Acceptable, but a silent
  empty-manifest on a future invariant break would be hard to debug — consider
  surfacing it as a `FreezeError` instead of `unwrap_or_default`.

## Verdict

- **Phase 0: done.**
- **Phase 1: structurally done, but NOT shippable** until the
  `rewrite_links` `as char` UTF-8 corruption is fixed and the wasm fixture is
  rebuilt — every real corpus page body is currently mojibake. Add a non-ASCII
  link-rewrite test alongside the fix.
- **Phase 2: done.** (Optionally add the bare-`localhost` fallback assertion and
  tidy the uncommitted-write placeholder.)
- **Phase 3: done.** (Optionally promote the `manifest_from_entries` silent
  fallback to a real error.)

What a human must finish: fix the non-ASCII rendering bug in
`crates/wanix-site-gen/src/links.rs::rewrite_links`, rebuild + re-vendor
`crates/wanix-site-gen/fixtures/site-gen.wasm` from `wasm-src/`, add a non-ASCII
regression test, and re-run `just check`. The optional items above are
follow-ups, not blockers.
