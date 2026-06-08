# In-Wanix Sites — Transport Plan (Option C)

Status: ordered implementation plan. One per-connection 9P session core; the
websocket is a thin framing adapter over it; `p9-listen` folds into `serve` as a
raw-9P export mode; `p9-ws` is retired; the serve 9P edge gets the trust guard.
Recorded as **ADR 0006**. Do NOT implement from this file in the Map step; the
Implement step executes it. Do NOT commit inside an implement step.

cd /Users/jesse/lw/wanix-sites before any work.

---

## 0. Decisions (owner-approved surface)

- **One session core.** `P9Server::serve_stream<R: Read, W: Write>`
  (`crates/wanix-9p/src/transport.rs:95`) is the single per-connection loop. The
  websocket becomes a `Read + Write` byte stream (`WebSocketDuplex`) handed to
  the SAME `serve_stream`. The duplicated ws loop in `p9_ws/connection.rs:59-126`
  is deleted. No `serve_duplex` sibling is needed: `serve_stream` already takes
  separate `R`/`W` and never reads and writes concurrently, and `WebSocketDuplex`
  is one owned object we can pass as both `reader` and `writer` only if it is
  `Clone`/splittable — it is not. **Decision:** add a thin
  `P9Server::serve_duplex<D: Read + Write>(&mut self, duplex: D)` that internally
  borrows the single `D` for both directions and runs the identical inner loop by
  delegating to a private `serve_stream_inner(&mut self, &mut dyn Read, &mut dyn
  Write)`. `serve_stream` and `serve_duplex` both call `serve_stream_inner`; the
  loop body is written once.
- **Raw-9P on serve.** New serve flag **`--p9 ADDR`** exports the served
  namespace (`Arc::clone(&roots.p9_root)`) as **raw 9P over TCP** on `ADDR`,
  default-bound to loopback. It carries the existing `--peer HEX` / `--grant
  ANAME:PREFIX:RIGHTS` policy plumbing (moved out of `p9_listen` into a shared
  module). This makes
  `wanix mount-write tcp://HOST:PORT '#sites/<host>' 'dir /abs'` work against a
  live serve. The bound raw-9P addr is reported in startup status and discovery.
- **Fold p9-listen into serve.** "Export a dir as raw 9P" = `wanix serve --root
  DIR --p9 ADDR` (no `--wanix-services`, optionally `--peer/--grant`). The
  standalone `p9-listen` subcommand is **retired**. Its policy plumbing
  (`grant.rs`, `ServePolicy`, the shared listener helper) is preserved and reused
  by serve.
- **Retire p9-ws.** The websocket 9P door already exists inside `serve`
  (`/.well-known/export9p`); the standalone `p9-ws` subcommand is redundant and
  is **retired**.
- **KEEP:** `p9-stdio` (pipe transport / QEMU-v86 bridge), `mesh-serve`
  (iroh/crypto trust), `mount-ls/cat/write`, `qjs/wasm/qjs-restore` launch flags.
- **`--addr`/`--listen` synonyms:** keep `--listen`, **deprecate `--addr`** (keep
  it parsing as a hidden synonym so existing scripts/tests pass; drop it from
  help/usage text).
- **Trust guard (load-bearing):** refuse `--wanix-services` whenever EITHER 9P
  door (the websocket door OR the new raw-`--p9` door) is bound to a
  non-loopback address. Mirror the mesh rule
  (`crates/wanix-cli/src/mesh/serve.rs:130-139`). Reuse `is_loopback_peer`
  (`crates/wanix-cli/src/serve/discovery/host.rs:40`) — generalize it to a
  `SocketAddr`/IP loopback check usable at bind time.
- **Policy on the serve edge:** when `--p9` carries `--peer`, build the raw-9P
  server with `P9Server::with_policy` (`crates/wanix-9p/src/lib.rs:137`) using the
  moved grant plumbing. The websocket door stays unauthenticated **but**
  `--wanix-services` is refused off-loopback for it, so RCE devices never reach a
  non-loopback peer.

### Why NOT fold raw-9P into the concurrent HTTP poller

`serve/concurrent.rs` busy-polls `accept()` on a 10ms sleep with no shutdown
signal (AGENTS.md "Serve concurrency" follow-up explicitly says do not extend it
without a shutdown signal first). The raw-9P listener therefore runs as its own
**blocking** accept loop on a dedicated thread (the existing p9-listen loop
shape), spawned when `--p9` is set, sharing `Arc<dyn FileSystem>`. We do NOT add
a connection cap or shutdown here; we reuse the existing per-connection thread
model p9-listen already used (`--once` for the HTTP door, blocking loop for raw
9P). This is the minimal, in-shape change.

---

## 1. The single-loop refactor (`wanix-9p`)

**File:** `crates/wanix-9p/src/transport.rs`

- Extract the body of `serve_stream` (`:104-125`) into a private
  `fn serve_stream_inner(&mut self, reader: &mut dyn Read, writer: &mut dyn
  Write) -> Result<P9TransportStats, P9TransportError>`.
- `pub fn serve_stream<R: Read, W: Write>(&mut self, mut reader: R, mut writer: W)`
  calls `self.serve_stream_inner(&mut reader, &mut writer)`.
- Add `pub fn serve_duplex<D: Read + Write>(&mut self, mut duplex: D) ->
  Result<P9TransportStats, P9TransportError>`. Since one `&mut D` cannot be
  borrowed as `&mut dyn Read` and `&mut dyn Write` simultaneously, the loop reads
  then writes sequentially — so `serve_duplex` runs its own copy of the SAME
  loop body, OR (preferred) we keep `serve_stream_inner` reading into a buffer and
  writing responses in the same iteration with a single `&mut D` reborrowed each
  call site. **Chosen shape:** `serve_duplex` calls a generic
  `serve_loop<F: FnMut read, G: FnMut write>` — but to avoid over-engineering,
  implement `serve_duplex` as: loop { read into buf via `(&mut duplex).read(..)`;
  for each frame, encode and `(&mut duplex).write_all(..)` } — i.e. the identical
  body, reborrowing `duplex` for each op. The loop text is shared by having
  `serve_stream` delegate to `serve_duplex` over a `ReadWritePair` adapter:
  - Add a tiny private `struct DuplexPair<R, W> { reader: R, writer: W }`
    implementing `Read` (forward to `reader`) and `Write` (forward to `writer`).
  - `serve_stream<R,W>(reader, writer)` = `self.serve_duplex(DuplexPair { reader,
    writer })`.
  - The loop lives exactly once, in `serve_duplex`.
- Keep `STREAM_READ_BUFFER_BYTES`, `P9TransportStats`, `P9TransportError`
  unchanged. Existing transport tests (`:143-241`) call `serve_stream` and stay
  valid. Add one test: `serve_duplex_round_trips_over_one_byte_stream` using a
  loopback `TcpStream` or an in-memory cursor pair to prove the duplex path.

**Public surface added:** `P9Server::serve_duplex`. Document it in the rustdoc the
same way `serve_stream` is documented. No new error types.

**Module-line risk:** `transport.rs` is 297 lines (incl. tests). The added
`serve_duplex` + `DuplexPair` (~25 prod lines) keeps production well under 350.
OK.

---

## 2. WebSocketDuplex adapter (serve-local)

**New file:** `crates/wanix-cli/src/serve/ws_duplex.rs` (serve-local; tungstenite
stays in the cli crate, never in the sync 9P core).

```rust
pub(super) struct WebSocketDuplex {
    socket: tungstenite::WebSocket<std::net::TcpStream>,
    read_buf: std::collections::VecDeque<u8>, // leftover bytes from a ws message
}

impl WebSocketDuplex {
    pub(super) fn new(socket: tungstenite::WebSocket<TcpStream>) -> Self
}

impl std::io::Read for WebSocketDuplex {
    // Drain read_buf first. When empty, socket.read():
    //   Message::Binary(b) -> push b into read_buf, then serve from it
    //   Message::Ping(b)   -> socket.send(Pong(b)); loop to read next message
    //   Message::Close(_)  -> Ok(0)
    //   Message::Text/Pong/Frame -> loop (ignore)
    //   Err(WsError::ConnectionClosed | AlreadyClosed) -> Ok(0)
    //   Err(other) -> Err(io::Error::other(other))
    // Returns 0 ONLY on clean close (maps to serve_stream's clean-EOF path).
}

impl std::io::Write for WebSocketDuplex {
    // write(buf): socket.send(Message::binary(buf.to_vec())); Ok(buf.len())
    //   -> one binary ws message per write_all; serve_stream calls write_all once
    //      per response frame, so one response frame == one binary message.
    //   Map WsError to io::Error.
    // flush(): socket.flush().map_err(io::Error::other)
}
```

Key correctness points:
- **Frame-split-across-ws-messages** is handled by `P9FrameBuffer` inside
  `serve_stream` (the core reassembles), so `read_buf` only needs to hold one
  ws message's worth of leftover bytes between `read()` calls; the core does the
  9P framing.
- A truncated final 9P frame surfaces as `P9TransportError::TruncatedFrame` from
  the core (replacing the old `P9WsConnectionError::TruncatedFrame`).
- Ping/Pong handled inside `Read` so liveness keepalives work without the core
  seeing them.

**Module-line risk:** ~90-110 lines incl. tests. OK.

---

## 3. Delete the duplicated ws session logic; rewire the ws door

**File:** `crates/wanix-cli/src/p9_ws/connection.rs` — **DELETE the file's
session logic.** Specifically:
- Delete `serve_websocket_connection`, `read_websocket_message`,
  `handle_websocket_message`, `handle_binary_websocket_message`,
  `close_websocket_connection`, and the `P9FrameBuffer` usage.
- `P9WsConnectionError` is still referenced by `serve/connection.rs`
  (`ServeConnectionError::WebSocket`) for the handshake error. **Decision:**
  collapse `P9WsConnectionError` to a serve-local `WebSocketError` carrying just
  `Handshake(String)` + an `Io`/`Transport` wrap, OR fold the handshake-failure
  string straight into `ServeConnectionError`. Chosen: move a minimal
  `WebSocketDoorError { Handshake(String), Transport(P9TransportError) }` into
  the new `ws_duplex.rs` (or `serve/connection.rs`) and delete the whole
  `p9_ws/connection.rs`. Keep its display/source test coverage by porting the two
  `#[cfg(test)]` cases to the new error's module.

**File:** `crates/wanix-cli/src/serve/connection.rs`
- `serve_websocket_request` (`:55-80`): replace the
  `serve_websocket_connection(Arc::clone(&roots.p9_root), socket)` call
  (`:78-79`) with:
  ```rust
  let mut server = P9Server::new(Arc::clone(&roots.p9_root));
  let duplex = WebSocketDuplex::new(socket);
  server.serve_duplex(duplex).map(|_stats| ()).map_err(ServeConnectionError::WebSocket9p)
  ```
- `ServeConnectionError::WebSocket(P9WsConnectionError)` (`:94`) becomes
  `WebSocket(String)` for handshake + a new `WebSocket9p(P9TransportError)` for
  the session, OR a single `WebSocket(WebSocketDoorError)`. Update the `Display`
  (`:103`) and `source()` (`:113`) arms, and the two connection-error tests
  (`:128-167`).
- `accept_websocket` (`:82-88`) keeps producing the handshake error in the new
  shape.
- Drop `use crate::p9_ws::{P9WsConnectionError, serve_websocket_connection};`
  (`:9`); add `use crate::serve::ws_duplex::WebSocketDuplex;` and `use
  wanix_9p::{P9Server, P9TransportError};`.

**File:** `crates/wanix-cli/src/serve.rs` — add `mod ws_duplex;` (`:6-15` module
list).

---

## 4. Shared raw-9P listener helper + grant plumbing (out of p9_listen)

**New file:** `crates/wanix-cli/src/serve/raw9p.rs` (serve-local home for the
raw-9P door), OR a top-level `crates/wanix-cli/src/raw9p.rs` shared by serve.
Chosen: `crates/wanix-cli/src/serve/raw9p.rs` since the only consumer is serve.

Move/define here (ported verbatim from `p9_listen/runtime.rs`):
- `struct ServePolicy { peer: PeerId, policy: Arc<dyn AttachPolicy> }`
  (`runtime.rs:19-23`).
- `fn serve_raw9p_listener(once: bool, listener: &TcpListener, root: Arc<dyn
  FileSystem>, policy: Option<&ServePolicy>, process_stderr: &mut dyn Write) ->
  Result<i32, CliError>` — the accept/serve/error loop (`runtime.rs:96-173`),
  renamed and generalized (message prefixes become `wanix-rust serve --p9:`).
- `fn serve_raw9p_connection(root, policy, stream: TcpStream) ->
  Result<P9TransportStats, P9TransportError>` (= old `serve_stream_connection`,
  `runtime.rs:160-173`) — uses `P9Server::with_policy` when policy is `Some`,
  else `P9Server::new`, then `serve_stream(reader, stream)` (raw TCP keeps the
  `try_clone` reader/writer split; it does NOT need `serve_duplex`).

**New file:** `crates/wanix-cli/src/serve/raw9p/grant.rs` — **move**
`crates/wanix-cli/src/p9_listen/grant.rs` verbatim (`GrantSpec`, `parse_peer`,
`build_policy`). 151 lines, no edits needed beyond `pub(in crate::serve)`
visibility.

**Signature of the shared helper:**
```rust
pub(super) fn serve_raw9p_listener(
    once: bool,
    listener: &std::net::TcpListener,
    root: std::sync::Arc<dyn wanix_fs::FileSystem>,
    policy: Option<&ServePolicy>,
    process_stderr: &mut dyn std::io::Write,
) -> Result<i32, crate::CliError>;
```

Port the policy/grant unit tests from `p9_listen/runtime.rs:179-398`
(authorized attach, read-only EACCES, revoke denies next attach,
`build_serve_policy` None without peer) into `serve/raw9p.rs` tests, retargeted
at the moved functions. Do NOT delete or weaken them — they are the trust-boundary
proof.

**Module-line risk:** `serve/raw9p.rs` production ~110 lines + tests; grant.rs
151. Both under 350. OK.

---

## 5. serve command: `--p9 ADDR`, `--peer`, `--grant`, deprecate `--addr`

**File:** `crates/wanix-cli/src/serve/command/options.rs`
- Add to `SERVE_VALUE_OPTIONS` (`:5-10`): `("--p9", ServeValueOption::P9)`,
  `("--peer", ServeValueOption::Peer)`, `("--grant", ServeValueOption::Grant)`.
- Add `P9`, `Peer`, `Grant` to `ServeValueOption` (`:32-38`) + `label`
  (`:41-48`) + `value_name` (`:50-56`): `P9 => "HOST:PORT"`, `Peer => "HEX"`,
  `Grant => "ANAME:PREFIX:RIGHTS"`.
- Keep `--addr` in the table as a hidden synonym of `--listen` (already is).

**File:** `crates/wanix-cli/src/serve/command.rs`
- Add to `ServeCommand` (`:11-18`): `p9_addr: Option<String>`, `peer:
  Option<PeerId>`, `grants: Vec<GrantSpec>` (import `GrantSpec`/`parse_peer` from
  the moved `serve::raw9p::grant`).
- Extend `ServeCommandParts` (`:109-116`) with the same fields + setters.
- `apply_value` (`:70-77`): route `ServeValueOption::P9 => set_p9_addr`,
  `Peer => set_peer` (`parse_peer`), `Grant => grants.push(GrantSpec::parse(..))`.
- `finish` (`:160-168`): validate `--grant` requires `--peer` (mirror
  `p9_listen/command.rs:129-133`); validate `--peer`/`--grant` require `--p9`
  (a policy with no raw door is dead config → usage error). Default
  `p9_addr: None`.
- Update the parser unit tests in `serve/tests.rs` and add: `--p9` parses;
  `--grant` without `--peer` errors; `--peer` without `--p9` errors.

**Decision — `--p9` default loopback:** `--p9` takes an explicit `HOST:PORT`. If
the operator passes `--p9 :PORT` reuse `normalize_listen_addr` →
`0.0.0.0:PORT` (non-loopback) which then trips the trust guard if combined with
`--wanix-services`. Document that `--p9 127.0.0.1:PORT` is the loopback form.

---

## 6. Bind the raw-9P listener + trust guard in serve runtime

**File:** `crates/wanix-cli/src/serve.rs`
- In `run_serve_with_listener_inner` (`:50-70`), after `serve_roots_for_listener`
  and BEFORE serving connections, if `command.p9_addr.is_some()`:
  1. **Trust guard:** compute loopback-ness of BOTH doors:
     - HTTP/ws door: `local_addr` (the bound HTTP listener addr).
     - raw-9P door: parse `p9_addr` to `SocketAddr` (bind it first to learn the
       real addr if `:0`).
     If `command.wanix_services` AND either door's IP is non-loopback → return
     `CliError::usage(...)` mirroring `mesh/serve.rs:130-139` (message: serve
     `--wanix-services` binds `#task`/`#agent` (RCE); refused on a non-loopback
     9P endpoint; bind the door to loopback or drop `--wanix-services`).
  2. Bind `TcpListener::bind(&command.p9_addr)`; build `Option<ServePolicy>` from
     `command.peer`/`command.grants` via `serve::raw9p::grant::build_policy`
     against `Arc::clone(&roots.p9_root)`.
  3. Spawn a dedicated thread running `serve::raw9p::serve_raw9p_listener(/*once=*/
     command.once, &raw_listener, Arc::clone(&roots.p9_root), policy.as_ref(),
     &mut <thread-local stderr writer>)`. Because `process_stderr` is `&mut dyn
     Write` and not `Send`, the raw-9P loop writes to its own `io::stderr()` (or a
     cloned channel). **Decision:** the raw-9P loop logs to `std::io::stderr()`
     directly inside the thread (matches the live-process model; serve already
     only runs raw-9P in the live binary, never in collected/captured mode).
  4. The raw-9P bound addr is captured for status/discovery (see §7).

- **Trust-guard insertion point precisely:** a new helper
  `fn enforce_services_trust_boundary(command: &ServeCommand, http_addr:
  SocketAddr, p9_addr: Option<SocketAddr>) -> Result<(), CliError>` called from
  `run_serve_with_listener_inner` right after `serve_roots_for_listener`. It also
  guards the **websocket** door: if `wanix_services && !http_addr.ip().is_loopback()`
  → refuse, because the ws 9P door rides the HTTP listener and is unauthenticated.

> Note: today `serve` defaults to `127.0.0.1:7654` and `--listen :PORT` →
> `0.0.0.0`. This guard is what makes a `--listen :PORT --wanix-services` (RCE on
> all interfaces, the current silent hole) a hard usage error.

**Module-line risk:** `serve.rs` is 145 lines; adding the guard helper + raw-9P
bootstrap (~50 lines) stays under 350. If it crowds, move the raw-9P bootstrap
into `serve/raw9p.rs` as `start_raw9p_door(command, p9_root, process_stderr) ->
Result<Option<thread::JoinHandle<()>>, CliError>`.

---

## 7. Discovery JSON + startup status

**File:** `crates/wanix-cli/src/serve/discovery.rs`
- `serve_discovery_json` (`:35-91`): the `p9` route object (`:60-61`) currently
  advertises only `websocket`. Add a `tcp` field when the raw-9P door is bound.
  Shape change (additive, the cockpit's `routes.p9.websocket` read is unchanged):
  ```json
  "p9":{
    "websocket":"ws://HOST/.well-known/export9p",
    "tcp":"tcp://HOST:PORT",            // present only when --p9 is bound
    "transport":"direct-binary-websocket",
    "protocol":"9p2000.L",
    "supportedProtocols":["9P2000.L","9P2000.L.Google.2"]
  }
  ```
  The `tcp` URL host is the raw-9P bound addr (loopback), NOT the HTTP `host`
  header. Thread the raw-9P `SocketAddr` into `ServeRoots` (new field
  `pub(super) p9_tcp_addr: Option<SocketAddr>`) so discovery can render it.
- Add `ServeRoots.p9_tcp_addr` in `serve/roots.rs` (`:21-46`) and set it in
  `ServeRoots::new` (default `None`; serve runtime sets it after binding `--p9`).
  Since `ServeRoots::new` runs before the raw-9P bind, add a setter
  `roots.set_p9_tcp_addr(addr)` called from `run_serve_with_listener_inner` after
  the raw listener binds, OR pass `p9_addr` into `ServeRoots::new`. **Decision:**
  pass an extra `p9_tcp_addr: Option<SocketAddr>` arg to `ServeRoots::new` — but
  that breaks the ~30 `ServeRoots::new(...)` test call sites. **Better:** keep
  `ServeRoots::new` arity unchanged and add a `pub(super) fn
  with_p9_tcp_addr(mut self, addr: Option<SocketAddr>) -> Self` builder; serve
  runtime calls `roots.with_p9_tcp_addr(Some(addr))`. Zero test-call-site churn.
- Add a discovery shape test: with a raw-9P addr set, `"tcp":"tcp://` appears;
  without, the `tcp` key is absent.

**File:** `crates/wanix-cli/src/serve.rs`
- `serve_url_status` / `write_serve_startup_status` (`:88-108`, `:134-142`): when
  the raw-9P door is bound, emit a second status line:
  `wanix-rust serve: raw 9P listening on tcp://<addr>/`.

**Cockpit TS (NOT covered by `just check` — update for correctness):**
- `workbench/src/wanix/p9.ts:24-29` — add optional `tcp?: string;` to
  `WanixP9Route`. No behavior change: `fromRoute` (`:72-76`) still requires
  `route.websocket` (browsers cannot dial raw TCP). The `tcp` field is
  informational only.
- `workbench/src/web/cockpit-self-check.ts` — if it asserts the `p9` route
  shape, allow the new optional `tcp` field (do not require it). Re-grep for
  `routes.p9` / `export9p` there before editing; if it does not read the p9
  route shape, no change is needed.

---

## 8. Retire `p9-ws` and `p9-listen` subcommands

Delete the standalone commands; preserve all reusable logic (already moved in
§4: grant plumbing + listener helper into `serve/raw9p*`).

**Delete files:**
- `crates/wanix-cli/src/p9_ws.rs`
- `crates/wanix-cli/src/p9_ws/command.rs`
- `crates/wanix-cli/src/p9_ws/connection.rs`
- `crates/wanix-cli/src/p9_ws/runtime.rs`
- `crates/wanix-cli/src/p9_listen.rs`
- `crates/wanix-cli/src/p9_listen/command.rs`
- `crates/wanix-cli/src/p9_listen/runtime.rs`
- `crates/wanix-cli/src/p9_listen/grant.rs` (after moving to
  `serve/raw9p/grant.rs` in §4)

**Edit dispatch + module declarations:**
- `crates/wanix-cli/src/lib.rs:16,18` — remove `mod p9_listen;` and `mod p9_ws;`.
- `crates/wanix-cli/src/collected.rs`:
  - import list (`:5-9`) — drop `p9_listen`, `p9_ws`.
  - `run_collected_command` (`:21-23`) — `"p9-stdio" | "p9-listen" | "p9-ws"` →
    just `"p9-stdio"`.
  - `run_9p_collected_command` (`:72-87`) — delete the `p9-listen`/`p9-ws` arms;
    keep `p9-stdio`. (It can collapse back into the match in
    `run_collected_command`, but keeping the helper for `p9-stdio` is fine.)
- `crates/wanix-cli/src/process_io.rs`:
  - import (`:6-9`) — drop `p9_listen`, `p9_ws`.
  - `run_streaming_command` (`:113-137`) — `"p9-stdio" | "p9-listen" | "p9-ws"` →
    `"p9-stdio"`.
  - `run_9p_streaming_command` (`:162-184`) — delete the `p9-listen`/`p9-ws`
    arms.

**Help text — `crates/wanix-cli/src/help.rs`:**
- Delete lines `:36-38` (`p9-listen ... --addr HOST:PORT [--once] [--peer ...]`
  and `p9-ws ... --addr HOST:PORT [--once]`).
- Update the `serve` usage line (`:59-60`) to:
  `wanix-rust serve [--root DIR | DIR] [--listen HOST:PORT] [--p9 HOST:PORT
  [--peer HEX --grant ANAME:PREFIX:RIGHTS ...]] [--bundle NAME] [--wanix-services]
  [--once]`. Drop `--addr` from help (kept as hidden synonym).

---

## 9. Tests that MUST change (retired subcommands + new surface)

Do NOT weaken/delete coverage; retarget it at the new surface.

**Delete (cover removed subcommands; their value moves to serve raw-9P tests):**
- `crates/wanix-cli/src/p9_ws.rs` tests block (parse, bind error, listening
  message, `p9_ws_once_serves_host_file_over_binary_websocket`). The websocket
  round-trip value is preserved by the **serve** ws door tests in
  `serve/tests.rs` (already exercise `/.well-known/export9p`) plus the new
  `serve_duplex` core test (§1) — verify `serve/tests.rs` has a ws 9P round-trip;
  if not, ADD one against the serve ws door so ws coverage is not lost.
- `crates/wanix-cli/src/p9_listen.rs` tests block (`p9_listen_streaming_reports_
  bind_errors`, `p9_listen_startup_message_reports_bound_address`,
  `p9_listen_once_serves_host_file_over_tcp`). Re-create equivalents as serve
  `--p9` tests: bind error, startup message, raw-TCP round-trip against a live
  serve raw-9P door (move into `serve/raw9p.rs` tests or `serve/tests.rs`).
- `crates/wanix-cli/src/p9_listen/command.rs` parser tests → port to serve
  `--p9`/`--peer`/`--grant` parser tests in `serve/tests.rs`.
- `crates/wanix-cli/src/p9_listen/runtime.rs` policy tests (authorized attach,
  read-only EACCES, revoke-denies-next, `build_serve_policy` None) → move to
  `serve/raw9p.rs` (§4).
- `crates/wanix-cli/src/p9_ws/connection.rs` error display/source tests → port to
  the new `WebSocketDoorError` module (§3).

**Edit:**
- `crates/wanix-cli/src/collected.rs` tests (`:116-150`):
  `collected_streaming_commands_require_live_process_io_after_parsing` (`:117`)
  lists `p9-listen`, `p9-ws` — remove both rows; keep `serve`.
  `collected_streaming_commands_preserve_parser_errors` (`:135`) uses
  `p9-listen` — retarget to `serve` (or a kept command).
- `crates/wanix-cli/src/process_io.rs` tests (`:268-369`):
  `p9_streaming_command_reports_listener_parse_errors` (`:284`),
  `p9_streaming_command_reports_websocket_parse_errors` (`:300`),
  `p9_streaming_command_routes_listener_runtime_errors` (`:334`),
  `p9_streaming_command_routes_websocket_runtime_errors` (`:355`) — DELETE (they
  test removed subcommands). Keep the `p9-stdio` cases. Add a serve `--p9`
  runtime-error case if `run_streaming_command` is the right seam, otherwise the
  serve bind-error path is covered in `serve/tests.rs`.
- `crates/wanix-cli/src/lib.rs:618-619` — the help-output test asserts
  `"wanix-rust p9-listen"` and `"wanix-rust p9-ws"` are present. **Flip** these:
  assert they are ABSENT and assert the new `serve ... [--p9 HOST:PORT ...]`
  string is present. Keep the `p9-stdio` assertion (`:617`).
- `crates/wanix-cli/src/serve/tests.rs:2810-2828`
  (`serve_has_no_site_flag_sites_are_registered_through_files`) — keep as-is
  (it correctly asserts `--site` is rejected; this is NOT stale, it is the
  regression guard for the removed flag).

**`--site` scrub:** the only `--site` mentions are intentional regression guards
(`crates/wanix-sites/src/tests.rs:258` comment, `serve/tests.rs:2813-2816`). No
stale runtime `--site` references exist in the parser. Action: confirm `grep -rn
"\-\-site" crates/` shows only those guard/comment lines; scrub any prose in
`docs/` that still describes `--site` as a live flag (the design doc already says
it was removed).

---

## 10. Docs

- `docs/design/in-wanix-sites.md` "Open transport gap" (`:238-252`): update to
  record that the gap is now CLOSED by `serve --p9 ADDR` (raw 9P alongside HTTP)
  + `mount-write tcp://...`. Keep the websocket-client option noted as an
  alternative for browser-only contexts.
- `AGENTS.md` / `CLAUDE.md` capability map: `p9-ws` is retired; raw-9P export is
  now a serve mode (`serve --p9`); the serve 9P edge now has the
  `--wanix-services`-off-loopback refusal trust guard. Update the
  `p9-stdio`/`p9-listen`/`p9-ws` bullet to drop `p9-ws`/`p9-listen` and describe
  `serve --p9`.
- Doc corpus mentions of `p9-ws`/`p9-listen` (`docs/site/content/...`,
  `docs/integration/plan.md`, `docs/rust-vs-go-wanix.md`,
  `docs/mesh-the-missing-half-of-9p.md`): update prose to the new surface where
  load-bearing; non-load-bearing historical references can stay but should not
  describe them as current commands.

---

## 11. ADR 0006

**New file:** `docs/adrs/0006-serve-9p-transport-and-trust-boundary.md`. Record:
- **Decision:** one per-connection 9P session core (`serve_stream`/`serve_duplex`
  delegating to a single loop); the websocket is a `WebSocketDuplex` framing
  adapter over that core, not a second server; raw 9P over TCP is a serve mode
  (`--p9 ADDR`), not a separate `p9-listen`/`p9-ws` subcommand.
- **Trust boundary:** the serve 9P edge refuses `--wanix-services` on any
  non-loopback door (ws or raw), mirroring the mesh exec-device rule; raw-9P
  attaches may be capability-gated via `--peer/--grant` (`P9Server::with_policy`).
- **Consequences:** `p9-ws`/`p9-listen` retired; `mount-write tcp://` works
  against a live serve; `p9-stdio` (pipe/QEMU bridge) and `mesh-serve` (crypto
  trust) are unaffected; `--addr` deprecated in favor of `--listen`.
- Point implementation status at the moved/added tests and commit messages.
- Add the ADR to the root **ADR Index** in `CLAUDE.md`/`AGENTS.md`.

---

## 12. Quality gate

Run `just check` (fmt on the justfile package list; `tools/check-module-lines.sh`
≤350; `clippy --workspace --all-targets -D warnings`; `cargo test --workspace
--locked`). No `Cargo.toml` dependency changes are expected (tungstenite already
a cli dep; no new crates) — but if any `Cargo.toml` changes, run a build to
refresh `Cargo.lock` (tests are `--locked`). Keep tokio/iroh out of `wanix-9p`
and the new serve-local modules. Hold no fs/namespace lock across a call into
another fs. New public contract: `P9Server::serve_duplex` (typed, documented);
new serve-local newtypes (`WebSocketDuplex`, `ServePolicy`, `WebSocketDoorError`)
— no public raw `i32`/flags.

### Module-line watchlist
- `crates/wanix-9p/src/transport.rs` (297 incl. tests) — +~25 prod for
  `serve_duplex`/`DuplexPair`. OK.
- `crates/wanix-cli/src/serve.rs` (145) — +guard helper + raw-9P bootstrap.
  Offload to `serve/raw9p.rs` if it nears the warn line.
- `crates/wanix-cli/src/serve/raw9p.rs` (new, ~110 prod) and
  `serve/raw9p/grant.rs` (moved, 151) — OK.
- `crates/wanix-cli/src/serve/ws_duplex.rs` (new, ~100) — OK.
- `crates/wanix-cli/src/serve/discovery.rs` (188) — small additive change. OK.
- `crates/wanix-cli/src/serve/command.rs` (170) — +3 fields/setters/validation.
  Watch the warn line; parts of parsing already live in `command/options.rs`.

---

## Ordered execution

1. §1 `serve_duplex`/`DuplexPair` in `wanix-9p` + test. `just check`.
2. §2 `WebSocketDuplex` (`serve/ws_duplex.rs`) + unit tests.
3. §3 rewire serve ws door to `serve_duplex`; introduce `WebSocketDoorError`;
   delete `p9_ws/connection.rs` session logic. `just check` (serve ws tests).
4. §4 move grant plumbing + listener helper into `serve/raw9p*`; port policy
   tests. (Keep p9_listen compiling for now or move atomically — prefer moving
   atomically with §8.)
5. §5 serve `--p9`/`--peer`/`--grant` parsing + deprecate `--addr` in help.
6. §6 bind raw-9P listener thread + trust guard (`enforce_services_trust_boundary`).
7. §7 discovery `tcp` route + startup status + `ServeRoots::with_p9_tcp_addr`;
   cockpit TS `tcp?` field.
8. §8 delete `p9_ws`/`p9_listen` files; fix dispatch/module decls/help.
9. §9 retarget/flip all affected tests; ensure ws + raw-TCP round-trip coverage
   lands on serve. `just check`.
10. §10 docs; §11 ADR 0006 + index. `just check`. (A later workflow step commits.)
