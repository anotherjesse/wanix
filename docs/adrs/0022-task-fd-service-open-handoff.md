# ADR 0022: Task Fd Service Open Handoff

## Status

Accepted.

## Context

Wanix tasks expose open fds through `#task/<id>/fd/<n>`. Parents can wire a
child fd by writing a control command such as:

```text
bind #task/1/fd/4 fd/0
```

Before this ADR, the Rust task service opened fd paths as live proxies back to
the source task and fd number. If the parent closed fd 4 after binding it into a
child, the child could lose access because later I/O re-resolved the now-closed
parent fd number.

That is fragile for task handoff demos where JavaScript opens a file through
WASI, binds it into a child task, and then closes its own fd.

## Decision

Opening `#task/<id>/fd/<n>` captures a clone of the source task's open-file
handle at service-open time. The clone shares the same underlying file and
offset, but no longer depends on the source fd number remaining installed in the
source task table.

The opened service file still enforces the read/write access requested by the
service open.

## Consequences

`ctl bind #task/<parent>/fd/<n> fd/<m>` becomes a stable fd handoff: the child
keeps the open file handle even if the parent closes its fd after the bind.

This is still a shared open-file handle, not an independent re-open of the path.
Offsets and writes are shared with other clones of the same open handle.
