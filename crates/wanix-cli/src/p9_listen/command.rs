use std::ffi::OsString;
use std::path::PathBuf;

use wanix_9p::PeerId;

use super::grant::{GrantSpec, parse_peer};
use crate::{CliError, command_args::named_arg};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct P9ListenCommand {
    pub(in crate::p9_listen) root_path: PathBuf,
    pub(in crate::p9_listen) addr: String,
    pub(in crate::p9_listen) once: bool,
    /// The explicitly supplied verified peer identity, present only when the
    /// connection should be authorized through a default-deny grant table.
    pub(in crate::p9_listen) peer: Option<PeerId>,
    /// Capability grants for `peer`, scoped to `root_path` at attach time.
    pub(in crate::p9_listen) grants: Vec<GrantSpec>,
}

pub(crate) fn parse_p9_listen_command(args: &[OsString]) -> Result<P9ListenCommand, CliError> {
    let mut parts = P9ListenCommandParts::default();
    let mut i = 0;
    while i < args.len() {
        i = parse_p9_listen_arg(args, i, &mut parts)?;
    }

    parts.finish()
}

fn parse_p9_listen_arg(
    args: &[OsString],
    index: usize,
    parts: &mut P9ListenCommandParts,
) -> Result<usize, CliError> {
    match P9ListenArg::from_arg(&args[index]) {
        Some(P9ListenArg::Value(option)) => apply_p9_listen_value_arg(args, index, parts, option),
        Some(P9ListenArg::Flag(option)) => {
            parts.apply_flag(option)?;
            Ok(index + 1)
        }
        None => Err(CliError::usage(format!(
            "unexpected p9-listen argument: {}",
            args[index].to_string_lossy()
        ))),
    }
}

fn apply_p9_listen_value_arg(
    args: &[OsString],
    option_index: usize,
    parts: &mut P9ListenCommandParts,
    option: P9ListenValueOption,
) -> Result<usize, CliError> {
    let value_index = option_index + 1;
    let value = args.get(value_index).ok_or_else(|| {
        CliError::usage(format!(
            "{} expects {}",
            option.label(),
            option.value_name()
        ))
    })?;
    parts.apply_value(option, value)?;
    Ok(value_index + 1)
}

#[derive(Default)]
struct P9ListenCommandParts {
    root_path: Option<PathBuf>,
    addr: Option<String>,
    once: bool,
    peer: Option<PeerId>,
    grants: Vec<GrantSpec>,
}

impl P9ListenCommandParts {
    fn apply_value(
        &mut self,
        option: P9ListenValueOption,
        value: &OsString,
    ) -> Result<(), CliError> {
        match option {
            P9ListenValueOption::Root => set_single_path(
                &mut self.root_path,
                value,
                "p9-listen accepts only one --root",
            ),
            P9ListenValueOption::Addr => {
                if self.addr.is_some() {
                    return Err(CliError::usage("p9-listen accepts only one --addr"));
                }
                self.addr = Some(value.to_string_lossy().into_owned());
                Ok(())
            }
            P9ListenValueOption::Peer => {
                if self.peer.is_some() {
                    return Err(CliError::usage("p9-listen accepts only one --peer"));
                }
                self.peer = Some(parse_peer(&value.to_string_lossy())?);
                Ok(())
            }
            P9ListenValueOption::Grant => {
                self.grants
                    .push(GrantSpec::parse(&value.to_string_lossy())?);
                Ok(())
            }
        }
    }

    fn apply_flag(&mut self, option: P9ListenFlagOption) -> Result<(), CliError> {
        match option {
            P9ListenFlagOption::Once => {
                if self.once {
                    return Err(CliError::usage("p9-listen accepts only one --once"));
                }
                self.once = true;
            }
        }
        Ok(())
    }

    fn finish(self) -> Result<P9ListenCommand, CliError> {
        let root_path = self
            .root_path
            .ok_or_else(|| CliError::usage("p9-listen requires --root DIR"))?;
        let addr = self
            .addr
            .ok_or_else(|| CliError::usage("p9-listen requires --addr HOST:PORT"))?;
        if !self.grants.is_empty() && self.peer.is_none() {
            return Err(CliError::usage(
                "p9-listen --grant requires --peer HEX to name the authorized peer",
            ));
        }
        Ok(P9ListenCommand {
            root_path,
            addr,
            once: self.once,
            peer: self.peer,
            grants: self.grants,
        })
    }
}

#[derive(Clone, Copy)]
enum P9ListenArg {
    Value(P9ListenValueOption),
    Flag(P9ListenFlagOption),
}

impl P9ListenArg {
    fn from_arg(arg: &OsString) -> Option<Self> {
        named_arg(arg, P9_LISTEN_VALUE_OPTIONS)
            .map(Self::Value)
            .or_else(|| named_arg(arg, P9_LISTEN_FLAG_OPTIONS).map(Self::Flag))
    }
}

const P9_LISTEN_VALUE_OPTIONS: &[(&str, P9ListenValueOption)] = &[
    ("--root", P9ListenValueOption::Root),
    ("--addr", P9ListenValueOption::Addr),
    ("--peer", P9ListenValueOption::Peer),
    ("--grant", P9ListenValueOption::Grant),
];

const P9_LISTEN_FLAG_OPTIONS: &[(&str, P9ListenFlagOption)] =
    &[("--once", P9ListenFlagOption::Once)];

#[derive(Clone, Copy)]
enum P9ListenValueOption {
    Root,
    Addr,
    Peer,
    Grant,
}

impl P9ListenValueOption {
    fn label(self) -> &'static str {
        match self {
            Self::Root => "p9-listen --root",
            Self::Addr => "p9-listen --addr",
            Self::Peer => "p9-listen --peer",
            Self::Grant => "p9-listen --grant",
        }
    }

    fn value_name(self) -> &'static str {
        match self {
            Self::Root => "DIR",
            Self::Addr => "HOST:PORT",
            Self::Peer => "HEX",
            Self::Grant => "ANAME:PREFIX:RIGHTS",
        }
    }
}

#[derive(Clone, Copy)]
enum P9ListenFlagOption {
    Once,
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

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn parse_p9_listen_requires_root_and_addr() {
        let error =
            parse_p9_listen_command(&[OsString::from("--root"), OsString::from(".")]).unwrap_err();
        assert!(error.to_string().contains("requires --addr HOST:PORT"));

        let command = parse_p9_listen_command(&[
            OsString::from("--root"),
            OsString::from("."),
            OsString::from("--addr"),
            OsString::from("127.0.0.1:0"),
            OsString::from("--once"),
        ])
        .unwrap();

        assert_eq!(command.root_path, PathBuf::from("."));
        assert_eq!(command.addr, "127.0.0.1:0");
        assert!(command.once);
    }

    #[test]
    fn parse_p9_listen_accepts_peer_and_repeated_grants() {
        let command = parse_p9_listen_command(&[
            OsString::from("--root"),
            OsString::from("."),
            OsString::from("--addr"),
            OsString::from("127.0.0.1:0"),
            OsString::from("--peer"),
            OsString::from("ab".repeat(32)),
            OsString::from("--grant"),
            OsString::from("projects/foo:projects/foo:rw"),
            OsString::from("--grant"),
            OsString::from("docs:docs:ro"),
        ])
        .unwrap();

        assert!(command.peer.is_some());
        assert_eq!(command.grants.len(), 2);
    }

    #[test]
    fn parse_p9_listen_rejects_grant_without_peer() {
        let error = parse_p9_listen_command(&[
            OsString::from("--root"),
            OsString::from("."),
            OsString::from("--addr"),
            OsString::from("127.0.0.1:0"),
            OsString::from("--grant"),
            OsString::from("docs:docs:ro"),
        ])
        .unwrap_err();
        assert!(
            error.to_string().contains("--grant requires --peer"),
            "{error}"
        );
    }

    #[test]
    fn parse_p9_listen_reports_missing_option_values() {
        for (option, expected) in [
            ("--root", "p9-listen --root expects DIR"),
            ("--addr", "p9-listen --addr expects HOST:PORT"),
        ] {
            let error = parse_p9_listen_command(&[OsString::from(option)]).unwrap_err();
            assert!(
                error.to_string().contains(expected),
                "{option} produced {error}"
            );
        }
    }
}
