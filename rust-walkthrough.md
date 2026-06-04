# Rust Wanix Port Walkthrough

This walkthrough is for trying the Rust-native Wanix port from the workspace
root. It is written from a user perspective: run the commands, look at what
happens, then read the short developer asides to see which architecture piece
you just exercised.

The north star is a Wanix core that runs outside Chrome. Wasmtime hosts the
execution substrate, QuickJS/WASI is the first serious task runtime, and Wanix
owns process identity, namespaces, fd semantics, cwd/env/cmd, stdio, and exit
status.

## 0. Start From The Workspace Root

```sh
cd /Users/jesse/lw/wanix
git status --short
```

You may see local untracked notes or work in progress. The commands below use
the Rust workspace and the checked-in examples.

Build the native demo CLI once:

```sh
cargo build --locked --package wanix-cli
WANIX=./target/debug/wanix-rust
```

Check the CLI surface:

```sh
$WANIX --help
```

You should see commands for:

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
wanix-rust rootfs
wanix-rust qemu
wanix-rust serve
```

> Developer aside: `wanix-cli` is intentionally only composition and demo
> plumbing. The binary drives `wanix-task`, `wanix-vfs`, `wanix-wasi`, and
> `wanix-qjs`; it is not where core task or filesystem semantics live.

## 1. Run JavaScript Outside Chrome

Run the first native QuickJS task demo:

```sh
$WANIX qjs examples/qjs-demo.js
```

Expected output:

```text
outside Chrome: true
task id: 1
Wanix ES module loader
hello from a Wanix namespace
```

What happened:

- `examples/qjs-demo.js` ran in QuickJS, hosted by Wasmtime.
- `std.loadFile("main.js")` read from the Wanix namespace, not the host path
  directly.
- `std.writeFile("hello.txt", ...)` wrote inside the task namespace.
- `std.loadFile("#task/self/id")` reached the task service filesystem.
- `import { runtime } from "./qjs-demo-lib.js"` loaded an ES module from the
  Wanix namespace.

> Developer aside: this is the first vertical slice from `AGENTS.md`: JavaScript
> runs outside Chrome with access to a Wanix namespace. QuickJS is the engine
> inside a Wanix `qjs` task; it does not own process identity. That split lives
> across `wanix-qjs-engine` for Wasmtime/QuickJS mechanics, `wanix-qjs` for the
> task driver, and `wanix-wasi` for Wanix-owned Preview 1 syscall semantics.

## 2. Pass Process Context Into The Task

Create a tiny script outside the repo:

```sh
mkdir -p /tmp/wanix-walkthrough
cat > /tmp/wanix-walkthrough/context.js <<'JS'
import * as std from "qjs:std";
import * as os from "qjs:os";

const bytes = new Uint8Array(64);
const n = os.read(0, bytes.buffer, 0, bytes.length);
const stdin = Array.from(bytes.slice(0, n))
  .map((byte) => String.fromCharCode(byte))
  .join("");

std.out.puts("argv: " + scriptArgs.join("|") + "\n");
std.out.puts("mode: " + std.getenv("MODE") + "\n");
std.out.puts("cwd source: " + std.loadFile("main.js").includes("qjs:std") + "\n");
std.out.puts("task: " + std.loadFile("#task/self/id").trim() + "\n");
std.out.puts("stdin: " + stdin + "\n");
std.out.flush();
JS
```

Run it with cwd, env, stdin, and script arguments:

```sh
$WANIX qjs \
  --env MODE=walkthrough \
  --cwd app \
  --stdin "hello fd0" \
  /tmp/wanix-walkthrough/context.js \
  -- alpha "two words"
```

Expected output:

```text
argv: main.js|alpha|two words
mode: walkthrough
cwd source: true
task: 1
stdin: hello fd0
```

> Developer aside: `scriptArgs`, `std.getenv`, `#task/self/id`, and fd `0`
> are all Wanix task state flowing into QuickJS through live WASI and service
> files. The old `globalThis.Wanix` bridge is gone; guest code should use
> `qjs:std`, `qjs:os`, `scriptArgs`, and `#task`.

## 3. Mount A Host Directory Explicitly

Create a host directory that Wanix can see through a named mount:

```sh
mkdir -p /tmp/wanix-host
printf 'native mount' > /tmp/wanix-host/input.txt
```

Run the host-mount demo:

```sh
$WANIX qjs --mount /tmp/wanix-host=host examples/qjs-host-mount.js
```

Expected output:

```text
host input: native mount
host output: mounted output for native mount
```

Now inspect the host-visible file the task wrote:

```sh
cat /tmp/wanix-host/output.txt
```

Expected output:

```text
mounted output for native mount
```

> Developer aside: host files are not ambient authority. `wanix-cli` creates a
> rooted `LocalFs` and binds it into the task namespace at `host`. The task sees
> a Wanix namespace path, while `wanix-fs::LocalFs` protects the host root
> boundary.

## 4. Use The QuickJS fd API

Run the fd demo:

```sh
$WANIX qjs examples/qjs-fd-demo.js
```

Expected output:

```text
read fd: 4
saw fd API: true
write fd: 5
bytes: 21
hello from a Wanix fd
```

What happened:

- `qjs:os.open("main.js", os.O_RDONLY)` opened a Wanix namespace file.
- `qjs:os.read(...)` read through a WASI fd.
- `qjs:os.open(..., os.O_CREAT | os.O_TRUNC, ...)` created a namespace file.
- `qjs:os.write(...)` wrote through the fd.
- `qjs:std.loadFile(...)` read the created file back through the namespace.

> Developer aside: dynamic regular-file WASI fds are mirrored into the Wanix
> task fd table at the same number. This is why task service files such as
> `#task/self/fd/<n>` can see selected dynamic fds while Wanix still owns global
> fd semantics.

## 5. Create And Control A Child Task

Run the task-spawn demo and capture stderr separately:

```sh
$WANIX qjs examples/qjs-task-spawn.js 2>/tmp/wanix-spawn.err
cat /tmp/wanix-spawn.err
```

Expected stdout from the first command:

```text
parent task: 1
child task: 2
child task 2 args qjs-task-spawn-child.js|alpha|two words||beta mode spawned stdin stdin from parent
child exit: 5
```

Expected stderr from `cat /tmp/wanix-spawn.err`:

```text
child stderr mode spawned
```

What happened:

- The parent read `#task/new/qjs` to allocate a child task.
- The parent wrote the child's `cmd`, `env`, and `dir` service files.
- The parent wired child fds with `#task/<id>/ctl` bind commands.
- The parent started the child and read the child's `exit` file.

> Developer aside: this is Wanix process semantics, not a QuickJS process model.
> `wanix-task` owns task ids, `#task`, fd tables, and driver dispatch.
> `wanix-qjs` registers the `qjs` driver so a task can choose QuickJS as its
> execution engine.

## 6. Exercise More Wanix-Owned WASI Filesystem Calls

These examples are small smoke tests for specific Preview 1 calls backed by the
Wanix namespace:

```sh
$WANIX qjs examples/qjs-append-demo.js
$WANIX qjs examples/qjs-utimes-demo.js
$WANIX qjs examples/qjs-mkdir-demo.js
$WANIX qjs examples/qjs-readdir-demo.js
$WANIX qjs examples/qjs-rename-demo.js
$WANIX qjs examples/qjs-unlink-demo.js
$WANIX qjs examples/qjs-rmdir-demo.js
```

You should see outputs like:

```text
os bytes: 3
log: start-os-std-fdopen

utimes: 0
atime: 1000
mtime: 2000
message: timestamped

mkdir: 0
message: hello from a created directory

listing: a.txt,b.txt,nested
nested:
```

> Developer aside: these demos prove the current live-WASI surface. Wanix owns
> path open, read/write, append flags, fd flag mutation, timestamp mutation,
> create/remove directory, unlink, rename, and readdir semantics. The engine's
> read-only virtual files remain engine fixture support only; Wanix runtime
> paths should go through live providers.

## 7. Try Stdin Sources

Pass stdin as a CLI string:

```sh
$WANIX qjs --stdin "from inline stdin" examples/qjs-stdin-demo.js
```

Expected output:

```text
stdin: from inline stdin
task id: 1
```

Pass stdin from a host file:

```sh
printf 'from stdin file\n' > /tmp/wanix-stdin.txt
$WANIX qjs --stdin-file /tmp/wanix-stdin.txt examples/qjs-stdin-demo.js
```

Pass stdin from a native pipe:

```sh
printf 'from host pipe\n' | $WANIX qjs --stdin-file - examples/qjs-stdin-demo.js
```

> Developer aside: all three forms end up as Wanix task fd `0`. The guest uses
> `qjs:os.read(0, ...)`, so this is another process/fd proof rather than a
> JavaScript-specific helper.

## 8. Try Terminal-Backed JavaScript

Run JavaScript with fd 0/1/2 connected through a Wanix terminal device:

```sh
$WANIX qjs-term --stdin "from native stdin" examples/qjs-term-demo.js
```

Expected output:

```text
terminal task: 1
terminal id: 1
terminal input: from native stdin
terminal stderr: same screen
```

Run the bundled QuickJS shell through the same terminal-backed task path:

```sh
printf 'write note.txt hello shell\nls\ncat note.txt\nid\npwd\nexit\n' | $WANIX qjs-shell
```

Expected output:

```text
shell task: 1
$ wrote note.txt
$ note.txt
$ hello shell
$ 1
$ .
$ bye
```

> Developer aside: `qjs-term` and `qjs-shell` prove that terminal behavior is a
> Wanix device contract, not browser xterm glue. `qjs-shell --raw` puts native
> stdin in raw mode on Unix hosts, then feeds bytes through `#term/<id>/data` so
> the guest shell owns echo, simple editing, Ctrl-D, and command dispatch.
> The bundled shell has a small filesystem command set (`cd`, `ls`, `cat`,
> `write`, `mkdir`, `rm`, `rmdir`, `mv`, and `cp`) for Wanix namespace demos,
> plus a synchronous `qjs SCRIPT [ARGS...] [< STDIN] [> STDOUT] [2> STDERR]`
> launcher that allocates a child `qjs` task through `#task/new/qjs`, wires
> terminal stdio or namespace files to fd `0`/`1`/`2`, and waits on the child's
> `exit` file.
> Served/workbench shell sessions keep WASI rooted at the served namespace root
> while `#task/self/dir` tracks the logical shell cwd, so `cd` can navigate the
> served tree without changing normal qjs script cwd semantics.
> Clients that own a terminal resource can release it by writing `close` to
> `#term/<id>/ctl`; that cleans up the terminal resource without pretending to
> be task cancellation.

## 9. Inspect Protocol, Editor, And VM Entrypoints

The Rust port also exposes the same namespace shape to external clients. These
commands are long-running or need protocol/VM clients, so treat them as the
entrypoint map rather than a linear copy/paste script:

```sh
$WANIX p9-stdio --root /tmp/wanix-host
$WANIX p9-listen --root /tmp/wanix-host --addr 127.0.0.1:5640
$WANIX p9-ws --root /tmp/wanix-host --addr 127.0.0.1:7654

$WANIX serve --root /tmp/wanix-host --listen 127.0.0.1:7654 --bundle fs9p --wanix-services
$WANIX serve --root /tmp/wanix-host --listen 127.0.0.1:7654 --bundle workbench-fs9p --wanix-services
$WANIX serve --root /tmp/wanix-host --listen 127.0.0.1:7654 --bundle direct-v86
```

When you have a Linux guest-root archive:

```sh
$WANIX rootfs --archive extras/dist/alpine-linux.tgz --out /tmp/wanix-rootfs
$WANIX qemu --root /tmp/wanix-rootfs --exec
```

For scripts, editors, or browser launchers that want one prepared-root handoff,
ask `rootfs` for JSON:

```sh
$WANIX rootfs --archive extras/dist/alpine-linux.tgz --out /tmp/wanix-rootfs-json --json
```

When `serve` points at a prepared root, loopback clients can discover the same
trusted-local handoff through HTTP:

```sh
$WANIX serve --root /tmp/wanix-rootfs --listen 127.0.0.1:7654 --bundle direct-v86
curl http://127.0.0.1:7654/.well-known/wanix.json
curl http://127.0.0.1:7654/.well-known/rootfs.json
```

The generated direct-v86 page also reads that discovery route. When the handoff
is available, it renders a rootfs summary and exposes the full manifest as
`window.wanixRootfsHandoff` for local browser tooling. It also renders copyable
QEMU and direct-v86 serve commands from the trusted local manifest.

For host-specific 9P policy, tune the generated QEMU command before launching:

```sh
$WANIX qemu --root /tmp/wanix-rootfs --security-model none --mount-tag wanixroot
```

If the prepared root includes `/boot/initrd`, QEMU handoffs include `-initrd`
automatically; pass `--initrd PATH` to use a different initrd.

For scripts or editor integration, ask for the same validated handoff as JSON:

```sh
$WANIX qemu --root /tmp/wanix-rootfs --json
```

`--mount-tag` also updates the generated default kernel cmdline. If you replace
the full cmdline with `--cmdline`, include the matching `root=TAG` yourself.
`--json` is an inspection format; it cannot be combined with `--exec`.

> Developer aside: `p9-*` and `serve` are the path toward Linux/v86/editor
> clients browsing the same Wanix namespace. The workbench bootstrap passes the
> discovered direct-9P route into the extension, the workbench client negotiates
> Google.2 `walkgetattr` when available, and `serve --wanix-services` exports
> `#task` and `#term` over that route so browser/workbench clients can start qjs
> tasks, attach terminals, start served qjs-shell sessions in the requested
> Wanix cwd, forward resizes through `#term/<id>/winch`, close owned terminals
> through `#term/<id>/ctl`, and observe exit state.
> `rootfs` and `qemu` are the native VM handoff path, not a VM manager yet.
> `rootfs --json` emits `wanix-rootfs.v1` with the prepared root, boot markers,
> default QEMU manifest, and direct-v86 serve argv. `/.well-known/rootfs.json`
> publishes that prepared-root handoff from `serve` for loopback clients; the
> route is trusted-local because the manifest includes host paths and launch
> argv. The shell and JSON QEMU outputs share one validated argv so scripts and
> future UI surfaces can consume the handoff without reinterpreting a shell
> string.
> Prepared-root initrds are part of that argv contract when present.
> QEMU mount tags and local 9P security models stay explicit because they
> affect the guest boot contract and host filesystem trust boundary.

## 10. Snapshot And Restore In One Command

Run the restore demo:

```sh
$WANIX qjs-restore examples/qjs-snapshot-before.js examples/qjs-snapshot-after.js
echo "exit=$?"
```

Expected output:

```text
before task: 1
after task: 2
vm state: preserved from task 1
namespace: namespace from task 1
exit=7
```

What happened:

- The `before` script wrote `globalThis.snapshotMessage`.
- Wanix snapshotted the QuickJS/Wasm VM image.
- The `after` script ran in a new task with host resources reattached.
- The VM global persisted, while task identity changed from `1` to `2`.

> Developer aside: snapshots are VM images, not serialized host resources.
> Wanix reattaches namespace, stdio, env/cwd/cmd, and live WASI provider state
> around the restored QuickJS memory image. Open dynamic fds currently block
> snapshots, which keeps the boundary explicit.

## 11. Persist A Snapshot Across CLI Invocations

Create a host directory and snapshot path:

```sh
mkdir -p /tmp/wanix-persist
SNAP=/tmp/wanix-persist/quickjs.snapshot
```

Create the snapshot:

```sh
$WANIX qjs-snapshot \
  --env MODE=before \
  --mount /tmp/wanix-persist=host \
  --snapshot "$SNAP" \
  examples/qjs-persist-before.js
```

Expected output:

```text
snapshot task: 1
```

Resume from the snapshot with different host state:

```sh
$WANIX qjs-resume \
  --env MODE=after \
  --mount /tmp/wanix-persist=host \
  --snapshot "$SNAP" \
  examples/qjs-persist-after.js
echo "exit=$?"
```

Expected output:

```text
resume task: 1
vm: vm from task 1 mode before
reattached mode: after
host: host before task 1
exit=6
```

Inspect the host-visible files:

```sh
cat /tmp/wanix-persist/persist-before.txt
cat /tmp/wanix-persist/persist-after.txt
```

Expected output:

```text
host before task 1
host after task 1
```

> Developer aside: this is the persistent form of the VM-image boundary. The
> snapshot bytes are tied to the exact QuickJS WASM module identity. Host mounts
> and env are supplied again during resume; they are not stored as ambient host
> capabilities inside the snapshot.

## 12. Run The Rust Quality Gate

Before committing an improvement cycle, use the workspace gate:

```sh
cargo fmt --package wanix-9p --package wanix-cli --package wanix-fs --package wanix-protocol --package wanix-qjs --package wanix-qjs-engine --package wanix-task --package wanix-term --package wanix-vfs --package wanix-wasi --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --locked
```

> Developer aside: the crate boundaries are part of the port strategy. Core
> Wanix contracts stay below Wasmtime and QuickJS. `wanix-task` must not depend
> on `wanix-wasi` or `wanix-qjs`; `wanix-qjs-engine` must not learn Wanix task
> identity. When adding behavior, keep the ownership boundary sharp.

## 13. Clean Up Temporary Files

```sh
rm -rf /tmp/wanix-walkthrough \
       /tmp/wanix-host \
       /tmp/wanix-spawn.err \
       /tmp/wanix-stdin.txt \
       /tmp/wanix-persist
```

## What To Notice

- The user-facing runtime is already native: no Chrome is involved.
- JavaScript runs as a Wanix task, not as a separate QuickJS process model.
- Guest code uses ordinary QuickJS modules and WASI-shaped APIs:
  `qjs:std`, `qjs:os`, `scriptArgs`, `std.getenv`, and `#task`.
- Wanix owns the filesystem, namespace, fd, task, stdio, env/cwd/cmd, and exit
  semantics.
- QuickJS/Wasmtime mechanics are isolated in the engine crate so future runtime
  work can evolve Wanix policy without copying browser-era bridge APIs forward.
