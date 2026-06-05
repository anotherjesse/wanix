use std::ffi::OsString;

const QEMU_OPTIONS: &[QemuOption] = &[
    QemuOption::value("--root", "qemu --root", "DIR", QemuOptionKind::Root),
    QemuOption::value("--kernel", "qemu --kernel", "PATH", QemuOptionKind::Kernel),
    QemuOption::value("--initrd", "qemu --initrd", "PATH", QemuOptionKind::Initrd),
    QemuOption::value(
        "--cmdline",
        "qemu --cmdline",
        "TEXT",
        QemuOptionKind::Cmdline,
    ),
    QemuOption::value("--append", "qemu --append", "TEXT", QemuOptionKind::Append),
    QemuOption::value(
        "--qemu-bin",
        "qemu --qemu-bin",
        "PATH",
        QemuOptionKind::QemuBin,
    ),
    QemuOption::value(
        "--memory-mb",
        "qemu --memory-mb",
        "N",
        QemuOptionKind::MemoryMb,
    ),
    QemuOption::value(
        "--p9-msize",
        "qemu --p9-msize",
        "N",
        QemuOptionKind::P9Msize,
    ),
    QemuOption::value(
        "--mount-tag",
        "qemu --mount-tag",
        "TAG",
        QemuOptionKind::MountTag,
    ),
    QemuOption::value(
        "--security-model",
        "qemu --security-model",
        "MODEL",
        QemuOptionKind::SecurityModel,
    ),
    QemuOption::flag("--json", "qemu --json", QemuOptionKind::Json),
    QemuOption::flag("--no-kvm", "qemu --no-kvm", QemuOptionKind::NoKvm),
    QemuOption::flag("--exec", "qemu --exec", QemuOptionKind::Exec),
];

#[derive(Clone, Copy)]
pub(in crate::qemu::parse) struct QemuOption {
    flag: &'static str,
    label: &'static str,
    expected: Option<&'static str>,
    pub(super) kind: QemuOptionKind,
}

#[derive(Clone, Copy)]
pub(super) enum QemuOptionKind {
    Root,
    Kernel,
    Initrd,
    Cmdline,
    Append,
    QemuBin,
    MemoryMb,
    P9Msize,
    MountTag,
    SecurityModel,
    Json,
    NoKvm,
    Exec,
}

impl QemuOption {
    const fn value(
        flag: &'static str,
        label: &'static str,
        expected: &'static str,
        kind: QemuOptionKind,
    ) -> Self {
        Self {
            flag,
            label,
            expected: Some(expected),
            kind,
        }
    }

    const fn flag(flag: &'static str, label: &'static str, kind: QemuOptionKind) -> Self {
        Self {
            flag,
            label,
            expected: None,
            kind,
        }
    }

    pub(in crate::qemu::parse) fn from_arg(arg: &OsString) -> Option<Self> {
        let arg = arg.to_str()?;
        QEMU_OPTIONS
            .iter()
            .find(|option| option.flag == arg)
            .copied()
    }

    pub(in crate::qemu::parse) fn label(self) -> &'static str {
        self.label
    }

    pub(in crate::qemu::parse) fn expected_value(self) -> Option<&'static str> {
        self.expected
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use super::QemuOption;

    #[test]
    fn qemu_options_report_expected_value_names() {
        let value_options = [
            ("--root", Some("DIR")),
            ("--kernel", Some("PATH")),
            ("--initrd", Some("PATH")),
            ("--cmdline", Some("TEXT")),
            ("--append", Some("TEXT")),
            ("--qemu-bin", Some("PATH")),
            ("--memory-mb", Some("N")),
            ("--p9-msize", Some("N")),
            ("--mount-tag", Some("TAG")),
            ("--security-model", Some("MODEL")),
        ];

        for (flag, expected) in value_options {
            let option = QemuOption::from_arg(&OsString::from(flag)).unwrap();
            assert_eq!(option.expected_value(), expected);
        }
    }

    #[test]
    fn qemu_flags_do_not_expect_values() {
        for flag in ["--json", "--no-kvm", "--exec"] {
            let option = QemuOption::from_arg(&OsString::from(flag)).unwrap();
            assert_eq!(option.expected_value(), None);
        }
    }
}
