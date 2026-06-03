# ADR 0004: Go As Migration Oracle

## Status

Accepted

## Context

The Go implementation contains the current Wanix behavior, but its package
shape is not necessarily the best structure for a Rust-native runtime. A direct
file-by-file translation would preserve incidental structure and delay the
externally visible runtime goal.

## Decision

Use the Go implementation as a semantic oracle during migration. Port behavior
and contracts first: filesystem behavior, namespace bind/resolve semantics,
task allocation, fd handling, API protocols, and service resources.

## Consequences

- Rust crate boundaries may differ from Go package boundaries.
- Go tests and examples should inform compatibility tests where useful.
- Divergences should be explicit and documented when they affect public
  behavior.
- Cleanup and refactoring are justified when they protect compatibility,
  trust boundaries, or the next visible demo outcome.
