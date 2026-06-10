# Wanix Site — Research Briefs (appendix)

> Raw research that grounds the plan. Six briefs.



---

# Core architecture and foundational concepts of the Wanix Rust-native port

## Wanix Rust-native core: architecture & foundational concepts

Wanix is "Plan 9 reincarnated" in Rust. The north star (AGENTS.md:5-10, ADR 0001) is a Rust-native core that runs *outside Chrome*, with Wasmtime as the execution substrate and QuickJS/WASI as the first task runtime. Browser support is a frontend, not the foundation. The deepest design commitment is Plan 9's: **everything is a file, namespaces are per-process, and the file is the universal currency** — which is what makes both local services and the cross-machine mesh fall out of one contract.

### Everything is a file
The whole system is built on one trait, `wanix_fs::FileSystem` (`crates/wanix-fs/src/traits.rs:153-334`), plus a per-handle `File` trait (`traits.rs:66-150`). Every capability is a `FileSystem`: in-memory (`MemFs`), host-directory-backed (`LocalFs`), the task device (`TaskFs`), terminals, and the service devices `#kv`/`#pipe`/`#plumb`/`#cas`/`#agent`. Because each device is "a plain `FileSystem`," it imports across the mesh for free (AGENTS.md:59-60). Paths are `NormalizedPath` (`crates/wanix-fs/src/path.rs`): relative, slash-separated, no `.`/`..`, with `.` as the root — the Go `io/fs.ValidPath` shape. Service devices are named with a leading `#` (e.g. `#task`, `#term`, `#kv`); these are hidden from union directory listings (`crates/wanix-vfs/src/readdir.rs:101-103`).

The `File` trait carries device-aware semantics beyond plain bytes: `read_ready`/`write_ready` (default ready, overridden by queue-aware devices like terminals — ADR 0003), `is_seekable` (regular files seek; service streams refuse it), and `content_hash` (`traits.rs:182-205`), the *only* hook by which the 9P control plane offloads large file bytes to a content-addressed data plane (BLAKE3-verified blobs), threshold 256 KiB (`lib.rs:22-31`).

### Per-process namespaces / VFS binding & resolution
`wanix-vfs` owns the Plan 9 namespace. A `Namespace` (`crates/wanix-vfs/src/lib.rs:33-52`) is itself a `FileSystem` whose state is a `BTreeMap<destination, Vec<BindTarget>>`. `bind()` (`crates/wanix-vfs/src/binding.rs:60-82`) splices any `FileSystem` at a destination path with `BindPosition::{First, Replace, Last}` — this is union mounting. Resolution (`crates/wanix-vfs/src/resolution.rs:10-29`) finds all candidate bind targets whose destination is a prefix of the requested path, rebases the remainder onto each target's source, and sorts by **longest-destination-first** (most specific binding wins). `open`/`metadata`/mutations iterate candidates and take the first that resolves (`lib.rs:55-94`), giving union-directory semantics; `read_dir` merges entries across all bindings into one synthesized view (`readdir.rs`). Because `Namespace` is `Clone`, a child task gets a private copy of the parent's binding table — that is per-process namespaces (table.rs:99-101).

Two refinements matter for the mesh trust boundary. `SubtreeFs` (`crates/wanix-vfs/src/subtree.rs`) re-roots a backing FS at a prefix and gates every method through one central `require(Read|Write)` check returning `PermissionDenied` — "a capability is a bind" (mesh doc Slice 2). It also closes a symlink-escape: re-rooting only rewrites the path *string*, so a symlink inside a granted prefix pointing at a sibling still inside the backing root could escape; every symlink-dereferencing method calls `confine_to_prefix` (`traits.rs:229`, overridden by host-backed FS to canonicalize and reject out-of-prefix targets; no-op for opaque-symlink `MemFs`).

### Tasks & the fd table
`wanix-task` owns process identity, independent of any engine (ADR 0001, ADR 0002). A `Task` (`crates/wanix-task/src/task.rs:16-19`) is `Arc<Mutex<TaskState>>`; state (`task/state.rs:10-22`) holds `id`, `parent`, `kind`, `cmd`/`cmd_argv`/`env`/`dir` (file-oriented task control), `exit`, a private `Namespace`, and an `FdTable`. `TaskId` is a `NonZeroU64`; root tasks start at 1. The `FdTable` (`crates/wanix-task/src/fd.rs:128-231`) reserves fd 0/1/2 (STDIN/STDOUT/STDERR) and allocates dynamic fds from 3; `OpenFile` wraps `Arc<Mutex<Box<dyn File>>>` so an fd bind captures a shared open-file handle that survives the source task closing its fd (ADR 0002).

`TaskTable` (`crates/wanix-task/src/table.rs`) is the shared registry + driver map. Allocation auto-binds a `#task` view into the new task's namespace at `#task` with `Replace` (table.rs:184-192), so every task sees its own `#task/self/...`. `TaskFs` (`crates/wanix-task/src/task_fs.rs`) exposes `#task/new/<kind>` (allocate), `#task/<id>/{cmd,env,dir,exit,fd/<n>,...}`, and `#task/self`. A `TaskDriver` (`crates/wanix-task/src/driver.rs`) has `check()` (auto-select by program suffix, e.g. `.js`→qjs, `.wasm`→wasm) and `start()`. Critically, `wanix-task` must NOT depend on `wanix-wasi`/`wanix-qjs`/`wanix-wasm` (AGENTS.md:133-138); the runtime crates register drivers into the table from above. `task_command.rs` is the single extraction of program/argv/env/cwd shared by all WASI drivers.

### The 9P contract
9P is the wire protocol for every external client (Linux, v86, editor, browser) and for the mesh (ADR 0004). It is split: `wanix-protocol` is dependency-free wire codecs (frame split, tag extraction, version negotiation, typed 9P2000.L + Google.1/.2 op codecs); `wanix-9p` is the server mapping fids→`FileSystem` objects (`crates/wanix-9p/src/lib.rs:85-93`, `dispatch.rs` lists the full op set: version/attach/walk/(walkgetattr)/lopen/lcreate/read/write/readdir/getattr/setattr/clunk/mkdir/rename/remove/symlink/link/readlink/statfs/lock...). Version negotiation rejects unsupported versions; `Tauth` is deliberately `ENOSYS` (`session.rs:71-74`) because identity is a transport property, not in-band. Errors map to Linux errnos. The same server contract runs over stdio, TCP, WebSocket, and `serve`'s HTTP composition — transports are adapters that preserve binary frame boundaries. The mesh's `wanix-9p-client::RemoteFs` is the *photographic negative*: a `FileSystem` that encodes T-messages and decodes R-messages over any `Box<dyn Duplex>`, reusing the same codecs — this is Plan 9 *import* (`/n/<peer>`), the half Wanix was missing (mesh doc Slice 1). Because devices are files, importing a peer brings `#kv`/`#cas`/`#agent` across for free.

### Wasmtime as substrate
Wasmtime hosts guests; it is "not allowed to inherit host filesystem or process semantics by default" (ADR 0001). QuickJS/WASI is the first task runtime, compiled `wasm32-wasi` the second — both are WASI guests but neither becomes a second process model (ADR 0002). `wanix-wasi` owns Preview-1 syscall semantics backed by Wanix namespaces/fds (not host WASI). Host files enter only through explicit rooted `LocalFs` mounts with escape checks — not ambient authority (ADR 0001, walkthrough §3). The compiled-artifact cache (`wanix-module-cache`) is a real trust boundary: `deserialize` is `unsafe` native-code loading, so the cache dir is owner-private with fd-based TOCTOU-free verification (ADR 0002).

### Crate layering & dependency rules
Strict downward layering keeps core contracts free of engines and network (AGENTS.md:100-138): `wanix-fs` → `wanix-vfs`/`wanix-task`; `wanix-protocol`/`wanix-9p`; runtime crates (`wanix-qjs*`, `wanix-wasm`, `wanix-wasi*`) sit above and pull in Wasmtime; service devices are plain `FileSystem`s over `wanix-fs`. **`wanix-mesh` is the single async/iroh edge** — tokio and iroh appear nowhere else, including the sync 9P core it reuses (AGENTS.md:135-138). Modules stay under 250–350 non-test lines; public contracts use explicit newtypes, not raw `i32` flags (AGENTS.md:249-260).


**Honest caveats / do-not-overclaim:**
- Tauth is deliberately ENOSYS; there is no in-band 9P authentication. Public/multi-user auth and Ethernet/vnet are explicitly unimplemented trust-boundary work (AGENTS.md:246-247, session.rs:71-74).
- The 9P session/namespace seam is incomplete: handle_attach decodes uname/aname but in the plain (non-policy) path every fid resolves through one shared P9Server.root; per-principal namespaces are a queued follow-up (AGENTS.md:376-380).
- Mesh attach scoping is single-attach-per-connection in v1; multi-attach per-fid sub-namespaces are deferred (mesh doc Slice 2:657-659).
- The served #agent uses a deterministic FakeEngine; the real codex app-server engine is local-trust CLI only (AGENTS.md:230-235).
- The serve 9P WebSocket handles one frame at a time per connection, so a blocking read (e.g. #plumb/<topic>/recv) cannot interleave with a write on the same connection — live pub/sub needs a second connection (AGENTS.md:242-247).
- serve has no shutdown signal and spawns an uncapped detached thread per connection; the cockpit's #plumb self-check probes only the publish path, and v86-shared-demo is still a no-op stub (AGENTS.md:357-371).
- Compiled wasm runtime is a command-style WASI subset: poll_oneoff is NOSYS, no readiness (AGENTS.md:162, wanix-wasi-host).
- The content-addressed data-plane offload is a FileSystem hook (content_hash) with a defined threshold, not yet a fully wired blob-fetch path in the default serve; most filesystems return Ok(None) (traits.rs:182-205).
- QEMU/v86 paths are validated launch/handoff contracts, not a VM supervisor; rootfs build and VM lifecycle are explicitly out of scope (ADR 0005, AGENTS.md:215-219).


---

# Task runtimes (qjs + compiled wasm), the shared WASI/fd contract, and the #term device + qjs-shell story

## Task runtimes and the terminal/shell story (Wanix Rust port)

Wanix runs code outside Chrome as **Wanix tasks** on Wasmtime. Two WASI Preview 1 task runtimes share one task/namespace/fd contract: **QuickJS** (`.js`, interpreted) and **compiled `wasm32-wasi`** (`.wasm`, near-native). ADR 0002 (`docs/adrs/0002-quickjs-wasi-task-runtime.md`) is the durable boundary; ADR 0003 (`docs/adrs/0003-terminal-device-and-shell-lifecycle.md`) governs `#term` and the shell.

### Running JS outside Chrome (qjs)
`wanix-qjs` (`crates/wanix-qjs/src/driver.rs`, `runner.rs`, `lib.rs`) adapts the engine crate `wanix-qjs-engine` (imported as `rust_wasi_quickjs`) into a Wanix task driver. QuickJS-NG is compiled to a WASI Preview 1 WebAssembly *reactor* and hosted by Wasmtime (`crates/wanix-qjs-engine/docs/architecture.md`). `QuickJsTaskDriver::check` claims a task whose program ends with `.js`; `start` calls `run_task_with_runtime_limits`, which reads the script from the task's namespace, builds a live WASI host (`WanixQuickJsWasiHost`, `crates/wanix-qjs/src/wasi_host.rs`), wires `scriptArgs`/env/cwd, runs the script (as ES module if it has `import`/`export`, else plain eval — `uses_module_syntax`), drains bounded event-loop work, writes output through the task fds, and records `Task::set_exit`. Guest JS uses `qjs:std`, `qjs:os`, `scriptArgs`, stdio, env, and service files; a `CONSOLE_PRELUDE` (`lib.rs:127`) installs `print`/`console` on top of WASI stdout/stderr because the fixture itself does not install `console`. The engine caches the ~1.7 MiB QuickJS module on disk (cold compile ~550 ms → warm deserialize ~0.5 ms, ~1000x).

### Compiled wasm task driver
`wanix-wasm` (`crates/wanix-wasm/src/driver.rs`, `runner.rs`, `lib.rs`) is the second WASI task runtime. `WasmTaskDriver::check` matches `.wasm`; `start` reads the module bytes from the task's own namespace, compiles via `WasiRunner::from_bytes_cached`, builds a live `WasiConfig` from the task (namespace, cwd preopen, env, argv, fds 0/1/2) and runs `_start`, recovering the exit code from `proc_exit` (which traps the guest after the host's exit hook — `crates/wanix-wasi-host/src/process.rs`). Unlike qjs, the wasm runner uses the standalone linker `wanix-wasi-host` (`add_to_linker`), which registers the Preview 1 command subset and crucially makes **`poll_oneoff` return `ERRNO_NOSYS`** (`crates/wanix-wasi-host/src/core.rs:37`) — so it is command-style only (`_start`, fd/path I/O, args/env, clock, exit), no readiness polling. A `.wasm` cmd auto-starts through `#task/new` exactly like a `.js` cmd; tests prove a qjs task and a wasm task built from the same `Namespace` share one filesystem (`crates/wanix-wasm/src/driver.rs` tests; `crates/wanix-cli/tests/shared_vfs_differential.rs`).

### The shared WASI/fd contract (ADR 0002)
`wanix-wasi` (`crates/wanix-wasi/src/lib.rs`, `task_config.rs`, `ctx.rs`) owns Preview 1 semantics backed by Wanix namespaces and WASI fds — **not** host OS filesystem semantics. `task_wasi_config(task)` is the single shared builder used by *both* runtimes: namespace backs WASI, task cwd is the root-preopen source, argv/env come from `wanix_task`, and fds 0/1/2 become guest stdio via `TaskFdFile`. Dynamically opened **regular-file** fds are mirrored into the task fd table through a `WasiFdObserver` (`TaskWasiFdMirror`) and released on guest close (directory fds stay WASI-internal). This fd-mirroring contract is shared, not reimplemented per runtime. Service paths matter: `WasiCtx::resolve_path` checks `is_rooted_service_path` (`ctx.rs:137`), so a guest can open any `#name` device path (e.g. `#kv/<key>`, `#task/self/dir`) from the namespace root regardless of cwd; cwd remapping does not re-root `#term`/`#task`. The two runtimes share `WasiCtx` and `task_wasi_config` but NOT the linker — qjs keeps its own engine-local host import path (snapshot blockers, live fd readiness, restore reattachment, richer `poll_oneoff`); only the wasm runner uses `wanix-wasi-host`.

### Compiled-artifact cache
`wanix-module-cache` (`crates/wanix-module-cache/src/lib.rs`) is the single audited cache for both runtimes: key = `sha256(wasm-bytes)`, advisory recompile-on-mismatch, loaded via `unsafe Module::deserialize`. Because deserialize is arbitrary-code-execution-equivalent, the cache dir is a trust boundary: on Unix, fd-based verification opens the leaf dir `O_NOFOLLOW|O_DIRECTORY`, `fstat`s it (owner==euid, no `0o022` bits), then `openat`s the artifact `O_NOFOLLOW` and `fstat`s it (regular file, owner, no group/other write) and reads from that same fd (no TOCTOU). Writes are equally hardened (`O_CREAT|O_EXCL|O_NOFOLLOW` + `renameat`). Defaults are owner-private per-user dirs; qjs uses `WANIX_QJS_CACHE_DIR`/`qjs-module-cache`, wasm uses `WANIX_WASM_CACHE_DIR`/`wasm-module-cache` (distinct so they never collide).

### The #term device + qjs-shell
`wanix-term` (`crates/wanix-term/src/lib.rs`) depends only on `wanix-fs` and implements the Plan 9-style `#term` service: reading `new` allocates a resource id; each resource exposes `id`, `ctl`, `data`, `program`, `winch`. Writes to `data` are read from `program`; writes to `program` are read from `data` with lone `\n`→`\r\n` (`files/stream.rs`). `winch` broadcasts `columns rows\n` to subscribers (`files/winch.rs`). The only `ctl` command is `close`, which removes the resource and invalidates handles (resource cleanup, not task signalling). Terminal read readiness is queue-aware. Terminal-backed tasks bind fd 0/1/2 to the `program` side; humans/browsers/editors attach to `data` (`crates/wanix-cli/src/qjs_term/terminal.rs::attach_task_terminal`).

The bundled shell is a **guest JS program** (`examples/qjs-term-shell-demo.js`, included via `include_str!` in `crates/wanix-cli/src/qjs_term.rs`), not a separate process model. It reads `#task/self/dir` as its cwd, opens `#term/<id>/winch`, registers `os.setReadHandler(0, ...)` and dispatches commands. Builtins: `cd ls cat write mkdir rm rmdir mv cp ln -s readlink stat lstat` (filesystem), `env setenv unsetenv` (task env via `#task/self/env`), `ps` (`#task` table), plus `pwd id status size echo later exit`. Child tasks launch by writing `#task/new/qjs` then the child's `cmd`/`env`/`dir`/`ctl` (`bind ... fd/0|1|2`, then `start`), supporting `< > 2>` stdio redirection and inheriting parent fds/env; exit is read back from `#task/<id>/exit`. Raw mode (`--raw`, `WANIX_QJS_SHELL_RAW=1`) gives the guest echo, backspace, Ctrl-C line cancel (`^C`), Ctrl-D exit; cooked mode buffers lines. Resize flows in through `winch` and is reported by `size`. Served sessions (`crates/wanix-cli/src/qjs_term/session.rs::QjsShellSession`) pump bounded ready-IO turns on input/resize and release the owned terminal via `#term/<id>/ctl` `close` on drop.

### Guest-JS guardrails
Guest JS must use `qjs:std`, `qjs:os`, `scriptArgs`, stdio, env, and service files — **no `globalThis.Wanix`** helpers and no read-only virtual-file projection bridge (ADR 0002 Consequences; engine read-only virtual files are fixture-only). Browser/editor/VM terminal clients forward control bytes (0x03/0x04); they do not invent task cancellation.

**Honest caveats / do-not-overclaim:**
- The compiled-wasm runtime is command-style only: wanix-wasi-host registers poll_oneoff as ERRNO_NOSYS, so wasm guests have no readiness/poll support (no async I/O). Only qjs has a richer poll_oneoff with live fd readiness.
- qjs and wasm share WasiCtx and task_wasi_config but NOT the linker — wanix-wasi-host is used only by the wasm runner; qjs keeps its own engine-local host import path. They are two host paths, not one.
- The shell is a single bundled JS guest program (examples/qjs-term-shell-demo.js) with a fixed builtin set; it has no pipes (|), no real job control, no command resolution beyond builtins + the synchronous `qjs` launcher, and no general external-program execution.
- Child qjs launches are synchronous and foreground-only (the shell reads #task/<id>/exit to block); there is no persistent foreground child terminal ownership or background jobs.
- Terminal #term/<id>/ctl supports only `close`; there is no signal delivery, process-group, or task-cancellation semantics — control bytes (0x03/0x04) are terminal input handled by the guest shell, not kernel signals.
- qjs bounded-execution knobs (interrupt poll budget, memory limit, event-loop wait budget, ready-IO turns) are host policy for an already-evaluated runtime, explicitly NOT a general scheduler, cancellation model, or checkpoint format.
- QuickJS snapshots are VM memory images, not Wanix task checkpoints; open dynamic descriptors block snapshot, and all live host state (namespace, fds, providers, clocks, callbacks) must be reattached on restore.
- Engine read-only virtual files exist only as standalone-engine fixture support; Wanix runtime paths must use live WASI providers, and guest code must avoid globalThis.Wanix.
- The module cache's strong fd-based ownership verification is Unix-only; on non-Unix it degrades to an 'is a directory' check and relies on the per-user default location for trust.
- Resize wakeups are not yet independent of stdin handling, and winch draining happens inside the read handler; AGENTS.md lists true resize wakeups, persistent foreground child terminal ownership, and cancellation as queued follow-ups.
- wanix-wasm fixtures include a small Rust guest (fixtures/rust-guest.wasm); broader real-world wasm/JS workloads beyond fixtures and demos are not demonstrated in-tree.


---

# Service Devices (#kv, #pipe, #plumb, #cas, #agent)

## Wanix Service Devices — reference brief

The mesh layer rests on five "service devices," each a plain `wanix_fs::FileSystem`. Because they are just filesystems, every one of them imports across the mesh for free (`/n/A/#kv/...`) with no device-specific networking. They are bound into the served namespace by `--wanix-services` in `crates/wanix-cli/src/serve/roots.rs` (`bind_host_and_terminal`, lines 122–172). The single source of the inspectable set is `INSPECTABLE_SERVICE_DEVICES = ["#task","#term","#kv","#pipe","#plumb","#cas","#agent"]` (`roots.rs:119`).

### `#kv` — key/value store (`crates/wanix-kv/src/lib.rs`, `files.rs`)
The smallest "real database inside Wanix": an in-memory `Arc<RwLock<BTreeMap<String, Vec<u8>>>>` (`lib.rs:21,33`). Durability is explicitly a follow-up (`lib.rs:6`).
- **`#kv/<key>`** is a file. Open read-only → `KvReadFile` serving a *snapshot taken at open* so a concurrent overwrite cannot tear an in-flight read (`files.rs:7-33`). Open with `write|create|truncate` → `KvWriteFile`, which buffers writes and commits the whole buffer on `Drop` (close) under one write lock (`files.rs:55-76`). The key is created eagerly at open (`ensure_key`, `lib.rs:93-100`) so a stat between open and close (the 9P `Tlcreate` flow) still finds it.
- **list `#kv`** (`read_dir` on root) enumerates keys (`lib.rs:142-154`); **remove** a key deletes the entry (`remove_file`, `lib.rs:156-165`).
- Path rule: a single segment is a key; any `/` in the path → `NotFound` (`parse_path`, `lib.rs:109-118`). Modes: value file `0o666`, dir `0o555`.
- Recipe 04 (`docs/recipes/04-tiny-http-app-with-kv.md`) builds a counter handler doing read-modify-write on `#kv/counter`; state survives between requests only because the `KvDevice` lives for the serve process's lifetime. In a standalone `wanix qjs` (no serve) `#kv` is unbound, so a fresh device resets the counter each run.

### `#pipe` — in-memory byte channels (`crates/wanix-pipe/`)
Shaped like `#term`: allocate then operate by id.
- **`#pipe/new`** — read-only; allocates a channel on first read and returns `<id>\n` (`files.rs:NewPipeFile`, `open.rs:14-17`). `alloc` increments `next_id` and inserts a `PipeChannel` (`lib.rs:78-89`).
- **`#pipe/<id>/id`** — read-only snapshot of the id (`open.rs:18-22`).
- **`#pipe/<id>/data`** — the unidirectional stream. Each handle is *exactly one direction*: opening with both read and write → `NotSupported`; `create|truncate` rejected (`open.rs:27-44`). Read end (`PipeReader`) drains buffered bytes, blocking until data or EOF; write end (`PipeWriter`) appends and, on `Drop`, decrements the writer count (`files.rs:90-145`). EOF (`read` returns `Ok(0)`) happens *only* when there are no buffered bytes **and** the last writer has been dropped (`channel.rs:75-96`). A 50 ms `wait_timeout` re-check bounds a missed wakeup; it is deliberately *not* an EOF signal (`channel.rs:7-15`).

### `#plumb` — plumber bus / best-effort pub-sub (`crates/wanix-plumb/`)
Plan 9 plumber idiom; a topic directory exists on demand (any name is a valid topic; no allocation step — `lib.rs:141-148`).
- **`#plumb/<topic>/send`** — strictly write-only (`files.rs:require_write_only`). One write = one `PlumbEnvelope` parsed as JSON `{kind, from, to, body}` (`envelope.rs`), serialized to one newline-terminated line, and published through the injected `PlumbPort` (`files.rs:43-65`). `kind` is the required routing tag; `from`/`to`/`body` default to empty/null; unknown JSON fields are rejected (`#[serde(deny_unknown_fields)]`, `envelope.rs:29`); envelopes capped at `MAX_ENVELOPE_LEN` = 64 KiB.
- **`#plumb/<topic>/recv`** — strictly read-only; blocking stream of the newline-JSON envelopes the subscription received *since it opened* (`files.rs:67-94`, `port.rs`).
- **Delivery is best-effort epidemic, NOT a durable queue**: a subscriber not listening when a message is sent never sees it; no acknowledgement; durable handoff belongs in `#kv` or a capsule (`lib.rs:1-16`, `envelope.rs:1-10`). The transport is a `PlumbPort`: `LocalPlumbPort` (in-process fan-out to live subscribers, `local.rs`) for single-node; `wanix-mesh`'s `GossipPlumbPort` maps each topic to an iroh-gossip topic. A `recv` subscriber's `LineBuffer` is capped at 16×`MAX_ENVELOPE_LEN` and drops oldest bytes under flood (`buffer.rs:30,62-79`). Topic names bounded at 255 bytes (`path.rs:MAX_TOPIC_LEN`); the root listing of "known" topics is a cosmetic, bounded (4096) best-effort set (`lib.rs:49-58`).
- **Single-frame serve caveat (live recv):** the single-connection serve handles one frame at a time, so a blocking `Tread` on `#plumb/<topic>/recv` cannot be interleaved with a `send` on the *same* connection — it would freeze the connection. The cockpit self-check therefore probes only the publish path; live end-to-end delivery needs a second 9P connection or concurrent frame handling (`docs/integration/STATUS.md:471-474`, `CLAUDE.md:243`).

### `#cas` — content-addressed store / venti (`crates/wanix-cas/`)
Blobs operable as ordinary files, keyed by BLAKE3 `ContentHash`. The device wraps any `ContentStore`; served binding uses the owner-private on-disk `LocalCasStore::open_default()` (`roots.rs:157-162`).
- **`#cas/<hash>`** — read-only; returns the blob's bytes (store-verified), stat reports length; writing refused (`device.rs:139-145`). The `<hash>` is validated via `ContentHash::from_hex` before any lookup, rejecting hostile inputs.
- **`#cas/ingest`** — write-then-read-hash. Writing bytes and closing stores them as a blob (`IngestFile` buffers, commits on `Drop`, `device/files.rs:62-125`); a subsequent *read* of `#cas/ingest` returns the lowercase-hex hash of the most recently ingested blob (`last_ingest` slot, `device.rs:124-138`). Writes are capped at `MAX_BLOB_SIZE` (256 MiB) *at write time*, and an over-cap stream poisons the handle so no truncated blob is published.
- **`#cas/have/<hash>`** — read-only presence probe; returns `1\n` if present locally, `0\n` if not (`device.rs:110-117,146-151`).
- The keyspace is **not enumerable** by design: root lists only `ingest` and `have/`; `have/` lists nothing (`device.rs:167-179`). Every read re-hashes bytes end-to-end (`store.rs:verify_hash`); the local store is owner-private with atomic writes and fd-verification (`local.rs`). Equal bytes deduplicate to one entry. `#cas` also backs `.wcap` capsules: a world's files become blobs, the sorted manifest is itself a blob whose hash *is* the capsule id, and `materialize` re-applies path safety + size/fan-out caps (`capsule.rs`).

### `#agent` — an LLM session as files (`crates/wanix-agent/`)
Shaped like `#term`/`#pipe`: read `new` to allocate, then operate `<id>/...`. Backed by a pluggable `AgentEngine` producing `AgentSession`s (`engine.rs:15-84`). Path parsing in `path.rs`; file handles in `files.rs`; device in `lib.rs`.
- **`#agent/new`** — read-only; first read allocates a session (starting an engine session) and returns `<id>\n` (`files.rs:NewAgentFile`, `lib.rs:alloc:87-94`).
- **`#agent/<id>/id`** — read-only `<id>\n`.
- **`#agent/<id>/prompt`** — write; each write `submit`s the (trim-trailing) text as a new turn (`files.rs:96-110`).
- **`#agent/<id>/events`** — read; blocking, streaming normalized JSONL events (one event per line; e.g. `turn.started`, `message.delta`, `message`, `approval.needed`, `turn.completed`). EOF only on session close (`files.rs:112-136`, `engine.rs:EventStream:139-166`).
- **`#agent/<id>/reply`** — read; opening blocks until the latest turn completes and returns its final assistant message as one single-read, EOF-terminating chunk. This is what makes one agent delegating to another clean (`engine.rs:wait_reply:71-80`, `lib.rs:161-165`). Recipe 05 has agent A block on `cat #agent/B/reply` while B works.
- **`#agent/<id>/pending`** — read; JSON array of open approval requests, e.g. `[{"id":"req-1","action":"..."}]` (`lib.rs:154-160`, `fake.rs:pending:126-134`).
- **`#agent/<id>/ctl`** — write; control verbs `close`, `approve <req>`, `deny <req>` (`files.rs:CtlFile:155-185`). `approve`/`deny` resolve a parked approval via `AgentDevice::resolve` → `AgentSession::resolve`; `close` removes the session and ends the event stream.
- **`#agent/<id>/status`** — read; one-line `<engine> <state> turns=<n>` (`lib.rs:145-149`, `fake.rs:status:111-124`).
- **Approvals are files**: a powerful action (edit/command) parks in `pending` and nothing happens until a human writes `approve <id>` to `ctl` — the trust gate (Recipe 01, "What you just exercised").
- **Engine duality**: the **served** `#agent` deliberately uses the deterministic `FakeEngine` (`roots.rs:163-170`) because the real `codex app-server` bridge requires auth + unattended execution and is **local-trust only**, staying on the CLI path. `FakeEngine` replies `you said: <prompt>`, and a prompt prefixed `approve:` parks an approval to exercise the trust boundary (`fake.rs`). The CLI `wanix agent` defaults to `CodexEngine` (codex app-server subprocess against a confined Wanix world) and falls back to `FakeEngine` under `--fake` (`crates/wanix-cli/src/agent.rs:42,83-90`; `crates/wanix-agent/src/codex.rs:48-90`). The wire shape (the file tree) is identical across engines, so anything learned against the fake transfers. Agents can also delegate to remote agents via `RouterEngine`/`RemoteEngine`, and `POST /agent` exposes the agent as a loopback network service.

### Cross-cutting invariants
All five devices follow the same blocking-stream EOF contract (`Ok(0)` only at true end-of-stream, 50 ms re-check timeout never signals EOF). All are clone-cheap (shared `Arc` state) so binding into several namespaces routes to one device. `wanix-mesh` is the only async/iroh crate; the devices themselves are synchronous and iroh-free, which is what lets them live in core crates and import across the mesh unchanged.

**Honest caveats / do-not-overclaim:**
- #kv is in-memory only (Arc<RwLock<BTreeMap>>); values do not survive a serve restart. Durability/content-addressed backing is an explicit follow-up; persist by freezing into a capsule (crates/wanix-kv/src/lib.rs:6).
- Live #plumb/<topic>/recv cannot be exercised end-to-end over a single serve connection: the serve handles one frame at a time, so a blocking recv would freeze the connection. The cockpit self-check probes only the publish path; true live delivery needs a second 9P connection or concurrent frame handling (docs/integration/STATUS.md:471-474, CLAUDE.md:243).
- #plumb is best-effort epidemic pub/sub, not a durable queue: a subscriber that was not listening when a message was sent never sees it, and there is no acknowledgement. recv buffers are lossy (oldest bytes dropped) under flood (envelope.rs:1-10, buffer.rs:30).
- The served #agent uses the deterministic FakeEngine, NOT a real LLM. The real codex app-server engine is local-trust only and lives on the CLI path (wanix agent); do not claim the served cockpit runs a real LLM (roots.rs:163-170, crates/wanix-cli/src/agent.rs).
- The #kv-backed HTTP-app route (GET/POST /apps/<name>) is NOT yet wired on this branch; only POST /agent exists as a loopback qjs-shaped HTTP endpoint. Recipe 04 drives the handler via the qjs CLI or the agent endpoint, not a real app route (docs/recipes/04-tiny-http-app-with-kv.md:131-156).
- #cas blob keyspace is intentionally not enumerable: read_dir on root lists only ingest and have/, and have/ lists nothing. A CAS is looked up by hash, never browsed (device.rs:167-179).
- #cas reading #cas/ingest before any ingest returns an empty string (last_ingest is None -> default), not an error (device.rs:131-137).
- #cas/ingest writes are capped at MAX_BLOB_SIZE = 256 MiB; an over-cap stream is rejected and the handle poisoned so no truncated blob is published (store.rs:24, device/files.rs:90-100,111-117).
- #agent/<id>/ctl currently supports only close, approve <req>, and deny <req>; any other verb returns NotSupported (files.rs:168-177).
- Module-line health: wanix-agent/src/codex.rs (~307) and exec_server.rs (~283) sit above the 250-line warn limit and are queued for splitting (CLAUDE.md Queued Follow-ups).


---

# mesh

## The Mesh: Wanix's distributed half

Wanix is "Plan 9 reincarnated" — per-process namespaces, file-shaped services, a 9P server that can *export* any namespace. The mesh builds the half that was missing: the 9P *client*, and on top of it, distributed identity, capability grants, content-addressed bulk transfer, and remote exec — all over iroh QUIC. The argument of the philosophy essay (`docs/mesh-the-missing-half-of-9p.md`) is precise: "everything is a file" is load-bearing because then *one* file-transport protocol (9P) plus *one* placement operation (namespace bind) gives network transparency for every service at once. You write a 9P client once, and every file-shaped service any node exports (`#term`, `#task`, `#kv`, `#agent`) becomes reachable. The essay's second claim is that **agents are the operators these mechanisms always needed** — humans found per-process namespace rearrangement too fiddly to use daily; an LLM that reads state by `cat`, mutates by `write`, and lists capabilities by `ls` finds it native.

### RemoteFs — the import half (`crates/wanix-9p-client/src/remote.rs`)
`RemoteFs` implements the fully synchronous `wanix_fs::FileSystem` by speaking 9P to a server over any `Box<dyn Duplex>` (anything `Read + Write + Send`). It is the photographic negative of the server's `serve_stream`: where the server decodes T-messages/encodes R-messages, the client does the inverse, reusing the *existing* `wanix-protocol` codecs (no new wire format). It depends on only `wanix-fs` + `wanix-protocol` — no transport, no async, no iroh (`Cargo.toml`). Concurrency matches the server's strict serial loop: the connection lives behind `Arc<Mutex<P9Conn>>`, one request outstanding; concurrent callers serialize on the mutex. Bind a `RemoteFs` into a `Namespace` and a remote tree — files and `#`-devices alike — becomes part of the local namespace. That is Plan 9 import realized. Five hardening corrections are baked in (`docs/mesh-the-missing-half-of-9p.md` §"five corrections", verified in source): (1) frame-size ceiling ≤ negotiated `msize` *before* allocating a body, so a hostile server can't OOM the importer; (2) honest seekability — a `Tgetattr` at open decides if a file is regular; non-regular service streams report `is_seekable()==false` and refuse `seek` rather than fabricating offsets; (3) RAII `ScratchFid`/`RemoteFile` guards that clunk fids on every drop path including panic; (4) bounded `read_dir` cookie loops (max entries, max iterations, bail on non-advancing cookie); (5) `O_APPEND` delegated to the server, never raced client-side. The client offers `9P2000.L.Google.2` so a capable server collapses walk+stat into one `Twalkgetattr`.

### 9P over iroh QUIC + ALPN (`crates/wanix-mesh`)
`wanix-mesh` is the *only* async crate and the only one that touches iroh or tokio (CLAUDE.md dependency rule). A `MeshNode` (`src/node.rs`) owns a held multi-thread tokio runtime and binds **one `iroh::Endpoint` per node** from the persisted node secret key. It can bind `Public` (n0 preset: relays + DNS discovery, crosses NATs) or `bind_local`/`Minimal` (relays + DNS disabled, pinned to one IP socket — the testable/LAN form). It exports a Wanix namespace as 9P over QUIC under ALPN `WANIX_9P_ALPN = b"wanix/9p/1"` (`src/lib.rs`). The sync↔async bridge (`src/duplex.rs`) is the single seam: `BlockingReader`/`BlockingWriter`/`BlockingDuplex` drive iroh's async `SendStream`/`RecvStream` on a *held* runtime `Handle`, never `Handle::current().block_on` on a worker thread (which panics) — inbound runs inside `spawn_blocking`, outbound `FileSystem` calls run on non-runtime threads. Inbound (`src/handler.rs`): `P9ProtocolHandler::accept` reads the verified peer key, then per accepted bidi stream runs the unchanged `P9Server::serve_stream` in `spawn_blocking`. Concurrency is capped: `MAX_CONCURRENT_SESSIONS = 512` (one blocking-pool thread per live session) plus a per-op deadline (`DEFAULT_OP_DEADLINE = 30s`) on writes — the idle read is deliberately *not* deadline-bound so an idle mount isn't torn down. One important reality note: the `Cargo.toml` pins **iroh `=1.0.0-rc.1` + iroh-blobs `0.102.0` + iroh-gossip `0.100.0`** — the blueprint had guessed ~0.97/~0.96, and the manifest comment documents the correction to "what compiles."

### Identity: persisted ed25519, the key is the address (`crates/wanix-id`)
A node's identity is a 32-byte ed25519 seed (`src/identity.rs`). `NodeIdentity::load_or_create` reads/creates `~/.wanix/node.key` with `0600` permissions, stable across restarts; the seed is serialized raw (not via an iroh re-export). `PeerId` (`src/peer.rs`) is the raw 32-byte public key — equality is by key, hex is the 64-char display. The public key is simultaneously the identity *and* the iroh `EndpointId` (the dialable address), which is what makes `/n/<node>` workable on the open internet. The QUIC handshake authenticates the peer's key in the transport, so Plan 9's factotum becomes a property of the connection: in-band `Tauth` stays `ENOSYS` forever. The server *never* trusts a client-claimed `uname` (`src/handler.rs` reads `connection.remote_id()`; 0-RTT is never used, so identity is proven before any `Tattach`).

### Default-deny grants / AttachPolicy: a capability is a bind
Authorization is the Plan 9 way — not an ACL bolted on, but a re-rooted namespace. `AttachPolicy::evaluate(peer, aname) -> Option<Authorization>` (`crates/wanix-id/src/policy.rs`) is the whole trust boundary as one pure function. `GrantTablePolicy` consults a `GrantTable` (`src/grant.rs`): an `Arc<RwLock<Vec<Grant>>>`, **default-deny**, matched exactly by `(peer, aname)` with no wildcards; clones share one backing list so it is live-editable. A matching `Grant::authorize()` materializes into a `wanix_vfs::SubtreeFs` re-rooted at the grant prefix with `Rights` — "a capability is a bind" in five lines. `SubtreeFs` (in `wanix-vfs`, not `wanix-id`) enforces rights *inside the filesystem* via one central `require(right)` per method, so even code holding a `SubtreeFs` can't mutate past its grant; it also closes a symlink-escape hole via a `confine_to_prefix` hook (`LocalFs` canonicalizes resolved targets and rejects out-of-prefix; opaque `MemFs` is a no-op). `P9Server::with_policy(root, peer, policy)` installs the scoped root at attach or denies with EACCES; `with_policy(None)` is byte-for-byte the old server.

### Peers at /n/<peer>, devices imported for free
Because `RemoteFs` is just a `FileSystem`, `Namespace::bind(remote, ".", "n/<node>", …)` mounts a remote namespace and every device imports across the mesh for free: `/n/A/#kv/<key>`, `/n/A/#agent/...` "just work" through the one client. **Honest caveat (recipe 02):** the shipped `mount-*` verbs always bind at `/n/remote` (`MOUNT_POINT` in `mount.rs`), a single mount slot — the peer-id lives in the `iroh://<peer>?addr=...` ticket, not yet in the namespace prefix. Per-peer `/n/<peer-id>/` mounts are a queued follow-up (the "9P session/namespace seam" in CLAUDE.md). Blocking streaming opens (`#agent/<id>/events`, `#agent/<id>/reply`, `#plumb/<topic>/recv`) would deadlock the serial connection, so `StreamingImportFs` (`src/streaming.rs`) gives each such open its own dedicated bidi stream.

### #cpu exec plane — cpu(1) over the mesh (`crates/wanix-cpu`, `crates/wanix-mesh/src/cpu`)
A caller dials a node (ALPN `wanix/cpu/1`), opens two role-sorted bidi streams (control + export; a 1-byte role discriminator written first because QUIC streams arrive in first-write order, not open order), and **reverse-exports** a scoped namespace. The acceptor runs `run_job` — the *exact* local launch pattern `allocate_root → task.bind(world, ".", ".") → configure → start`, only the world is a `RemoteFs` over the export stream. Compute travels to the data. The reverse export is an `ExportScope` (`src/scope.rs`): the job subtree as a `SubtreeFs`, **read-only by default** (opt into `--write`), plus only explicitly granted services — never the whole host root. The acceptor is **grant-allowlisted** (`CpuAcceptor`, `src/cpu/handler.rs`): a non-allowlisted peer's connection is closed before any stream is accepted. Two honest v1 limits stated plainly in the crate doc: output is delivered as a **single batch after `start` returns**, not streamed (the task model has no output until the guest completes); and `CpuEvent::Cancel` stops the caller *draining*, not the remote computation (no driver abort hook).

### wanix capsule — CAS-backed world snapshots (`crates/wanix-cas`, recipe 03)
`wanix capsule save DIR` freezes a directory tree onto the content-addressed plane (venti applied to a whole world): every file becomes a BLAKE3 blob in a `ContentStore` (identical bytes dedup); a deterministic sorted `WorldManifest` (`<hex-hash> <path>` per line) is itself a blob, and **that manifest blob's hash is the capsule id**. `load <id> DIR` fetches the manifest, every referenced blob, and materializes the tree. **Honest reality (recipe 03):** there is *no* `.wcap` archive file in the shipped CLI — the capsule is the manifest blob plus its referenced blobs in the CAS store. Defenses: `MAX_MANIFEST_ENTRIES` (100k), `CAPSULE_MANIFEST_MAX_BYTES` (16 MiB), `MAX_BLOB_SIZE` (256 MiB), `NormalizedPath` re-validation (no `..`/absolute/escape), and every `get` re-hashes bytes before returning. Over the mesh, `IrohCasStore` (`crates/wanix-mesh/src/cas.rs`) fetches blobs from provider tickets over `iroh_blobs::ALPN` on the same shared endpoint (one identity, control + data planes), BLAKE3-verified end-to-end while streaming and clamped to `MAX_BLOB_SIZE`. Not portable: live mesh peers/tickets, ephemeral fds/handles, live `#kv` device state, symlinks/permissions/xattrs.

### Trust-boundary gaps (do not overclaim)
No ethernet/vnet. No public multi-user auth. Exec devices (`#task`/`#agent`/`#cpu`) are local-trust only — `mesh-serve --wanix-services` is *refused* on the public endpoint (parse rejection, `crates/wanix-cli/src/mesh/serve.rs`); the public endpoint with no `--peer`/`--grant`/`--insecure-open` is also refused (default-deny on the global transport). Per-principal namespaces and grant lifecycle are unfinished (single-attach-per-connection; `#grant` device not yet wired). The serve 9P websocket handles one frame at a time, so blocking pub/sub needs a second connection. The blob plane (iroh-blobs) is upstream-flagged experimental. A `#cpu` cancel does not stop remote computation.

**Honest caveats / do-not-overclaim:**
- No ethernet/vnet and no public multi-user auth — both explicitly unimplemented trust-boundary work.
- Exec devices (#task/#agent/#cpu) are local-trust only: mesh-serve --wanix-services is refused on the public endpoint, and the public endpoint with no --peer/--grant/--insecure-open is refused outright.
- The shipped mount-* verbs bind a peer at the single slot /n/remote, not /n/<peer-id>; per-peer namespace prefixes are a queued follow-up (the 9P session/namespace seam). The peer id lives in the iroh:// ticket, not the namespace path.
- There is no .wcap archive file in the shipped CLI — a capsule is the manifest blob plus its referenced blobs sitting in the CAS store; .wcap appears only in older idea/journal docs.
- #cpu v1 delivers stdout/stderr/exit as a single batch after the guest completes, not incrementally; CpuEvent::Cancel stops the caller draining, not the remote computation (no driver abort hook).
- iroh is pinned to =1.0.0-rc.1 (with iroh-blobs 0.102 / iroh-gossip 0.100), a release-candidate wave; the blueprint's guessed versions (~0.97/~0.96) were wrong and the blob plane is upstream-flagged experimental.
- Per-principal namespaces and grant lifecycle are unfinished: single-attach-per-connection, and a #grant device an agent edits/revokes is described as file-shaped but not yet wired.
- The serve 9P websocket handles one frame at a time per connection, so a blocking #plumb recv cannot interleave with a write on the same connection — live pub/sub needs a second connection.
- RemoteFs read_dir is documented as O(n^2) over WAN and a best-effort, non-atomic snapshot; per-entry metadata is listing-only unless Twalkgetattr supplies more.
- The mount-* allow-side over plain TCP is proven only by serve_stream-level loopback tests, not a two-process copy-paste, because the verbs cannot yet send aname=<subtree> (no --aname flag).


---

# Serve surface, browser cockpit, and demos (Rust-native Wanix)

## Serve surface, cockpit, and demos

This brief covers the `wanix serve` HTTP/9P composition layer, the browser
"cockpit" (a Code OSS / VS Code web extension under `workbench/`), and every
demoable thing wired through them. It reflects the *current* state on the
`cockpit-mesh-integration` branch, where browser validation has reached **PASS**
and the cockpit features are wired live to the cpu mesh devices over direct 9P
(`docs/integration/STATUS.md`, the three "Update 2026-06-07" sections supersede
the early FAIL sections).

### Serve transports and discovery

There is **one per-connection 9P session core** (`P9Server::serve_duplex`,
ADR 0006), and every transport is a thin adapter over it. The CLI exposes
`p9-stdio` as a standalone subcommand and folds the rest into `serve`
(`crates/wanix-cli/src/lib.rs`):
- `p9-stdio` — 9P over binary stdio (process pipe), proven by the
  `p9_stdio_*` tests in `lib.rs`. Still its own subcommand (the pipe /
  QEMU-v86 console bridge).
- `serve` — the local composition surface that combines static HTTP, discovery,
  direct 9P over WebSocket (a `WebSocketDuplex` framing adapter handed to
  `serve_duplex`, not a second server), the qjs-shell WebSocket, the HTTP-app
  route, and an optional raw-9P-over-TCP door on one listener (default
  `127.0.0.1:7654`, `crates/wanix-cli/src/serve/command.rs`; flags:
  `--root/positional`, `--listen` (`--addr` deprecated synonym), `--p9 HOST:PORT`
  with optional `--peer HEX`/`--grant ANAME:PREFIX:RIGHTS`, `--bundle`,
  `--wanix-services`, `--once`). Normal mode accepts concurrent HTTP + 9P
  WebSocket clients (`serve/concurrent.rs`); `--once` is the deterministic
  single-connection test mode.
- `serve --p9 HOST:PORT` exports the served namespace as raw 9P over TCP
  (default-bound to loopback) on a dedicated accept thread (`serve/raw9p`,
  which reuses the retired `p9-listen` grant/accept logic). It is a serve mode,
  not a subcommand. Because it speaks raw TCP 9P, `mount-write tcp://HOST:PORT
  '#sites/<host>' 'dir /abs'` mutates a *live* `serve` namespace from the CLI.
  `--peer`/`--grant` build the server through `P9Server::with_policy`
  (default-deny `AttachPolicy`); `--grant` requires `--peer`, and a policy
  without `--p9` is a usage error.

The standalone `p9-listen` (raw TCP) and `p9-ws` (WebSocket listener)
subcommands are **retired**: their session loops duplicated the one core, and
their reusable accept/grant logic now lives under serve. The trust boundary
lives on this edge — `--wanix-services` (which binds the `#task`/`#agent` exec
devices = RCE) is refused whenever either 9P door (the websocket door on the
HTTP listener, or the raw `--p9` door) is bound to a non-loopback address;
the previously silent `--listen :PORT --wanix-services` RCE hole is now a hard
usage error.

The discovery document at `/.well-known/wanix.json`
(`crates/wanix-cli/src/serve/discovery.rs`) is the contract the cockpit and v86
pages consume rather than hard-coding routes. It advertises (verbatim shape in
`serve_discovery_json`): `routes.p9` (`ws://host/.well-known/export9p`,
transport `direct-binary-websocket`, protocol `9p2000.L`, supportedProtocols
`["9P2000.L","9P2000.L.Google.2"]`), `routes.rootfs`, `routes.qjsShell`,
`routes.httpApp`, `routes.ethernet` (status `not-implemented`), a `v86` block
(asset paths, boot JSON, cmdline, msize, memory sizes), `services`, and the
selected `bundle`. Two other well-known routes: `/.well-known/export9p`
(binary 9P WebSocket) and `/.well-known/rootfs.json` (the loopback-only
`wanix-rootfs.v1` prepared-root handoff — returns 403 to non-loopback peers,
`rootfs_handoff_response`). ADR 0005 (`docs/adrs/0005-serve-and-client-handoffs.md`)
is the governing boundary record.

### --wanix-services device set

`--wanix-services` builds a Wanix `Namespace` (`crates/wanix-cli/src/serve/roots.rs`,
`serve_services_namespace`) binding the host root plus the service devices and
`#task`. The inspectable set is a single source of truth:
`roots::INSPECTABLE_SERVICE_DEVICES = ["#task","#term","#kv","#pipe","#plumb",
"#cas","#agent"]`, advertised in discovery as `services.devices`
(`serve_services_json`). `#agent` here uses the deterministic `FakeEngine`
(real codex is local-trust-only on the CLI path). Drivers are derived from the
registry (`serve_task_table`: `noop`, `qjs`, `wasm`; `auto` is implied),
advertised as `services.drivers` so discovery cannot drift from what
`#task/new/<kind>` can launch. The qjs-shell WebSocket and HTTP-app route only
turn `available` when `--wanix-services` is set.

### Bundles

`crates/wanix-cli/src/serve/html.rs` resolves three bundle names
(`serve.rs:26-27`, `direct_v86.rs:10`):
- `fs9p` — browser filesystem client over direct 9P (`fs9p.html`).
- `workbench-fs9p` — the browser cockpit: a Code OSS / vscode-web launch path
  (`workbench_fs9p.html`) that passes the discovered direct-9P route into the
  extension. Its static assets (vscode-web under `workbench/code`, the compiled
  extension under `workbench/dist`, `media/`) are served from the repo
  `workbench/` tree, not the served root, via `workbench_asset_response` in
  `serve/http/routes.rs` (with `Cache-Control: no-store`), so a disposable root
  still boots. The plan's Slice 8 (`--bundle cockpit` rename, retiring the
  `workbench/code/` vendor dependency) is **not yet done** — the bundle name is
  still `workbench-fs9p`.
- `direct-v86` — browser v86 handoff (`direct_v86.html`); serves built-in v86
  assets (libv86.mjs, v86.wasm, seabios, vgabios) and a `direct_v86_boot_json`
  with kernel/initrd/init readiness markers.

### rootfs / qemu / v86 handoffs

These are VM launch/attachment contracts, not a VM supervisor (ADR 0005).
`serve` exposes `/.well-known/rootfs.json` (`wanix-rootfs.v1`, loopback-only,
gated on `rootfs_handoff_readiness`: an executable `/bin/init` and a kernel).
The `direct-v86` bundle surfaces a boot block in discovery. The CLI also has
`rootfs` (prepare/validate a guest root), `qemu` (emit `wanix-qemu-virtio9p.v1`
argv/policy handoff), and `direct-v86` page generation — described in AGENTS.md.

### The cockpit operator surface

The cockpit is a VS Code web extension (`workbench/src/web/`) driving everything
over direct 9P through `WanixP9Handle` (`workbench/src/wanix/p9.ts`) and
`WanixHandle`. `p9.ts` implements filesystem-shaped ops (readDir, readFile,
writeFile, mkdir, rename, copy, symlink, removeAll) and crucially
`isLiveServiceStream()` — `#term/*`, `#pipe/*/data`, `#plumb/*/send|recv`,
`#agent/*/events` use the streaming walk+open path (open existing fid, read/write
at offset 0) rather than one-shot readFile/writeFile, since the allocator-owned
service files reject a create and a live subscription must not drain to EOF.
Live writable streams open `O_WRONLY` (pipe/plumb are strictly unidirectional;
term tolerates write-only).

The extension registers ~40 commands (`extension.ts`, grep at lines 181-541).
The activity-bar **WANIX: SYSTEM** tree (`system-view.ts`, the real 1900+-line
implementation) shows categories: Actions, Tour, Reports, Data Stores, Checks,
Activity, Routes, Route Runs, Agent, Tasks, Terminals, Namespace, Drivers
(confirmed live in `wanix-cockpit.png`). The Namespace category lists bound
service devices from discovery `services.devices` (`inspect-kv.png` shows
`/ local served root`, `#task task service`, `#term terminal device`,
`#kv key/value store`). The service inspector (`service-inspector.ts`, scheme
`wanix-inspect:`) renders a device directory read-only with an allocator/
metadata/control/stream taxonomy (`unsafeFileReason`): it links safe metadata
(`id`,`status`,`kind`,`exit`,…) and child dirs but leaves allocator files
(`#agent/new`, `#pipe/new`, `#cas/ingest`), `ctl`, and streams as plain text
with a reason, so clicking never accidentally allocates. Confirmed in
`inspect-kv.png` (empty directory, the "Note" block about reachable hidden
service dirs).

The five live-wired cockpit features (STATUS.md final update):
1. **Inspect devices** — `service-inspector` over 9P (`#kv` empty, `#agent`
   `new`→allocator, `#cas` `have/`+`ingest`).
2. **Agent repair demo** (`agent-repair-demo.ts`) — `repairViaAgent` opens an
   `#agent` session via `#agent/new`, submits an `approve:`-prefixed prompt,
   polls `pending`, resolves the parked approval via `ctl approve <id>`, applies
   the patch, re-runs. One click fixes `/agent/broken.js` so `result.txt` reads
   "FIXED BY WANIX AGENT" (`agent-repair.png`). Falls back to the in-process
   deterministic `repairQjsProgram` if `#agent` is unreachable.
3. **qjs→wasm→qjs duet** (`duet-demo.ts`) — producer (qjs) writes
   `shared/in.txt`, transform (`rust-guest.wasm`) reads it and writes
   `shared/out.txt`, verify (qjs) checks the output equals
   "rust-wasm saw: hello from qjs duet" — all on one shared FS (`duet.png`).
4. **HTTP apps + #kv** (`http-app-demo.ts` + Rust `/.wanix/app/<name>`) — the
   counter demo backs request state on `#kv/http-counter`; curl and the cockpit
   increment one shared counter (`http-counter.png`).
5. **Self-check** (`cockpit-self-check.ts`) — probes drivers, `#task`/`#term`,
   qjs-shell route, http-app route, direct-v86 route, report storage, `#agent`,
   `#kv`, `#pipe`, `#cas`, `#mesh` peers, and `#plumb`, writing
   `/.wanix/cockpit-check.{json,md}` (`self-check.png`). Known constraint: the
   `#plumb` probe only verifies the publish path because a blocking `recv` would
   deadlock the single-frame-at-a-time serve WebSocket — live receive needs a
   second 9P connection.

### The /.wanix/app/<name> HTTP route

`crates/wanix-cli/src/serve/http/app.rs` resolves `apps/<name>.js` or
`apps/<name>.wasm` in the served namespace, allocates a `#task/new/{qjs,wasm}`,
binds `fd/1`/`fd/2` to trace files under `.wanix/http/`, sets `cmd`/`dir`/`env`
(`WANIX_HTTP_APP`, `WANIX_HTTP_TARGET`), writes `start`, reads `exit`, and
returns stdout with `X-Wanix-Task-Id`/`X-Wanix-Stdout-Path`/`X-Wanix-Stderr-Path`
headers. Loopback-only and `--wanix-services`-gated. WASI guests can resolve any
`#name` device path (e.g. `#kv/<key>`) so the qjs handler reaches `#kv` for
counter state.

### Gaps and honest caveats

The bundle is still `workbench-fs9p` (no `--bundle cockpit`). The mesh panel
(Slice 3), browser-as-peer (Slice 9), and most of the 10-slice plan
(`docs/integration/plan.md`) are queued/partial — the cockpit talks to the
local single-node serve; `/n/<peer>` mounts and the mesh panel are not yet in
the shipped surface. `v86-shared-demo` is still a `// STUB:` no-op in
`extension.ts`. `#plumb` live receive is publish-only over serve. The early
STATUS.md sections describe a FAIL state that the later updates fixed.

**Honest caveats / do-not-overclaim:**
- The served bundle is still named workbench-fs9p, not cockpit; Slice 8 (rename --bundle cockpit and retire the workbench/code/ vscode-web vendor dependency) is queued, not done.
- The mesh panel (verified peers, GrantTable, /n/<node> mounts) is Slice 3 and is NOT in the shipped cockpit; the cockpit currently drives a single-node local serve, not cross-peer mounts.
- Browser-as-peer over WebSocket (Slice 9, session-token attach to a remote serve) is queued; full ed25519 + iroh QUIC in the browser is explicitly deferred.
- v86-shared-demo is still a // STUB: no-op in workbench/src/web/extension.ts (line 15).
- #plumb live receive in the self-check probes the PUBLISH path only — a blocking recv would deadlock the single-frame-at-a-time serve WebSocket; end-to-end delivery needs a second 9P connection.
- The served #agent uses a deterministic FakeEngine, not a real LLM; the codex app-server engine is local-trust CLI-only. The agent-repair demo also keeps an in-process deterministic fallback (repairQjsProgram) if #agent is unreachable.
- Ethernet/vnet route is advertised in discovery as status not-implemented; rootfs/app routes are loopback-only.
- STATUS.md's early sections describe a FAIL render state (missing static route, stubbed system-view); those were fixed in the later same-day updates that reached PASS — do not cite the FAIL state as current.
- Discovery and rootfs/qemu/v86 handoff JSON is still hand-built format! strings (typed-struct cleanup is a queued AGENTS follow-up).


---

# Philosophy, Positioning &amp; Use-Cases (the "why" / visionary brief)

## Wanix (Rust-native port): the "why"

### One-line thesis
Wanix is **Plan 9 reincarnated for the age of agents**: a Rust-native operating-system core that runs *outside Chrome*, treats everything as a file, gives every process its own rearrangeable namespace, and now reaches across machines through a Plan 9-style **mesh** — so devices, compute, and AI agents compose across nodes through one 9P contract. (`docs/rust-vs-go-wanix.md`, `CLAUDE.md` north star, `README.md`.)

### The Plan 9 lineage and "the missing half of 9P"
The deepest Plan 9 idea is *not* "everything is a file" for its own sake — it is that **the namespace is per-process and you can rearrange it**, and that two operations build the view: `export` (serve a subtree as 9P) and `import` (splice a remote 9P service into *your* namespace at `/n/<name>`). Because services are files, *one* file-transport protocol (9P) plus *one* placement operation (bind/mount) buys network transparency "for free, across every service at once" — you write a 9P client once and every file-shaped service any node exports becomes reachable (`docs/mesh-the-missing-half-of-9p.md:63-96`).

For a long time Wanix could **export** but not **import** — "It had half of 9P." The mesh work builds the other half: `wanix-9p-client::RemoteFs` is a synchronous `FileSystem` that *is* a 9P client, so `bind`-ing it at `/n/<node>` makes a remote namespace — regular files **and** `#`-devices alike — part of your local tree (`docs/mesh-the-missing-half-of-9p.md:1-18, 451-496`). The mesh then re-creates the rest of the Plan 9 cast, each as one slice: **factotum → the QUIC handshake** (the ed25519 key is both identity and address; `Tauth` stays ENOSYS forever); **a capability is a bind** (a grant is a re-rooted `SubtreeFs`, not an ACL); **venti → `#cas`** (BLAKE3 content-addressed blobs + portable capsules); **cpu(1) → `#cpu`** (send the job to the data); **plumber → `#plumb`** (typed gossip coordination). Verified in code: all of `wanix-9p-client`, `wanix-id`, `wanix-mesh`, `wanix-cas`, `wanix-cpu`, `wanix-plumb` exist as crates.

### Why Rust-native-outside-Chrome matters
The original Go Wanix proved a Plan 9 OS could live *in the browser*, where the browser was the host/kernel. The Rust port **moves the host boundary**: "what if Wanix can be a native operating environment, and the browser is one excellent client of it?" (`docs/rust-vs-go-wanix.md:13-17`). The argument is concrete, not aesthetic: if the browser tab is the kernel, the runtime dies when the tab dies — no long-running agents, scheduled jobs, scale-out, server-side collaboration, durable terminals, or externally reachable endpoints (`docs/rust-vs-go-wanix.md:104-107, 615-642`). Moving the core to a native Rust host (Wasmtime substrate, owned 9P protocol, explicit mounts/handoffs) makes "cloud when you need durability, local/browser when you need immediacy" possible (`docs/rust-vs-go-wanix.md:643-652`). WASI is deliberately inverted: not "delegate to the host" but "an adapter *into* Wanix-owned semantics" — host directories are explicit mounts, never ambient authority (`docs/rust-vs-go-wanix.md:228-256`).

### The agent / AI-infra angle (the strongest "why now")
The core insight: **per-process namespaces and file-shaped services were always powerful but humans found them too fiddly to wield at full strength — an agent does not have that problem.** "An LLM operating a confined world as files finds per-process namespaces *native*, not fiddly. It reads state by `cat`, mutates by `write`, lists capabilities by `ls`, spawns work by writing to a control file… The thing humans found too sharp to hold all the time is the thing an agent reaches for naturally" (`docs/mesh-the-missing-half-of-9p.md:98-122`). Wanix already had the agent (`#agent` — an LLM you can `cat`); what it lacked was **reach**, and the mesh supplies it: bind `/n/<node>` and the agent's `cat`/`write`/`ls` vocabulary extends unchanged to files and services on another machine. The composition `#agent` + `#cpu` + mesh = **agents and compute that compose across machines**: "send the agent to the data" (run the task *on* the node that holds the files, against its fast local namespace, with the caller's namespace reverse-exported), and `#plumb` lets two agents on two machines hand work off by typed message (`Slice 6` and `Slice 7`, `docs/mesh-the-missing-half-of-9p.md:1824-1917, 2092-2199`).

### Capability security as a selling point
Security in Wanix is not bolted on — it *is* the architecture. "A capability is a bind": a grant re-roots the peer inside a `SubtreeFs` whose `.` *is* the granted subtree, so "they cannot name a path outside it because, in their namespace, there is no outside" (`docs/mesh-the-missing-half-of-9p.md:511-518, 562-573`). The grant table is **default-deny**, keyed by the *cryptographically verified* peer key (not a client-claimed `uname`), and `mesh-serve` *refuses* to export a public endpoint without explicit grants or a loud `--insecure-open` opt-in (`Slice 2`/`Slice 3`/`Slice 4` corrections). Exec devices (`#task`/`#agent`/`#cpu`) are **local-trust only** until public auth lands — file-sharing is never confused with remote code execution (`docs/mesh-the-missing-half-of-9p.md:1358-1387`). The microkernel framing in the README is "capability-oriented" by design. Crucially for agents, trust is also about **legibility**, not just permission prompts: the "traceable dynamic namespaces" thesis is that an agentic runtime should make the chain `prompt → plan → command → task → terminal log → filesystem mutation → result` visible, addressable, and forkable, so a user can ask "what changed, and why?" and rewind/fork the world (`docs/traceable-dynamic-namespaces.md:15-117, 517-527`). "Managed Wanix state can rewind. External effects can be traced" (`:272-275`).

### Scaling story (rooms, not houses)
Wanix runs each program in a tiny WebAssembly sandbox — "a locked room inside a shared building" — at ~0.25 MB vs ~10 MB for an OS process, roughly 40× smaller, so 1,000 programs fit in a few hundred MB instead of 10 GB (`docs/scaling-eli5.md:9-23`). Throughput scales with cores then holds flat; a single program does ~500k file ops/sec, 50 hit ~9M ops/sec (`:25-39`). Per task you choose the engine in the *same* secure room: quick JavaScript (QuickJS, interpreted), or Rust/Go/C/Zig compiled to wasm for ~100× speed, or a full VM for maximum isolation (`:49-66`). **Honesty caveat (from the doc's own team note): per-room memory isolation is real today, but the hard CPU/memory limits that make a room safe for *arbitrary untrusted* code (Wasmtime epoch/fuel preemption, linear-memory caps) are not wired up yet** — keep public copy to "cheap, scalable isolation," do not claim "safe for arbitrary untrusted code" (`docs/scaling-eli5.md:89-95`).

### Concrete use-cases (all grounded in shipped or demonstrated capability)
- **Personal compute mesh** — your laptop, phone, and a cloud node each run a Wanix node; mount any one at `/n/<peer>` and operate its files/devices as your own, NAT-crossed over iroh QUIC, gated by per-peer grants (`README.md` Mesh section; `docs/recipes/02-mount-remote-peer.md`).
- **AI agents operating on your files across devices** — `#agent` repairs a broken program with approvals-as-files; over the mesh it reaches a peer's `#kv`/`#cas`/files; two agents on two machines collaborate via `#plumb` (`docs/recipes/01,02,05`; cockpit agent-repair demo verified live, `STATUS.md:445-449`).
- **Portable worlds via capsules** — `wanix capsule save` freezes a whole world (the directory an agent built) into a CAS-backed, BLAKE3-verified, deduplicated `.wcap`; hand someone one hash and they materialize the entire world, verifying every blob (`docs/recipes/03`; Slice 5; `crates/wanix-cli/src/capsule.rs` verified).
- **Browser-native OS / operator cockpit** — a Code OSS / VS Code web extension drives the whole namespace over direct 9P: inspect service devices, run the agent repair demo, run a qjs→wasm→qjs duet on one shared FS, serve HTTP apps at `/.wanix/app/<name>` with `#kv`-backed state, self-check the device set (`README.md`; `STATUS.md:432-471`, verified in headless Chromium).
- **Distributed dev environments** — `#cpu` runs your task on the node holding the source, against its fast local namespace, outputs returning over the wire — Plan 9 cpu(1) over the open internet, with a default read-only jail (`Slice 6`).

**Honest caveats / do-not-overclaim:**
- Safe-for-untrusted-code is NOT claimable yet: per-room memory isolation is real, but Wasmtime CPU/memory hard limits (epoch/fuel preemption, linear-memory caps) are not wired up — say 'cheap, scalable isolation', not 'safe for arbitrary untrusted code' (docs/scaling-eli5.md:89-95).
- Exec/agent devices (#task/#agent/#cpu) are local-trust ONLY: --wanix-services is refused on the public endpoint regardless of grants; public/multi-user auth is explicitly unimplemented. Do not imply you can safely expose agents/compute to arbitrary peers on the open internet (Slice 4 correction; CLAUDE.md trust-boundary notes).
- The served #agent uses a deterministic FakeEngine; the REAL codex-backed agent runs only on the local-trust CLI path. Browser/cockpit agent demos are not running a live LLM by default (CLAUDE.md; docs/ideas-2026-06.md:181-191).
- Traceable/forkable/rewindable namespaces (gitfs, ns fork, time-travel mounts, the Activity causal feed) are an EXPLORATORY product vision, NOT shipped. Only a first slice (#plumb provenance + qjs-shell mutation frame) exists; the doc is explicitly 'not an ADR' (docs/traceable-dynamic-namespaces.md:1-13, header).
- The mesh/agent layer has NO ADRs yet — its contracts are not stabilized; design lives in docs/mesh-blueprint.md and the slices doc. Frame mesh features as recent, fast-moving work, not frozen API.
- #plumb live receive across the serve WebSocket is publish-path only in the cockpit self-check: the serve connection handles one frame at a time, so a blocking recv would deadlock — end-to-end live pub/sub over a single serve connection is not yet wired (CLAUDE.md; STATUS.md:471-475).
- The Go Wanix implementation is still BROADER as a browser web-OS (web components, full v86/workbench breadth); the Rust port is narrower but deeper at the native runtime boundary. Do not claim Rust has feature parity with Go (docs/rust-vs-go-wanix.md:26-27, 109-135).
- Snapshots are QuickJS VM-state only, not whole-process checkpoints — task identity, namespace, fds, cwd, env, host mounts are NOT captured and must be reattached. 'Portable worlds' via capsules is real for directory trees; live mesh peers and ephemeral handles are explicitly NOT portable (docs/rust-vs-go-wanix.md:296-327; CLAUDE.md capsule note).
- Native QEMU and direct-v86 are validated HANDOFFS, not Wanix-supervised VM lifecycle; Ethernet/vnet is reserved-but-unimplemented. Don't claim Wanix boots/manages VMs end-to-end (docs/rust-vs-go-wanix.md:559-603, 653-670).
- Cockpit v86-shared-demo is still a no-op STUB; the cockpit is an integration surface that works for the listed demos but is not the full Go browser environment (STATUS.md:471; CLAUDE.md Queued Follow-ups).
