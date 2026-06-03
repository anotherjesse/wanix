use anyhow::Result;
use rust_wasi_quickjs::QuickJsModule;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use wasmtime::Engine;

fn main() -> Result<()> {
    let engine = Engine::default();
    let module = QuickJsModule::from_file(&engine, "fixtures/quickjs.wasm")?;
    let mut runtime = module.create_runtime()?;

    runtime.eval_discard("globalThis.limitState = 'snapshotted'")?;
    let snapshot_bytes = runtime.snapshot()?.try_to_bytes()?;
    let mut restored = module.restore_runtime_from_bytes(&snapshot_bytes)?;

    restored.set_memory_limit(8 * 1024 * 1024)?;
    restored.set_max_stack_size(1024 * 1024)?;

    let interrupt_polls = Arc::new(AtomicUsize::new(0));
    let polls_for_handler = Arc::clone(&interrupt_polls);
    restored.set_interrupt_handler(move || polls_for_handler.fetch_add(1, Ordering::SeqCst) > 0)?;

    let err = restored
        .eval_discard("while (true) {}")
        .expect_err("interrupt handler should stop the infinite loop");
    assert!(format!("{err:#}").contains("interrupted"));

    println!(
        "runtime limits restored: state={} interrupt_polls={}",
        restored.eval_string("limitState")?,
        interrupt_polls.load(Ordering::SeqCst)
    );
    Ok(())
}
