use std::ffi::OsString;
use std::fs;
use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tungstenite::{Message, connect, stream::MaybeTlsStream};
use wanix_protocol::{
    P9_RATTACH, P9_RGETATTR, P9_RLERROR, P9_RLINK, P9_RLOPEN, P9_RREAD, P9_RREADLINK, P9_RREMOVE,
    P9_RRENAME, P9_RSETATTR, P9_RSYMLINK, P9_RVERSION, P9_RWALK, P9_RWALKGETATTR, P9_RWRITE,
    P9_SETATTR_GID, P9_SETATTR_UID, P9_VERSION_9P2000_L, P9_VERSION_9P2000_L_GOOGLE_2, P9Frame,
    P9SetAttr, p9_decode_rgetattr, p9_decode_rlerror, p9_decode_rlink, p9_decode_rread,
    p9_decode_rreadlink, p9_decode_rremove, p9_decode_rrename, p9_decode_rwalk,
    p9_decode_rwalkgetattr, p9_decode_rwrite, p9_tattach, p9_tauth, p9_tgetattr, p9_tlink,
    p9_tlopen, p9_tmknod, p9_tread, p9_treadlink, p9_tremove, p9_trename, p9_tsetattr, p9_tsymlink,
    p9_tversion, p9_twalk, p9_twalkgetattr, p9_twrite, p9_txattrcreate, p9_txattrwalk,
};

use super::direct_v86::{DIRECT_V86_BUNDLE, direct_v86_asset_response};
use super::discovery::{rootfs_handoff_response, serve_discovery_json};
use super::http::HttpStatus;
use super::roots::ServeRoots;
use super::terminal_ws::{parse_terminal_resize_message, qjs_shell_cwd_from_target};
use super::{
    DEFAULT_SERVE_ADDR, FS9P_BUNDLE, ServeCommand, WORKBENCH_FS9P_BUNDLE, parse_serve_command,
    run_serve_streaming, run_serve_with_listener, run_serve_with_listener_for_connections,
};

const EBADF: u32 = 9;
const ENOSYS: u32 = 38;
const P9_O_WRONLY: u32 = 0o1;
const P9_O_RDWR: u32 = 0o2;
const EOPNOTSUPP: u32 = 95;

#[test]
fn parse_serve_uses_go_like_defaults_and_options() {
    let default_command = parse_serve_command(&[]).unwrap();
    assert_eq!(default_command.root_path, PathBuf::from("."));
    assert_eq!(default_command.addr, DEFAULT_SERVE_ADDR);
    assert_eq!(default_command.bundle, None);
    assert!(!default_command.wanix_services);
    assert!(!default_command.once);

    let command = parse_serve_command(&[
        OsString::from("examples"),
        OsString::from("--listen"),
        OsString::from(":7654"),
        OsString::from("--bundle"),
        OsString::from("vm-workbench"),
        OsString::from("--wanix-services"),
        OsString::from("--once"),
    ])
    .unwrap();

    assert_eq!(command.root_path, PathBuf::from("examples"));
    assert_eq!(command.addr, "0.0.0.0:7654");
    assert_eq!(command.bundle, Some("vm-workbench".to_owned()));
    assert!(command.wanix_services);
    assert!(command.once);

    let duplicate_dir =
        parse_serve_command(&[OsString::from("examples"), OsString::from("dist")]).unwrap_err();
    assert!(duplicate_dir.to_string().contains("only one directory"));
}

#[test]
fn parse_serve_reports_missing_option_values() {
    for (option, expected) in [
        ("--root", "serve --root expects DIR"),
        ("--addr", "serve --addr expects HOST:PORT"),
        ("--listen", "serve --listen expects HOST:PORT"),
        ("--bundle", "serve --bundle expects NAME"),
    ] {
        let error = parse_serve_command(&[OsString::from(option)]).unwrap_err();
        assert!(
            error.to_string().contains(expected),
            "{option} produced {error}"
        );
    }
}

#[test]
fn run_serve_streaming_reports_bind_errors() {
    let command = ServeCommand {
        root_path: PathBuf::from("."),
        addr: "127.0.0.1:notaport".to_owned(),
        bundle: None,
        wanix_services: false,
        once: true,
    };
    let mut stderr = Vec::new();

    let error = run_serve_streaming(command, &mut stderr).unwrap_err();

    assert_eq!(error.exit_code(), 1);
    assert!(
        error
            .to_string()
            .contains("failed to bind serve address 127.0.0.1:notaport"),
        "{error}"
    );
    assert!(stderr.is_empty());
}

#[test]
fn serve_once_returns_static_file_with_browser_isolation_headers() {
    let root = temp_dir("wanix-cli-serve-http");
    fs::write(root.join("index.html"), b"wanix serve").unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let command = ServeCommand {
        root_path: root,
        addr: addr.to_string(),
        bundle: None,
        wanix_services: false,
        once: true,
    };

    let handle = thread::spawn(move || {
        let mut stderr = Vec::new();
        let exit_code = run_serve_with_listener(command, listener, &mut stderr).unwrap();
        (exit_code, stderr)
    });

    let mut stream = TcpStream::connect(addr).unwrap();
    stream
        .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n")
        .unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).unwrap();
    let (exit_code, stderr) = handle.join().unwrap();

    assert_eq!(exit_code, 0);
    let stderr = String::from_utf8(stderr).unwrap();
    assert!(
        stderr.contains("wanix-rust serve: listening on http://127.0.0.1:"),
        "{stderr}"
    );
    assert!(stderr.contains("files with Wanix overlay"), "{stderr}");
    let response = String::from_utf8(response).unwrap();
    assert!(response.starts_with("HTTP/1.1 200 OK\r\n"), "{response}");
    assert!(
        response.contains("Cross-Origin-Opener-Policy: same-origin\r\n"),
        "{response}"
    );
    assert!(
        response.contains("Cross-Origin-Embedder-Policy: require-corp\r\n"),
        "{response}"
    );
    assert!(
        response.contains("Access-Control-Allow-Origin: *\r\n"),
        "{response}"
    );
    assert!(response.ends_with("wanix serve"), "{response}");
}

#[test]
fn serve_once_returns_mjs_static_file_as_javascript() {
    let root = temp_dir("wanix-cli-serve-mjs");
    fs::write(root.join("client.mjs"), b"export const ok = true;\n").unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let command = ServeCommand {
        root_path: root,
        addr: addr.to_string(),
        bundle: None,
        wanix_services: false,
        once: true,
    };

    let handle = thread::spawn(move || {
        let mut stderr = Vec::new();
        let exit_code = run_serve_with_listener(command, listener, &mut stderr).unwrap();
        (exit_code, stderr)
    });

    let response = http_request(addr, b"GET /client.mjs HTTP/1.1\r\nHost: localhost\r\n\r\n");
    let (exit_code, _stderr) = handle.join().unwrap();

    assert_eq!(exit_code, 0);
    let response = String::from_utf8(response).unwrap();
    assert!(response.starts_with("HTTP/1.1 200 OK\r\n"), "{response}");
    assert!(
        response.contains("Content-Type: text/javascript; charset=utf-8\r\n"),
        "{response}"
    );
    assert!(
        response.ends_with("export const ok = true;\n"),
        "{response}"
    );
}

#[test]
fn serve_once_runs_qjs_http_app_route() {
    let root = temp_dir("wanix-cli-serve-http-app");
    fs::create_dir_all(root.join("apps")).unwrap();
    fs::write(
        root.join("apps/hello.js"),
        br##"
import * as std from "qjs:std";

const app = std.getenv("WANIX_HTTP_APP");
const target = std.getenv("WANIX_HTTP_TARGET");
std.out.puts("app " + app + "\n");
std.out.puts("target " + target + "\n");
std.out.puts("argv " + scriptArgs.join("|") + "\n");
std.writeFile("ran.txt", "ran " + target);
"##,
    )
    .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let command = ServeCommand {
        root_path: root.clone(),
        addr: addr.to_string(),
        bundle: None,
        wanix_services: true,
        once: true,
    };

    let handle = thread::spawn(move || {
        let mut stderr = Vec::new();
        let exit_code = run_serve_with_listener(command, listener, &mut stderr).unwrap();
        (exit_code, stderr)
    });

    let response = http_request(
        addr,
        b"GET /.wanix/app/hello?from=browser HTTP/1.1\r\nHost: localhost\r\n\r\n",
    );
    let (exit_code, _stderr) = handle.join().unwrap();

    assert_eq!(exit_code, 0);
    let (headers, body) = http_response_parts(&response);
    assert!(headers.starts_with("HTTP/1.1 200 OK\r\n"), "{headers}");
    assert!(
        headers.contains("Content-Type: text/plain; charset=utf-8\r\n"),
        "{headers}"
    );
    assert!(headers.contains("X-Wanix-Task-Id: 2\r\n"), "{headers}");
    assert!(
        headers.contains("X-Wanix-Stdout-Path: /.wanix/http/2.out\r\n"),
        "{headers}"
    );
    assert!(
        headers.contains("X-Wanix-Stderr-Path: /.wanix/http/2.err\r\n"),
        "{headers}"
    );
    assert_eq!(
        body,
        b"app hello\ntarget /.wanix/app/hello?from=browser\nargv hello.js|/.wanix/app/hello?from=browser\n"
    );
    assert_eq!(
        fs::read(root.join("apps/ran.txt")).unwrap(),
        b"ran /.wanix/app/hello?from=browser"
    );
    assert!(
        root.join(".wanix/http/2.out").exists(),
        "route stdout trace should be visible in the served filesystem"
    );
}

#[test]
fn serve_once_reports_bundle_url_when_configured() {
    let root = temp_dir("wanix-cli-serve-bundle");
    fs::write(root.join("index.html"), b"wanix serve bundle").unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let command = ServeCommand {
        root_path: root,
        addr: addr.to_string(),
        bundle: Some("vm-workbench".to_owned()),
        wanix_services: false,
        once: true,
    };

    let handle = thread::spawn(move || {
        let mut stderr = Vec::new();
        let exit_code = run_serve_with_listener(command, listener, &mut stderr).unwrap();
        (exit_code, stderr)
    });

    let response = http_request(addr, b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n");
    let (exit_code, stderr) = handle.join().unwrap();

    assert_eq!(exit_code, 0);
    assert!(
        String::from_utf8(response)
            .unwrap()
            .ends_with("wanix serve bundle")
    );
    let stderr = String::from_utf8(stderr).unwrap();
    assert!(
        stderr.contains("bundle available at http://127.0.0.1:"),
        "{stderr}"
    );
    assert!(stderr.contains("/?bundle=vm-workbench"), "{stderr}");
}

#[test]
fn serve_once_reports_terminal_workbench_url_when_services_enabled() {
    let root = temp_dir("wanix-cli-serve-workbench-terminal-url");
    fs::write(root.join("index.html"), b"wanix serve terminal url").unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let command = ServeCommand {
        root_path: root,
        addr: addr.to_string(),
        bundle: Some(WORKBENCH_FS9P_BUNDLE.to_owned()),
        wanix_services: true,
        once: true,
    };

    let handle = thread::spawn(move || {
        let mut stderr = Vec::new();
        let exit_code = run_serve_with_listener(command, listener, &mut stderr).unwrap();
        (exit_code, stderr)
    });

    let _response = http_request(addr, b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n");
    let (exit_code, stderr) = handle.join().unwrap();

    assert_eq!(exit_code, 0);
    let stderr = String::from_utf8(stderr).unwrap();
    assert!(stderr.contains("/?bundle=workbench-fs9p&term"), "{stderr}");
}

#[test]
fn serve_once_returns_direct_v86_bundle_page() {
    let root = temp_dir("wanix-cli-serve-direct-v86");
    fs::write(root.join("index.html"), b"static index").unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let command = ServeCommand {
        root_path: root,
        addr: addr.to_string(),
        bundle: Some(DIRECT_V86_BUNDLE.to_owned()),
        wanix_services: false,
        once: true,
    };

    let handle = thread::spawn(move || {
        let mut stderr = Vec::new();
        let exit_code = run_serve_with_listener(command, listener, &mut stderr).unwrap();
        (exit_code, stderr)
    });

    let response = http_request(
        addr,
        b"GET /?bundle=direct-v86&bzimage=/boot/kernel HTTP/1.1\r\n\
              Host: demo.local:7654\r\n\
              \r\n",
    );
    let (exit_code, stderr) = handle.join().unwrap();

    assert_eq!(exit_code, 0);
    let stderr = String::from_utf8(stderr).unwrap();
    assert!(stderr.contains("/?bundle=direct-v86"), "{stderr}");
    let response = String::from_utf8(response).unwrap();
    assert!(response.starts_with("HTTP/1.1 200 OK\r\n"), "{response}");
    assert!(
        response.contains("Content-Type: text/html; charset=utf-8\r\n"),
        "{response}"
    );
    assert!(
        response.contains("fetch(\"/.well-known/wanix.json\""),
        "{response}"
    );
    assert!(
        response.contains("const v86Assets = discovery.v86?.assets || {}"),
        "{response}"
    );
    assert!(
        response.contains("const DEFAULT_KERNEL_URL = \"/boot/bzImage\""),
        "{response}"
    );
    assert!(
        !response.contains("__WANIX_DEFAULT_"),
        "unexpanded template placeholder leaked: {response}"
    );
    assert!(
        response.contains("const v86Boot = discovery.v86?.boot || {}"),
        "{response}"
    );
    assert!(
        response.contains("void loadRootfsHandoff(discovery.routes?.rootfs || {})"),
        "{response}"
    );
    assert!(response.contains("route = route || {}"), "{response}");
    assert!(
        response.contains("window.wanixRootfsRoute = route"),
        "{response}"
    );
    assert!(
        response.contains("window.wanixRootfsHandoff = null"),
        "{response}"
    );
    assert!(
        response.contains("rootfsHandoffCommands = null"),
        "{response}"
    );
    assert!(response.contains("copyQemu.disabled = true"), "{response}");
    assert!(response.contains("copyServe.disabled = true"), "{response}");
    assert!(
        response.contains("window.wanixRootfsHandoff = handoff"),
        "{response}"
    );
    assert!(response.contains("rootfsHandoffCommands = {"), "{response}");
    assert!(response.contains("qemuArgv,"), "{response}");
    assert!(response.contains("qemuCommand:"), "{response}");
    assert!(response.contains("serveArgv,"), "{response}");
    assert!(response.contains("serveCommand:"), "{response}");
    assert!(
        response.contains("function shellCommand(argv)"),
        "{response}"
    );
    assert!(
        response.contains("function shellQuoteArg(value)"),
        "{response}"
    );
    assert!(
        response.contains("await navigator.clipboard.writeText(command)"),
        "{response}"
    );
    assert!(
        response.contains("Rootfs handoff fetch failed: "),
        "{response}"
    );
    assert!(
        response
            .contains("const { V86 } = await import(v86Assets.module || \"/v86/lib/libv86.mjs\")"),
        "{response}"
    );
    assert!(
        response.contains("kernel.value = v86Boot.kernel || DEFAULT_KERNEL_URL"),
        "{response}"
    );
    assert!(
        response.contains("if (v86Boot.initrd) initrd.value = v86Boot.initrd"),
        "{response}"
    );
    assert!(
        response.contains("filesystem: { proxy_url: proxyUrl }"),
        "{response}"
    );
    assert!(
        response.contains(
            "const DEFAULT_CMDLINE = \"console=hvc0 init=/bin/init rw root=host9p rootfstype=9p"
        ),
        "{response}"
    );
    assert!(
        response.contains("const DEFAULT_MEMORY_SIZE = 1073741824"),
        "{response}"
    );
    assert!(
        response.contains("const DEFAULT_VGA_MEMORY_SIZE = 8388608"),
        "{response}"
    );
    assert!(
        response.contains("cmdline.value = discovery.v86?.defaultCmdline || DEFAULT_CMDLINE"),
        "{response}"
    );
    assert!(
        response.contains("if (params.get(\"p9-msize\")) cmdline.value = cmdline.value.replace"),
        "{response}"
    );
    assert!(
        response.contains("if (params.get(\"cmdline\")) cmdline.value = params.get(\"cmdline\")"),
        "{response}"
    );
    assert!(
        response.contains(
            "if (params.get(\"append\")) cmdline.value = [cmdline.value, params.get(\"append\")]"
        ),
        "{response}"
    );
    assert!(response.contains("cmdline: cmdline.value"), "{response}");
    assert!(response.contains("virtio_console: true"), "{response}");
    assert!(
        response.contains("bzimage_initrd_from_filesystem: false"),
        "{response}"
    );
    assert!(response.contains("autostart: true"), "{response}");
    assert!(
        response.contains("memory_size: discovery.v86?.memorySize || DEFAULT_MEMORY_SIZE"),
        "{response}"
    );
    assert!(
        response.contains("wasm_path: v86Assets.wasm || \"/v86/bundle/v86.wasm\""),
        "{response}"
    );
    assert!(
        response.contains("bios: { url: v86Assets.bios || \"/v86/bundle/seabios.bin\" }"),
        "{response}"
    );
    assert!(
        response.contains("<div id=\"screen\"><canvas></canvas><div></div></div>"),
        "{response}"
    );
    assert!(
        response.contains("<textarea id=\"serial\" spellcheck=\"false\" readonly></textarea>"),
        "{response}"
    );
    assert!(
        response.contains("<pre id=\"boot-log\" aria-live=\"polite\"></pre>"),
        "{response}"
    );
    assert!(
        response.contains("<pre id=\"rootfs\" aria-live=\"polite\"></pre>"),
        "{response}"
    );
    assert!(
        response.contains("<label for=\"rootfs\">rootfs handoff</label>"),
        "{response}"
    );
    assert!(
        response.contains("<button id=\"copy-qemu\" disabled>Copy QEMU command</button>"),
        "{response}"
    );
    assert!(
        response.contains("<button id=\"copy-serve\" disabled>Copy serve command</button>"),
        "{response}"
    );
    assert!(
        response.contains("const hvc0 = document.querySelector(\"#serial\")"),
        "{response}"
    );
    assert!(
        response.contains("window.wanixV86BootLog = []"),
        "{response}"
    );
    assert!(response.contains("function startVm()"), "{response}");
    assert!(
        response.contains("if (params.get(\"autostart\") === \"1\") startVm()"),
        "{response}"
    );
    assert!(response.contains("connectBootStatus(vm)"), "{response}");
    assert!(
        response.contains("vm.add_listener(\"download-progress\""),
        "{response}"
    );
    assert!(
        response.contains("vm.add_listener(\"emulator-started\""),
        "{response}"
    );
    assert!(
        response.contains("vm.add_listener(\"virtio-console0-output-bytes\", appendHvc0)"),
        "{response}"
    );
    assert!(
        response.contains("vm.bus.send(\"virtio-console0-input-bytes\", bytes)"),
        "{response}"
    );
    assert!(response.contains("control === \"c\" ? 3 : 4"), "{response}");
    assert!(
        response.contains("window.wanixV86SendHvc0 = input =>"),
        "{response}"
    );
    assert!(
        response.contains("if (!vm) throw new Error(\"v86 is not started\")"),
        "{response}"
    );
    assert!(
        response.contains("return hvc0Encoder.encode(String(input))"),
        "{response}"
    );
    assert!(
        response.contains("vm.bus.send(\"virtio-console0-resize\", [columns, rows])"),
        "{response}"
    );
    assert!(
        response.contains("vm.add_listener(\"emulator-ready\", resize)"),
        "{response}"
    );
    assert!(
        response.contains("window.addEventListener(\"unhandledrejection\""),
        "{response}"
    );
    assert!(response.contains("window.wanixV86 = vm"), "{response}");
    assert!(response.contains("connectHvc0(vm)"), "{response}");
    assert!(!response.contains("static index"), "{response}");
}

#[test]
fn serve_once_returns_fs9p_bundle_page() {
    let root = temp_dir("wanix-cli-serve-fs9p");
    fs::write(root.join("index.html"), b"static index").unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let command = ServeCommand {
        root_path: root,
        addr: addr.to_string(),
        bundle: Some(FS9P_BUNDLE.to_owned()),
        wanix_services: false,
        once: true,
    };

    let handle = thread::spawn(move || {
        let mut stderr = Vec::new();
        let exit_code = run_serve_with_listener(command, listener, &mut stderr).unwrap();
        (exit_code, stderr)
    });

    let response = http_request(
        addr,
        b"GET /?bundle=fs9p HTTP/1.1\r\nHost: demo.local:7654\r\n\r\n",
    );
    let (exit_code, stderr) = handle.join().unwrap();

    assert_eq!(exit_code, 0);
    let stderr = String::from_utf8(stderr).unwrap();
    assert!(stderr.contains("/?bundle=fs9p"), "{stderr}");
    let response = String::from_utf8(response).unwrap();
    assert!(response.starts_with("HTTP/1.1 200 OK\r\n"), "{response}");
    assert!(
        response.contains("Content-Type: text/html; charset=utf-8\r\n"),
        "{response}"
    );
    assert!(
        response.contains("fetch(\"/.well-known/wanix.json\""),
        "{response}"
    );
    assert!(
        response.contains("new P9Client(discovery.routes.p9.websocket)"),
        "{response}"
    );
    assert!(response.contains("new WebSocket(this.url)"), "{response}");
    assert!(
        response.contains("this.socket.binaryType = \"arraybuffer\""),
        "{response}"
    );
    for operation in [
        "Tversion",
        "Tattach",
        "Twalk",
        "Tlopen",
        "Tlcreate",
        "Tsymlink",
        "Treadlink",
        "Tread",
        "Twrite",
        "Treaddir",
        "Trenameat",
        "Tunlinkat",
    ] {
        assert!(
            response.contains(operation),
            "{operation} missing from {response}"
        );
    }
    assert!(response.contains("async readText(path)"), "{response}");
    assert!(
        response.contains("async writeText(path, value)"),
        "{response}"
    );
    assert!(response.contains("async readlink(path)"), "{response}");
    assert!(
        response.contains("async symlinkPath(target, linkPath)"),
        "{response}"
    );
    assert!(
        response.contains("async rename(oldPath, newPath)"),
        "{response}"
    );
    assert!(response.contains("async remove(path)"), "{response}");
    assert!(
        response.contains("async readdirPage(fid, offset)"),
        "{response}"
    );
    assert!(response.contains("payload.u64(offset)"), "{response}");
    assert!(response.contains("entries.push(...page)"), "{response}");
    assert!(response.contains("window.wanixP9 = client"), "{response}");
    assert!(!response.contains("static index"), "{response}");
    assert!(!response.contains("globalThis.Wanix"), "{response}");
}

#[test]
fn serve_once_returns_workbench_fs9p_bundle_page() {
    let root = temp_dir("wanix-cli-serve-workbench-fs9p");
    fs::write(root.join("index.html"), b"static index").unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let command = ServeCommand {
        root_path: root,
        addr: addr.to_string(),
        bundle: Some(WORKBENCH_FS9P_BUNDLE.to_owned()),
        wanix_services: false,
        once: true,
    };

    let handle = thread::spawn(move || {
        let mut stderr = Vec::new();
        let exit_code = run_serve_with_listener(command, listener, &mut stderr).unwrap();
        (exit_code, stderr)
    });

    let response = http_request(
        addr,
        b"GET /?bundle=workbench-fs9p HTTP/1.1\r\nHost: demo.local:7654\r\n\r\n",
    );
    let (exit_code, stderr) = handle.join().unwrap();

    assert_eq!(exit_code, 0);
    let stderr = String::from_utf8(stderr).unwrap();
    assert!(stderr.contains("/?bundle=workbench-fs9p"), "{stderr}");
    let response = String::from_utf8(response).unwrap();
    assert!(response.starts_with("HTTP/1.1 200 OK\r\n"), "{response}");
    assert!(
        response.contains("Content-Type: text/html; charset=utf-8\r\n"),
        "{response}"
    );
    assert!(
        response.contains(
            "const discoveryUrl = new URL(\"/.well-known/wanix.json\", location.href).href"
        ),
        "{response}"
    );
    assert!(response.contains("fetch(discoveryUrl"), "{response}");
    assert!(
        response.contains("new URL(\"vs/loader.js\", outDir)"),
        "{response}"
    );
    assert!(
        response.contains("new URL(\"vs/workbench/workbench.web.main.css\", outDir)"),
        "{response}"
    );
    assert!(
        response.contains("amdRequire.config({ baseUrl: outRoot })"),
        "{response}"
    );
    assert!(
        response.contains("[\"vs/workbench/workbench.web.main\"]"),
        "{response}"
    );
    assert!(
        response.contains("additionalBuiltinExtensions: [wb.URI.parse(extensionRoot.href)]"),
        "{response}"
    );
    assert!(
            response.contains("extensionEnabledApiProposals: { [extensionId]: [\"ipc\", \"fileSearchProvider\", \"textSearchProvider\"] }"),
            "{response}"
        );
    assert!(
        response.contains("const activationChannel = new MessageChannel()"),
        "{response}"
    );
    assert!(
        response.contains("const workbenchConfig = { discoveryUrl }"),
        "{response}"
    );
    assert!(response.contains("const openPath = params.get(\"open\")"));
    assert!(response.contains("workbenchConfig.open = openPath"));
    assert!(
        response.contains("workbenchConfig.p9 = discovery.routes.p9"),
        "{response}"
    );
    assert!(
        response.contains("if (discovery.services?.task && discovery.services?.term)"),
        "{response}"
    );
    assert!(
        response.contains(
            "workbenchConfig.qjsTask = discovery.services.drivers?.includes(\"qjs\") || false"
        ),
        "{response}"
    );
    assert!(
        response.contains("workbenchConfig.drivers = discovery.services.drivers || []"),
        "{response}"
    );
    assert!(
        response.contains("if (discovery.routes?.qjsShell?.websocket)"),
        "{response}"
    );
    assert!(
        response.contains("workbenchConfig.qjsShellUrl = discovery.routes.qjsShell.websocket"),
        "{response}"
    );
    assert!(
        response.contains("workbenchConfig.httpApp = discovery.routes.httpApp"),
        "{response}"
    );
    assert!(
        response.contains("const directV86Url = new URL(location.href)"),
        "{response}"
    );
    assert!(
        response.contains("directV86Url.searchParams.set(\"bundle\", \"direct-v86\")"),
        "{response}"
    );
    assert!(response.contains("workbenchConfig.v86 = {"), "{response}");
    assert!(
        response.contains("launchUrl: directV86Url.href"),
        "{response}"
    );
    assert!(
        response.contains("rootfs: discovery.routes?.rootfs || {}"),
        "{response}"
    );
    assert!(
        response.contains("event.data.port.postMessage({ config: workbenchConfig })"),
        "{response}"
    );
    assert!(response.contains("type: params.get(\"type\") || \"noop\""));
    assert!(response.contains("workbenchConfig.term = params.has(\"term\")"));
    assert!(
        response.contains("messagePorts: new Map([[extensionId, activationChannel.port1]])"),
        "{response}"
    );
    assert!(
        response.contains("nameLong: \"Wanix Workbench\""),
        "{response}"
    );
    assert!(
        response.contains("workspace: { folderUri: wb.URI.parse(workspaceUri) }"),
        "{response}"
    );
    assert!(
        response.contains("next.searchParams.set(\"workspace\", folder)"),
        "{response}"
    );
    assert!(!response.contains("todo: handle openFolder"), "{response}");
    assert!(
        response.contains("const workspaceUri = params.get(\"workspace\") || \"wanix:/\""),
        "{response}"
    );
    assert!(
        response.contains("const rawAssets = params.get(\"assets\") || \"/workbench/\""),
        "{response}"
    );
    assert!(
        response.contains("globalThis.wanixWorkbench ="),
        "{response}"
    );
    assert!(!response.contains("event.data.wanix"), "{response}");
    assert!(!response.contains("new WanixHandle"), "{response}");
    assert!(!response.contains("globalThis.Wanix"), "{response}");
    assert!(!response.contains("static index"), "{response}");
}

#[test]
fn serve_once_returns_workbench_assets_outside_served_root() {
    let root = temp_dir("wanix-cli-serve-workbench-assets");
    fs::create_dir_all(root.join("workbench/code/out")).unwrap();
    fs::write(
        root.join("workbench/code/out/nls.messages.js"),
        b"not bundled",
    )
    .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let command = ServeCommand {
        root_path: root,
        addr: addr.to_string(),
        bundle: Some(WORKBENCH_FS9P_BUNDLE.to_owned()),
        wanix_services: false,
        once: true,
    };

    let handle = thread::spawn(move || {
        let mut stderr = Vec::new();
        let exit_code = run_serve_with_listener(command, listener, &mut stderr).unwrap();
        (exit_code, stderr)
    });

    let response = http_request(
        addr,
        b"GET /workbench/code/out/nls.messages.js HTTP/1.1\r\nHost: localhost\r\n\r\n",
    );
    let (exit_code, _stderr) = handle.join().unwrap();

    assert_eq!(exit_code, 0);
    let response = String::from_utf8(response).unwrap();
    assert!(response.starts_with("HTTP/1.1 200 OK\r\n"), "{response}");
    assert!(
        response.contains("Content-Type: text/javascript; charset=utf-8\r\n"),
        "{response}"
    );
    assert!(!response.ends_with("not bundled"), "{response}");
    assert!(
        response.contains("globalThis._VSCODE_NLS_MESSAGES="),
        "{response}"
    );
}

#[test]
fn serve_once_returns_direct_v86_embedded_asset_over_static_collision() {
    let root = temp_dir("wanix-cli-serve-direct-v86-assets");
    fs::create_dir_all(root.join("v86/bundle")).unwrap();
    fs::write(root.join("v86/bundle/v86.wasm"), b"not wasm").unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let command = ServeCommand {
        root_path: root,
        addr: addr.to_string(),
        bundle: Some(DIRECT_V86_BUNDLE.to_owned()),
        wanix_services: false,
        once: true,
    };

    let handle = thread::spawn(move || {
        let mut stderr = Vec::new();
        let exit_code = run_serve_with_listener(command, listener, &mut stderr).unwrap();
        (exit_code, stderr)
    });

    let response = http_request(
        addr,
        b"GET /v86/bundle/v86.wasm HTTP/1.1\r\nHost: localhost\r\n\r\n",
    );
    let (exit_code, _stderr) = handle.join().unwrap();

    assert_eq!(exit_code, 0);
    let header_end = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .unwrap()
        + 4;
    let headers = String::from_utf8_lossy(&response[..header_end]);
    let body = &response[header_end..];
    assert!(headers.starts_with("HTTP/1.1 200 OK\r\n"), "{headers}");
    assert!(
        headers.contains("Content-Type: application/wasm\r\n"),
        "{headers}"
    );
    assert!(
        body.starts_with(b"\0asm"),
        "body did not start with wasm magic"
    );
    assert_ne!(body, b"not wasm");
}

#[test]
fn direct_v86_embedded_asset_table_serves_browser_runtime_assets() {
    let root = temp_dir("wanix-cli-direct-v86-asset-table");
    let roots = ServeRoots::new(
        &root,
        "127.0.0.1:0".parse().unwrap(),
        Some(DIRECT_V86_BUNDLE.to_owned()),
        false,
    )
    .unwrap();

    for (path, content_type, prefix) in [
        (
            "v86/lib/libv86.mjs",
            "text/javascript; charset=utf-8",
            Some(b";let module".as_slice()),
        ),
        (
            "v86/lib/mod.js",
            "text/javascript; charset=utf-8",
            Some(b"export { V86 }".as_slice()),
        ),
        (
            "v86/lib/offscreen.js",
            "text/javascript; charset=utf-8",
            None,
        ),
        ("v86/bundle/v86.wasm", "application/wasm", Some(b"\0asm")),
        ("v86/bundle/seabios.bin", "application/octet-stream", None),
        ("v86/bundle/vgabios.bin", "application/octet-stream", None),
    ] {
        let response = direct_v86_asset_response(&roots, Path::new(path)).unwrap();
        assert_eq!(response.status.status_line(), "200 OK");
        assert_eq!(response.content_type, content_type);
        assert!(!response.body.is_empty(), "{path} was empty");
        if let Some(prefix) = prefix {
            assert!(
                response.body.starts_with(prefix),
                "{path} body did not start with expected bytes"
            );
        }
    }

    let other_roots = ServeRoots::new(
        &root,
        "127.0.0.1:0".parse().unwrap(),
        Some("vm-workbench".to_owned()),
        false,
    )
    .unwrap();
    assert!(direct_v86_asset_response(&other_roots, Path::new("v86/bundle/v86.wasm")).is_none());
}

#[test]
fn serve_once_keeps_direct_v86_asset_paths_static_without_direct_bundle() {
    let root = temp_dir("wanix-cli-serve-static-v86-asset");
    fs::create_dir_all(root.join("v86/bundle")).unwrap();
    fs::write(root.join("v86/bundle/v86.wasm"), b"static wasm").unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let command = ServeCommand {
        root_path: root,
        addr: addr.to_string(),
        bundle: Some("vm-workbench".to_owned()),
        wanix_services: false,
        once: true,
    };

    let handle = thread::spawn(move || {
        let mut stderr = Vec::new();
        let exit_code = run_serve_with_listener(command, listener, &mut stderr).unwrap();
        (exit_code, stderr)
    });

    let response = http_request(
        addr,
        b"GET /v86/bundle/v86.wasm HTTP/1.1\r\nHost: localhost\r\n\r\n",
    );
    let (exit_code, _stderr) = handle.join().unwrap();

    assert_eq!(exit_code, 0);
    let header_end = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .unwrap()
        + 4;
    let headers = String::from_utf8_lossy(&response[..header_end]);
    let body = &response[header_end..];
    assert!(headers.starts_with("HTTP/1.1 200 OK\r\n"), "{headers}");
    assert!(
        headers.contains("Content-Type: application/wasm\r\n"),
        "{headers}"
    );
    assert_eq!(body, b"static wasm");
}

#[test]
fn serve_once_keeps_other_bundles_on_static_root() {
    let root = temp_dir("wanix-cli-serve-other-bundle");
    fs::write(root.join("index.html"), b"static bundle index").unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let command = ServeCommand {
        root_path: root,
        addr: addr.to_string(),
        bundle: Some("vm-workbench".to_owned()),
        wanix_services: false,
        once: true,
    };

    let handle = thread::spawn(move || {
        let mut stderr = Vec::new();
        let exit_code = run_serve_with_listener(command, listener, &mut stderr).unwrap();
        (exit_code, stderr)
    });

    let response = http_request(
        addr,
        b"GET /?bundle=direct-v86 HTTP/1.1\r\nHost: localhost\r\n\r\n",
    );
    let (exit_code, _stderr) = handle.join().unwrap();

    assert_eq!(exit_code, 0);
    let response = String::from_utf8(response).unwrap();
    assert!(response.ends_with("static bundle index"), "{response}");
    assert!(!response.contains("filesystem: { proxy_url: proxyUrl }"));
}

#[test]
fn serve_once_returns_well_known_discovery_document() {
    let root = temp_dir("wanix-cli-serve-discovery");
    fs::create_dir_all(root.join("boot")).unwrap();
    fs::write(root.join("index.html"), b"wanix serve discovery").unwrap();
    fs::write(root.join("boot/bzImage"), b"kernel").unwrap();
    fs::write(root.join("boot/initrd"), b"initrd").unwrap();
    write_boot_init(&root);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let command = ServeCommand {
        root_path: root,
        addr: addr.to_string(),
        bundle: Some("vm-workbench".to_owned()),
        wanix_services: false,
        once: true,
    };

    let handle = thread::spawn(move || {
        let mut stderr = Vec::new();
        let exit_code = run_serve_with_listener(command, listener, &mut stderr).unwrap();
        (exit_code, stderr)
    });

    let response = http_request(
        addr,
        b"GET /.well-known/wanix.json HTTP/1.1\r\nHost: demo.local:7654\r\n\r\n",
    );
    let (exit_code, _stderr) = handle.join().unwrap();

    assert_eq!(exit_code, 0);
    let response = String::from_utf8(response).unwrap();
    assert!(response.starts_with("HTTP/1.1 200 OK\r\n"), "{response}");
    assert!(
        response.contains("Content-Type: application/json\r\n"),
        "{response}"
    );
    assert!(
        response.contains(
            "\"p9\":{\"websocket\":\"ws://demo.local:7654/.well-known/export9p\",\
                 \"transport\":\"direct-binary-websocket\",\"protocol\":\"9p2000.L\",\
                 \"supportedProtocols\":[\"9P2000.L\",\"9P2000.L.Google.2\"]}"
        ),
        "{response}"
    );
    assert!(
        response.contains(
            "\"rootfs\":{\"url\":\"http://demo.local:7654/.well-known/rootfs.json\",\
                 \"kind\":\"wanix-rootfs.v1\",\"status\":\"available\",\"ready\":true}"
        ),
        "{response}"
    );
    assert!(
        response.contains(
            "\"ethernet\":{\"websocket\":\"ws://demo.local:7654/.well-known/ethernet\",\
                 \"status\":\"not-implemented\"}"
        ),
        "{response}"
    );
    assert!(
        response.contains("\"qjsShell\":{\"status\":\"disabled\"}"),
        "{response}"
    );
    assert!(
        response.contains("\"bundle\":\"vm-workbench\""),
        "{response}"
    );
    assert!(
        response.contains("\"assets\":{\"module\":\"/v86/lib/libv86.mjs\""),
        "{response}"
    );
    assert!(
        response.contains("\"mod\":\"/v86/lib/mod.js\""),
        "{response}"
    );
    assert!(
        response.contains("\"offscreen\":\"/v86/lib/offscreen.js\""),
        "{response}"
    );
    assert!(
        response.contains("\"wasm\":\"/v86/bundle/v86.wasm\""),
        "{response}"
    );
    assert!(
        response.contains("\"bios\":\"/v86/bundle/seabios.bin\""),
        "{response}"
    );
    assert!(
        response.contains("\"vgaBios\":\"/v86/bundle/vgabios.bin\""),
        "{response}"
    );
    assert!(
            response
                .contains("\"boot\":{\"kernel\":\"/boot/bzImage\",\"initrd\":\"/boot/initrd\",\"init\":\"/bin/init\",\"ready\":true}"),
            "{response}"
        );
    assert!(
        response.contains("\"defaultCmdline\":\"console=hvc0 init=/bin/init rw root=host9p"),
        "{response}"
    );
    assert!(response.contains("\"p9Msize\":131072"), "{response}");
    assert!(
        response
            .contains("\"memorySize\":1073741824,\"vgaMemorySize\":8388608,\"virtioConsole\":true"),
        "{response}"
    );
    assert!(response.contains("\"services\":null"), "{response}");
}

#[test]
fn serve_once_returns_rootfs_handoff_manifest() {
    let root = temp_dir("wanix-cli-serve-rootfs-handoff");
    fs::create_dir_all(root.join("boot")).unwrap();
    fs::write(root.join("boot/bzImage"), b"kernel").unwrap();
    fs::write(root.join("boot/initrd"), b"initrd").unwrap();
    write_boot_init(&root);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let command = ServeCommand {
        root_path: root.clone(),
        addr: addr.to_string(),
        bundle: Some(DIRECT_V86_BUNDLE.to_owned()),
        wanix_services: true,
        once: true,
    };

    let handle = thread::spawn(move || {
        let mut stderr = Vec::new();
        let exit_code = run_serve_with_listener(command, listener, &mut stderr).unwrap();
        (exit_code, stderr)
    });

    let response = http_request(
        addr,
        b"GET /.well-known/rootfs.json HTTP/1.1\r\nHost: demo.local:7654\r\n\r\n",
    );
    let (exit_code, _stderr) = handle.join().unwrap();

    assert_eq!(exit_code, 0);
    let (headers, body) = http_response_parts(&response);
    assert!(headers.starts_with("HTTP/1.1 200 OK\r\n"), "{headers}");
    assert!(
        headers.contains("Content-Type: application/json\r\n"),
        "{headers}"
    );
    let manifest: serde_json::Value = serde_json::from_slice(body).unwrap();
    let root = fs::canonicalize(root).unwrap();
    let kernel = root.join("boot/bzImage");
    let initrd = root.join("boot/initrd");
    let init = root.join("bin/init");
    let qemu = &manifest["qemu"];
    let serve = &manifest["serveDirectV86"];

    assert_eq!(manifest["kind"], "wanix-rootfs.v1");
    assert_eq!(manifest["rootPath"], root.display().to_string());
    assert_eq!(manifest["kernelRoute"], "/boot/bzImage");
    assert_eq!(manifest["kernelPath"], kernel.display().to_string());
    assert_eq!(manifest["initRoute"], "/bin/init");
    assert_eq!(manifest["initPath"], init.display().to_string());
    assert_eq!(qemu["kind"], "wanix-qemu-virtio9p.v1");
    assert_eq!(qemu["rootPath"], root.display().to_string());
    assert_eq!(qemu["kernelPath"], kernel.display().to_string());
    assert_eq!(qemu["initrdPath"], initrd.display().to_string());
    assert_eq!(qemu["mountTag"], "host9p");
    assert_eq!(qemu["securityModel"], "mapped-xattr");
    assert_eq!(serve["bundle"], "direct-v86");
    assert_eq!(serve["wanixServices"], true);
    assert_eq!(serve["p9Msize"], 131072);
    assert_eq!(serve["argv"][0], "wanix-rust");
    assert_eq!(serve["argv"][1], "serve");
    assert_eq!(serve["argv"][2], root.display().to_string());
}

#[test]
fn serve_rootfs_handoff_reports_unprepared_roots_and_reserves_route() {
    let root = temp_dir("wanix-cli-serve-rootfs-handoff-missing");
    fs::create_dir_all(root.join(".well-known")).unwrap();
    fs::write(root.join(".well-known/rootfs.json"), b"not static").unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let command = ServeCommand {
        root_path: root,
        addr: addr.to_string(),
        bundle: Some(DIRECT_V86_BUNDLE.to_owned()),
        wanix_services: false,
        once: true,
    };

    let handle = thread::spawn(move || {
        let mut stderr = Vec::new();
        let exit_code = run_serve_with_listener(command, listener, &mut stderr).unwrap();
        (exit_code, stderr)
    });

    let response = http_request(
        addr,
        b"GET /.well-known/rootfs.json HTTP/1.1\r\nHost: localhost\r\n\r\n",
    );
    let (exit_code, _stderr) = handle.join().unwrap();

    assert_eq!(exit_code, 0);
    let response = String::from_utf8(response).unwrap();
    assert!(
        response.starts_with("HTTP/1.1 409 Conflict\r\n"),
        "{response}"
    );
    assert!(response.contains("missing a guest kernel"), "{response}");
    assert!(!response.contains("not static"), "{response}");
}

#[test]
fn serve_rootfs_handoff_rejects_non_loopback_peers_without_paths() {
    let root = temp_dir("wanix-cli-serve-rootfs-handoff-remote");
    fs::create_dir_all(root.join("boot")).unwrap();
    fs::write(root.join("boot/bzImage"), b"kernel").unwrap();
    write_boot_init(&root);
    let roots = ServeRoots::new(&root, "0.0.0.0:7654".parse().unwrap(), None, false).unwrap();
    let response = rootfs_handoff_response(&roots, "192.0.2.1:12345".parse().unwrap());
    let status = response.status.status_line();
    let body = String::from_utf8(response.body).unwrap();

    assert_eq!(status, "403 Forbidden");
    assert!(body.contains("loopback clients"), "{body}");
    assert!(!body.contains(&fs::canonicalize(root).unwrap().display().to_string()));
    assert!(!body.contains("bzImage"), "{body}");
}

#[test]
fn serve_wanix_services_root_exports_task_and_terminal_services() {
    let root = temp_dir("wanix-cli-serve-services-root");
    fs::write(root.join("host.txt"), b"host file").unwrap();
    let roots = ServeRoots::new(
        &root,
        "127.0.0.1:7654".parse().unwrap(),
        Some(WORKBENCH_FS9P_BUNDLE.to_owned()),
        true,
    )
    .unwrap();

    let discovery = serve_discovery_json(
        &roots,
        b"GET /.well-known/wanix.json HTTP/1.1\r\nHost: demo.local:7654\r\n\r\n",
        "127.0.0.1:12345".parse().unwrap(),
    );
    assert!(
        discovery.contains(
            "\"services\":{\"task\":\"#task\",\"term\":\"#term\",\
                 \"drivers\":[\"noop\",\"qjs\",\"wasm\"]}"
        ),
        "{discovery}"
    );
    let discovery_json: serde_json::Value = serde_json::from_str(&discovery).unwrap();
    let qjs_shell = &discovery_json["routes"]["qjsShell"];
    assert_eq!(
        qjs_shell["websocket"],
        "ws://demo.local:7654/.well-known/qjs-shell"
    );
    assert_eq!(qjs_shell["protocol"], "wanix-qjs-shell.v1");
    assert_eq!(qjs_shell["mode"], "raw-bytes");
    assert_eq!(qjs_shell["status"], "available");
    assert_eq!(qjs_shell["cwdQuery"], "cwd");
    assert_eq!(qjs_shell["defaultCwd"], ".");
    assert_eq!(
        qjs_shell["resize"][0],
        "{\"type\":\"resize\",\"columns\":COLS,\"rows\":ROWS}"
    );
    assert_eq!(qjs_shell["resize"][1], "resize COLS ROWS");
    assert_eq!(qjs_shell["exitMessage"], "{\"type\":\"exit\",\"code\":N}");
    assert_eq!(
        qjs_shell["terminalLifecycle"],
        "owned-resource-closed-on-session-close"
    );
    let http_app = &discovery_json["routes"]["httpApp"];
    assert_eq!(http_app["url"], "http://demo.local:7654/.wanix/app/{name}");
    assert_eq!(http_app["protocol"], "wanix-http-app.v1");
    assert_eq!(http_app["status"], "available");
    assert_eq!(http_app["route"], "/.wanix/app/<name>");
    assert_eq!(http_app["source"], "apps/<name>.js");
    assert_eq!(http_app["response"], "stdout");
    assert_eq!(http_app["scope"], "loopback");

    let mut server = wanix_9p::P9Server::new(roots.p9_root.clone());
    let response = server
        .handle_frame(&p9_tattach(1, 1, 0xffff_ffff, "workbench", "", 0).unwrap())
        .unwrap();
    assert_eq!(response.message_type(), P9_RATTACH);

    let response = server
        .handle_frame(&p9_twalk(2, 1, 2, &["#term", "new"]).unwrap())
        .unwrap();
    assert_eq!(response.message_type(), P9_RWALK);
    assert_eq!(p9_decode_rwalk(&response).unwrap().len(), 2);
    assert_eq!(
        server
            .handle_frame(&p9_tlopen(3, 2, 0))
            .unwrap()
            .message_type(),
        P9_RLOPEN
    );
    let response = server.handle_frame(&p9_tread(4, 2, 0, 64)).unwrap();
    assert_eq!(response.message_type(), P9_RREAD);
    assert_eq!(p9_decode_rread(&response).unwrap(), b"1\n");

    let response = server
        .handle_frame(&p9_twalk(5, 1, 3, &["#task", "self", "id"]).unwrap())
        .unwrap();
    assert_eq!(response.message_type(), P9_RWALK);
    assert_eq!(p9_decode_rwalk(&response).unwrap().len(), 3);
    assert_eq!(
        server
            .handle_frame(&p9_tlopen(6, 3, 0))
            .unwrap()
            .message_type(),
        P9_RLOPEN
    );
    let response = server.handle_frame(&p9_tread(7, 3, 0, 64)).unwrap();
    assert_eq!(response.message_type(), P9_RREAD);
    assert_eq!(p9_decode_rread(&response).unwrap(), b"1\n");

    let response = server
        .handle_frame(&p9_twalk(8, 1, 4, &["#task", "new", "noop"]).unwrap())
        .unwrap();
    assert_eq!(response.message_type(), P9_RWALK);
    assert_eq!(p9_decode_rwalk(&response).unwrap().len(), 3);
    assert_eq!(
        server
            .handle_frame(&p9_tlopen(9, 4, 0))
            .unwrap()
            .message_type(),
        P9_RLOPEN
    );
    let response = server.handle_frame(&p9_tread(10, 4, 0, 64)).unwrap();
    assert_eq!(response.message_type(), P9_RREAD);
    assert_eq!(p9_decode_rread(&response).unwrap(), b"2\n");
}

#[test]
fn serve_wanix_services_terminal_winch_broadcasts_over_9p() {
    let root = temp_dir("wanix-cli-serve-services-winch");
    let roots = ServeRoots::new(
        &root,
        "127.0.0.1:7654".parse().unwrap(),
        Some(WORKBENCH_FS9P_BUNDLE.to_owned()),
        true,
    )
    .unwrap();
    let mut server = wanix_9p::P9Server::new(roots.p9_root.clone());

    assert_eq!(
        server
            .handle_frame(&p9_tattach(1, 1, 0xffff_ffff, "workbench", "", 0).unwrap())
            .unwrap()
            .message_type(),
        P9_RATTACH
    );
    assert_eq!(
        p9_read_file(&mut server, p9_file(1, 2, 100, &["#term", "new"])),
        b"1\n"
    );

    let response = server
        .handle_frame(&p9_twalk(110, 1, 3, &["#term", "1", "winch"]).unwrap())
        .unwrap();
    assert_eq!(response.message_type(), P9_RWALK);
    assert_eq!(p9_decode_rwalk(&response).unwrap().len(), 3);
    assert_eq!(
        server
            .handle_frame(&p9_tlopen(111, 3, 0))
            .unwrap()
            .message_type(),
        P9_RLOPEN
    );

    let response = server
        .handle_frame(&p9_twalk(120, 1, 4, &["#term", "1", "winch"]).unwrap())
        .unwrap();
    assert_eq!(response.message_type(), P9_RWALK);
    assert_eq!(p9_decode_rwalk(&response).unwrap().len(), 3);
    assert_eq!(
        server
            .handle_frame(&p9_tlopen(121, 4, P9_O_RDWR))
            .unwrap()
            .message_type(),
        P9_RLOPEN
    );
    let response = server
        .handle_frame(&p9_twrite(122, 4, 0, b"100 40\n").unwrap())
        .unwrap();
    assert_eq!(response.message_type(), P9_RWRITE);
    assert_eq!(p9_decode_rwrite(&response).unwrap(), 7);

    let response = server.handle_frame(&p9_tread(130, 3, 0, 64)).unwrap();
    assert_eq!(response.message_type(), P9_RREAD);
    assert_eq!(p9_decode_rread(&response).unwrap(), b"100 40\n");
}

#[test]
fn serve_wanix_services_terminal_ctl_close_is_visible_over_9p() {
    let root = temp_dir("wanix-cli-serve-services-term-close");
    let roots = ServeRoots::new(
        &root,
        "127.0.0.1:7654".parse().unwrap(),
        Some(WORKBENCH_FS9P_BUNDLE.to_owned()),
        true,
    )
    .unwrap();
    let mut server = wanix_9p::P9Server::new(roots.p9_root.clone());

    assert_eq!(
        server
            .handle_frame(&p9_tattach(1, 1, 0xffff_ffff, "workbench", "", 0).unwrap())
            .unwrap()
            .message_type(),
        P9_RATTACH
    );
    assert_eq!(
        p9_read_file(&mut server, p9_file(1, 2, 100, &["#term", "new"])),
        b"1\n"
    );

    let response = server
        .handle_frame(&p9_twalk(110, 1, 3, &["#term", "1", "data"]).unwrap())
        .unwrap();
    assert_eq!(response.message_type(), P9_RWALK);
    assert_eq!(
        server
            .handle_frame(&p9_tlopen(111, 3, P9_O_RDWR))
            .unwrap()
            .message_type(),
        P9_RLOPEN
    );
    let response = server
        .handle_frame(&p9_twrite(112, 3, 0, b"before close").unwrap())
        .unwrap();
    assert_eq!(response.message_type(), P9_RWRITE);

    let response = server
        .handle_frame(&p9_twalk(120, 1, 4, &["#term", "1", "ctl"]).unwrap())
        .unwrap();
    assert_eq!(response.message_type(), P9_RWALK);
    assert_eq!(
        server
            .handle_frame(&p9_tlopen(121, 4, P9_O_WRONLY))
            .unwrap()
            .message_type(),
        P9_RLOPEN
    );
    let response = server
        .handle_frame(&p9_twrite(122, 4, 0, b"clo").unwrap())
        .unwrap();
    assert_eq!(response.message_type(), P9_RWRITE);
    let response = server
        .handle_frame(&p9_twrite(123, 3, 0, b"still open").unwrap())
        .unwrap();
    assert_eq!(response.message_type(), P9_RWRITE);
    let response = server
        .handle_frame(&p9_twrite(124, 4, 0, b"se\n").unwrap())
        .unwrap();
    assert_eq!(response.message_type(), P9_RWRITE);

    let response = server
        .handle_frame(&p9_twalk(140, 1, 6, &["#term", "1", "data"]).unwrap())
        .unwrap();
    assert_eq!(response.message_type(), P9_RLERROR);
    let response = server
        .handle_frame(&p9_twrite(150, 3, 0, b"after close").unwrap())
        .unwrap();
    assert_eq!(response.message_type(), P9_RLERROR);
    assert_eq!(p9_decode_rlerror(&response).unwrap().ecode, EBADF);
}

#[test]
fn serve_wanix_services_can_start_qjs_task_through_9p_task_service() {
    let root = temp_dir("wanix-cli-serve-services-qjs-task");
    fs::write(
        root.join("main.js"),
        br##"
import * as std from "qjs:std";

const id = std.loadFile("#task/self/id").trim();
std.out.puts("serve qjs task " + id + "\n");
std.writeFile("generated.txt", "generated by task " + id);
"##,
    )
    .unwrap();
    fs::write(root.join("stdout.txt"), b"").unwrap();
    let roots = ServeRoots::new(
        &root,
        "127.0.0.1:7654".parse().unwrap(),
        Some(WORKBENCH_FS9P_BUNDLE.to_owned()),
        true,
    )
    .unwrap();
    let mut server = wanix_9p::P9Server::new(roots.p9_root.clone());

    assert_eq!(
        server
            .handle_frame(&p9_tattach(1, 1, 0xffff_ffff, "workbench", "", 0).unwrap())
            .unwrap()
            .message_type(),
        P9_RATTACH
    );
    assert_eq!(
        p9_read_file(&mut server, p9_file(1, 2, 100, &["#task", "new", "qjs"])),
        b"2\n"
    );
    p9_write_file(
        &mut server,
        p9_file(1, 3, 110, &["#task", "2", "cmd"]),
        b"main.js\n",
    );
    p9_write_file(
        &mut server,
        p9_file(1, 4, 120, &["#task", "2", "dir"]),
        b".\n",
    );
    p9_write_file(
        &mut server,
        p9_file(1, 5, 130, &["#task", "2", "ctl"]),
        b"bind stdout.txt fd/1\n",
    );
    p9_write_file(
        &mut server,
        p9_file(1, 6, 140, &["#task", "2", "ctl"]),
        b"start\n",
    );

    assert_eq!(
        p9_read_file(&mut server, p9_file(1, 7, 150, &["#task", "2", "exit"])),
        b"0\n"
    );
    assert_eq!(
        fs::read(root.join("stdout.txt")).unwrap(),
        b"serve qjs task 2\n"
    );
    assert_eq!(
        fs::read(root.join("generated.txt")).unwrap(),
        b"generated by task 2"
    );
}

#[test]
fn serve_discovery_falls_back_to_listener_address_without_host_header() {
    let root = temp_dir("wanix-cli-serve-discovery-no-host");
    let roots = ServeRoots::new(
        &root,
        "0.0.0.0:7654".parse().unwrap(),
        Some("quote\"bundle".to_owned()),
        false,
    )
    .unwrap();
    let body = serve_discovery_json(
        &roots,
        b"GET /.well-known/wanix.json HTTP/1.1\r\n\r\n",
        "127.0.0.1:12345".parse().unwrap(),
    );

    assert!(
        body.contains("\"websocket\":\"ws://localhost:7654/.well-known/export9p\""),
        "{body}"
    );
    assert!(body.contains("\"bundle\":\"quote\\\"bundle\""), "{body}");
    assert!(
        body.contains("\"defaultCmdline\":\"console=hvc0 init=/bin/init rw root=host9p"),
        "{body}"
    );

    let ipv6_roots = ServeRoots::new(&root, "[::1]:7654".parse().unwrap(), None, false).unwrap();
    let ipv6_body = serve_discovery_json(
        &ipv6_roots,
        b"GET /.well-known/wanix.json HTTP/1.1\r\n\r\n",
        "[::1]:12345".parse().unwrap(),
    );
    assert!(
        ipv6_body.contains("\"websocket\":\"ws://[::1]:7654/.well-known/export9p\""),
        "{ipv6_body}"
    );
}

#[test]
fn serve_discovery_finds_direct_v86_legacy_top_level_kernel() {
    let root = temp_dir("wanix-cli-serve-discovery-legacy-kernel");
    fs::write(root.join("bzImage"), b"kernel").unwrap();
    let roots = ServeRoots::new(
        &root,
        "127.0.0.1:7654".parse().unwrap(),
        Some(DIRECT_V86_BUNDLE.to_owned()),
        false,
    )
    .unwrap();

    let body = serve_discovery_json(
        &roots,
        b"GET /.well-known/wanix.json HTTP/1.1\r\nHost: demo.local:7654\r\n\r\n",
        "127.0.0.1:12345".parse().unwrap(),
    );

    assert!(
        body.contains(
            "\"boot\":{\"kernel\":\"/bzImage\",\"ready\":false,\"missing\":[\"/bin/init\"]}"
        ),
        "{body}"
    );
}

#[test]
fn serve_discovery_reports_direct_v86_boot_readiness_gaps() {
    let root = temp_dir("wanix-cli-serve-discovery-boot-gaps");
    write_boot_init(&root);
    let roots = ServeRoots::new(
        &root,
        "127.0.0.1:7654".parse().unwrap(),
        Some(DIRECT_V86_BUNDLE.to_owned()),
        false,
    )
    .unwrap();

    let body = serve_discovery_json(
        &roots,
        b"GET /.well-known/wanix.json HTTP/1.1\r\nHost: demo.local:7654\r\n\r\n",
        "127.0.0.1:12345".parse().unwrap(),
    );

    assert!(
        body.contains(
            "\"boot\":{\"init\":\"/bin/init\",\"ready\":false,\"missing\":[\"/boot/bzImage\"]}"
        ),
        "{body}"
    );
}

#[cfg(unix)]
#[test]
fn serve_discovery_requires_executable_direct_v86_init() {
    let root = temp_dir("wanix-cli-serve-discovery-non-executable-init");
    fs::create_dir_all(root.join("boot")).unwrap();
    fs::create_dir_all(root.join("bin")).unwrap();
    fs::write(root.join("boot/bzImage"), b"kernel").unwrap();
    fs::write(root.join("bin/init"), b"init").unwrap();
    let roots = ServeRoots::new(
        &root,
        "127.0.0.1:7654".parse().unwrap(),
        Some(DIRECT_V86_BUNDLE.to_owned()),
        false,
    )
    .unwrap();

    let body = serve_discovery_json(
        &roots,
        b"GET /.well-known/wanix.json HTTP/1.1\r\nHost: demo.local:7654\r\n\r\n",
        "127.0.0.1:12345".parse().unwrap(),
    );
    let discovery: serde_json::Value = serde_json::from_str(&body).unwrap();
    let boot = &discovery["v86"]["boot"];
    let rootfs = &discovery["routes"]["rootfs"];

    assert_eq!(boot["kernel"], "/boot/bzImage");
    assert!(boot["init"].is_null());
    assert_eq!(boot["ready"], false);
    assert_eq!(boot["missing"][0], "/bin/init");
    assert_eq!(rootfs["status"], "unprepared");
    assert_eq!(rootfs["ready"], false);
    assert_eq!(rootfs["missing"][0], "/bin/init");
}

#[test]
fn serve_discovery_reports_rootfs_handoff_status() {
    let root = temp_dir("wanix-cli-serve-discovery-rootfs-status");
    write_boot_init(&root);
    let roots = ServeRoots::new(
        &root,
        "0.0.0.0:7654".parse().unwrap(),
        Some(DIRECT_V86_BUNDLE.to_owned()),
        false,
    )
    .unwrap();

    let body = serve_discovery_json(
        &roots,
        b"GET /.well-known/wanix.json HTTP/1.1\r\nHost: demo.local:7654\r\n\r\n",
        "127.0.0.1:12345".parse().unwrap(),
    );
    let discovery: serde_json::Value = serde_json::from_str(&body).unwrap();
    let rootfs = &discovery["routes"]["rootfs"];
    assert_eq!(
        rootfs["url"],
        "http://demo.local:7654/.well-known/rootfs.json"
    );
    assert_eq!(rootfs["kind"], "wanix-rootfs.v1");
    assert_eq!(rootfs["status"], "unprepared");
    assert_eq!(rootfs["ready"], false);
    assert_eq!(rootfs["missing"][0], "/boot/bzImage");

    let remote_body = serve_discovery_json(
        &roots,
        b"GET /.well-known/wanix.json HTTP/1.1\r\nHost: demo.local:7654\r\n\r\n",
        "192.0.2.1:12345".parse().unwrap(),
    );
    let remote_discovery: serde_json::Value = serde_json::from_str(&remote_body).unwrap();
    assert_eq!(remote_discovery["routes"]["rootfs"]["status"], "local-only");
}

#[test]
fn serve_discovery_does_not_overpromise_invalid_rootfs_handoff() {
    let parent = temp_dir("wanix-cli-serve-discovery-rootfs-invalid");
    let root = parent.join("with,comma");
    fs::create_dir_all(root.join("boot")).unwrap();
    fs::write(root.join("boot/bzImage"), b"kernel").unwrap();
    write_boot_init(&root);
    let roots = ServeRoots::new(
        &root,
        "127.0.0.1:7654".parse().unwrap(),
        Some(DIRECT_V86_BUNDLE.to_owned()),
        false,
    )
    .unwrap();

    let body = serve_discovery_json(
        &roots,
        b"GET /.well-known/wanix.json HTTP/1.1\r\nHost: demo.local:7654\r\n\r\n",
        "127.0.0.1:12345".parse().unwrap(),
    );
    let discovery: serde_json::Value = serde_json::from_str(&body).unwrap();
    let rootfs = &discovery["routes"]["rootfs"];
    assert_eq!(rootfs["status"], "invalid");
    assert_eq!(rootfs["ready"], false);
    assert!(
        rootfs["error"]
            .as_str()
            .unwrap()
            .contains("cannot contain ','"),
        "{rootfs}"
    );
    assert!(rootfs.get("missing").is_none(), "{rootfs}");
}

#[test]
fn serve_discovery_ignores_unsafe_host_header() {
    let root = temp_dir("wanix-cli-serve-discovery-unsafe-host");
    let roots = ServeRoots::new(&root, "127.0.0.1:7654".parse().unwrap(), None, false).unwrap();
    let body = serve_discovery_json(
        &roots,
        b"GET /.well-known/wanix.json HTTP/1.1\r\nHost: demo.local:7654/escape\r\n\r\n",
        "127.0.0.1:12345".parse().unwrap(),
    );

    assert!(
        body.contains("\"websocket\":\"ws://127.0.0.1:7654/.well-known/export9p\""),
        "{body}"
    );
}

#[test]
fn serve_once_exports_9p_over_binary_websocket() {
    let root = temp_dir("wanix-cli-serve-ws");
    fs::write(root.join("hello.txt"), b"hello serve").unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let command = ServeCommand {
        root_path: root,
        addr: addr.to_string(),
        bundle: None,
        wanix_services: false,
        once: true,
    };

    let handle = thread::spawn(move || {
        let mut stderr = Vec::new();
        let exit_code = run_serve_with_listener(command, listener, &mut stderr).unwrap();
        (exit_code, stderr)
    });

    let mut socket = connect(format!("ws://{addr}/")).unwrap().0;
    send_p9_requests(
        &mut socket,
        [
            p9_tversion(1, 8192, P9_VERSION_9P2000_L).unwrap(),
            p9_tattach(2, 1, 0xffff_ffff, "root", "", 0).unwrap(),
            p9_twalk(3, 1, 2, &["hello.txt"]).unwrap(),
            p9_tgetattr(4, 2, u64::MAX),
            p9_tlopen(5, 2, 0),
            p9_tread(6, 2, 0, 11),
        ],
    );

    let frames = read_binary_frames(&mut socket, 6);
    socket.close(None).unwrap();
    let (exit_code, _stderr) = handle.join().unwrap();

    assert_eq!(exit_code, 0);
    assert_eq!(
        frame_types(&frames),
        [
            P9_RVERSION,
            P9_RATTACH,
            P9_RWALK,
            P9_RGETATTR,
            P9_RLOPEN,
            P9_RREAD
        ]
    );
    let attr = p9_decode_rgetattr(&frames[3]).unwrap();
    assert_eq!(attr.size, 11);
    assert_eq!(attr.mode & 0o170000, 0o100000);
    assert_eq!(p9_decode_rread(&frames[5]).unwrap(), b"hello serve");
}

#[test]
fn serve_concurrent_loop_serves_http_while_9p_websocket_stays_open() {
    let root = temp_dir("wanix-cli-serve-concurrent-http");
    fs::write(root.join("index.html"), b"wanix serve concurrent").unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let command = ServeCommand {
        root_path: root,
        addr: addr.to_string(),
        bundle: Some(DIRECT_V86_BUNDLE.to_owned()),
        wanix_services: false,
        once: false,
    };

    let handle = thread::spawn(move || {
        let mut stderr = Vec::new();
        let exit_code =
            run_serve_with_listener_for_connections(command, listener, &mut stderr, 2).unwrap();
        (exit_code, stderr)
    });

    let mut socket = connect_export9p_websocket(addr);
    send_p9_requests(
        &mut socket,
        [p9_tversion(1, 8192, P9_VERSION_9P2000_L).unwrap()],
    );
    let frames = read_binary_frames(&mut socket, 1);
    assert_eq!(frame_types(&frames), [P9_RVERSION]);

    let response = http_request(
        addr,
        b"GET /.well-known/wanix.json HTTP/1.1\r\nHost: demo.local:7654\r\n\r\n",
    );
    let response = String::from_utf8(response).unwrap();
    assert!(response.starts_with("HTTP/1.1 200 OK\r\n"), "{response}");
    assert!(
        response.contains("\"websocket\":\"ws://demo.local:7654/.well-known/export9p\""),
        "{response}"
    );

    socket.close(None).unwrap();
    let (exit_code, stderr) = handle.join().unwrap();
    let stderr = String::from_utf8(stderr).unwrap();
    assert_eq!(exit_code, 0, "{stderr}");
}

#[test]
fn serve_concurrent_loop_serves_two_9p_websockets() {
    let root = temp_dir("wanix-cli-serve-concurrent-9p");
    fs::write(root.join("hello.txt"), b"hello concurrent").unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let command = ServeCommand {
        root_path: root,
        addr: addr.to_string(),
        bundle: None,
        wanix_services: false,
        once: false,
    };

    let handle = thread::spawn(move || {
        let mut stderr = Vec::new();
        let exit_code =
            run_serve_with_listener_for_connections(command, listener, &mut stderr, 2).unwrap();
        (exit_code, stderr)
    });

    let mut first = connect_export9p_websocket(addr);
    let mut second = connect_export9p_websocket(addr);
    let requests = || {
        [
            p9_tversion(1, 8192, P9_VERSION_9P2000_L).unwrap(),
            p9_tattach(2, 1, 0xffff_ffff, "root", "", 0).unwrap(),
            p9_twalk(3, 1, 2, &["hello.txt"]).unwrap(),
            p9_tlopen(4, 2, 0),
            p9_tread(5, 2, 0, 16),
        ]
    };
    send_p9_requests(&mut first, requests());
    send_p9_requests(&mut second, requests());

    let first_frames = read_binary_frames(&mut first, 5);
    let second_frames = read_binary_frames(&mut second, 5);
    first.close(None).unwrap();
    second.close(None).unwrap();
    let (exit_code, stderr) = handle.join().unwrap();
    let stderr = String::from_utf8(stderr).unwrap();

    assert_eq!(exit_code, 0, "{stderr}");
    for frames in [&first_frames, &second_frames] {
        assert_eq!(
            frame_types(frames),
            [P9_RVERSION, P9_RATTACH, P9_RWALK, P9_RLOPEN, P9_RREAD]
        );
        assert_eq!(p9_decode_rread(&frames[4]).unwrap(), b"hello concurrent");
    }
}

#[test]
fn serve_once_exports_9p_on_well_known_export_path() {
    let root = temp_dir("wanix-cli-serve-export9p");
    fs::write(root.join("hello.txt"), b"hello export").unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let command = ServeCommand {
        root_path: root,
        addr: addr.to_string(),
        bundle: None,
        wanix_services: false,
        once: true,
    };

    let handle = thread::spawn(move || {
        let mut stderr = Vec::new();
        let exit_code = run_serve_with_listener(command, listener, &mut stderr).unwrap();
        (exit_code, stderr)
    });

    let mut socket = connect_export9p_websocket(addr);
    send_p9_requests(
        &mut socket,
        [
            p9_tversion(1, 8192, P9_VERSION_9P2000_L).unwrap(),
            p9_tattach(2, 1, 0xffff_ffff, "root", "", 0).unwrap(),
            p9_twalk(3, 1, 2, &["hello.txt"]).unwrap(),
            p9_tsetattr(
                4,
                2,
                P9_SETATTR_UID | P9_SETATTR_GID,
                &P9SetAttr {
                    uid: 1000,
                    gid: 1001,
                    ..P9SetAttr::default()
                },
            ),
            p9_tgetattr(5, 2, u64::MAX),
            p9_tlopen(6, 2, 0),
            p9_tread(7, 2, 0, 12),
        ],
    );

    let frames = read_binary_frames(&mut socket, 7);
    socket.close(None).unwrap();
    let (exit_code, _stderr) = handle.join().unwrap();

    assert_eq!(exit_code, 0);
    assert_eq!(
        frame_types(&frames),
        [
            P9_RVERSION,
            P9_RATTACH,
            P9_RWALK,
            P9_RSETATTR,
            P9_RGETATTR,
            P9_RLOPEN,
            P9_RREAD
        ]
    );
    let attr = p9_decode_rgetattr(&frames[4]).unwrap();
    assert_eq!(attr.uid, 1000);
    assert_eq!(attr.gid, 1001);
    assert_eq!(p9_decode_rread(&frames[6]).unwrap(), b"hello export");
}

#[test]
#[cfg(unix)]
fn serve_once_exports_symlink_readlink_over_direct_9p() {
    let root = temp_dir("wanix-cli-serve-export9p-symlink");
    fs::write(root.join("target.txt"), b"linked data").unwrap();
    let root_check = root.clone();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let command = ServeCommand {
        root_path: root,
        addr: addr.to_string(),
        bundle: None,
        wanix_services: false,
        once: true,
    };

    let handle = thread::spawn(move || {
        let mut stderr = Vec::new();
        let exit_code = run_serve_with_listener(command, listener, &mut stderr).unwrap();
        (exit_code, stderr)
    });

    let mut socket = connect_export9p_websocket(addr);
    send_p9_requests(
        &mut socket,
        [
            p9_tversion(1, 8192, P9_VERSION_9P2000_L).unwrap(),
            p9_tattach(2, 1, 0xffff_ffff, "root", "", 0).unwrap(),
            p9_tsymlink(3, 1, "link.txt", "target.txt", 0).unwrap(),
            p9_twalk(4, 1, 2, &["link.txt"]).unwrap(),
            p9_treadlink(5, 2),
            p9_tlopen(6, 2, 0),
            p9_tread(7, 2, 0, 11),
        ],
    );

    let frames = read_binary_frames(&mut socket, 7);
    socket.close(None).unwrap();
    let (exit_code, stderr) = handle.join().unwrap();
    let stderr = String::from_utf8(stderr).unwrap();

    assert_eq!(exit_code, 0, "{stderr}");
    assert_eq!(
        frame_types(&frames),
        [
            P9_RVERSION,
            P9_RATTACH,
            P9_RSYMLINK,
            P9_RWALK,
            P9_RREADLINK,
            P9_RLOPEN,
            P9_RREAD
        ]
    );
    assert_eq!(p9_decode_rreadlink(&frames[4]).unwrap(), "target.txt");
    assert_eq!(p9_decode_rread(&frames[6]).unwrap(), b"linked data");
    assert_eq!(
        fs::read_link(root_check.join("link.txt")).unwrap(),
        std::path::PathBuf::from("target.txt")
    );
}

#[test]
fn serve_once_exports_qjs_shell_terminal_websocket() {
    let root = temp_dir("wanix-cli-serve-qjs-shell-ws");
    fs::create_dir(root.join("app")).unwrap();
    fs::write(root.join("app").join("inside.txt"), b"inside app\n").unwrap();
    fs::write(
        root.join("foreground-child.js"),
        r##"import * as std from "qjs:std";
import * as os from "qjs:os";

const bytes = new Uint8Array(64);
const count = os.read(0, bytes.buffer, 0, bytes.length);
if (count < 0) {
  throw new Error("stdin read failed: " + count);
}
const text = Array.from(bytes.slice(0, count)).map((byte) => String.fromCharCode(byte)).join("");
std.out.puts("served foreground stdin " + text.trimEnd() + "\n");
std.exit(0);
"##,
    )
    .unwrap();
    fs::write(
        root.join("terminal-child.js"),
        r#"import * as std from "qjs:std";

std.out.puts("served terminal child stdout\n");
std.err.puts("served terminal child stderr\n");
std.exit(0);
"#,
    )
    .unwrap();
    fs::write(
        root.join("child.js"),
        r##"import * as std from "qjs:std";
import * as os from "qjs:os";

function readStdin() {
  const bytes = new Uint8Array(64);
  const count = os.read(0, bytes.buffer, 0, bytes.length);
  if (count < 0) {
    throw new Error("stdin read failed: " + count);
  }
  return Array.from(bytes.slice(0, count)).map((byte) => String.fromCharCode(byte)).join("");
}

std.out.puts("served child task " + std.loadFile("#task/self/id").trim() + "\n");
std.out.puts("served child cwd " + std.loadFile("#task/self/dir").trim() + "\n");
std.out.puts("served child argv " + scriptArgs.join("|") + "\n");
std.out.puts("served child stdin " + readStdin().trimEnd() + "\n");
std.out.puts("served child mode " + std.getenv("MODE") + "\n");
std.out.flush();
std.err.puts("served child stderr " + scriptArgs[1] + "\n");
std.err.flush();
std.exit(6);
"##,
    )
    .unwrap();
    fs::write(root.join("visible.txt"), b"served root").unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let command = ServeCommand {
        root_path: root,
        addr: addr.to_string(),
        bundle: Some(WORKBENCH_FS9P_BUNDLE.to_owned()),
        wanix_services: true,
        once: false,
    };

    let handle = thread::spawn(move || {
        let mut stderr = Vec::new();
        let exit_code =
            run_serve_with_listener_for_connections(command, listener, &mut stderr, 1).unwrap();
        (exit_code, stderr)
    });

    let mut socket = connect(format!("ws://{addr}/.well-known/qjs-shell?cwd=app"))
        .unwrap()
        .0;
    if let MaybeTlsStream::Plain(stream) = socket.get_mut() {
        stream
            .set_read_timeout(Some(Duration::from_secs(60)))
            .unwrap();
    }
    let initial = socket.read().unwrap();
    match initial {
        Message::Binary(bytes) => assert_eq!(bytes.as_ref(), b"shell task: 1\r\n$ "),
        other => panic!("expected initial terminal output, got {other:?}"),
    }

    socket
        .send(Message::binary(b"echo nope\x03echo yes\n".as_slice()))
        .unwrap();
    match socket.read().unwrap() {
        Message::Binary(bytes) => {
            assert_eq!(bytes.as_ref(), b"echo nope^C\r\n$ echo yes\r\nyes\r\n$ ")
        }
        other => panic!("expected Ctrl-C line cancellation output, got {other:?}"),
    }

    socket
        .send(Message::Text(
            r#"{"type":"resize","columns":100,"rows":40}"#.into(),
        ))
        .unwrap();
    socket
        .send(Message::binary(
            b"pwd\nls\ncat inside.txt\nstat inside.txt\nsize\nlater idle\n".as_slice(),
        ))
        .unwrap();
    match socket.read().unwrap() {
            Message::Binary(bytes) => assert_eq!(
                bytes.as_ref(),
                b"pwd\r\napp\r\n$ ls\r\ninside.txt\r\n$ cat inside.txt\r\ninside app\r\n$ stat inside.txt\r\ninside.txt type file mode 100000 size 11\r\n$ size\r\nsize 100 40\r\n$ later idle\r\nscheduled\r\n"
            ),
            other => panic!("expected scheduled terminal output, got {other:?}"),
        }
    match socket.read().unwrap() {
        Message::Binary(bytes) => assert_eq!(bytes.as_ref(), b"later: idle\r\n$ "),
        other => panic!("expected idle-pumped terminal output, got {other:?}"),
    }

    socket
        .send(Message::binary(
            b"qjs ../foreground-child.js\nserved foreground input\n".as_slice(),
        ))
        .unwrap();
    match socket.read().unwrap() {
        Message::Binary(bytes) => assert_eq!(
            bytes.as_ref(),
            b"qjs ../foreground-child.js\r\nserved foreground stdin served foreground input\r\n$ "
        ),
        other => panic!("expected foreground child terminal output, got {other:?}"),
    }

    socket
            .send(Message::binary(
                b"qjs ../terminal-child.js\nsetenv MODE served-mode\nqjs ../child.js from ws < inside.txt > child-out.txt 2> child-err.txt\nstatus\ncat child-out.txt\ncat child-err.txt\nexit\n".as_slice(),
            ))
            .unwrap();
    let mut transcript = Vec::new();
    let mut exit = None;
    while exit.is_none() {
        match socket.read().unwrap() {
            Message::Binary(bytes) => transcript.extend_from_slice(bytes.as_ref()),
            Message::Text(text) => exit = Some(text.to_string()),
            Message::Close(_) => break,
            Message::Ping(bytes) => socket.send(Message::Pong(bytes)).unwrap(),
            Message::Pong(_) | Message::Frame(_) => {}
        }
    }
    let _ = socket.close(None);
    let (exit_code, stderr) = handle.join().unwrap();
    let stderr = String::from_utf8(stderr).unwrap();

    assert_eq!(exit_code, 0, "{stderr}");
    assert_eq!(
            transcript,
            b"qjs ../terminal-child.js\r\nserved terminal child stdout\r\nserved terminal child stderr\r\n$ setenv MODE served-mode\r\n$ qjs ../child.js from ws < inside.txt > child-out.txt 2> child-err.txt\r\nqjs exit 6\r\n$ status\r\nstatus 6\r\n$ cat child-out.txt\r\nserved child task 4\r\nserved child cwd .\r\nserved child argv child.js|from|ws\r\nserved child stdin inside app\r\nserved child mode served-mode\r\n$ cat child-err.txt\r\nserved child stderr from\r\n$ exit\r\nbye\r\n"
        );
    assert_eq!(exit.as_deref(), Some("{\"type\":\"exit\",\"code\":0}"));
}

#[test]
fn qjs_shell_websocket_cwd_query_uses_wanix_paths() {
    assert_eq!(
        qjs_shell_cwd_from_target(Some("/.well-known/qjs-shell"))
            .map(|cwd| cwd.to_string())
            .unwrap_or_else(|_| panic!("default qjs shell cwd should parse")),
        "."
    );
    assert_eq!(
        qjs_shell_cwd_from_target(Some("/.well-known/qjs-shell?term=1"))
            .map(|cwd| cwd.to_string())
            .unwrap_or_else(|_| panic!("query without cwd should use default qjs shell cwd")),
        "."
    );
    assert_eq!(
        qjs_shell_cwd_from_target(Some("/.well-known/qjs-shell?cwd=%2Fapp%2Fsub"))
            .map(|cwd| cwd.to_string())
            .unwrap_or_else(|_| panic!("absolute qjs shell cwd should parse")),
        "app/sub"
    );
    assert_eq!(
        qjs_shell_cwd_from_target(Some("/.well-known/qjs-shell?cwd=%2F"))
            .map(|cwd| cwd.to_string())
            .unwrap_or_else(|_| panic!("root qjs shell cwd should parse")),
        "."
    );

    let response =
        qjs_shell_cwd_from_target(Some("/.well-known/qjs-shell?cwd=../escape")).unwrap_err();
    assert!(matches!(response.status, HttpStatus::BadRequest));
}

#[test]
fn qjs_shell_websocket_resize_messages_parse_json_and_text() {
    assert_eq!(
        parse_terminal_resize_message(r#"{"type":"resize","columns":100,"rows":40}"#),
        Some((100, 40))
    );
    assert_eq!(
        parse_terminal_resize_message("resize 120 50"),
        Some((120, 50))
    );
    assert_eq!(parse_terminal_resize_message("resize 0 50"), None);
}

#[test]
fn serve_once_exports_google_2_walkgetattr_on_well_known_path() {
    let root = temp_dir("wanix-cli-serve-export9p-google2");
    fs::write(root.join("hello.txt"), b"hello google2").unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let command = ServeCommand {
        root_path: root,
        addr: addr.to_string(),
        bundle: None,
        wanix_services: false,
        once: true,
    };

    let handle = thread::spawn(move || {
        let mut stderr = Vec::new();
        let exit_code = run_serve_with_listener(command, listener, &mut stderr).unwrap();
        (exit_code, stderr)
    });

    let mut socket = connect_export9p_websocket(addr);
    send_p9_requests(
        &mut socket,
        [
            p9_tversion(1, 8192, P9_VERSION_9P2000_L_GOOGLE_2).unwrap(),
            p9_tattach(2, 1, 0xffff_ffff, "root", "", 0).unwrap(),
            p9_twalkgetattr(3, 1, 2, &["hello.txt"]).unwrap(),
            p9_tlopen(4, 2, 0),
            p9_tread(5, 2, 0, 13),
        ],
    );

    let frames = read_binary_frames(&mut socket, 5);
    socket.close(None).unwrap();
    let (exit_code, _stderr) = handle.join().unwrap();

    assert_eq!(exit_code, 0);
    assert_eq!(
        frame_types(&frames),
        [
            P9_RVERSION,
            P9_RATTACH,
            P9_RWALKGETATTR,
            P9_RLOPEN,
            P9_RREAD
        ]
    );
    let walk = p9_decode_rwalkgetattr(&frames[2]).unwrap();
    assert_eq!(walk.valid, u64::MAX);
    assert_eq!(walk.qids.len(), 1);
    assert_eq!(walk.attr.size, 13);
    assert_eq!(p9_decode_rread(&frames[4]).unwrap(), b"hello google2");
}

#[test]
fn serve_once_exports_9p_compatibility_probes_on_well_known_path() {
    let root = temp_dir("wanix-cli-serve-export9p-compat-probes");
    fs::write(root.join("target.txt"), b"target").unwrap();
    let root_check = root.clone();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let command = ServeCommand {
        root_path: root,
        addr: addr.to_string(),
        bundle: None,
        wanix_services: false,
        once: true,
    };

    let handle = thread::spawn(move || {
        let mut stderr = Vec::new();
        let exit_code = run_serve_with_listener(command, listener, &mut stderr).unwrap();
        (exit_code, stderr)
    });

    let mut socket = connect_export9p_websocket(addr);
    send_p9_requests(
        &mut socket,
        [
            p9_tversion(1, 8192, P9_VERSION_9P2000_L).unwrap(),
            p9_tauth(2, 9, "root", "", 0).unwrap(),
            p9_tattach(3, 1, 0xffff_ffff, "root", "", 0).unwrap(),
            p9_twalk(4, 1, 2, &["target.txt"]).unwrap(),
            p9_tmknod(5, 1, "tty0", 0o020620, 4, 0, 0).unwrap(),
            p9_tlink(6, 1, 2, "hard.txt").unwrap(),
            p9_txattrwalk(7, 2, 3, "user.foo").unwrap(),
            p9_txattrcreate(8, 2, "user.foo", 12, 0).unwrap(),
            p9_trename(9, 2, 1, "renamed.txt").unwrap(),
            p9_tgetattr(10, 2, u64::MAX),
            p9_tremove(11, 2),
            p9_tgetattr(12, 2, u64::MAX),
        ],
    );

    let frames = read_binary_frames(&mut socket, 12);
    socket.close(None).unwrap();
    let (exit_code, _stderr) = handle.join().unwrap();

    assert_eq!(exit_code, 0);
    assert_eq!(
        frame_types(&frames),
        [
            P9_RVERSION,
            P9_RLERROR,
            P9_RATTACH,
            P9_RWALK,
            P9_RLERROR,
            P9_RLINK,
            P9_RLERROR,
            P9_RLERROR,
            P9_RRENAME,
            P9_RGETATTR,
            P9_RREMOVE,
            P9_RLERROR
        ]
    );
    assert_eq!(
        frames.iter().map(P9Frame::tag).collect::<Vec<_>>(),
        vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]
    );
    assert_eq!(p9_decode_rlerror(&frames[1]).unwrap().ecode, ENOSYS);
    assert_eq!(p9_decode_rlerror(&frames[4]).unwrap().ecode, EOPNOTSUPP);
    p9_decode_rlink(&frames[5]).unwrap();
    assert_eq!(p9_decode_rlerror(&frames[6]).unwrap().ecode, EOPNOTSUPP);
    assert_eq!(p9_decode_rlerror(&frames[7]).unwrap().ecode, EOPNOTSUPP);
    p9_decode_rrename(&frames[8]).unwrap();
    assert_eq!(p9_decode_rgetattr(&frames[9]).unwrap().size, 6);
    p9_decode_rremove(&frames[10]).unwrap();
    assert_eq!(p9_decode_rlerror(&frames[11]).unwrap().ecode, EBADF);
    assert!(!root_check.join("target.txt").exists());
    assert!(!root_check.join("renamed.txt").exists());
    assert_eq!(fs::read(root_check.join("hard.txt")).unwrap(), b"target");
}

#[test]
fn serve_once_rejects_reserved_ethernet_websocket_path() {
    let root = temp_dir("wanix-cli-serve-ethernet");
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let command = ServeCommand {
        root_path: root,
        addr: addr.to_string(),
        bundle: None,
        wanix_services: false,
        once: true,
    };

    let handle = thread::spawn(move || {
        let mut stderr = Vec::new();
        let exit_code = run_serve_with_listener(command, listener, &mut stderr).unwrap();
        (exit_code, stderr)
    });

    let response = http_request(
        addr,
        b"GET /.well-known/ethernet HTTP/1.1\r\n\
              Host: localhost\r\n\
              Connection: Upgrade\r\n\
              Upgrade: websocket\r\n\
              Sec-WebSocket-Version: 13\r\n\
              Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
              \r\n",
    );
    let (exit_code, _stderr) = handle.join().unwrap();

    assert_eq!(exit_code, 0);
    let response = String::from_utf8(response).unwrap();
    assert!(
        response.starts_with("HTTP/1.1 501 Not Implemented\r\n"),
        "{response}"
    );
    assert!(
        response.contains("ethernet websocket bridge is not implemented"),
        "{response}"
    );
}

#[test]
fn serve_http_keeps_well_known_routes_reserved() {
    let root = temp_dir("wanix-cli-serve-well-known-http");
    fs::create_dir(root.join(".well-known")).unwrap();
    fs::write(root.join(".well-known").join("export9p"), b"not static").unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let command = ServeCommand {
        root_path: root,
        addr: addr.to_string(),
        bundle: None,
        wanix_services: false,
        once: true,
    };

    let handle = thread::spawn(move || {
        let mut stderr = Vec::new();
        let exit_code = run_serve_with_listener(command, listener, &mut stderr).unwrap();
        (exit_code, stderr)
    });

    let response = http_request(
        addr,
        b"GET /.well-known/export9p HTTP/1.1\r\nHost: localhost\r\n\r\n",
    );
    let (exit_code, _stderr) = handle.join().unwrap();

    assert_eq!(exit_code, 0);
    let response = String::from_utf8(response).unwrap();
    assert!(
        response.starts_with("HTTP/1.1 400 Bad Request\r\n"),
        "{response}"
    );
    assert!(
        response.contains("websocket upgrade required"),
        "{response}"
    );
    assert!(!response.contains("not static"), "{response}");
}

#[test]
fn serve_http_keeps_qjs_shell_route_reserved_when_services_enabled() {
    let root = temp_dir("wanix-cli-serve-qjs-shell-http");
    fs::create_dir(root.join(".well-known")).unwrap();
    fs::write(root.join(".well-known").join("qjs-shell"), b"not static").unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let command = ServeCommand {
        root_path: root,
        addr: addr.to_string(),
        bundle: Some(WORKBENCH_FS9P_BUNDLE.to_owned()),
        wanix_services: true,
        once: true,
    };

    let handle = thread::spawn(move || {
        let mut stderr = Vec::new();
        let exit_code = run_serve_with_listener(command, listener, &mut stderr).unwrap();
        (exit_code, stderr)
    });

    let response = http_request(
        addr,
        b"GET /.well-known/qjs-shell HTTP/1.1\r\nHost: localhost\r\n\r\n",
    );
    let (exit_code, _stderr) = handle.join().unwrap();

    assert_eq!(exit_code, 0);
    let response = String::from_utf8(response).unwrap();
    assert!(
        response.starts_with("HTTP/1.1 400 Bad Request\r\n"),
        "{response}"
    );
    assert!(
        response.contains("websocket upgrade required"),
        "{response}"
    );
    assert!(!response.contains("not static"), "{response}");
}

#[derive(Copy, Clone)]
struct P9TestFile<'a> {
    root_fid: u32,
    fid: u32,
    tag_base: u16,
    path: &'a [&'a str],
}

fn p9_file<'a>(root_fid: u32, fid: u32, tag_base: u16, path: &'a [&'a str]) -> P9TestFile<'a> {
    P9TestFile {
        root_fid,
        fid,
        tag_base,
        path,
    }
}

fn p9_walk_open_file(server: &mut wanix_9p::P9Server, file: P9TestFile<'_>, mode: u32) {
    let response = server
        .handle_frame(&p9_twalk(file.tag_base, file.root_fid, file.fid, file.path).unwrap())
        .unwrap();
    assert_eq!(response.message_type(), P9_RWALK);
    assert_eq!(p9_decode_rwalk(&response).unwrap().len(), file.path.len());
    assert_eq!(
        server
            .handle_frame(&p9_tlopen(file.tag_base + 1, file.fid, mode))
            .unwrap()
            .message_type(),
        P9_RLOPEN
    );
}

fn p9_read_file(server: &mut wanix_9p::P9Server, file: P9TestFile<'_>) -> Vec<u8> {
    p9_walk_open_file(server, file, 0);
    let response = server
        .handle_frame(&p9_tread(file.tag_base + 2, file.fid, 0, 4096))
        .unwrap();
    assert_eq!(response.message_type(), P9_RREAD);
    p9_decode_rread(&response).unwrap()
}

fn p9_write_file(server: &mut wanix_9p::P9Server, file: P9TestFile<'_>, bytes: &[u8]) {
    p9_walk_open_file(server, file, P9_O_WRONLY);
    let response = server
        .handle_frame(&p9_twrite(file.tag_base + 2, file.fid, 0, bytes).unwrap())
        .unwrap();
    assert_eq!(response.message_type(), P9_RWRITE);
    assert_eq!(p9_decode_rwrite(&response).unwrap(), bytes.len() as u32);
}

fn read_binary_frames<S: Read + Write>(
    socket: &mut tungstenite::WebSocket<S>,
    count: usize,
) -> Vec<P9Frame> {
    let mut frames = Vec::new();
    while frames.len() < count {
        if let Message::Binary(bytes) = socket.read().unwrap() {
            frames.push(P9Frame::decode(&bytes).unwrap());
        }
    }
    frames
}

fn connect_export9p_websocket(
    addr: std::net::SocketAddr,
) -> tungstenite::WebSocket<MaybeTlsStream<TcpStream>> {
    connect(format!("ws://{addr}/.well-known/export9p"))
        .unwrap()
        .0
}

fn send_p9_requests<S: Read + Write, const N: usize>(
    socket: &mut tungstenite::WebSocket<S>,
    frames: [P9Frame; N],
) {
    socket
        .send(Message::binary(request_stream(frames)))
        .unwrap();
}

fn request_stream<const N: usize>(frames: [P9Frame; N]) -> Vec<u8> {
    let mut stream = Vec::new();
    for frame in frames {
        stream.extend_from_slice(&frame.encode().unwrap());
    }
    stream
}

fn frame_types(frames: &[P9Frame]) -> Vec<u8> {
    frames.iter().map(P9Frame::message_type).collect()
}

fn http_request(addr: std::net::SocketAddr, request: &[u8]) -> Vec<u8> {
    let mut stream = TcpStream::connect(addr).unwrap();
    stream.write_all(request).unwrap();
    let mut response = Vec::new();
    match stream.read_to_end(&mut response) {
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::ConnectionReset && !response.is_empty() => {}
        Err(error) => panic!("failed to read HTTP response: {error}"),
    }
    response
}

fn http_response_parts(response: &[u8]) -> (String, &[u8]) {
    let header_end = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .unwrap()
        + 4;
    (
        String::from_utf8_lossy(&response[..header_end]).into_owned(),
        &response[header_end..],
    )
}

fn temp_dir(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("{name}-{}-{nanos}", std::process::id()));
    fs::create_dir_all(&path).unwrap();
    path
}

fn write_boot_init(root: &Path) {
    let bin = root.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let init = bin.join("init");
    fs::write(&init, b"init").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        fs::set_permissions(init, fs::Permissions::from_mode(0o755)).unwrap();
    }
}
