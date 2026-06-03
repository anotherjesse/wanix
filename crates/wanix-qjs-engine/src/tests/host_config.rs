use super::*;

#[test]
fn default_host_config_matches_current_behavior() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;

    assert_eq!(vm.eval_number("Date.now()")?, 1_700_000_000_000.0);
    assert_eq!(vm.eval_number("new Date(0).getTimezoneOffset()")?, 0.0);
    Ok(())
}

#[test]
fn host_config_controls_clock_and_timezone() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let config = QuickJsHostConfig::new()
        .with_clock_time_ns(1_700_000_123_000_000_000)
        .with_timezone_offset_seconds(3_600)
        .with_random_byte(0x7a)
        .with_stdout_capture(true)
        .with_stderr_capture(true);
    assert_eq!(config.clock_time_ns(), 1_700_000_123_000_000_000);
    assert_eq!(config.timezone_offset_seconds(), 3_600);
    assert_eq!(config.random_byte(), 0x7a);
    assert!(config.captures_stdout());
    assert!(config.captures_stderr());

    let mut vm = QuickJsRuntime::create_with_host_config(&engine, &module, config)?;
    assert_eq!(vm.eval_number("Date.now()")?, 1_700_000_123_000.0);
    assert_eq!(vm.eval_number("new Date(0).getTimezoneOffset()")?, -60.0);
    Ok(())
}

#[test]
fn host_config_controls_stdio_capture_limits() {
    assert_eq!(QuickJsHostConfig::new().stdout_capture_byte_limit(), None);
    assert_eq!(QuickJsHostConfig::new().stderr_capture_byte_limit(), None);

    let config = QuickJsHostConfig::new()
        .with_limited_stdout_capture(128)
        .with_limited_stderr_capture(64);
    assert!(config.captures_stdout());
    assert!(config.captures_stderr());
    assert_eq!(config.stdout_capture_byte_limit(), Some(128));
    assert_eq!(config.stderr_capture_byte_limit(), Some(64));

    let config = QuickJsHostConfig::new()
        .with_stdout_capture(true)
        .with_stdout_capture_byte_limit(Some(32))
        .with_stdout_capture_byte_limit(None)
        .with_stderr_capture_byte_limit(Some(16));
    assert!(config.captures_stdout());
    assert!(!config.captures_stderr());
    assert_eq!(config.stdout_capture_byte_limit(), None);
    assert_eq!(config.stderr_capture_byte_limit(), Some(16));

    let config = QuickJsHostConfig::new().with_limited_stdio_capture(8);
    assert!(config.captures_stdout());
    assert!(config.captures_stderr());
    assert_eq!(config.stdout_capture_byte_limit(), Some(8));
    assert_eq!(config.stderr_capture_byte_limit(), Some(8));
}

#[test]
fn host_config_debug_redacts_virtual_file_contents() -> Result<()> {
    let config = QuickJsHostConfig::new()
        .with_clock_time_ns(123)
        .with_read_only_virtual_file("/secret/policy.json", b"SUPER_SECRET_SENTINEL")?;

    let debug = format!("{config:?}");

    assert!(debug.contains("QuickJsHostConfig"));
    assert!(debug.contains("clock_time_ns: 123"));
    assert!(debug.contains("read_only_virtual_file_count: 1"));
    assert!(!debug.contains("SUPER_SECRET_SENTINEL"));
    assert!(!debug.contains("policy.json"));
    Ok(())
}

#[test]
fn restore_reattaches_host_config() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let create_config = QuickJsHostConfig::new().with_clock_time_ns(1_700_000_000_000_000_000);
    let restore_config = QuickJsHostConfig::new()
        .with_clock_time_ns(1_800_000_000_000_000_000)
        .with_timezone_offset_seconds(-7_200);

    let mut vm = QuickJsRuntime::create_with_host_config(&engine, &module, create_config)?;
    vm.eval_discard("globalThis.hostConfigMarker = 'snapshotted'")?;
    assert_eq!(vm.eval_number("Date.now()")?, 1_700_000_000_000.0);

    let snapshot = vm.snapshot()?;
    drop(vm);

    let mut restored =
        QuickJsRuntime::restore_with_host_config(&engine, &module, &snapshot, restore_config)?;
    assert_eq!(restored.eval_string("hostConfigMarker")?, "snapshotted");
    assert_eq!(restored.eval_number("Date.now()")?, 1_800_000_000_000.0);
    assert_eq!(
        restored.eval_number("new Date(0).getTimezoneOffset()")?,
        120.0
    );
    Ok(())
}

#[test]
fn module_owned_helpers_reattach_host_config_from_bytes() -> Result<()> {
    let (_engine, module) = quickjs_fixture()?;
    let create_config = QuickJsHostConfig::new().with_clock_time_ns(1_700_000_111_000_000_000);
    let restore_config = QuickJsHostConfig::new()
        .with_clock_time_ns(1_900_000_222_000_000_000)
        .with_timezone_offset_seconds(5_400);

    let mut vm = module.create_runtime_with_host_config(create_config)?;
    vm.eval_discard("globalThis.moduleHostConfigMarker = 'snapshotted by module'")?;
    assert_eq!(vm.eval_number("Date.now()")?, 1_700_000_111_000.0);

    let bytes = vm.snapshot()?.try_to_bytes()?;
    drop(vm);

    let mut restored =
        module.restore_runtime_from_bytes_with_host_config(&bytes, restore_config)?;
    assert_eq!(
        restored.eval_string("moduleHostConfigMarker")?,
        "snapshotted by module"
    );
    assert_eq!(restored.eval_number("Date.now()")?, 1_900_000_222_000.0);
    assert_eq!(
        restored.eval_number("new Date(0).getTimezoneOffset()")?,
        -90.0
    );
    Ok(())
}
