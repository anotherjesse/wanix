# ADR 0050: Rust Terminal Device Foundation

## Status

Accepted

## Context

The Go Wanix runtime exposes terminals through a `#term` service. Reading
`#term/new` allocates a resource id, and each resource exposes `data`,
`program`, and `winch` files. Browser xterm elements attach to `data`; tasks
bind stdin/stdout/stderr to `program`.

The Rust port already has Wanix task fds, namespace binding, live QuickJS/WASI
stdio, and bounded ready-IO turns, but it did not have a Rust-native terminal
device. That meant interactive shell work had nowhere principled to attach
besides one-shot stdin/stdout fixtures.

## Decision

Add a `wanix-term` crate that depends only on `wanix-fs` for production code.
It implements the first Rust terminal filesystem contract:

- `new` allocates incrementing terminal resource ids.
- `<id>/id` reports the resource id.
- `<id>/data` is the terminal side.
- `<id>/program` is the program side.
- `<id>/winch` broadcasts writes to open readers.

Writes to `data` are read from `program`. Writes to `program` are read from
`data`, with lone `\n` mapped to `\r\n` to match the Go terminal output
behavior used by browser xterm integrations.

## Consequences

Rust Wanix now has a terminal service foundation that can be bound into a task
namespace and attached to task fds. This is the first direct step toward native
interactive shell demos outside Chrome.

The initial Rust terminal files are nonblocking in the sense that a read with
no buffered bytes returns `0`. This does not yet add terminal allocation through
the Rust CLI, raw/cooked line discipline, terminal resize semantics beyond
basic `winch` broadcast, a native interactive shell command, browser xterm
integration, 9P export, qemu/v86 wiring, or VS Code/serve integration.
