# ADR 0019: Parse `#task/ctl bind` Operands as Shell Words

## Status

Accepted.

## Context

Wanix tasks expose fd wiring through the `#task/<id>/ctl` control file. The
current Rust task service supports:

```text
bind <namespace-path> fd/<n>
bind #task/<id>/fd/<n> fd/<m>
```

Before this ADR, `ctl bind` split the control line on raw whitespace. That made
ordinary Wanix paths containing spaces impossible to bind, even though the Rust
path validator and the Go `io/fs.ValidPath` shape allow those path components.
ADR 0015 already adopted a small shell-word grammar for `#task/cmd` so
file-controlled qjs tasks can preserve exact argv.

## Decision

Parse `#task/ctl` `bind` commands with the same shell-word grammar used for
`#task/cmd`:

- whitespace separates words outside quotes;
- single and double quotes can preserve spaces inside operands;
- backslash escapes the next character outside quotes;
- incomplete quoted writes remain pending while the control file accumulates
  more bytes.

`start` remains a simple prefix-accumulated control command. `bind` still
accepts exactly two operands after the command: a namespace source path and an
fd destination.

## Consequences

JavaScript and other file-oriented task controllers can now wire fds from Wanix
paths such as `child stdin.txt` without falling back to helper host APIs or
renaming files around the control grammar.

This is still not a full shell language. The control file does not perform
expansion, globbing, command substitution, redirection, or environment lookup.
