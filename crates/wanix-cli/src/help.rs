use std::ffi::OsString;

use super::CliOutput;

/// Copy-pasteable first commands, shown above the full usage list so a new user
/// has something to run before reading the wall.
const QUICK_START: &str = concat!(
    "quick start:\n",
    "  wanix qjs main.js                                              ",
    "# run JavaScript as a Wanix task\n",
    "  wanix serve --root /tmp/wanix-root --bundle workbench-fs9p     ",
    "# the browser cockpit\n",
    "  open the printed URL in a browser                              ",
    "# operate the served namespace\n",
    "  wanix SUBCOMMAND --help                                        ",
    "# usage for one subcommand\n",
    "  wanix mesh-serve --help                                        ",
    "# the Plan 9 mesh: serve + mount a namespace across machines\n",
);

pub(super) const USAGE: &str = concat!(
    "usage: wanix qjs [--env KEY=VALUE ...] [--cwd DIR] ",
    "[--stdin TEXT | --stdin-file PATH|-] [--event-loop-ms N] [--ready-io-turns N] ",
    "[--interrupt-after N] [--memory-limit-bytes N] ",
    "[--mount HOST=GUEST ...] <script.js> [-- arg ...]\n",
    "       wanix qjs-term [--env KEY=VALUE ...] [--cwd DIR] ",
    "[--stdin TEXT | --stdin-file PATH|-] [--event-loop-ms N] [--ready-io-turns N] ",
    "[--feed-after-eval TEXT ...] [--feed-after-eval-file PATH|- ...] ",
    "[--feed-after-eval-lines PATH|- ...] ",
    "[--resize-after-eval COLSxROWS ...] ",
    "[--interrupt-after N] [--memory-limit-bytes N] ",
    "[--mount HOST=GUEST ...] <script.js> [-- arg ...]\n",
    "       wanix qjs-shell [--raw] [--env KEY=VALUE ...] [--cwd DIR] ",
    "[--event-loop-ms N] [--ready-io-turns N] ",
    "[--interrupt-after N] [--memory-limit-bytes N] ",
    "[--mount HOST=GUEST ...] [--mount-mesh (IROH_URL=GUEST | NAME[=GUEST]) ...]\n",
    "       wanix qjs-snapshot [--env KEY=VALUE ...] [--cwd DIR] ",
    "[--stdin TEXT | --stdin-file PATH|-] [--interrupt-after N] ",
    "[--memory-limit-bytes N] [--event-loop-ms N] [--ready-io-turns N] ",
    "[--mount HOST=GUEST ...] ",
    "--snapshot FILE <script.js> [-- arg ...]\n",
    "       wanix qjs-resume [--env KEY=VALUE ...] [--cwd DIR] ",
    "[--stdin TEXT | --stdin-file PATH|-] [--interrupt-after N] ",
    "[--memory-limit-bytes N] [--event-loop-ms N] [--ready-io-turns N] ",
    "[--mount HOST=GUEST ...] ",
    "--snapshot FILE <script.js> [-- arg ...]\n",
    "       wanix qjs-restore [--cwd DIR] [--before-env KEY=VALUE ...] ",
    "[--after-env KEY=VALUE ...] [--before-arg VALUE ...] [--after-arg VALUE ...] ",
    "[--mount HOST=GUEST ...] <before.js> <after.js>\n",
    "       wanix wasm [--env KEY=VALUE ...] [--cwd DIR] ",
    "[--stdin TEXT | --stdin-file PATH|-] [--mount-mesh (IROH_URL=GUEST | NAME[=GUEST]) ...] ",
    "FILE.wasm [args...] ",
    "(command-style WASI subset, no poll readiness; .wasm is also a first-class Wanix task driver; ",
    "--mount-mesh imports a served namespace at GUEST over the native mesh wire — a bare catalog ",
    "NAME resolves through ~/.wanix/catalog at launch and mounts at n/NAME)\n",
    "       wanix sh [-c LINE] [--env KEY=VALUE ...] [--cwd DIR] ",
    "[--mount-mesh (IROH_URL=GUEST | NAME[=GUEST]) ...]\n",
    "         (the Wanix-native shell as a wasm task: -c runs one line against the --cwd root ",
    "with #pipe, the command bin, and any mesh mounts bound; without -c it is an interactive ",
    "REPL on the host terminal — the guest shell owns echo and line editing, Ctrl-D exits)\n",
    "       wanix p9-stdio --root DIR\n",
    "       wanix mesh-serve (--root DIR | --volume NAME) [--key FILE] [--addr IP:PORT] ",
    "[--peer HEX --grant ANAME:PREFIX:RIGHTS ...] [--wanix-services] [--cpu] [--insecure-open]\n",
    "         (--wanix-services binds the #task/#agent exec devices = remote code execution, and ",
    "--cpu binds the #cpu exec plane; both are loopback-only, so each requires a loopback ",
    "--addr 127.0.0.1:PORT and is refused on any ",
    "non-loopback endpoint — a LAN --addr is mDNS-discoverable. ",
    "--insecure-open exports the host directory read-write, NOT the exec devices, to anyone with the ticket)\n",
    "       wanix cpu --node iroh://PEER[?addr=IP:PORT] [--cwd DIR] [--write] ",
    "[--env KEY=VALUE ...] -- KIND PROGRAM [ARG ...]\n",
    "         (Plan 9 cpu over the mesh: reverse-exports DIR read-only by default and runs ",
    "KIND PROGRAM on the data node against it; --write opts the export into read-write)\n",
    "       wanix mount-ls (tcp://HOST:PORT | iroh://PEER[?addr=IP:PORT] | NAME) [PATH]\n",
    "       wanix mount-cat (tcp://HOST:PORT | iroh://PEER | NAME) PATH [--follow]\n",
    "         (--follow streams incrementally on one open handle with no byte cap until EOF — ",
    "the consumer for never-EOF device streams like #pipe/<id>/data; Ctrl-C is plain process exit)\n",
    "       wanix mount-write (tcp://HOST:PORT | iroh://PEER | NAME) PATH TEXT\n",
    "         (iroh://PEER is the resource identity, found by always-on mDNS on the LAN/same machine; ",
    "?addr=IP:PORT is only a direct-route hint — a stale hint falls back to mDNS/relay discovery, ",
    "and no route can ever mount a peer that fails the identity check; a bare NAME — no scheme, ",
    "no slash — resolves through ~/.wanix/catalog at invocation time)\n",
    "       wanix rootfs --archive FILE.tgz --out DIR [--json]\n",
    "       wanix new (--js NAME | --rust NAME) [--dir DIR]\n",
    "       wanix catalog add NAME IROH_URL [--description TEXT] [--tags a,b] [--force]\n",
    "       wanix catalog (show NAME | rm NAME | ls [--no-probe])\n",
    "         (the local address book under ~/.wanix/catalog — ADR 0007 Layer 1 naming, no ACLs: ",
    "one NAME -> iroh:// ticket per entry, names are lowercase [a-z0-9-] so they can never be ",
    "mistaken for a ticket or path; ls probes each entry with one bounded dial and renders ",
    "online/offline/unknown — offline means nothing answered, which pre-ACL is all it can know)\n",
    "       wanix recipe save NAME --mount (NAME[=PATH] | IROH_URL=PATH) ... ",
    "[--description TEXT] [--run LINE] [--force]\n",
    "         (a saved mount+run composition at ~/.wanix/recipes/<name>.recipe — authored ",
    "explicitly, never captured; NAME targets resolve through the catalog at save time and the ",
    "resolved address is recorded as a drift-check hint; a bare NAME mounts at n/NAME)\n",
    "       wanix recipe run NAME [-- ARG ...]\n",
    "         (resolves the binds through the catalog at launch — drift from the saved hint warns ",
    "loudly, a vanished entry falls back to the hint — composes the mounts, and runs the recipe's ",
    "run line through sh -c with the -- words appended; a recipe without a run line opens an ",
    "interactive sh over its mounts)\n",
    "       wanix volume (create NAME | ls)   (persistent volumes under ~/.wanix/volumes)\n",
    "       wanix volume serve (--volume NAME ... | --all) [--listen IP:PORT] ",
    "[--register NAME] [--insecure-open]",
    "   (one mesh endpoint + ticket per volume; use --listen port 0 for multiple; ",
    "--register NAME writes/updates a catalog entry per announced ticket — NAME itself for one ",
    "resource, NAME-<resource> for several)\n",
    "       wanix tool serve [--config TOOLS.toml] [--tool NAME ...] [--listen IP:PORT] ",
    "[--register NAME] [--insecure-open]\n",
    "         (ToolFS devices over the native mesh — one endpoint + ticket per tool, ",
    "use --listen port 0 for multiple; built-ins: model (deterministic fake), sha256, upper; ",
    "--config tools.toml serves real host programs with a host-fixed command/argv, ",
    "stdin/tempfile mapping, empty env, and a private per-job temp cwd; ",
    "every connection sees only its own jobs, keyed by the verified peer id)\n",
    "       wanix app serve --app DIR --state DIR [--name NAME] [--listen IP:PORT] ",
    "[--restart on-failure] [--register NAME] [--insecure-open]\n",
    "         (a guest-defined AppResource over the native mesh: runs DIR's qjs app — see ",
    "examples/chatroom — as a resident task behind the wanix-appfs adapter, durable state ",
    "mounted at /state; attribution and presence come from the verified peer id — presented ",
    "as iroh:<hex> — never the payload; --restart on-failure re-runs an exited guest with ",
    "capped backoff behind the same ticket)\n",
    "       wanix agent [--fake] [--cwd DIR] [--world DIR] <prompt>\n",
    "       wanix capsule (save DIR | load CAPSULE_ID DIR) [--store DIR]\n",
    "       wanix qemu --root DIR [--kernel PATH] [--initrd PATH] [--cmdline TEXT] [--append TEXT ...] ",
    "[--qemu-bin PATH] [--memory-mb N] ",
    "[--mount-tag TAG] [--security-model MODEL] [--p9-msize N] ",
    "[--json] [--no-kvm] [--exec]\n",
    "       wanix serve [--root DIR | DIR] [--listen HOST:PORT] ",
    "[--p9 HOST:PORT [--peer HEX --grant ANAME:PREFIX:RIGHTS ...]] ",
    "[--bind NAME=DIR | NAME=iroh://PEER | NAME=CATALOG_NAME ...] ",
    "[--bundle NAME] [--wanix-services] [--once]\n",
    "       wanix --help",
);

pub(super) fn help_output() -> CliOutput {
    CliOutput::new(
        format!(
            "wanix: {}\n\n{QUICK_START}\n{USAGE}\n",
            wanix_qjs::FIRST_DEMO_TARGET
        )
        .into_bytes(),
        Vec::new(),
        0,
    )
}

/// True when `args` ask for help: `--help` or `-h` among the leading flags.
///
/// The scan stops at `--` and at the first operand (any argument that does not
/// start with `-`), so a guest program's own arguments are never stolen:
/// `wasm app.wasm -h` hands `-h` to the guest, while `qjs --help` and
/// `serve --help` answer with usage.
pub(super) fn wants_help(args: &[OsString]) -> bool {
    args.iter()
        .map(OsString::as_os_str)
        .take_while(|arg| *arg != "--" && arg.as_encoded_bytes().starts_with(b"-"))
        .any(|arg| arg == "--help" || arg == "-h")
}

/// Builds `--help` output for one subcommand: its usage lines extracted from
/// [`USAGE`]. Returns `None` when the command has no usage entry, so the caller
/// falls through to normal dispatch (and its unknown-command error).
pub(super) fn subcommand_help_output(command: &str) -> Option<CliOutput> {
    let lines = usage_lines_for(command)?;
    let mut text = String::new();
    for (index, line) in lines.iter().enumerate() {
        text.push_str(if index == 0 { "usage: " } else { "       " });
        text.push_str(line);
        text.push('\n');
    }
    text.push_str("(see 'wanix --help' for all commands)\n");
    Some(CliOutput::new(text.into_bytes(), Vec::new(), 0))
}

/// Extracts the [`USAGE`] lines belonging to `command`: every `wanix
/// <command> ...` form plus the indented parenthetical notes that follow one.
fn usage_lines_for(command: &str) -> Option<Vec<&'static str>> {
    if command.starts_with('-') {
        // A flag is never a subcommand (the `wanix --help` usage line
        // would otherwise match itself).
        return None;
    }
    let mut matched = Vec::new();
    let mut last_matched = false;
    for raw in USAGE.lines() {
        let line = raw.trim_start_matches("usage: ").trim_start();
        if let Some(rest) = line.strip_prefix("wanix ") {
            last_matched = rest.split_whitespace().next() == Some(command);
            if last_matched {
                matched.push(line);
            }
        } else if last_matched {
            // An indented continuation note for the matched command line.
            matched.push(line);
        }
    }
    if matched.is_empty() {
        None
    } else {
        Some(matched)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wants_help_sees_only_the_leading_flag_cluster() {
        let args = |values: &[&str]| values.iter().map(OsString::from).collect::<Vec<_>>();
        assert!(wants_help(&args(&["--help"])));
        assert!(wants_help(&args(&["-h"])));
        assert!(wants_help(&args(&["--raw", "--help"])));
        // The scan stops at the first operand and at `--`: a guest program's
        // own `-h`/`--help` is never stolen.
        assert!(!wants_help(&args(&["app.wasm", "-h"])));
        assert!(!wants_help(&args(&["script.js", "--", "--help"])));
        assert!(!wants_help(&args(&["script.js"])));
        assert!(!wants_help(&args(&[])));
    }

    #[test]
    fn subcommand_help_extracts_only_the_named_command() {
        let output = subcommand_help_output("qjs").unwrap();
        let text = String::from_utf8(output.stdout().to_vec()).unwrap();
        assert!(text.starts_with("usage: wanix qjs "), "{text}");
        assert!(!text.contains("wanix qjs-term"), "{text}");
        assert!(text.contains("wanix --help"), "{text}");
    }

    #[test]
    fn subcommand_help_keeps_continuation_notes_with_their_command() {
        let text = String::from_utf8(
            subcommand_help_output("mesh-serve")
                .unwrap()
                .stdout()
                .to_vec(),
        )
        .unwrap();
        assert!(text.contains("wanix mesh-serve"), "{text}");
        assert!(text.contains("--insecure-open"), "{text}");
    }

    #[test]
    fn subcommand_help_collects_every_form_of_a_command() {
        let text =
            String::from_utf8(subcommand_help_output("volume").unwrap().stdout().to_vec()).unwrap();
        assert!(text.contains("volume (create NAME | ls)"), "{text}");
        assert!(text.contains("volume serve"), "{text}");
    }

    #[test]
    fn subcommand_help_is_none_for_unknown_commands() {
        assert!(subcommand_help_output("bogus").is_none());
        // `--help` is a flag line in USAGE, never a command match.
        assert!(subcommand_help_output("--help").is_none());
    }
}
