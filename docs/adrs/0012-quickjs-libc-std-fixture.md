# ADR 0012: QuickJS Libc Std Fixture

## Status

Accepted

## Context

Wanix now has live QuickJS WASI hooks that can forward Preview 1 process,
stdio, fd, preopen, and path metadata calls into Wanix-owned semantics. The old
checked-in QuickJS WASM fixture did not initialize QuickJS-NG libc modules, so
guest JavaScript could not import `qjs:std` or `qjs:os` and exercise that WASI
surface directly.

The `rust-wasi-quickjs` reference checkout already contains the C adapter and
QuickJS-NG sources needed to build a libc-enabled reactor, but the original
fixture build omitted `quickjs-libc.c` and used `-DQJS_BUILD_LIBC=0`.

## Decision

Replace the checked-in `wanix-qjs-engine` fixture with a QuickJS build that
links `quickjs-libc.c`, defines `QJS_BUILD_LIBC`, and initializes
`qjs:std`, `qjs:os`, and `qjs:bjson` during default runtime creation.

Keep the source checkout external for now, matching ADR 0010. The fixture is
rebuilt from the reference adapter with these local source/build changes:

- include `quickjs-libc.h` in `c/interface.c`
- add a context helper that calls `js_init_module_std(ctx, "qjs:std")`,
  `js_init_module_os(ctx, "qjs:os")`, `js_init_module_bjson(ctx, "qjs:bjson")`,
  and `js_std_add_helpers(ctx, -1, NULL)`
- call `js_std_init_handlers(rt)` before creating the default context and
  `js_std_free_handlers(rt)` during runtime destruction
- make the module normalizer return `qjs:` specifiers unchanged before calling
  the Rust host normalizer, so built-in stdlib modules coexist with Wanix
  namespace module loaders
- add `quickjs-ng/quickjs-libc.c` to `QJS_SRCS`
- replace `-DQJS_BUILD_LIBC=0` with `-DQJS_BUILD_LIBC`

The engine crate still owns only Wasmtime import wiring and guest-memory
copying. Wanix task, fd, namespace, cwd/env/cmd, and exit policy stay in
`wanix-wasi`, `wanix-task`, and `wanix-qjs`.

## Consequences

Guest JavaScript can now import QuickJS-NG standard modules from the bundled
fixture. The first committed proof is `qjs:std` stdio: `std.out.puts(...)` and
`std.err.puts(...)` write through WASI fd 1 and 2, and Wanix-backed qjs tasks
route those bytes through task stdio fds.

The libc-enabled fixture imports additional Preview 1 functions. At the time
of this ADR, unsupported directory mutation, timestamp mutation, fd flag
mutation, and non-empty polling were defined by the engine as explicit `NOSYS`
surfaces until Wanix owned those semantics. Later ADRs move individual calls
from that fallback surface into live Wanix-backed providers.

Changing the fixture changes the module SHA-256 used by snapshot identity
validation. Snapshots produced by the older non-libc fixture are expected to be
rejected by the new module, preserving the exact-build snapshot contract.

ADR 0013 adds the first filesystem read semantic pass for `std.loadFile(...)`
and `os.open(...)`/`os.read(...)`, then extends the same rights projection to
basic create/truncate writes through `std.writeFile(...)` and `os.write(...)`.
Directory-specific open modes, fd flag mutation, and richer libc compatibility
remain follow-up work.
