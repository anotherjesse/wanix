//! `wanix new` project scaffolding: emit a working qjs or rust-wasm starter
//! project (hello-world program, README, and — for rust — a standalone
//! Cargo.toml + `.cargo/config.toml` targeting `wasm32-wasip1`).

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use crate::{CliError, CliOutput};

#[cfg(test)]
mod tests;

const JS_MAIN: &str = "import * as std from \"qjs:std\";\n\
import * as fs from \"lib/wanix/fs.js\";\n\
import { print, args } from \"lib/wanix/process.js\";\n\
\n\
print(\"hello from a Wanix qjs task\");\n\
print(\"args: \" + JSON.stringify(args()));\n\
\n\
// Write then read a file back through the SDK fs helpers (Wanix-backed WASI).\n\
fs.createText(\"hello.txt\", \"hello world\\n\");\n\
std.out.puts(\"hello.txt = \" + fs.readText(\"hello.txt\"));\n\
std.out.flush();\n";

const JS_README: &str = "# {{name}} (Wanix qjs task)\n\n\
Run with the Wanix CLI:\n\n\
    wanix-rust qjs main.js\n\n\
`main.js` uses the `qjs:std`/`qjs:os` runtime and the Wanix guest SDK in\n\
`lib/wanix/`. Type definitions live in `lib/wanix/*.d.ts`; `tsconfig.json`\n\
points an editor's TypeScript service at them.\n";

const RUST_MAIN: &str = "//! Hello-world Wanix wasm task: read an input file and write an uppercased\n\
//! copy, all through std::fs (Wanix-backed WASI syscalls).\n\
\n\
fn main() {\n\
    let args: Vec<String> = std::env::args().collect();\n\
    let input = args.get(1).cloned().unwrap_or_else(|| \"/in.txt\".to_string());\n\
    let output = args.get(2).cloned().unwrap_or_else(|| \"/out.txt\".to_string());\n\
    let contents =\n\
        std::fs::read_to_string(&input).unwrap_or_else(|_| \"hello world\\n\".to_string());\n\
    std::fs::write(&output, contents.to_uppercase()).expect(\"write output\");\n\
    println!(\"wrote {input} -> {output}\");\n\
}\n";

const RUST_CARGO_TOML: &str = "[package]\n\
name = \"{{name}}\"\n\
version = \"0.1.0\"\n\
edition = \"2021\"\n\
\n\
# Standalone workspace so this project detaches from any parent Cargo workspace.\n\
[workspace]\n\
\n\
[[bin]]\n\
name = \"{{name}}\"\n\
path = \"src/main.rs\"\n\
\n\
[profile.release]\n\
opt-level = \"s\"\n\
strip = true\n";

const RUST_CONFIG: &str = "[build]\n\
target = \"wasm32-wasip1\"\n";

const RUST_README: &str = "# {{name}} (Wanix wasm task)\n\n\
Build, then run with the Wanix CLI:\n\n\
    cargo build --release --target wasm32-wasip1\n\
    wanix-rust wasm target/wasm32-wasip1/release/{{name}}.wasm /in.txt /out.txt\n\n\
The default target is `wasm32-wasip1` (see `.cargo/config.toml`). If the target\n\
is missing: `rustup target add wasm32-wasip1`.\n";

const TSCONFIG: &str = include_str!("../../../examples/tsconfig.wanix-qjs.json");
const SDK_INDEX: &str = include_str!("../../../examples/lib/wanix/index.js");
const SDK_BYTES: &str = include_str!("../../../examples/lib/wanix/bytes.js");
const SDK_FS: &str = include_str!("../../../examples/lib/wanix/fs.js");
const SDK_PROCESS: &str = include_str!("../../../examples/lib/wanix/process.js");
const SDK_TASK: &str = include_str!("../../../examples/lib/wanix/task.js");
const SDK_DTS_SDK: &str = include_str!("../../../examples/lib/wanix/wanix-sdk.d.ts");
const SDK_DTS_QJS: &str = include_str!("../../../examples/lib/wanix/wanix-qjs.d.ts");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NewKind {
    Js,
    Rust,
}

/// A parsed `new` command: template kind, project name, and target parent dir.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct NewCommand {
    kind: NewKind,
    name: String,
    parent: PathBuf,
}

/// Parses `new (--js NAME | --rust NAME) [--dir DIR]`.
///
/// # Errors
///
/// Returns a usage error when the kind is missing or doubled, the name is
/// missing/invalid, or an unknown option is supplied.
pub(super) fn parse_new_command(args: &[OsString]) -> Result<NewCommand, CliError> {
    let mut kind: Option<NewKind> = None;
    let mut name: Option<String> = None;
    let mut parent: Option<PathBuf> = None;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        let arg = arg
            .to_str()
            .ok_or_else(|| CliError::usage("new: arguments must be valid UTF-8"))?;
        match arg {
            "--js" => set_kind(&mut kind, NewKind::Js)?,
            "--rust" => set_kind(&mut kind, NewKind::Rust)?,
            "--dir" => {
                let value = iter
                    .next()
                    .ok_or_else(|| CliError::usage("new: --dir requires a directory"))?;
                parent = Some(PathBuf::from(value));
            }
            other if other.starts_with('-') => {
                return Err(CliError::usage(format!("new: unknown option {other}")));
            }
            other => {
                if name.replace(other.to_owned()).is_some() {
                    return Err(CliError::usage("new: expected exactly one project NAME"));
                }
            }
        }
    }
    let kind = kind.ok_or_else(|| CliError::usage("new: specify --js NAME or --rust NAME"))?;
    let name = name.ok_or_else(|| CliError::usage("new: missing project NAME"))?;
    if name.is_empty() || name.contains('/') || name.contains('\\') {
        return Err(CliError::usage(
            "new: NAME must not contain path separators",
        ));
    }
    Ok(NewCommand {
        kind,
        name,
        parent: parent.unwrap_or_else(|| PathBuf::from(".")),
    })
}

fn set_kind(slot: &mut Option<NewKind>, kind: NewKind) -> Result<(), CliError> {
    if slot.replace(kind).is_some() {
        return Err(CliError::usage("new: specify only one of --js or --rust"));
    }
    Ok(())
}

/// Writes the starter project to `<parent>/<name>` and reports the next step.
///
/// # Errors
///
/// Returns an error when the target directory is non-empty or a file cannot be
/// written.
pub(super) fn run_new_command(command: NewCommand) -> Result<CliOutput, CliError> {
    let root = command.parent.join(&command.name);
    ensure_empty_target(&root)?;
    let files = match command.kind {
        NewKind::Js => js_files(),
        NewKind::Rust => rust_files(&command.name),
    };
    for (relpath, content) in files {
        write_project_file(&root, relpath, &content)?;
    }
    let shown = root.display();
    let stdout = match command.kind {
        NewKind::Js => {
            format!("created js project {shown}\nnext: wanix-rust qjs {shown}/main.js\n")
        }
        NewKind::Rust => format!(
            "created rust project {shown}\nnext: cd {shown} && cargo build --release --target wasm32-wasip1\n"
        ),
    };
    Ok(CliOutput::new(stdout.into_bytes(), Vec::new(), 0))
}

fn js_files() -> Vec<(&'static str, String)> {
    vec![
        ("main.js", JS_MAIN.to_owned()),
        ("README.md", render(JS_README, "")),
        ("tsconfig.json", TSCONFIG.to_owned()),
        ("lib/wanix/index.js", SDK_INDEX.to_owned()),
        ("lib/wanix/bytes.js", SDK_BYTES.to_owned()),
        ("lib/wanix/fs.js", SDK_FS.to_owned()),
        ("lib/wanix/process.js", SDK_PROCESS.to_owned()),
        ("lib/wanix/task.js", SDK_TASK.to_owned()),
        ("lib/wanix/wanix-sdk.d.ts", SDK_DTS_SDK.to_owned()),
        ("lib/wanix/wanix-qjs.d.ts", SDK_DTS_QJS.to_owned()),
    ]
}

fn rust_files(name: &str) -> Vec<(&'static str, String)> {
    vec![
        ("Cargo.toml", render(RUST_CARGO_TOML, name)),
        (".cargo/config.toml", RUST_CONFIG.to_owned()),
        ("src/main.rs", RUST_MAIN.to_owned()),
        ("README.md", render(RUST_README, name)),
    ]
}

fn render(template: &str, name: &str) -> String {
    template.replace("{{name}}", name)
}

fn ensure_empty_target(root: &Path) -> Result<(), CliError> {
    if !root.exists() {
        return Ok(());
    }
    let empty = root.is_dir()
        && fs::read_dir(root)
            .map_err(|error| CliError::new(format!("new: read {}: {error}", root.display()), 1))?
            .next()
            .is_none();
    if empty {
        return Ok(());
    }
    Err(CliError::new(
        format!("new: target {} must be empty", root.display()),
        1,
    ))
}

fn write_project_file(root: &Path, relpath: &str, content: &str) -> Result<(), CliError> {
    let path = root.join(relpath);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            CliError::new(format!("new: create {}: {error}", parent.display()), 1)
        })?;
    }
    fs::write(&path, content)
        .map_err(|error| CliError::new(format!("new: write {}: {error}", path.display()), 1))
}
