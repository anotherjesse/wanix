# Rust Wanix and Go Wanix: Moving the Host Boundary

Wanix began as a web-native operating environment: Plan 9 ideas, per-task
namespaces, file-like services, WebAssembly programs, browser storage, v86, and
interactive UI in the page. The original Go implementation proved that this was
not just an aesthetic. It showed that you could build something that felt like
an operating system inside the browser, where the browser was not merely a
viewer but the host.

The Rust implementation keeps that north star, but it moves the host boundary.
That is the most important difference.

Go Wanix asks: what if the browser can be the operating environment?

Rust Wanix asks: what if Wanix can be a native operating environment, and the
browser is one excellent client of it?

That difference is not cosmetic. It changes the runtime substrate, the module
boundaries, the meaning of WASI, the command-line surface, the protocol work,
the way terminal I/O is modeled, and the path toward running Wanix in the
cloud.

This post is a snapshot of where that port stands today. It is not a claim that
Rust Wanix has caught up with everything the Go version can do. It has not. The
Go version is still broader as a browser system. The Rust version is narrower,
but it is deeper at the native runtime boundary.

## The Go Implementation Proved The Web OS

The original Go implementation is much more than a prototype runtime. It is a
broad browser-native Wanix system.

At the top level, it exposes Plan 9-style services such as:

- `#task`
- `#term`
- `#vm`
- `#ramfs`
- `#pipe`
- `#signal`
- `#web`
- `#wanix`

The root task binds `#wanix/version` and `#task` into the namespace. Tasks have
a namespace, kind, command, environment, cwd, exit state, parent link, fd table,
driver, and worker reference. The task service exposes file-controlled state
and control files: `ctl`, `id`, `kind`, `cmd`, `alias`, `env`, `dir`, `exit`,
`binds`, and `ns`. Commands like `ctl bind`, `ctl unbind`, and `ctl start`
make the process model file-driven in the Plan 9 style.

The namespace implementation is also directly Plan 9-inspired. Bindings can be
added, removed, cloned, ordered, and resolved through subpaths. Union directory
views synthesize the combined result. This is one of the major pieces that
carries forward into Rust: the idea that process environment is not just env
vars and cwd, but a namespace assembled from file services.

The Go filesystem layer is large and practical. It wraps Go's filesystem
interfaces with Wanix operations such as create, mkdir, remove, rename,
chmod/chown, chtimes, truncate, symlink/readlink, xattrs, watches, open files,
and helper adapters. It includes or integrates many backing stores and service
filesystems: memory filesystems, host local filesystems, tar, copy-on-write,
HTTP, R2, pipes, signals, 9P adapters, browser File System Access/OPFS,
IndexedDB, DOM, caches, downloads, and JavaScript-object-backed filesystems.

That breadth matters. The Go version is not just a runtime. It is a working
web environment.

## Browser-Native Means Browser-Close

The Go implementation's center of gravity is the browser.

Go compiles to WebAssembly. The runtime talks to the page through `syscall/js`.
Browser capabilities are close to the core: DOM, service workers, IndexedDB,
File System Access, caches, downloads, xterm.js, Web Workers, and web UI
components are all part of how the system comes alive.

The browser bootstrap creates services like `#term`, `#web`, `#vm`, `#pipe`,
`#signal`, `#ramfs`, and `#js`. It installs JavaScript methods on the
`<wanix-system>` element for opening API ports, opening 9P ports, updating
terminals, and setting up namespace bindings declared in HTML.

That web component surface is one of the Go implementation's strongest
user-facing achievements:

- `<wanix-system>` loads the Wanix wasm runtime and exposes handles and 9P
  import.
- `<wanix-bind>` binds namespace paths, fetched files, archive contents, or
  imported 9P namespaces.
- `<wanix-task>` allocates tasks, sets command/env/cwd/alias, optionally
  allocates a terminal, binds terminal I/O to fd 0/1/2, and starts the task.
- `<wanix-term>` renders xterm.js against `#term/<id>/data`, with raw or
  line-mode input.
- `<wanix-vm>` starts the v86 VM path as a Wanix task.
- `<wanix-workbench>` integrates VS Code web with a `wanix:/` workspace and a
  Wanix-backed extension bridge.

That is a very different user experience from a native CLI. You can author a
page that declares a system, binds resources into it, starts tasks, attaches a
terminal, boots a VM, or opens a workbench. The browser is the shell, the
display server, the persistence layer, and the runtime host.

That is powerful. It is also the limitation.

If the laptop closes, the runtime stops. If the tab dies, the environment dies.
If you want long-running work, scaling, cloud deployment, multi-user hosting,
or jobs that continue without an active browser, you eventually run into the
fact that the browser is the kernel.

## The Go Version's Capabilities Are Still Broader

It is worth saying this plainly: the Go implementation currently covers more
of Wanix as a browser operating environment than the Rust implementation does.

The Go side has:

- a browser runtime bootstrap;
- a browser-facing CBOR/RPC API;
- JavaScript handles that wrap fds as Web Streams;
- namespace setup through custom elements;
- web-native filesystems and browser resources;
- xterm.js terminal integration;
- v86 integration as a browser task;
- VS Code web/workbench integration;
- 9P export through the existing `p9kit` adapter;
- an Ethernet/vnet route in the native `serve` command;
- demos that compose these pieces in HTML.

The v86 path is especially important. The browser VM integration reads v86
assets and guest files through Wanix, configures a 9P root, bridges virtio
console output, and exposes guest 9P over a message channel. The workbench path
is similarly important because it shows Wanix as an environment you can use,
not just boot.

The Rust port should not pretend those surfaces do not exist. They are the
reason the port has a useful semantic target.

## Capability Snapshot

Here is the short comparative shape before getting into the Rust architecture:

| Area | Go Wanix today | Rust Wanix today |
| --- | --- | --- |
| Runtime foundation | Browser Go/Wasm, `syscall/js`, Web Workers, browser services | Native Rust host, Wasmtime substrate, QuickJS/WASI first task runtime |
| Primary user experience | HTML/custom elements, browser terminal, v86, workbench demos | Native CLI demos, terminal-backed QuickJS, Rust-served workbench/direct-v86 demos, 9P exports, native `serve` |
| Browser role | Runtime host | Client/frontend/deployment target |
| Task model | Broad process-like resources with namespace, fds, driver, worker, service files | Typed task model and `#task` filesystem independent of QuickJS/WASI |
| Filesystems | Large Go `io/fs`-based toolkit plus browser storage/services | Rust filesystem traits, `MemFs`, explicit rooted `LocalFs`, VFS, WASI projection |
| JavaScript runtime | Browser JS plus GoJS/WASI workers | QuickJS running outside Chrome as a Wanix task |
| Terminal | Browser xterm integration over `#term` resources | Native `#term` device contract used by `qjs-term`, `qjs-shell`, and the Rust-served qjs-shell route |
| VM/browser integration | Broad v86 and workbench integration in browser | Rust-served direct-v86 page with embedded assets, 9P discovery, hvc0 bridge, boot-smoke diagnostics, plus native QEMU handoff |
| 9P | `p9kit` adapter over external p9 library, plus Go serve routes | Owned protocol codecs and Wanix-backed 9P server over stdio/TCP/WebSocket, used by browser filesystem and workbench clients |
| Persistence/mobility | Browser/session-oriented | QuickJS VM snapshot/resume with explicit host-resource reattachment |
| Cloud readiness | Possible through browser-facing serve pieces, but browser remains close to runtime | Runtime can run without Chrome; protocols and mounts are explicit, but orchestration/auth remain future work |

## The Rust Port Is Not A File-By-File Translation

The Rust implementation treats the Go version as a semantic oracle, not as a
package tree to copy.

That is a crucial distinction. A file-by-file translation would preserve too
many incidental browser-era decisions. Instead, the Rust port asks which Wanix
contracts should survive the move:

- filesystem behavior;
- namespace bind and resolve semantics;
- task allocation and service files;
- fd handling;
- terminal resources;
- 9P protocol behavior;
- JavaScript task execution;
- observable user-facing demos.

Then it redraws the lower-level architecture around a native runtime.

The north star is: build a Rust-native Wanix core that runs outside Chrome,
with Wasmtime as the execution substrate and QuickJS/WASI as the first serious
task runtime. Browser support remains important, but it becomes a frontend or
deployment option, not the foundation of the runtime.

The first visible demo target is intentionally small:

```sh
wanix-rust qjs main.js
```

JavaScript runs outside Chrome. It reads and writes files through a Wanix
namespace. It prints through Wanix stdio. It exits with an observable status.
It can read `#task/self/id` and prove that task context crossed into QuickJS.

That is the minimum useful proof that the host boundary moved.

## The Crate Graph Encodes The New Boundary

The Rust workspace is split into smaller crates with stricter dependency
direction:

```text
wanix-fs
  -> wanix-vfs
  -> wanix-task

wanix-protocol
wanix-9p -> wanix-fs + wanix-protocol
wanix-term -> wanix-fs
wanix-wasi -> wanix-fs + wanix-vfs
wanix-qjs-engine -> Wasmtime + QuickJS WASM fixture
wanix-qjs -> wanix-task + wanix-wasi + wanix-qjs-engine
wanix-cli -> runtime crates for orchestration
```

The important rule is that lower-level Wanix contracts do not depend upward.
`wanix-task` does not depend on `wanix-wasi` or `wanix-qjs`. Core filesystem
and namespace crates do not know about Wasmtime. 9P wire codecs live in
`wanix-protocol`, while the 9P server adapter lives in `wanix-9p`.

That is different from the Go shape, where broad working capability and
browser integration naturally pulled more concerns together through Go
interfaces, contexts, workers, and browser entrypoints.

The Rust split is not about aesthetic purity. It is about future runtime
mobility. If QuickJS is the first task driver, not the last one, then the task
model cannot belong to QuickJS. If browser support is one client, not the
kernel, then browser services cannot define the core filesystem. If 9P is how
external clients browse a namespace, protocol parsing cannot be buried in a
demo route.

## WASI Is A Wanix Boundary

One of the most important Rust differences is the meaning of WASI.

In many systems, WASI is a way to delegate filesystem and fd behavior to the
host. Rust Wanix deliberately goes the other direction. WASI becomes an adapter
into Wanix-owned semantics.

When QuickJS opens a file, reads a directory, writes stdout, renames a path,
creates a symlink, polls readiness, truncates a file, changes timestamps, or
uses fd flags, that operation flows through Wanix namespace and fd policy.
The guest sees familiar QuickJS APIs such as `qjs:std`, `qjs:os`,
`scriptArgs`, stdin, stdout, stderr, and environment variables. But the
authority boundary is Wanix.

That is why host directories are exposed through explicit mounts:

```sh
wanix-rust qjs --mount /host/project=workspace main.js
```

The guest sees a Wanix namespace path. The host path is not ambient authority.
That distinction matters for local demos, and it matters even more for cloud
execution.

In a cloud setting, you want to be able to say: this task gets this namespace,
these mounts, these fds, these capabilities, this terminal, this protocol
export. You do not want "the host filesystem" to leak in because that happened
to be convenient for a demo.

## QuickJS Is The Engine, Not The Process Model

Rust Wanix treats QuickJS as an engine inside a Wanix task. It does not let
QuickJS define the process model.

Wanix owns:

- task identity;
- task table allocation;
- cwd;
- env;
- argv/cmd;
- stdio;
- fd table;
- namespace;
- exit status;
- `#task` service files.

`wanix-qjs-engine` owns the mechanics of hosting the QuickJS WebAssembly
reactor under Wasmtime. `wanix-qjs` adapts that engine into Wanix task
semantics. `wanix-wasi` adapts QuickJS/WASI calls into Wanix filesystem and fd
operations.

That separation is what makes the runtime portable. QuickJS is the first
serious task runtime because JavaScript is a good first demo and a useful
interactive language. But the architecture is not "a QuickJS app with a file
API." It is Wanix with a QuickJS task driver.

## Snapshots Point Toward Mobility, But They Are Not Full Checkpoints

The Rust QuickJS engine supports snapshot and restore. That is one of the
places where the cloud direction becomes concrete.

The CLI exposes:

```sh
wanix-rust qjs-snapshot --snapshot state.bin main.js
wanix-rust qjs-resume --snapshot state.bin resume.js
wanix-rust qjs-restore before.js after.js
```

The snapshot is a QuickJS WebAssembly VM image. It captures VM memory and the
engine state needed to restore JavaScript execution. It does not capture the
entire Wanix process.

That boundary is explicit. A snapshot does not include:

- task identity;
- namespace bindings;
- cwd;
- argv;
- env;
- fd table entries;
- stdio attachments;
- host mounts;
- terminal resources.

Those are Wanix host resources, and they must be reattached when the VM image
is restored.

That sounds like a limitation, and it is. But it is also the right limitation
to name early. If this is going to run in the cloud, the system needs a clear
line between portable VM state and deployment-specific host resources. A
snapshot that accidentally smuggles in local file descriptors or browser
objects would be less useful, not more.

The current snapshot work is not "pause a whole operating system and resume it
somewhere else." It is closer to: preserve JavaScript VM state, then reattach
it to a fresh Wanix task environment. That is a realistic foundation for
mobility without pretending the harder process-checkpointing questions are
solved.

## Terminal I/O Becomes A Device Contract

The Go implementation already has a `#term` shape: reading `#term/new`
allocates a terminal resource, and each terminal exposes `id`, `data`,
`program`, and `winch`. It uses pipe-backed files and signal broadcasting to
connect programs and terminal UI.

Rust keeps that shape and makes it a native filesystem-backed device contract.

The Rust `wanix-term` crate exposes:

- `#term/new`;
- `#term/<id>/id`;
- `#term/<id>/data`;
- `#term/<id>/program`;
- `#term/<id>/winch`.

The CLI builds on that with:

```sh
wanix-rust qjs-term main.js
wanix-rust qjs-shell
wanix-rust qjs-shell --raw
```

`qjs-term` runs JavaScript as a Wanix `qjs` task with fd 0/1/2 bound through a
terminal. It can feed input after eval, feed line-by-line scripts, pump
ready-IO handlers, and send deterministic resize events. `qjs-shell` runs the
bundled QuickJS shell source through the same terminal-backed task runtime.

This is not yet a fully concurrent, production interactive scheduler. Raw mode
still relies on the native host to put stdin into raw mode, but bytes then flow
through `#term/<id>/data` and the guest shell owns echo, simple editing, Ctrl-D,
and command dispatch. Signal handling, cancellation, and richer lifecycle
control need more work.

But the direction is important: terminal behavior is not browser xterm
plumbing. It is a Wanix device surface. A browser xterm, a local terminal, a
cloud control plane, or a future collaborative UI should all be able to talk to
the same kind of resource.

That is exactly the "these technologies can come down and run interactively
with you" part. The runtime can live elsewhere, while the terminal experience
can attach locally.

## 9P Moves From Adapter To Owned Protocol Surface

Both implementations care about 9P because 9P is a natural way to expose a
Wanix namespace to external clients.

The Go implementation uses `p9kit` to adapt Wanix filesystems to an external
9P server library. That gives it a broad, working bridge for browser/v86
experiments. The native Go `serve` command exposes WebSocket 9P and an
Ethernet/vnet route.

The Rust implementation owns more of the protocol stack internally:

- `wanix-protocol` owns dependency-free 9P frame splitting, tag extraction,
  version negotiation, and typed 9P2000.L operation codecs.
- `wanix-9p` maps typed 9P frames onto Wanix filesystems.
- `wanix-cli` exposes those adapters over stdio, TCP, WebSocket, and HTTP
  `serve`.

The Rust 9P server already handles a meaningful set of operations: version,
attach, walk, open, create, read, write, readdir, clunk, getattr, setattr,
statfs, symlink, readlink, mkdir, rename, unlink, legacy rename/remove, fsync,
flush, hard-link creation when the backing filesystem supports shared-inode
semantics, append-open semantics, virtual uid/gid metadata, and compatibility
probes for auth, mknod, and xattrs.

Some operations are intentionally unsupported until Wanix grows backing
contracts. `Tauth` returns `ENOSYS`. Special files and xattrs are recognized as
compatibility probes rather than quietly faked as complete features. Hard links
are exposed through the filesystem contract and may still return unsupported
errors for virtual filesystems that do not model shared inodes.

That explicitness matters if Wanix is going to serve real clients. A cloud
runtime needs protocol contracts you can test and publish, not just a page that
happens to boot.

## The Rust CLI Surface Shows The New Direction

The native Rust CLI is already broader than the native Go CLI, even though the
Go browser runtime is still broader overall.

The Rust command surface includes:

```text
wanix-rust qjs
wanix-rust qjs-term
wanix-rust qjs-shell
wanix-rust qjs-snapshot
wanix-rust qjs-resume
wanix-rust qjs-restore
wanix-rust p9-stdio
wanix-rust p9-listen
wanix-rust p9-ws
wanix-rust qemu
wanix-rust serve
```

That set of commands says a lot about the intended shape:

- run JavaScript natively;
- run JavaScript with terminal-backed fds;
- run an interactive shell path;
- persist and resume VM state;
- expose a namespace over process stdio;
- expose a namespace over TCP;
- expose a namespace over WebSocket;
- print a native QEMU/KVM virtio-9p handoff command with explicit host 9P
  mount tag and security-model knobs for the same guest-root shape as
  direct-v86;
- serve browser assets, protocol routes, optional Wanix services, workbench,
  and direct-v86 entrypoints from one native listener.

This is not yet a cloud platform. But these are the pieces a cloud platform
needs more than it needs another browser-only demo.

## `serve` Is Where Browser And Native Meet Again

The Rust `serve` command is the clearest artifact of the new boundary.

It serves static files with browser isolation and CORS headers, and it exposes
the same Wanix filesystem root over a direct binary 9P WebSocket route:

```text
/.well-known/export9p
```

It also serves a discovery document:

```text
/.well-known/wanix.json
```

That document advertises the runtime, the direct 9P route, protocol details,
the optional bundle hint, the reserved Ethernet route, and v86 boot hints. The
Ethernet route is explicitly reported as not implemented. That is not as flashy
as pretending everything works, but it is a better contract.

With `--wanix-services`, `serve` exports more than a host directory. It builds a
Wanix namespace that binds the served root at `/` and exposes `#task` and
`#term` through the same direct 9P route. The service table advertises `noop`
and `qjs`, so a browser or editor client can allocate a `qjs` task through
`#task/new/qjs`, set `cmd`, `env`, `dir`, bind fds, start it, and observe exit
state through service files.

The Rust-served workbench path now uses that contract. The `workbench-fs9p`
bundle launches VS Code web against a `wanix:/` workspace, lets the extension
discover the Rust 9P route, populates Explorer through the direct 9P backend,
and can run the current JavaScript file as a real Wanix `qjs` task by driving
`#task` and `#term` over 9P. The served qjs-shell WebSocket route gives the
workbench a terminal/session path backed by the same native task and terminal
model.

With `--bundle direct-v86`, Rust `serve` generates a browser page that fetches
the discovery document and configures v86 `filesystem.proxy_url` to the Rust 9P
WebSocket route. The discovery data includes boot defaults such as the Linux
cmdline needed to mount the 9P export as `root=host9p`, memory settings, VGA
memory, and the virtio-console requirement. The direct-v86 page now also has a
repeatable smoke shape: it can autostart from `?autostart=1`, exposes a visible
boot log and `window.wanixV86BootLog`, reports v86 lifecycle status, sends
browser console resize events to hvc0, and reports boot readiness from the
served root by checking for a kernel and `/bin/init`.

That means the browser is still very much in the story. The difference is that
the browser page discovers and attaches to a native Wanix export. It is a
client of the runtime, not the runtime's foundation.

This is the bridge between the two worlds:

- run the core somewhere durable;
- expose namespaces through explicit protocols;
- attach browser UI, terminal UI, v86, or editor UI as clients;
- keep the web-native experience without requiring the web page to be the
  kernel.

## v86, QEMU, And Wasmtime Are Different Layers

The direct-v86 path should not be read as "boot Linux inside Wasmtime."

Wasmtime is the substrate Rust Wanix uses for WebAssembly task engines such as
QuickJS/WASI. A `qjs` task runs outside Chrome under a Wasmtime-hosted QuickJS
engine, with Wanix owning the WASI, fd, namespace, and task semantics.

v86 is different. v86 is an x86 emulator whose browser distribution is a
JavaScript/Wasm emulator runtime. In the current Rust path, Rust `serve` hosts
the page, the v86 assets, discovery, and the 9P root. The browser runs the v86
emulator, then mounts the Rust-served 9P export as the guest root filesystem.
That still moves the Wanix host boundary out of Chrome: Chrome is not the Wanix
kernel or filesystem host. But the x86 emulator itself is still running in the
browser.

Could v86 itself run under Wasmtime? Maybe as a future porting project, but not
by simply pointing Wasmtime at the existing v86 browser Wasm. The v86 browser
bundle is not a WASI program. It is driven by JavaScript glue and browser-like
host services: module loading, timers, typed-array memory views, asset fetches,
screen, keyboard, serial, network, and filesystem adapters. Running that
outside the browser would require a non-browser v86 host adapter or a different
JS/runtime embedding strategy, and it would still be an x86 emulator hosted by
Wanix, not a normal Wanix WASI task.

Native QEMU is the other path. `wanix-rust qemu` targets real host execution of
QEMU/KVM or software emulation with the same guest-root shape: a Linux kernel,
an hvc0 virtconsole, and a virtio-9p root mounted from a Wanix/host directory.
It can tune the generated guest mount tag and QEMU local 9P `security_model`
without changing the default `host9p`/`mapped-xattr` handoff; a fully replaced
kernel cmdline remains caller-owned. That path is closer to "use the hardware
when available." Today it is a validated handoff command, not a supervised
Wanix VM process, but it is the right native parity path for serious Linux VM
work.

So the current split is:

- Wasmtime runs Wanix task engines such as QuickJS/WASI.
- Browser v86 is a client/demo that boots an x86 guest against Rust Wanix over
  9P.
- Native QEMU is the likely hardware-accelerated VM path.
- A future "v86 outside the browser" path would be its own emulator-hosting
  project, not the default meaning of Wasmtime in Rust Wanix.

## What This Means For Cloud Execution

The cloud argument is not "Rust is better than Go because cloud." The argument
is more specific.

If Wanix only runs while a browser tab is alive, then it is limited to
interactive sessions. That is still useful. It is great for demos, education,
portable workspaces, client-side apps, and local-first experiments. But it
cannot naturally handle:

- work that continues after the laptop closes;
- scheduled jobs;
- scale-out workers;
- scale-down idle environments;
- server-side collaboration;
- remote namespace exports;
- durable terminals;
- cloud-hosted VM/browser handoffs;
- long-running agents or scripts;
- policy-controlled host mounts;
- externally reachable protocol endpoints.

Moving the runtime core outside Chrome changes that possibility space.

In the Rust model, a cloud Wanix instance could run the native core, attach
explicit storage mounts, run QuickJS/WASI tasks, expose a namespace over 9P,
persist selected VM state, and let clients attach through terminal, browser,
editor, or v86 routes. The browser becomes one way to interact with a running
environment, not the thing that keeps the environment alive.

This also supports the opposite direction: the same runtime ideas can come back
down to the user's machine. A local terminal can attach to a Wanix terminal.
A browser can boot v86 against a direct 9P route. A VS Code-style frontend can
discover a namespace and edit it. A local script can run through `qjs` with an
explicit host mount.

That is the interesting loop: cloud when you need durability and scale,
local/browser interaction when you need immediacy.

## What Rust Does Not Have Yet

The Rust implementation is still a vertical slice.

It does not yet have the full browser service surface from Go. It does not yet
have the whole web component authoring model. `qjs-shell` has a native raw path
and a served terminal/session route, but it is not a complete production
terminal scheduler with signals, cancellation, and rich lifecycle control.
Direct-v86 is becoming a reproducible browser boot smoke, but it is not a full
assembled VM distribution, not a vnet bridge, and not a native VM supervisor.
The Rust-served workbench path can browse, edit, search, open a qjs shell route,
forward terminal resizes, close terminals from observed task exit state, and run
a qjs task, but it is still an integration demo rather than the full Go browser
environment. Public auth, writable export policy, HTTPS, Ethernet/vnet, QEMU
supervision, multi-tenant isolation, and cloud orchestration remain open design
work.

Snapshots are not whole-process checkpoints. 9P has intentional unsupported
areas. Browser clients can discover the current Rust serve contract, but that
contract is still early.

Those limitations should be visible in the post because they are part of the
architecture. Rust Wanix is not claiming to be done. It is claiming that the
runtime boundary has moved far enough to start building in the cloud direction.

## What Carries Forward

The most important thing that carries forward from Go to Rust is the Wanix
model itself:

- everything useful should be file-like;
- namespaces are per task;
- service resources are mounted into those namespaces;
- task control is visible through files;
- protocols should expose namespaces instead of bypassing them;
- terminal and VM integration should be devices/resources, not special cases;
- browser UI matters because it is one of the best ways to interact with the
  system.

The Go implementation found that shape by making it real in the browser. The
Rust implementation is trying to make that shape stand outside the browser.

That is why the Go version remains the migration oracle. It contains the lived
behavior. It answers questions like: what should `#term` look like, how should
tasks expose control, what browser workflows matter, what does v86 need, what
does the workbench want, what does a namespace feel like when it is actually
used?

Rust answers a different set of questions: what are the minimal core contracts,
which crate owns which boundary, how does WASI flow through Wanix instead of
around it, how do we expose 9P without tying it to one page, how do we run a
task when Chrome is not present, and how do we keep host resources explicit
when VM state moves?

## The Short Version

Go Wanix proved that a Plan 9-inspired operating environment could live in the
browser.

Rust Wanix is rebuilding that environment so the browser is no longer the
runtime foundation.

The Go implementation is broader today as a web operating system. The Rust
implementation is narrower today, but it is laying down the native runtime,
WASI, terminal, snapshot, 9P, and serve contracts that make cloud execution
plausible.

The goal is not to leave the browser behind. The goal is to let Wanix keep
running when the browser is gone, and then let the browser, terminal, editor,
or VM attach when you come back.

That is the real architectural difference: not Go versus Rust as languages,
but browser-as-host versus Wanix-as-host.
