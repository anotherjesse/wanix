use std::ffi::OsString;

use crate::CliError;

use super::{QemuCommand, QemuOutputFormat, VALID_SECURITY_MODELS, validate_qemu_option_fragment};

mod options;

use options::{QemuOption, QemuOptions};

pub(crate) fn parse_qemu_command(args: &[OsString]) -> Result<QemuCommand, CliError> {
    let mut options = QemuOptions::default();
    let mut i = 0;
    while i < args.len() {
        let Some(option) = QemuOption::from_arg(&args[i]) else {
            return Err(CliError::usage(format!(
                "unknown qemu option: {}",
                args[i].to_string_lossy()
            )));
        };
        if let Some(expected) = option.expected_value() {
            i += 1;
            let value = args
                .get(i)
                .ok_or_else(|| CliError::usage(format!("{} expects {expected}", option.label())))?;
            option.apply_value(value, &mut options)?;
        } else {
            option.apply_flag(&mut options);
        }
        i += 1;
    }
    let root_path = options
        .root_path
        .ok_or_else(|| CliError::usage("qemu requires --root DIR"))?;
    if options.exec && options.output_format == QemuOutputFormat::Json {
        return Err(CliError::usage(
            "qemu --json cannot be combined with --exec",
        ));
    }
    Ok(QemuCommand {
        root_path,
        kernel_path: options.kernel_path,
        initrd_path: options.initrd_path,
        qemu_bin: options.qemu_bin,
        memory_mb: options.memory_mb,
        kvm: options.kvm,
        mount_tag: options.mount_tag,
        security_model: options.security_model,
        p9_msize: options.p9_msize,
        cmdline: options.cmdline,
        append: options.append,
        output_format: options.output_format,
        exec: options.exec,
    })
}

fn parse_memory_mb(arg: &OsString) -> Result<u32, CliError> {
    parse_positive_u32(arg, "qemu --memory-mb")
}

fn parse_p9_msize(arg: &OsString) -> Result<u32, CliError> {
    parse_positive_u32(arg, "qemu --p9-msize")
}

fn parse_positive_u32(arg: &OsString, label: &str) -> Result<u32, CliError> {
    let value = os_arg_to_string(arg, label)?;
    let parsed = value
        .parse::<u32>()
        .map_err(|_| CliError::usage(format!("{label} expects a positive integer")))?;
    if parsed == 0 {
        return Err(CliError::usage(format!(
            "{label} expects a positive integer"
        )));
    }
    Ok(parsed)
}

fn parse_qemu_option_value(arg: &OsString, label: &str) -> Result<String, CliError> {
    let value = os_arg_to_string(arg, label)?;
    validate_qemu_option_fragment(&value, label)?;
    Ok(value)
}

fn parse_mount_tag(arg: &OsString) -> Result<String, CliError> {
    let value = parse_qemu_option_value(arg, "qemu --mount-tag")?;
    if !value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(CliError::usage(
            "qemu --mount-tag accepts only ASCII letters, digits, '.', '_', and '-'",
        ));
    }
    Ok(value)
}

fn parse_security_model(arg: &OsString) -> Result<String, CliError> {
    let value = parse_qemu_option_value(arg, "qemu --security-model")?;
    if !VALID_SECURITY_MODELS.contains(&value.as_str()) {
        return Err(CliError::usage(format!(
            "qemu --security-model expects one of {}",
            VALID_SECURITY_MODELS.join(", ")
        )));
    }
    Ok(value)
}

fn os_arg_to_string(arg: &OsString, label: &str) -> Result<String, CliError> {
    arg.clone()
        .into_string()
        .map_err(|_| CliError::usage(format!("{label} expects UTF-8")))
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::path::PathBuf;

    use super::super::{DEFAULT_MEMORY_MB, DEFAULT_P9_MSIZE, QemuOutputFormat};
    use super::parse_qemu_command;

    fn args(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn parse_qemu_command_applies_values_and_flags() {
        let command = parse_qemu_command(&args(&[
            "--root",
            "/guest",
            "--kernel",
            "/kernel",
            "--initrd",
            "/initrd",
            "--append",
            "console=hvc0",
            "--append",
            "single",
            "--no-kvm",
            "--exec",
        ]))
        .unwrap();

        assert_eq!(command.root_path, PathBuf::from("/guest"));
        assert_eq!(command.kernel_path, Some(PathBuf::from("/kernel")));
        assert_eq!(command.initrd_path, Some(PathBuf::from("/initrd")));
        assert_eq!(command.memory_mb, DEFAULT_MEMORY_MB);
        assert_eq!(command.p9_msize, DEFAULT_P9_MSIZE);
        assert_eq!(command.append, ["console=hvc0", "single"]);
        assert!(!command.kvm);
        assert!(command.exec);
        assert_eq!(command.output_format, QemuOutputFormat::Shell);
    }

    #[test]
    fn parse_qemu_command_reports_parser_boundaries() {
        let cases = [
            (&[][..], "qemu requires --root DIR"),
            (&["--kernel"][..], "qemu --kernel expects PATH"),
            (
                &["--root", "/guest", "--bogus"][..],
                "unknown qemu option: --bogus",
            ),
            (
                &["--root", "/guest", "--json", "--exec"][..],
                "qemu --json cannot be combined with --exec",
            ),
        ];

        for (input, expected) in cases {
            let error = parse_qemu_command(&args(input)).unwrap_err();
            assert_eq!(error.exit_code(), 2);
            assert!(error.to_string().contains(expected));
        }
    }
}
