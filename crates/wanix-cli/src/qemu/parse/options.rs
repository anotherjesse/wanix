use std::ffi::OsString;
use std::path::PathBuf;

use crate::CliError;

use super::super::{
    DEFAULT_MEMORY_MB, DEFAULT_MOUNT_TAG, DEFAULT_P9_MSIZE, DEFAULT_QEMU_BIN,
    DEFAULT_SECURITY_MODEL, QemuOutputFormat,
};
use super::{
    os_arg_to_string, parse_memory_mb, parse_mount_tag, parse_p9_msize, parse_security_model,
};

mod definition;

pub(super) use definition::QemuOption;
use definition::QemuOptionKind;

pub(super) struct QemuOptions {
    pub(super) root_path: Option<PathBuf>,
    pub(super) kernel_path: Option<PathBuf>,
    pub(super) initrd_path: Option<PathBuf>,
    pub(super) qemu_bin: String,
    pub(super) memory_mb: u32,
    pub(super) kvm: bool,
    pub(super) mount_tag: String,
    pub(super) security_model: String,
    pub(super) p9_msize: u32,
    pub(super) cmdline: Option<String>,
    pub(super) append: Vec<String>,
    pub(super) output_format: QemuOutputFormat,
    pub(super) exec: bool,
}

impl Default for QemuOptions {
    fn default() -> Self {
        Self {
            root_path: None,
            kernel_path: None,
            initrd_path: None,
            qemu_bin: DEFAULT_QEMU_BIN.to_owned(),
            memory_mb: DEFAULT_MEMORY_MB,
            kvm: true,
            mount_tag: DEFAULT_MOUNT_TAG.to_owned(),
            security_model: DEFAULT_SECURITY_MODEL.to_owned(),
            p9_msize: DEFAULT_P9_MSIZE,
            cmdline: None,
            append: Vec::new(),
            output_format: QemuOutputFormat::Shell,
            exec: false,
        }
    }
}

impl QemuOption {
    pub(super) fn apply_value(
        self,
        value: &OsString,
        options: &mut QemuOptions,
    ) -> Result<(), CliError> {
        match self.kind {
            QemuOptionKind::Root | QemuOptionKind::Kernel | QemuOptionKind::Initrd => {
                self.apply_path_value(value, options)?
            }
            QemuOptionKind::Cmdline | QemuOptionKind::Append | QemuOptionKind::QemuBin => {
                self.apply_text_value(value, options)?
            }
            QemuOptionKind::MemoryMb
            | QemuOptionKind::P9Msize
            | QemuOptionKind::MountTag
            | QemuOptionKind::SecurityModel => self.apply_parsed_value(value, options)?,
            QemuOptionKind::Json | QemuOptionKind::NoKvm | QemuOptionKind::Exec => {}
        }
        Ok(())
    }

    fn apply_path_value(self, value: &OsString, options: &mut QemuOptions) -> Result<(), CliError> {
        match self.kind {
            QemuOptionKind::Root => options.root_path = Some(PathBuf::from(value)),
            QemuOptionKind::Kernel => set_single_path(
                &mut options.kernel_path,
                value,
                "qemu accepts only one --kernel",
            )?,
            QemuOptionKind::Initrd => set_single_path(
                &mut options.initrd_path,
                value,
                "qemu accepts only one --initrd",
            )?,
            _ => {}
        }
        Ok(())
    }

    fn apply_text_value(self, value: &OsString, options: &mut QemuOptions) -> Result<(), CliError> {
        match self.kind {
            QemuOptionKind::Cmdline => {
                if options.cmdline.is_some() {
                    return Err(CliError::usage("qemu accepts only one --cmdline"));
                }
                options.cmdline = Some(os_arg_to_string(value, self.label())?);
            }
            QemuOptionKind::Append => options.append.push(os_arg_to_string(value, self.label())?),
            QemuOptionKind::QemuBin => options.qemu_bin = os_arg_to_string(value, self.label())?,
            _ => {}
        }
        Ok(())
    }

    fn apply_parsed_value(
        self,
        value: &OsString,
        options: &mut QemuOptions,
    ) -> Result<(), CliError> {
        match self.kind {
            QemuOptionKind::MemoryMb => options.memory_mb = parse_memory_mb(value)?,
            QemuOptionKind::P9Msize => options.p9_msize = parse_p9_msize(value)?,
            QemuOptionKind::MountTag => options.mount_tag = parse_mount_tag(value)?,
            QemuOptionKind::SecurityModel => options.security_model = parse_security_model(value)?,
            _ => {}
        }
        Ok(())
    }

    pub(super) fn apply_flag(self, options: &mut QemuOptions) {
        match self.kind {
            QemuOptionKind::Json => options.output_format = QemuOutputFormat::Json,
            QemuOptionKind::NoKvm => options.kvm = false,
            QemuOptionKind::Exec => options.exec = true,
            _ => {}
        }
    }
}

fn set_single_path(
    target: &mut Option<PathBuf>,
    value: &OsString,
    duplicate_message: &str,
) -> Result<(), CliError> {
    if target.is_some() {
        return Err(CliError::usage(duplicate_message));
    }
    *target = Some(PathBuf::from(value));
    Ok(())
}
