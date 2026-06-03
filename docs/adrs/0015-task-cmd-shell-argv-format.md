# ADR 0015: Parse `#task/cmd` as Shell-Quoted Argv

## Status

Accepted.

## Context

Wanix tasks expose `cmd`, `env`, `dir`, `ctl`, and `exit` through `#task`.
The Go rc integration writes `#task/<id>/cmd` by shell-quoting every argv
element, so arguments containing spaces, quotes, or empty strings remain
representable as text.

The Rust qjs root CLI path already uses `TaskSpec`, which preserves exact argv.
Child qjs tasks launched through `#task/new/qjs` still wrote raw `cmd` text and
fell back to whitespace splitting, so `two words` and empty arguments were lost
before they reached WASI argv.

## Decision

Keep `cmd` as a raw text service file for compatibility and readback, but parse
successful writes into a typed argv view using a small POSIX-style shell word
grammar:

- whitespace separates words outside quotes;
- single quotes preserve bytes until the next single quote;
- double quotes preserve text while allowing `\"` and `\\`;
- backslash escapes the next character outside quotes;
- empty quoted words are retained as empty arguments;
- unterminated quotes or escapes reject the write.

`TaskSpec` remains the explicit launch contract used by programmatic callers.
The parsed raw `cmd` argv is the fallback for file-controlled tasks. `env` and
`dir` continue to be independent `#task` service fields.

## Consequences

JavaScript can spawn child qjs tasks through `#task` with exact argv, matching
the existing Go rc command-writing convention. Reading `cmd` still returns the
text written by the caller.

This is not a full shell language: there is no expansion, globbing, command
substitution, or redirection. A future explicit argv/spec service file can still
be added without changing this compatibility path.
