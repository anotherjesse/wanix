use super::fault::CleanupFault;
use crate::{QuickJsHostConfig, QuickJsModule, QuickJsRuntime};
use anyhow::Result;
use wasmtime::Engine;

const CLEANUP_RUNTIME_WAT: &str = include_str!("cleanup_runtime.wat");

#[derive(Clone, Copy)]
pub(super) enum CleanupEvent {
    GuestFree(u32),
    BytecodeFree(u32),
    CStringFree(u32),
    ValueFree(CleanupValue),
    JobRun,
}

impl CleanupEvent {
    fn push_log_line(self, log: &mut String) {
        match self {
            Self::GuestFree(ptr) => push_log_line(log, "G", &ptr.to_string()),
            Self::BytecodeFree(ptr) => push_log_line(log, "B", &ptr.to_string()),
            Self::CStringFree(ptr) => push_log_line(log, "C", &ptr.to_string()),
            Self::ValueFree(value) => push_log_line(log, "V", value.log_label()),
            Self::JobRun => log.push_str("J:run\n"),
        }
    }
}

#[derive(Clone, Copy)]
pub(super) enum CleanupValue {
    Eval,
    Global,
    Function,
    Undefined,
    StringValue,
    BigInt,
    Call,
    Exception,
}

impl CleanupValue {
    fn log_label(self) -> &'static str {
        match self {
            Self::Eval => "eval",
            Self::Global => "global",
            Self::Function => "function",
            Self::Undefined => "undefined",
            Self::StringValue => "string",
            Self::BigInt => "bigint",
            Self::Call => "call",
            Self::Exception => "exception",
        }
    }
}

pub(super) fn cleanup_runtime() -> Result<QuickJsRuntime> {
    cleanup_runtime_from_wat(CLEANUP_RUNTIME_WAT)
}

pub(super) fn cleanup_runtime_with_fault(fault: CleanupFault) -> Result<QuickJsRuntime> {
    cleanup_runtime_with_faults(&[fault])
}

pub(super) fn cleanup_runtime_with_faults(faults: &[CleanupFault]) -> Result<QuickJsRuntime> {
    let replacements: Vec<_> = faults.iter().map(|fault| fault.replacement()).collect();
    cleanup_runtime_with_replacements(&replacements)
}

fn cleanup_runtime_with_replacements(replacements: &[(&str, &str)]) -> Result<QuickJsRuntime> {
    let mut wat = CLEANUP_RUNTIME_WAT.to_string();

    for (from, to) in replacements {
        let replaced = wat.replacen(from, to, 1);
        assert_ne!(replaced, wat, "fixture replacement should match");
        wat = replaced;
    }

    cleanup_runtime_from_wat(&wat)
}

pub(super) fn expect_cleanup_log(vm: &mut QuickJsRuntime, expected: &[CleanupEvent]) -> Result<()> {
    let actual = take_cleanup_log(vm)?;
    let mut expected_log = String::new();
    for event in expected {
        event.push_log_line(&mut expected_log);
    }
    assert_eq!(actual, expected_log);
    Ok(())
}

fn take_cleanup_log(vm: &mut QuickJsRuntime) -> Result<String> {
    Ok(String::from_utf8(vm.take_captured_stdout())?)
}

fn push_log_line(log: &mut String, prefix: &str, value: &str) {
    log.push_str(prefix);
    log.push(':');
    log.push_str(value);
    log.push('\n');
}

fn cleanup_runtime_from_wat(wat: &str) -> Result<QuickJsRuntime> {
    let engine = Engine::default();
    let module = QuickJsModule::from_bytes(&engine, wat.as_bytes())?;
    QuickJsRuntime::create_with_host_config(
        &engine,
        &module,
        QuickJsHostConfig::new().with_stdout_capture(true),
    )
}
