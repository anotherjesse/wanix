use std::ffi::OsString;

#[derive(Clone, Copy)]
pub(super) enum QjsTermParseAction {
    TermOption(QjsTermOption),
    QjsValueOption,
    Rest,
}

impl QjsTermParseAction {
    pub(super) fn from_arg(arg: &OsString) -> Option<Self> {
        if let Some(option) = QjsTermOption::from_arg(arg) {
            return Some(Self::TermOption(option));
        }
        if qjs_option_takes_value(arg) {
            return Some(Self::QjsValueOption);
        }
        Some(Self::Rest)
    }
}

#[derive(Clone, Copy)]
pub(super) enum QjsTermOption {
    FeedBytes,
    FeedFile,
    FeedLines,
    Resize,
}

impl QjsTermOption {
    fn from_arg(arg: &OsString) -> Option<Self> {
        match arg.to_str()? {
            "--feed-after-eval" => Some(Self::FeedBytes),
            "--feed-after-eval-file" => Some(Self::FeedFile),
            "--feed-after-eval-lines" => Some(Self::FeedLines),
            "--resize-after-eval" => Some(Self::Resize),
            _ => None,
        }
    }
}

pub(super) fn value_name(label: &str) -> &'static str {
    match label {
        "qjs-term --feed-after-eval" => "text",
        "qjs-term --feed-after-eval-file" | "qjs-term --feed-after-eval-lines" => "PATH or -",
        "qjs-term --resize-after-eval" => "COLSxROWS",
        _ => "value",
    }
}

fn qjs_option_takes_value(arg: &OsString) -> bool {
    matches!(
        arg.to_str(),
        Some(
            "--env"
                | "--cwd"
                | "--stdin"
                | "--stdin-file"
                | "--event-loop-ms"
                | "--ready-io-turns"
                | "--interrupt-after"
                | "--memory-limit-bytes"
                | "--mount"
        )
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qjs_term_value_names_match_option_contracts() {
        assert_eq!(value_name("qjs-term --feed-after-eval"), "text");
        assert_eq!(value_name("qjs-term --feed-after-eval-file"), "PATH or -");
        assert_eq!(value_name("qjs-term --feed-after-eval-lines"), "PATH or -");
        assert_eq!(value_name("qjs-term --resize-after-eval"), "COLSxROWS");
        assert_eq!(value_name("qjs-term --unknown"), "value");
    }
}
