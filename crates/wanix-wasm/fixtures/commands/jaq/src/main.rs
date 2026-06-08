//! A small jq-clone (`jaq`) compiled to wasm32-wasip1 as a Wanix command fixture.
//!
//! Usage: `jaq <filter>` reads ONE JSON document from stdin, applies the jq
//! filter, and writes each output value as compact JSON (jq -c semantics), one
//! per line, to stdout.
//!
//! Exit codes:
//!   2  missing filter argument
//!   3  could not parse stdin as a single JSON document
//!   4  could not load/compile the filter
//!   5  runtime error while running the filter

use std::io::{self, Read, Write};
use std::process::ExitCode;

use jaq_core::load::{Arena, File, Loader};
use jaq_core::{data, unwrap_valr, Compiler, Ctx, Vars};
use jaq_json::Val;

fn main() -> ExitCode {
    // argv[1] is the filter string.
    let filter_src = match std::env::args().nth(1) {
        Some(s) => s,
        None => {
            eprintln!("usage: jaq <filter>   (reads one JSON document from stdin)");
            return ExitCode::from(2);
        }
    };

    // Read all of stdin to bytes and parse exactly one JSON document.
    let mut input_bytes = Vec::new();
    if let Err(e) = io::stdin().read_to_end(&mut input_bytes) {
        eprintln!("jaq: error reading stdin: {e}");
        return ExitCode::from(3);
    }
    let input = match jaq_json::read::parse_single(&input_bytes) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("jaq: invalid JSON input: {e}");
            return ExitCode::from(3);
        }
    };

    // Named filter definitions (`map`, `keys`, ...) from all three crates.
    let defs = jaq_core::defs()
        .chain(jaq_std::defs())
        .chain(jaq_json::defs());
    // Native functions; pin DataT to JustLut<Val> so the chained iterators
    // infer the same D and `with_funs` accepts them.
    let funs = jaq_core::funs::<data::JustLut<Val>>()
        .chain(jaq_std::funs())
        .chain(jaq_json::funs());

    let arena = Arena::default();
    let program = File {
        code: filter_src.as_str(),
        path: (),
    };

    let modules = match Loader::new(defs).load(&arena, program) {
        Ok(m) => m,
        Err(errs) => {
            eprintln!("jaq: failed to load filter: {errs:?}");
            return ExitCode::from(4);
        }
    };

    let filter = match Compiler::default().with_funs(funs).compile(modules) {
        Ok(f) => f,
        Err(errs) => {
            eprintln!("jaq: failed to compile filter: {errs:?}");
            return ExitCode::from(4);
        }
    };

    let ctx = Ctx::<data::JustLut<Val>>::new(&filter.lut, Vars::new([]));

    let stdout = io::stdout();
    let mut out = stdout.lock();
    for result in filter.id.run((ctx, input)).map(unwrap_valr) {
        match result {
            Ok(val) => {
                if let Err(e) = writeln!(out, "{val}") {
                    eprintln!("jaq: error writing output: {e}");
                    return ExitCode::from(5);
                }
            }
            Err(e) => {
                eprintln!("jaq: runtime error: {e}");
                return ExitCode::from(5);
            }
        }
    }

    ExitCode::SUCCESS
}
