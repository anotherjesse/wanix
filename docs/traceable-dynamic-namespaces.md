> Editorial note (cpu branch): The ideas in this exploratory document are no
> longer purely speculative. The cpu branch's `#plumb` device and the
> `qjs-shell` mutation frame implement a concrete first slice of this
> substrate: shell-driven mutations and external effects flow through a
> traceable plumber boundary, and namespace state changes can be observed and
> correlated with the commands that caused them. Treat this document as the
> conceptual substrate for the cockpit Activity feed — the feed is the
> user-facing surface of the causal model described below, and `#plumb` is one
> of the first wires carrying real provenance through the system.

# Traceable Dynamic Namespaces

Status: exploratory product and architecture note, not an ADR.

Wanix should make it possible to answer a simple question:

```text
Why is the system like this?
```

As agents become able to write files, run commands, start services, edit code,
inspect logs, launch browsers, and chain those actions together, that question
becomes the difference between a magical system and an unnerving one. A useful
agentic runtime should not only let things happen. It should remember what
happened, why it happened, what changed, and what the world looked like before
and after.

The idea here is to make dynamic namespaces traceable and forkable. A namespace
is not just a path lookup table. It is the live shape of a system: which
volumes are mounted, which versions of those volumes are visible, which tasks
ran against them, which terminal logs and HTTP requests were part of the work,
and which mutations resulted.

In that world, a user can say:

```text
Show me what the agent changed.
Open the system as it was before that command.
Fork the exact state from this failing test.
Compare this route before and after the repair.
Restore the workspace to the moment before task 42.
```

That is the north star: everything Wanix owns can rewind, and everything Wanix
touches can be traced.

## What This Is

Traceable dynamic namespaces combine four Wanix-native ideas:

- named volumes, such as `gitfs`, `memfs`, `hostfs`, `httpfs`, `r2fs`,
  `sqlitefs`, or `automergefs`;
- namespace bindings, where a volume or sub-tree appears at a path like `/app`;
- causal contexts, where user actions, agent turns, commands, tasks, HTTP
  requests, and editor operations are grouped into explicit traces;
- snapshots, where the namespace state before and after an action can be
  addressed, mounted, compared, forked, or restored.

The user-facing model could stay small:

```sh
volume create git app --repo ./app.git --branch main --commit task
mount app /app
task run qjs /app/fix.js
ns fork trace:task/42:before --name before-fix
mount ns:before-fix /time/before
```

The unique part is not that Git appears as a filesystem. The unique part is
that execution, editing, serving, task logs, and filesystem history all share
one causally linked system model.

## Why This Should Exist

Agents make computers more dynamic. They can act across more surfaces, faster
than a human can follow line by line. That is exactly why the runtime beneath
them should become more inspectable, not less.

Today, many agent workflows are held together by terminal scrollback, file
diffs, hidden tool calls, and ad hoc summaries. That works for demos, but it
is a weak foundation for real work. The moment an agent can run commands,
modify source, create data, start a server, query a database, and react to a
browser, the user needs a coherent way to ask what happened.

Wanix is well-positioned to provide that coherence because it already treats
important system pieces as files and namespaces: tasks in `#task`, terminals in
`#term`, served roots, qjs and wasm runtimes, 9P mounts, and browser/editor
integration. Traceable namespaces extend that shape instead of replacing it.

The system should exist because it gives agentic computing a substrate with:

- provenance: every mutation can point back to the command, task, agent turn,
  terminal range, or request that caused it;
- reviewability: generated changes can be inspected as diffs connected to the
  execution that produced them;
- reversibility: managed state can be restored or forked from earlier points;
- reproducibility: a bug report can include the namespace, volume versions,
  task inputs, logs, and route/request context needed to replay it;
- collaboration: each user, agent, or task can work in its own namespace fork
  and merge intentionally;
- safety: destructive restore can be avoided by default because the past can
  be mounted as another namespace;
- teachability: the system can show its own model through files, not only
  through a bespoke UI.

This is especially important for agents because trust is not only about
permission prompts. Trust is also about legibility. A user should be able to
see the path from intent to effect:

```text
prompt -> plan -> command -> task -> terminal log -> filesystem mutation
       -> HTTP route change -> test result -> final state
```

If that chain is visible and addressable, agent work becomes much easier to
debug, audit, undo, and build on.

## What Feels Unique

Most development environments center the repository, the editor, the terminal,
or the container. Wanix can center the namespace.

That is a subtle but meaningful shift. A namespace can contain a repository,
but also task service files, terminals, generated data, mounted object stores,
HTTP routes, database state, Automerge documents, browser-visible files, and
future remote resources. It is the actual working world a task saw.

That means the unit of work can become:

```text
an actor operating in a namespace over time
```

instead of only:

```text
a process
a Git commit
a shell session
an editor buffer
a container
```

Once the namespace is the unit of work, new interactions become natural:

- click a terminal line and see the files changed by the command that printed
  it;
- click a file diff and see the agent message and task transcript that caused
  it;
- open a failed HTTP route exactly as it existed during the failing request;
- fork the system from before an attempted repair and try a different approach;
- compare two agents' fixes as two namespace branches;
- mount yesterday's `/app` at `/time/yesterday/app` without changing today's
  `/app`.

This is not different for the sake of being different. It is a better fit for
systems where code, execution, logs, data, browser state, and agents are all
live at the same time.

## The Product Feeling

The browser workbench should feel less like "an editor connected to a server"
and more like an operating-system cockpit. It should be obvious that Wanix is
tracking living objects:

```text
Volumes
  app       gitfs main abc123 mounted at /app
  scratch   memfs root 81aa20 mounted at /tmp
  notes     automergefs heads[...] mounted at /notes

Namespaces
  session   / -> root, /app -> app, /tmp -> scratch
  task-42   snapshot from repair run

Tasks
  42 qjs /app/fix.js exited 0

Trace
  user asked for repair
  agent edited /app/server.js
  task 42 ran tests
  app committed def456
```

A user should be able to move between those objects without losing the causal
thread. "What changed?" and "why did it change?" should be adjacent questions.

The most magical default is not destructive time travel. It is live forking:

```sh
ns fork trace:task/42:before --name before-repair
mount ns:before-repair /time/before-repair
```

Now the past is a place. The user can browse it, run against it, diff it, serve
it, or hand it to another agent without destroying the present.

## Why Gitfs Matters But Should Not Dominate

A Git-backed filesystem is a strong first proof because source code, configs,
small generated artifacts, and agent edits already want history. A `gitfs`
volume can make writes become commits, and those commits can carry Wanix
context:

```text
author: wanix task 42
actor: agent session abc
command: qjs /app/fix.js
namespace-before: ns@100
namespace-after: ns@101
terminal: #term/7 bytes 1804..2442
```

That makes agent changes reviewable with normal Git tools while also remaining
connected to Wanix task and terminal state.

But Git should not become the universal storage model. High-frequency logs,
large binary churn, databases, locks, caches, and collaborative documents often
need different backing stores. The volume abstraction is the important part:

```text
gitfs       source/config/history
memfs       scratch work and temporary forks
sqlitefs    transactional local data
automergefs collaborative documents and CRDT state
r2fs        object storage
httpfs      remote APIs or fetched artifacts
hostfs      explicit host directory access
```

Each volume driver should expose the same namespace and trace hooks while
using a storage model that fits its data.

## What Rewind Means

Rewind should have levels.

The safest level is inspectable rewind:

```sh
mount trace:task/42:before /time/task-42-before
```

This makes old state visible without changing current state.

The next level is forkable rewind:

```sh
ns fork trace:task/42:before --name retry-fix
task run --ns retry-fix qjs /app/alternate-fix.js
```

This creates a live branch from the past.

The strongest level is restorative rewind:

```sh
ns restore session trace:task/42:before
```

That should be explicit because it changes what the current session sees.

Different volume drivers can support different depths of rewind. A Git volume
can restore by commit. An Automerge volume can restore by heads. A database
volume might restore by transaction checkpoint. A memfs volume might restore
only while its snapshot is retained. A hostfs mount may be traceable but not
rewindable unless Wanix is explicitly managing a shadow log.

The honest rule is:

```text
Managed Wanix state can rewind.
External effects can be traced.
```

Sending an email, posting to a service, charging a card, or mutating an
unmanaged host directory cannot be made un-happened by wishful naming. Wanix
can still record that the effect happened, with the responsible context and
request/response metadata where permitted.

## Implementation Sketch

This section is intentionally lower in the note. The product value comes from
causal, rewindable work. The details below are one possible shape.

### Core Objects

```text
Volume
  id
  name
  driver
  driver config
  current head or root
  capabilities

Namespace
  id
  parent namespace id, optional
  mount table
  created by trace event

TraceContext
  id
  parent context id, optional
  actor
  intent or command
  started/finished timestamps
  namespace before
  namespace after
  log anchors
  mutation list
  external effect list

Mutation
  volume id
  path
  operation
  before version
  after version
  trace context id
```

### Service Files

Wanix can expose this through service files before it needs a polished UI:

```text
#volume/
  ctl
  app/
    root
    status
    log
    head
    ctl

#ns/
  self/
    ctl
    mounts
    snapshot
  session/
    ctl
    mounts

#trace/
  current
  events
  task/
    42/
      before
      after
      mutations
      logs
      effects
```

Example low-level operations:

```sh
echo 'create git name=app repo=/tmp/app.git branch=main commit=task' > '#volume/ctl'
echo 'bind #volume/app/root /app' > '#ns/self/ctl'
echo 'fork trace:task/42:before name=before-fix' > '#ns/session/ctl'
```

Friendlier CLI wrappers can come later:

```sh
volume create git app --repo /tmp/app.git --branch main --commit task
mount app /app
ns fork trace:task/42:before --name before-fix
```

### Commit And Snapshot Policies

Volume drivers should declare their mutation policy instead of pretending every
filesystem wants the same behavior:

```text
commit=op          each write/remove/rename is recorded immediately
commit=tx          batch until an explicit commit
commit=task        one coherent mutation set per Wanix task
commit=agent-step  one mutation set per agent action
commit=manual      user or tool decides when to checkpoint
```

For agent workflows, `commit=task` and `commit=agent-step` are likely the most
useful defaults. They keep history readable while preserving the connection
between action and effect.

### Task Integration

When a task starts, Wanix can record:

```text
namespace_before = current namespace snapshot
trace_context = new child context
terminal anchors = current terminal offsets
```

When the task exits, Wanix can record:

```text
namespace_after = resulting namespace snapshot
volume mutations = all managed writes attributed to the task context
terminal anchors = output range
exit status = task result
```

This turns a task from an isolated execution record into a causal bridge across
namespace state, logs, and file history.

### Namespace Semantics

Mounting is a namespace mutation, not a filesystem feature. `gitfs` should not
know whether it appears at `/app`, `/src`, or `/time/before/app`. It exposes a
root. The namespace binds that root to a path.

That keeps the model general:

```text
/app   -> volume app at commit def456
/tmp   -> volume scratch at root 81aa20
/notes -> volume notes at Automerge heads [...]
```

A task can inherit the session namespace by snapshot, or it can receive an
explicit namespace:

```sh
task run --ns retry-fix qjs /app/test.js
```

Session-level namespace changes should be visible and traceable. Task-local
namespace changes can be isolated unless explicitly promoted.

### Gitfs As First Driver

A pragmatic first `gitfs` can be worktree-backed:

```text
read/write through a Wanix fs adapter
track dirty paths
on checkpoint:
  git add -A
  git commit with Wanix trace metadata
  update volume head
```

That is enough to prove the product loop quickly. A later implementation can
move toward direct object/tree mutation for stronger isolation, cheaper forks,
and object-store backing.

The important first demo:

```text
1. mount git volume at /app
2. run an agent or qjs task that edits /app
3. show the task transcript
4. show the resulting commit
5. fork the namespace from before the task
6. mount before/after side by side
7. run tests in both
```

### External Effects

External effects need explicit capability boundaries. If a task can reach the
network, host filesystem, browser, or a third-party API, Wanix should be able
to record at least:

```text
effect kind
capability used
trace context
request metadata, if safe
response metadata, if safe
whether the effect is replayable, compensatable, or only observable
```

This prevents rewind from making false promises. Some effects can be replayed.
Some can be compensated. Some can only be remembered.

## A Possible First Milestone

The first useful version does not need universal time travel. It needs one
convincing loop:

1. Create a named `gitfs` volume.
2. Mount it into the served Wanix namespace at `/app`.
3. Run a qjs task or shell command that mutates `/app`.
4. Record a trace event connecting the task, terminal output, namespace before
   and after, and resulting commit.
5. Show that trace in the browser workbench.
6. Fork or mount the namespace from before the task.
7. Compare before and after from inside Wanix.

If that feels good, the abstraction is probably right.

## Open Questions

- What is the smallest snapshot representation that works across `memfs`,
  `gitfs`, and future database or Automerge volumes?
- Should namespace snapshots be content-addressed, append-only records, or
  reconstructable from trace logs?
- How much trace metadata belongs in Git commit messages versus Wanix service
  state?
- What is the privacy policy for terminal logs, prompts, HTTP bodies, and
  external request metadata?
- Which operations are allowed to mutate the session namespace, and which only
  mutate a task-local namespace?
- Can the workbench make this model obvious without hiding the service files
  that make it understandable?

## The Bet

The bet is that future agentic systems need more than permission prompts and
final diffs. They need a live, inspectable, forkable model of causality.

Wanix can provide that by treating namespaces as first-class system state and
by making tasks, volumes, logs, and external effects point back to the contexts
that caused them. Git-backed volumes are a strong first proof, but the deeper
idea is broader: a small operating system that can remember why it changed and
let the user keep working from any meaningful moment.
