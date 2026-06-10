use std::ffi::OsString;

use super::CliOutput;

/// Copy-pasteable first commands, shown above the full usage list so a new user
/// has something to run before reading the wall.
const QUICK_START: &str = concat!(
    "quick start:\n",
    "  wanix-rust qjs main.js                                       ",
    "# run JavaScript as a Wanix task\n",
    "  wanix-rust mesh-serve --root /tmp/share --addr 127.0.0.1:0   ",
    "# prints a dialable iroh:// ticket\n",
    "  wanix-rust mount-ls 'iroh://PEER?addr=IP:PORT'               ",
    "# paste the ticket the server printed\n",
    "  wanix-rust SUBCOMMAND --help                                 ",
    "# usage for one subcommand\n",
);

pub(super) const USAGE: &str = concat!(
    "usage: wanix-rust qjs [--env KEY=VALUE ...] [--cwd DIR] ",
    "[--stdin TEXT | --stdin-file PATH|-] [--event-loop-ms N] [--ready-io-turns N] ",
    "[--interrupt-after N] [--memory-limit-bytes N] ",
    "[--mount HOST=GUEST ...] <script.js> [-- arg ...]\n",
    "       wanix-rust qjs-term [--env KEY=VALUE ...] [--cwd DIR] ",
    "[--stdin TEXT | --stdin-file PATH|-] [--event-loop-ms N] [--ready-io-turns N] ",
    "[--feed-after-eval TEXT ...] [--feed-after-eval-file PATH|- ...] ",
    "[--feed-after-eval-lines PATH|- ...] ",
    "[--resize-after-eval COLSxROWS ...] ",
    "[--interrupt-after N] [--memory-limit-bytes N] ",
    "[--mount HOST=GUEST ...] <script.js> [-- arg ...]\n",
    "       wanix-rust qjs-shell [--raw] [--env KEY=VALUE ...] [--cwd DIR] ",
    "[--event-loop-ms N] [--ready-io-turns N] ",
    "[--interrupt-after N] [--memory-limit-bytes N] ",
    "[--mount HOST=GUEST ...] [--mount-mesh IROH_URL=GUEST ...]\n",
    "       wanix-rust qjs-snapshot [--env KEY=VALUE ...] [--cwd DIR] ",
    "[--stdin TEXT | --stdin-file PATH|-] [--interrupt-after N] ",
    "[--memory-limit-bytes N] [--event-loop-ms N] [--ready-io-turns N] ",
    "[--mount HOST=GUEST ...] ",
    "--snapshot FILE <script.js> [-- arg ...]\n",
    "       wanix-rust qjs-resume [--env KEY=VALUE ...] [--cwd DIR] ",
    "[--stdin TEXT | --stdin-file PATH|-] [--interrupt-after N] ",
    "[--memory-limit-bytes N] [--event-loop-ms N] [--ready-io-turns N] ",
    "[--mount HOST=GUEST ...] ",
    "--snapshot FILE <script.js> [-- arg ...]\n",
    "       wanix-rust qjs-restore [--cwd DIR] [--before-env KEY=VALUE ...] ",
    "[--after-env KEY=VALUE ...] [--before-arg VALUE ...] [--after-arg VALUE ...] ",
    "[--mount HOST=GUEST ...] <before.js> <after.js>\n",
    "       wanix-rust wasm [--env KEY=VALUE ...] [--cwd DIR] ",
    "[--stdin TEXT | --stdin-file PATH|-] [--mount-mesh IROH_URL=GUEST ...] FILE.wasm [args...] ",
    "(command-style WASI subset, no poll readiness; .wasm is also a first-class Wanix task driver; ",
    "--mount-mesh imports a served namespace at GUEST over the native mesh wire)\n",
    "       wanix-rust sh [-c LINE] [--env KEY=VALUE ...] [--cwd DIR] ",
    "[--mount-mesh IROH_URL=GUEST ...]\n",
    "         (the Wanix-native shell as a wasm task: -c runs one line against the --cwd root ",
    "with #pipe, the command bin, and any mesh mounts bound; without -c it is an interactive ",
    "REPL on the host terminal — the guest shell owns echo and line editing, Ctrl-D exits)\n",
    "       wanix-rust p9-stdio --root DIR\n",
    "       wanix-rust mesh-serve (--root DIR | --volume NAME) [--key FILE] [--addr IP:PORT] ",
    "[--peer HEX --grant ANAME:PREFIX:RIGHTS ...] [--wanix-services] [--insecure-open]\n",
    "         (--wanix-services binds the #task/#agent exec devices = remote code execution; ",
    "local-trust only, so it requires --addr IP:PORT and is refused on the public endpoint. ",
    "--insecure-open exports the host directory read-write, NOT the exec devices, to anyone with the ticket)\n",
    "       wanix-rust cpu --node iroh://PEER[?addr=IP:PORT] [--cwd DIR] [--write] ",
    "[--env KEY=VALUE ...] -- KIND PROGRAM [ARG ...]\n",
    "         (Plan 9 cpu over the mesh: reverse-exports DIR read-only by default and runs ",
    "KIND PROGRAM on the data node against it; --write opts the export into read-write)\n",
    "       wanix-rust mount-ls (tcp://HOST:PORT | iroh://PEER[?addr=IP:PORT]) [PATH]\n",
    "       wanix-rust mount-cat (tcp://HOST:PORT | iroh://PEER) PATH [--follow]\n",
    "         (--follow streams incrementally on one open handle with no byte cap until EOF — ",
    "the consumer for never-EOF device streams like #pipe/<id>/data; Ctrl-C is plain process exit)\n",
    "       wanix-rust mount-write (tcp://HOST:PORT | iroh://PEER) PATH TEXT\n",
    "         (iroh://PEER is the resource identity, found by always-on mDNS on the LAN/same machine; ",
    "?addr=IP:PORT is only a direct-route hint — a stale hint falls back to mDNS/relay discovery, ",
    "and no route can ever mount a peer that fails the identity check)\n",
    "       wanix-rust rootfs --archive FILE.tgz --out DIR [--json]\n",
    "       wanix-rust new (--js NAME | --rust NAME) [--dir DIR]\n",
    "       wanix-rust volume (create NAME | ls)   (persistent volumes under ~/.wanix/volumes)\n",
    "       wanix-rust volume serve (--volume NAME ... | --all) [--addr IP:PORT] [--insecure-open]",
    "   (one mesh endpoint + ticket per volume; use --addr port 0 for multiple)\n",
    "       wanix-rust tool serve [--config TOOLS.toml] [--tool NAME ...] [--listen IP:PORT] [--insecure-open]\n",
    "         (ToolFS devices over the native mesh — one endpoint + ticket per tool, ",
    "use --listen port 0 for multiple; built-ins: model (deterministic fake), sha256, upper; ",
    "--config tools.toml serves real host programs with a host-fixed command/argv, ",
    "stdin/tempfile mapping, empty env, and a private per-job temp cwd; ",
    "every connection sees only its own jobs, keyed by the verified peer id)\n",
    "       wanix-rust app serve --app DIR --state DIR [--name NAME] [--addr IP:PORT] ",
    "[--restart on-failure] [--insecure-open]\n",
    "         (a guest-defined AppResource over the native mesh: runs DIR's qjs app — see ",
    "examples/chatroom — as a resident task behind the wanix-appfs adapter, durable state ",
    "mounted at /state; attribution and presence come from the verified peer id — presented ",
    "as iroh:<hex> — never the payload; --restart on-failure re-runs an exited guest with ",
    "capped backoff behind the same ticket)\n",
    "       wanix-rust agent [--fake] [--cwd DIR] [--world DIR] <prompt>\n",
    "       wanix-rust capsule (save DIR | load CAPSULE_ID DIR) [--store DIR]\n",
    "       wanix-rust qemu --root DIR [--kernel PATH] [--initrd PATH] [--cmdline TEXT] [--append TEXT ...] ",
    "[--qemu-bin PATH] [--memory-mb N] ",
    "[--mount-tag TAG] [--security-model MODEL] [--p9-msize N] ",
    "[--json] [--no-kvm] [--exec]\n",
    "       wanix-rust serve [--root DIR | DIR] [--listen HOST:PORT] ",
    "[--p9 HOST:PORT [--peer HEX --grant ANAME:PREFIX:RIGHTS ...]] ",
    "[--bundle NAME] [--wanix-services] [--once]\n",
    "       wanix-rust --help",
);

pub(super) fn help_output() -> CliOutput {
    CliOutput::new(
        format!(
            "wanix-rust: {}\n\n{QUICK_START}\n{USAGE}\n",
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
    text.push_str("(see 'wanix-rust --help' for all commands)\n");
    Some(CliOutput::new(text.into_bytes(), Vec::new(), 0))
}

/// Extracts the [`USAGE`] lines belonging to `command`: every `wanix-rust
/// <command> ...` form plus the indented parenthetical notes that follow one.
fn usage_lines_for(command: &str) -> Option<Vec<&'static str>> {
    if command.starts_with('-') {
        // A flag is never a subcommand (the `wanix-rust --help` usage line
        // would otherwise match itself).
        return None;
    }
    let mut matched = Vec::new();
    let mut last_matched = false;
    for raw in USAGE.lines() {
        let line = raw.trim_start_matches("usage: ").trim_start();
        if let Some(rest) = line.strip_prefix("wanix-rust ") {
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
        assert!(text.starts_with("usage: wanix-rust qjs "), "{text}");
        assert!(!text.contains("wanix-rust qjs-term"), "{text}");
        assert!(text.contains("wanix-rust --help"), "{text}");
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
        assert!(text.contains("wanix-rust mesh-serve"), "{text}");
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
