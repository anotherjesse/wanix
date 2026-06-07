---
title: Quality Gates (just check)
slug: reference/quality-gates
pageType: developer
oneLiner: fmt, module-lines, clippy -D warnings, test, and the composite just check; stay under the 250-350 line module limit; use explicit newtypes over raw i32 flags.
audience: [developer]
tags: [cli, contributor, code-quality, caveat, shipped]
sourceRefs: [Justfile:3-15, tools/check-module-lines.sh:4-134, tools/module-line-baseline.txt:1-3, AGENTS.md:249-266, AGENTS.md:341-348, crates/wanix-module-cache/src/trust.rs:13-17]
seeAlso: [reference/crate-map-and-layering, reference/queued-follow-ups, reference/contributor-landing, learn/contribute-to-core]
prerequisites: [reference/crate-map-and-layering]
usedInFlows: [{flow: learn/contribute-to-core, step: 3}]
honestLimits: ["Strong fd-ownership verification in wanix-module-cache is Unix-only; on non-Unix the directory check degrades to is-a-directory and the artifact is read by path.", "The module-line script counts non-test lines with an awk heuristic, not the Rust compiler — it can miscount unusual #[cfg(test)] shapes.", "just check runs the full test suite serially with --locked; it is not a substitute for the cycle-level review pass."]
canonicalCaveatFor: []
---

# Quality Gates (just check)

fmt, module-lines, clippy -D warnings, test, and the composite just check; stay under the 250-350 line module limit; use explicit newtypes over raw i32 flags.

Before a cycle lands, one command has to be green. `just check` chains four gates — formatting, module size, lint, and the full test suite — and a cycle commit is not done until it passes (`AGENTS.md:262-266`). This page is the contributor-facing contract for those gates: what each recipe runs, where the 250/350-line module budget comes from, the small bits of code style the boundary enforces, and the cycle rhythm that wraps it all. The point is not bureaucracy. It is that the [layering rules](/reference/crate-map-and-layering) only stay true if every change passes through the same mechanical sieve.

## The just recipes

The gates live in the repo `Justfile` (`Justfile:3-15`). Run any one alone, or run the composite:

```sh
just fmt            # cargo fmt --check across all 22 crates
just module-lines   # bash tools/check-module-lines.sh
just clippy         # cargo clippy --workspace --all-targets -- -D warnings
just test           # cargo test --workspace --locked
just check          # fmt -> module-lines -> clippy -> test, in order
```

`check` is defined as `check: fmt module-lines clippy test` (`Justfile:15`), so it short-circuits on the first failure. A note on each: `fmt` names every `wanix-*` package explicitly and passes `--check`, so it reports drift rather than rewriting your tree. `clippy` runs `--all-targets` with `-D warnings`, meaning every lint is an error — a stray `unused_variable` fails the build. `test` passes `--locked`, so a stale `Cargo.lock` is a failure too; regenerate the lockfile as part of the change that needs it, never as a side effect. (Separate `quality*` recipes drive coverage tooling — `cargo llvm-cov`, `crap`, `rustqual` — but those are not part of `check` and are not a landing requirement.)

## The 250-350 line module budget

`module-lines` is the one gate that is bespoke. `tools/check-module-lines.sh` walks every `crates/**/src/**.rs` file, skips test files by path (`tests.rs`, `*_tests.rs`, anything under `tests/`), and counts *non-test, non-blank, non-comment* lines — it even strips `#[cfg(test)]` blocks with an awk brace-matcher so an inline test module does not count against the budget (`tools/check-module-lines.sh:21-89`).

Two thresholds apply (`tools/check-module-lines.sh:4-5`):

- **250 lines — preferred limit.** A module over 250 lines emits a `module-lines warning:` and keeps building. It is a signal to split before the file grows further, not a failure.
- **350 lines — hard limit.** A module over 350 lines is a `module-lines error:` and `just check` fails — unless an entry in `tools/module-line-baseline.txt` grandfathers it.

The baseline file is the registry of grandfathered over-limit modules: each line is `<max-allowed-count> <path>`, counts may shrink but must never grow (`tools/module-line-baseline.txt:1-3`). Right now it holds only comments — **nothing is exempted above the hard limit.** Three modules sit in the warn band and should be split before new feature work lands in them: `crates/wanix-agent/src/codex.rs` (307), `crates/wanix-agent/src/exec_server.rs` (283), and `crates/wanix-cli/src/serve/http/app.rs` (273). The script also flags stale baseline entries and tells you when a once-exempt file has dropped back under the limit so you can remove its entry. See [queued follow-ups](/reference/queued-follow-ups) for the live split list.

## Explicit types over raw i32

The code-quality guardrail is short but load-bearing (`AGENTS.md:258-260`): use explicit Rust types for public contracts, and avoid public raw `i32` flags, file descriptors, rights, or modes where a newtype or builder makes the trust boundary clearer. A 9P open mode or a capability right expressed as a bare integer hides which values are legal and lets the wrong one cross a boundary silently; a newtype makes the contract checkable at the call site. This is enforced by review, not by a lint, so it is on you and your reviewer.

One more invariant belongs here because it is a correctness rule, not style: **do not hold a namespace or filesystem lock while calling into another filesystem** (`AGENTS.md:255-256`). A device's `FileSystem` method can resolve into another device — and across the mesh, into a *remote* one — so holding your own lock across that call is how you deadlock the namespace.

## Cycle rules

The gates sit inside a rhythm (`AGENTS.md:341-348`): prefer the highest-leverage externally visible outcome, use cleanup only when it unblocks that outcome or protects a boundary, and **commit each completed cycle before starting the next.** Every roughly five feature commits, run a dedicated review/cleanup pass over the changes since the last one before adding more feature work. `just check` is the per-commit gate; the five-commit cleanup pass is the per-arc gate. Both are part of [contributing to core](/learn/contribute-to-core).

## See also

- [Crate Map & Dependency Direction](/reference/crate-map-and-layering) — the layering the gates protect.
- [Queued Follow-ups](/reference/queued-follow-ups) — the live module-split and cleanup list.
- [Contributor landing](/reference/contributor-landing) — where the gates fit in the contribution path.
- [Contribute to core](/learn/contribute-to-core) — the end-to-end flow that runs these checks.

## Status / honest limits

- **fd-ownership verification is Unix-only.** `wanix-module-cache` verifies the artifact directory and file through `O_NOFOLLOW` + `fstat` file descriptors on Unix; on non-Unix platforms there is no portable fd-ownership model, so the directory check degrades to "is a directory" and the artifact is read by path (`crates/wanix-module-cache/src/trust.rs:13-17`). The gates run identically on every platform, but that trust check does not.
- **`module-lines` is a heuristic, not the compiler.** It counts with awk by stripping blanks, `//` comments, and `#[cfg(test)]` blocks via brace-matching; an unusual test-module shape can miscount. Trust the warning, but read the file.
- **`just check` is the per-commit gate, not the whole bar.** It runs the suite serially with `--locked`; it does not replace the five-commit review/cleanup pass, and it does not run the optional `quality*` coverage recipes.
