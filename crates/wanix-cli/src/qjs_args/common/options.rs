use std::ffi::OsString;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CommonQjsOption {
    Env,
    Cwd,
    Stdin,
    StdinFile,
    EventLoopMs,
    ReadyIoTurns,
    InterruptAfter,
    MemoryLimitBytes,
    Mount,
    Separator,
}

impl CommonQjsOption {
    pub(super) fn from_arg(arg: &OsString) -> Option<Self> {
        let arg = arg.to_str()?;
        COMMON_QJS_OPTION_SPECS
            .iter()
            .find_map(|spec| (spec.name == arg).then_some(spec.option))
    }

    pub(super) fn name(self) -> &'static str {
        self.spec().name
    }

    pub(super) fn expected(self) -> &'static str {
        self.spec().expected
    }

    fn spec(self) -> &'static CommonQjsOptionSpec {
        COMMON_QJS_OPTION_SPECS
            .iter()
            .find(|spec| spec.option == self)
            .expect("common qjs option metadata is complete")
    }
}

struct CommonQjsOptionSpec {
    option: CommonQjsOption,
    name: &'static str,
    expected: &'static str,
}

const COMMON_QJS_OPTION_SPECS: &[CommonQjsOptionSpec] = &[
    CommonQjsOptionSpec {
        option: CommonQjsOption::Env,
        name: "--env",
        expected: "KEY=VALUE",
    },
    CommonQjsOptionSpec {
        option: CommonQjsOption::Cwd,
        name: "--cwd",
        expected: "a Wanix path",
    },
    CommonQjsOptionSpec {
        option: CommonQjsOption::Stdin,
        name: "--stdin",
        expected: "text",
    },
    CommonQjsOptionSpec {
        option: CommonQjsOption::StdinFile,
        name: "--stdin-file",
        expected: "PATH or -",
    },
    CommonQjsOptionSpec {
        option: CommonQjsOption::EventLoopMs,
        name: "--event-loop-ms",
        expected: "milliseconds",
    },
    CommonQjsOptionSpec {
        option: CommonQjsOption::ReadyIoTurns,
        name: "--ready-io-turns",
        expected: "a count",
    },
    CommonQjsOptionSpec {
        option: CommonQjsOption::InterruptAfter,
        name: "--interrupt-after",
        expected: "a count",
    },
    CommonQjsOptionSpec {
        option: CommonQjsOption::MemoryLimitBytes,
        name: "--memory-limit-bytes",
        expected: "a byte count",
    },
    CommonQjsOptionSpec {
        option: CommonQjsOption::Mount,
        name: "--mount",
        expected: "HOST=GUEST",
    },
    CommonQjsOptionSpec {
        option: CommonQjsOption::Separator,
        name: "--",
        expected: "",
    },
];
