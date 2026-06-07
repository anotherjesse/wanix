---
title: Three Serve Bundles
slug: concepts/three-serve-bundles
pageType: concept
oneLiner: fs9p (plain browser filesystem), workbench-fs9p (the cockpit / Code OSS shell), and direct-v86 (browser emulator handoff), each a generated HTML page that reads discovery.
audience: [newcomer, developer]
tags: [serve, browser, cockpit, v86, shipped, caveat]
sourceRefs:
  - crates/wanix-cli/src/serve.rs:26-27
  - crates/wanix-cli/src/serve/html.rs:1-49
  - crates/wanix-cli/src/serve/http/routes.rs:32-50
  - crates/wanix-cli/src/serve/direct_v86.rs:10-128
  - crates/wanix-cli/src/serve/discovery.rs:35-90
seeAlso:
  - concepts/discovery-document
  - concepts/browser-cockpit
  - concepts/loopback-only-handoffs
  - concepts/serve-composition-surface
prerequisites:
  - concepts/discovery-document
usedInFlows: []
honestLimits:
  - The cockpit bundle is still named --bundle workbench-fs9p; the --bundle cockpit rename (Slice 8) is queued, not shipped.
  - direct-v86's boot JSON reports readiness markers (kernel/init); it does not itself build a guest rootfs.
---

# Three Serve Bundles

fs9p (plain browser filesystem), workbench-fs9p (the cockpit / Code OSS shell), and direct-v86 (browser emulator handoff), each a generated HTML page that reads discovery.

`wanix-rust serve` exports a Wanix namespace over 9P. A bundle is the optional *browser front end* it ships alongside that export: when you pass `--bundle <name>`, `serve` hands the browser a generated HTML page whose only job is to fetch the discovery document and wire itself up to the routes it finds there. There are exactly three, and they are chosen by a single `match` (`crates/wanix-cli/src/serve/html.rs:13-20`). None of them is a runtime; each is a frontend over the same direct-9P contract.

## Show it: pick a bundle, get a page

```sh
cargo build --package wanix-cli
alias wanix-rust='./target/debug/wanix-rust'

# A plain filesystem browser
wanix-rust serve --bundle fs9p

# The cockpit (Code OSS workbench)
wanix-rust serve --wanix-services --bundle workbench-fs9p

# A browser v86 emulator handoff over a prepared guest root
wanix-rust serve --root ./guest-root --bundle direct-v86
```

Each invocation serves an `index.html` for that bundle and nothing else changes about the underlying 9P export. The three names are constants: `FS9P_BUNDLE = "fs9p"`, `WORKBENCH_FS9P_BUNDLE = "workbench-fs9p"` (`crates/wanix-cli/src/serve.rs:26-27`), and `DIRECT_V86_BUNDLE = "direct-v86"` (`crates/wanix-cli/src/serve/direct_v86.rs:10`). An unknown bundle name returns `None` from `bundle_html` and serves no page (`crates/wanix-cli/src/serve/html.rs:13-20`).

## fs9p: a browser filesystem client over direct 9P

`fs9p` is the smallest front end. The page is static HTML compiled into the binary (`include_str!("fs9p.html")`), with no substitution at all — `fs9p_bundle_html` just returns it verbatim (`crates/wanix-cli/src/serve/html.rs:42-44`). In the browser it does one thing: `fetch("/.well-known/wanix.json")`, read `discovery.routes.p9.websocket`, open a 9P client against that URL, and let you browse, read, and write files. It is the reference proof that a browser tab is just another 9P client — list a directory, cat a file, write one back, all over the same wire the CLI uses.

## workbench-fs9p: the cockpit

`workbench-fs9p` is the operator surface — a Code OSS / VS Code-web workbench whose Wanix extension drives the served namespace and the service devices over direct 9P. Its page is also static (`workbench_fs9p_bundle_html` returns the embedded HTML, `crates/wanix-cli/src/serve/html.rs:46-48`), and it reads the same discovery document, but it consumes far more of it: `routes.p9.websocket` for the filesystem, `routes.qjsShell.websocket` for terminals, `routes.httpApp` for the HTTP-app route, and the whole `services` block — `task`, `term`, `devices`, `drivers` — to decide what the activity-bar view can offer (`crates/wanix-cli/src/serve/workbench_fs9p.html:97-116`).

The detail worth pinning down: the workbench's *static assets* (the vscode-web bundle under `workbench/code`, the compiled extension under `workbench/dist`, media) are served from the repo's `workbench/` tree, **not** from the served user root. The route handler strips a `workbench/` prefix and reads from a canonicalized `../../workbench` directory next to the crate (`crates/wanix-cli/src/serve/http/routes.rs:32-50`). That separation is deliberate: a disposable or empty served root still boots the full cockpit, because the editor shell never lives inside the namespace it edits.

## direct-v86: built-in emulator assets plus a boot JSON

`direct-v86` hands a browser a v86 x86 emulator that boots a Linux guest whose root filesystem is the served 9P export. Unlike the other two, its HTML is *templated*: `direct_v86_bundle_html` substitutes the default kernel command line, kernel URL, and memory sizes into placeholders before serving (`crates/wanix-cli/src/serve/html.rs:22-40`).

The v86 runtime itself (the `libv86.mjs` module, `v86.wasm`, SeaBIOS, the VGA BIOS) is **built into the binary** with `include_bytes!` and served only when this bundle is active, under fixed `/v86/...` routes (`crates/wanix-cli/src/serve/direct_v86.rs:33-83`). The boot story is data: `direct_v86_boot_json` scans the static root for a kernel (`/boot/bzImage` or `/bzImage`), an optional initrd, and an executable `/bin/init`, then emits `{ "kernel": ..., "initrd": ..., "init": ..., "ready": <bool>, "missing": [...] }` (`crates/wanix-cli/src/serve/direct_v86.rs:102-128`). The browser reads this from discovery (`discovery.v86.boot`) and reports whether the guest is bootable before it tries.

## All three read the discovery document

The common spine is the [discovery document](/concepts/discovery-document) at `/.well-known/wanix.json` (`crates/wanix-cli/src/serve/discovery.rs:35-90`). Every bundle fetches it `{ cache: "no-store" }` and configures itself from `routes`, `services`, and `v86`. The bundle never hard-codes a websocket URL or a service set; it asks the server what exists and adapts. That is why the same three HTML pages work whether services are enabled or not, whether a rootfs is prepared or not, and at whatever host:port `serve` bound — the page is a thin renderer over discovery, and `serve` is the [composition surface](/concepts/serve-composition-surface) behind it.

## See also

- [Discovery document](/concepts/discovery-document) — the `/.well-known/wanix.json` contract every bundle reads.
- [Browser cockpit](/concepts/browser-cockpit) — what the `workbench-fs9p` activity-bar view actually drives over 9P.
- [Serve as composition surface](/concepts/serve-composition-surface) — how `serve` assembles the namespace, services, and bundle.
- [Loopback-only handoffs](/concepts/loopback-only-handoffs) — why the rootfs handoff and HTTP-app route are restricted to local clients.
- [Browser cockpit use case](/use-cases/browser-cockpit) — driving the mesh from the workbench.

## Status / honest limits

- **The cockpit bundle is still named `workbench-fs9p`.** The planned rename to `--bundle cockpit` (Slice 8) and retirement of the `workbench/code/` vscode-web vendor dependency are queued follow-ups, not shipped. Use `--bundle workbench-fs9p` today.
- **direct-v86 reports readiness; it does not build a rootfs.** `direct_v86_boot_json` only *probes* the static root for kernel/init markers and sets `"ready": false` with a `"missing"` list when they are absent (`crates/wanix-cli/src/serve/direct_v86.rs:89-128`). Preparing a guest root is a separate step.
- **The HTTP-app route and rootfs handoff that the cockpit and direct-v86 surface are loopback-only and services-gated.** The bundle page renders them when discovery advertises them; discovery only advertises them to trusted local clients. See [loopback-only handoffs](/concepts/loopback-only-handoffs).
- **A bundle is a frontend, not the runtime.** It carries no execution authority of its own; everything it does, it does as a 9P client against the served namespace.
