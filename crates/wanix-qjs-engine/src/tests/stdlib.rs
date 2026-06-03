use super::*;
use anyhow::Result;
use std::sync::{Arc, Mutex};

#[test]
fn fixture_exposes_quickjs_std_and_os_modules() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;

    vm.eval_module_discard(
        r#"
        import * as std from "qjs:std";
        import * as os from "qjs:os";
        globalThis.qjsStdOutPuts = typeof std.out.puts;
        globalThis.qjsOsOpen = typeof os.open;
        "#,
        "stdlib-modules.mjs",
    )?;

    assert_eq!(vm.eval_string("qjsStdOutPuts")?, "function");
    assert_eq!(vm.eval_string("qjsOsOpen")?, "function");
    Ok(())
}

#[test]
fn quickjs_std_stdout_uses_wasi_capture() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let config = QuickJsHostConfig::new().with_stdout_capture(true);
    let mut vm = QuickJsRuntime::create_with_host_config(&engine, &module, config)?;

    vm.eval_module_discard(
        r#"
        import * as std from "qjs:std";
        std.out.puts("via qjs std\n");
        std.out.flush();
        "#,
        "stdlib-stdout.mjs",
    )?;

    assert_eq!(vm.take_captured_stdout(), b"via qjs std\n");
    Ok(())
}

#[test]
fn quickjs_std_stdout_reattaches_capture_after_restore() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;
    vm.eval_module_discard(
        r#"
        import * as std from "qjs:std";
        globalThis.stdReadyBeforeSnapshot = typeof std.out.puts;
        "#,
        "stdlib-before-snapshot.mjs",
    )?;
    assert_eq!(vm.eval_string("stdReadyBeforeSnapshot")?, "function");
    let snapshot = vm.snapshot()?;
    drop(vm);

    let config = QuickJsHostConfig::new().with_stdout_capture(true);
    let mut restored =
        QuickJsRuntime::restore_with_host_config(&engine, &module, &snapshot, config)?;
    restored.eval_module_discard(
        r#"
        import * as std from "qjs:std";
        std.out.puts("after restore\n");
        std.out.flush();
        "#,
        "stdlib-after-restore.mjs",
    )?;

    assert_eq!(restored.take_captured_stdout(), b"after restore\n");
    Ok(())
}

#[test]
fn quickjs_std_import_coexists_with_rust_module_loader() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;
    let normalized = Arc::new(Mutex::new(Vec::new()));
    let normalized_for_loader = Arc::clone(&normalized);

    vm.set_module_loader_with_normalizer(
        move |_base_name, specifier| {
            normalized_for_loader
                .lock()
                .expect("test normalizer lock")
                .push(specifier.to_owned());
            match specifier {
                "./lib.js" => Ok("lib.js".to_owned()),
                other => Ok(other.to_owned()),
            }
        },
        |name| match name {
            "lib.js" => Ok("export const message = 'from rust loader';".to_owned()),
            other => anyhow::bail!("unexpected module load {other}"),
        },
    )?;
    vm.eval_module_discard(
        r#"
        import * as std from "qjs:std";
        import { message } from "./lib.js";
        globalThis.loaderMessage = message;
        globalThis.loaderStdOutPuts = typeof std.out.puts;
        "#,
        "stdlib-loader.mjs",
    )?;

    assert_eq!(vm.eval_string("loaderMessage")?, "from rust loader");
    assert_eq!(vm.eval_string("loaderStdOutPuts")?, "function");
    assert_eq!(
        normalized.lock().expect("test normalizer lock").as_slice(),
        &["./lib.js".to_owned()]
    );
    Ok(())
}
