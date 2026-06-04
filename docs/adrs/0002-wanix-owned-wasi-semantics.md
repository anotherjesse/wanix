# ADR 0002: Live Wanix-Owned WASI Semantics

## Status

Accepted

## Context

QuickJS runs in Wasmtime as a WASI guest, but Wanix cannot delegate process or
filesystem meaning to host WASI. Host WASI would expose the host filesystem,
rights model, clocks, fd table, and path rules instead of the namespace and task
state that Wanix owns.

Wanix tasks need Preview 1 calls to observe the same namespace, cwd, service
paths, stdio, fd table, metadata, and mutation behavior that native Wanix code
observes. The engine crate should decode guest memory and call a provider; it
should not decide Wanix filesystem or task policy.

## Decision

`wanix-wasi` owns the generic Preview 1 semantics used by Rust Wanix tasks.
It is backed by Wanix namespaces, task file descriptors, explicit preopens, and
Wanix filesystem traits rather than by host WASI filesystem semantics.

The Wanix WASI boundary includes:

- root and cwd handling through task namespaces;
- service paths such as `#task` staying rooted at the service namespace even
  when the ordinary root preopen maps to a task cwd;
- fd reads, writes, seek/tell, fd metadata, fd flags, and fd close behavior;
- rights projection that lets broad libc open requests become the Wanix-enforced
  rights reported on the opened fd;
- path open, stat, directory listing, directory create/remove, file unlink,
  same-filesystem rename, append mode, fdflag mutation, timestamp mutation,
  truncate, symlink metadata, readlink, and symlink creation;
- deterministic clock policy for `clock_time_get` and timestamp `*_NOW`
  updates;
- timer-only `poll_oneoff` plus fd readiness reporting through Wanix readiness
  hooks; and
- explicit unsupported errors for Preview 1 behavior without a Wanix contract.

When a QuickJS task opens dynamic regular-file WASI fds, the adapter mirrors the
visible fd into the Wanix task fd table at the same number and removes it when
WASI closes the fd. Directory fds can stay WASI-internal until a Wanix task fd
contract needs them. Snapshot logic rejects open dynamic WASI fds unless a
future ADR defines serializable virtual fd state.

## Consequences

JavaScript running through `qjs:std` and `qjs:os` reaches Wanix-owned
filesystem and fd semantics without `globalThis.Wanix` helpers and without a
read-only virtual projection bridge. Exact syscall behavior is pinned by
`wanix-wasi`, `wanix-qjs`, and CLI tests instead of by one ADR per operation.

Future WASI Preview 1 additions should extend this contract when they expose a
durable Wanix semantic boundary. Routine syscall coverage milestones should
live in tests, examples, and commit messages.

## Replaces

This ADR consolidates ADR 0013, ADR 0014, ADR 0020, and ADRs 0023 through 0037
into the live Wanix-owned WASI boundary.
