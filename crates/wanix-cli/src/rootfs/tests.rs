use std::ffi::OsString;
use std::fs;
use std::path::PathBuf;

use super::{
    RootfsOutputFormat, ensure_output_dir_ready, parse_rootfs_command, run_rootfs_command,
};

#[test]
fn parse_rootfs_command_accepts_required_paths_and_json_mode() {
    let command = parse_rootfs_command(&os_args([
        "--archive",
        "fixtures/rootfs.tgz",
        "--out",
        "target/rootfs",
        "--json",
    ]))
    .unwrap();

    assert_eq!(command.archive_path, PathBuf::from("fixtures/rootfs.tgz"));
    assert_eq!(command.out_path, PathBuf::from("target/rootfs"));
    assert_eq!(command.output_format, RootfsOutputFormat::Json);
}

#[test]
fn parse_rootfs_command_defaults_to_text_output() {
    let command = parse_rootfs_command(&os_args([
        "--archive",
        "fixtures/rootfs.tgz",
        "--out",
        "target/rootfs",
    ]))
    .unwrap();

    assert_eq!(command.output_format, RootfsOutputFormat::Text);
}

#[test]
fn parse_rootfs_command_reports_missing_required_options() {
    assert_usage_error(parse_rootfs_command(&[]), "rootfs requires --archive FILE");
    assert_usage_error(
        parse_rootfs_command(&os_args(["--archive", "rootfs.tgz"])),
        "rootfs requires --out DIR",
    );
    assert_usage_error(
        parse_rootfs_command(&os_args(["--out", "target/rootfs"])),
        "rootfs requires --archive FILE",
    );
}

#[test]
fn parse_rootfs_command_reports_option_boundary_errors() {
    assert_usage_error(
        parse_rootfs_command(&os_args(["--archive"])),
        "rootfs --archive expects FILE",
    );
    assert_usage_error(
        parse_rootfs_command(&os_args(["--out"])),
        "rootfs --out expects DIR",
    );
    assert_usage_error(
        parse_rootfs_command(&os_args(["--unknown"])),
        "unknown rootfs option: --unknown",
    );
}

#[test]
fn run_rootfs_command_reports_missing_archive() {
    let archive = temp_path("wanix-cli-rootfs-missing-archive.tgz");
    let out = temp_path("wanix-cli-rootfs-missing-archive-out");
    let _ = fs::remove_file(&archive);
    let _ = fs::remove_dir_all(&out);
    let command = parse_rootfs_command(&os_args([
        "--archive",
        archive.to_str().unwrap(),
        "--out",
        out.to_str().unwrap(),
    ]))
    .unwrap();

    assert_error(run_rootfs_command(command), 1, "rootfs --archive");
    assert!(!out.exists());
}

#[test]
fn ensure_output_dir_ready_creates_missing_directory() {
    let out = temp_path("wanix-cli-rootfs-missing-output");
    let _ = fs::remove_dir_all(&out);

    ensure_output_dir_ready(&out).unwrap();

    assert!(out.is_dir());
    let _ = fs::remove_dir_all(out);
}

#[test]
fn ensure_output_dir_ready_accepts_existing_empty_directory() {
    let out = temp_path("wanix-cli-rootfs-empty-output");
    let _ = fs::remove_dir_all(&out);
    fs::create_dir_all(&out).unwrap();

    ensure_output_dir_ready(&out).unwrap();

    let _ = fs::remove_dir_all(out);
}

#[test]
fn ensure_output_dir_ready_rejects_existing_file_or_non_empty_directory() {
    let file_out = temp_path("wanix-cli-rootfs-file-output");
    let _ = fs::remove_file(&file_out);
    fs::write(&file_out, b"not a directory").unwrap();

    let non_empty_out = temp_path("wanix-cli-rootfs-non-empty-output");
    let _ = fs::remove_dir_all(&non_empty_out);
    fs::create_dir_all(&non_empty_out).unwrap();
    fs::write(non_empty_out.join("marker"), b"busy").unwrap();

    assert_error(
        ensure_output_dir_ready(&file_out),
        1,
        "exists but is not a directory",
    );
    assert_error(ensure_output_dir_ready(&non_empty_out), 1, "must be empty");

    let _ = fs::remove_file(file_out);
    let _ = fs::remove_dir_all(non_empty_out);
}

fn assert_usage_error<T: std::fmt::Debug>(result: Result<T, crate::CliError>, expected: &str) {
    assert_error(result, 2, expected);
}

fn assert_error<T: std::fmt::Debug>(
    result: Result<T, crate::CliError>,
    exit_code: i32,
    expected: &str,
) {
    let error = result.unwrap_err();
    assert_eq!(error.exit_code(), exit_code);
    assert!(error.to_string().contains(expected));
}

fn os_args<const N: usize>(args: [&str; N]) -> Vec<OsString> {
    args.into_iter().map(OsString::from).collect()
}

fn temp_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("{name}-{}", std::process::id()))
}
