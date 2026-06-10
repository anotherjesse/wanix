use std::ffi::OsString;
use std::path::PathBuf;

use wanix_9p::PeerId;

use super::raw9p::grant::{GrantSpec, parse_peer};
use super::webdoor::WebBind;
use crate::CliError;

pub(crate) const DEFAULT_SERVE_ADDR: &str = "127.0.0.1:7654";

mod options;
use options::{ServeArg, ServeFlagOption, ServeValueOption, normalize_listen_addr};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ServeCommand {
    pub(super) root_path: PathBuf,
    pub(super) addr: String,
    pub(super) bundle: Option<String>,
    pub(super) wanix_services: bool,
    pub(super) once: bool,
    /// Raw-9P-over-TCP door address, present only when `--p9 HOST:PORT` is set.
    /// Exports the served namespace as raw 9P so `mount-write tcp://...` works
    /// against a live serve. Subject to the `--wanix-services` off-loopback
    /// trust guard.
    pub(super) p9_addr: Option<String>,
    /// Explicit verified peer identity for the raw-9P door, present only when
    /// `--peer HEX` gates `--p9` with a default-deny grant table.
    pub(super) peer: Option<PeerId>,
    /// Capability grants for `peer`, scoped to the served namespace at attach
    /// time. Requires `--peer`.
    pub(super) grants: Vec<GrantSpec>,
    /// WebDoor gateway origins, one per `--bind NAME=DIR|NAME=iroh://PEER`
    /// (repeatable). Subject to the loopback-only gateway trust rule.
    pub(super) binds: Vec<WebBind>,
}

pub(crate) fn parse_serve_command(args: &[OsString]) -> Result<ServeCommand, CliError> {
    ServeCommandParser::new(args).parse()
}

struct ServeCommandParser<'a> {
    args: &'a [OsString],
    index: usize,
    parts: ServeCommandParts,
}

impl<'a> ServeCommandParser<'a> {
    fn new(args: &'a [OsString]) -> Self {
        Self {
            args,
            index: 0,
            parts: ServeCommandParts::default(),
        }
    }

    fn parse(mut self) -> Result<ServeCommand, CliError> {
        while let Some(arg) = self.next_arg()? {
            self.apply_arg(arg)?;
        }
        self.parts.finish()
    }

    fn next_arg(&self) -> Result<Option<ServeArg>, CliError> {
        let Some(raw) = self.args.get(self.index) else {
            return Ok(None);
        };
        if let Some(arg) = ServeArg::from_arg(raw) {
            return Ok(Some(arg));
        }
        if raw.to_string_lossy().starts_with('-') {
            return Err(CliError::usage(format!(
                "unexpected serve argument: {}",
                raw.to_string_lossy()
            )));
        }
        Ok(Some(ServeArg::PositionalRoot))
    }

    fn apply_arg(&mut self, arg: ServeArg) -> Result<(), CliError> {
        match arg {
            ServeArg::Value(option) => self.apply_value(option),
            ServeArg::Flag(option) => self.apply_flag(option),
            ServeArg::PositionalRoot => self.set_positional_root(),
        }
    }

    fn apply_value(&mut self, option: ServeValueOption) -> Result<(), CliError> {
        let value = self.take_value(option)?;
        match option {
            ServeValueOption::Root => self.parts.set_root(value, "serve accepts only one --root"),
            ServeValueOption::Addr | ServeValueOption::Listen => self.parts.set_addr(value),
            ServeValueOption::Bundle => self.parts.set_bundle(value),
            ServeValueOption::P9 => self.parts.set_p9_addr(value),
            ServeValueOption::Peer => self.parts.set_peer(value),
            ServeValueOption::Grant => self.parts.push_grant(value),
            ServeValueOption::Bind => self.parts.push_bind(value),
        }
    }

    fn apply_flag(&mut self, option: ServeFlagOption) -> Result<(), CliError> {
        match option {
            ServeFlagOption::Once => self.parts.set_once()?,
            ServeFlagOption::WanixServices => self.parts.set_wanix_services()?,
        }
        self.index += 1;
        Ok(())
    }

    fn take_value(&mut self, option: ServeValueOption) -> Result<&'a OsString, CliError> {
        self.index += 1;
        let value = self.args.get(self.index).ok_or_else(|| {
            CliError::usage(format!(
                "{} expects {}",
                option.label(),
                option.value_name()
            ))
        })?;
        self.index += 1;
        Ok(value)
    }

    fn set_positional_root(&mut self) -> Result<(), CliError> {
        self.parts
            .set_root(&self.args[self.index], "serve accepts only one directory")?;
        self.index += 1;
        Ok(())
    }
}

#[derive(Default)]
struct ServeCommandParts {
    root_path: Option<PathBuf>,
    addr: Option<String>,
    bundle: Option<String>,
    wanix_services: bool,
    once: bool,
    p9_addr: Option<String>,
    peer: Option<PeerId>,
    grants: Vec<GrantSpec>,
    binds: Vec<WebBind>,
}

impl ServeCommandParts {
    fn set_root(&mut self, value: &OsString, duplicate_message: &str) -> Result<(), CliError> {
        if self.root_path.is_some() {
            return Err(CliError::usage(duplicate_message));
        }
        self.root_path = Some(PathBuf::from(value));
        Ok(())
    }

    fn set_addr(&mut self, raw_value: &OsString) -> Result<(), CliError> {
        if self.addr.is_some() {
            return Err(CliError::usage("serve accepts only one --addr or --listen"));
        }
        let value = raw_value.to_string_lossy();
        self.addr = Some(normalize_listen_addr(&value));
        Ok(())
    }

    fn set_bundle(&mut self, value: &OsString) -> Result<(), CliError> {
        if self.bundle.is_some() {
            return Err(CliError::usage("serve accepts only one --bundle"));
        }
        self.bundle = Some(value.to_string_lossy().into_owned());
        Ok(())
    }

    fn set_once(&mut self) -> Result<(), CliError> {
        if self.once {
            return Err(CliError::usage("serve accepts only one --once"));
        }
        self.once = true;
        Ok(())
    }

    fn set_wanix_services(&mut self) -> Result<(), CliError> {
        if self.wanix_services {
            return Err(CliError::usage("serve accepts only one --wanix-services"));
        }
        self.wanix_services = true;
        Ok(())
    }

    fn set_p9_addr(&mut self, value: &OsString) -> Result<(), CliError> {
        if self.p9_addr.is_some() {
            return Err(CliError::usage("serve accepts only one --p9"));
        }
        let raw = value.to_string_lossy();
        self.p9_addr = Some(normalize_listen_addr(&raw));
        Ok(())
    }

    fn set_peer(&mut self, value: &OsString) -> Result<(), CliError> {
        if self.peer.is_some() {
            return Err(CliError::usage("serve accepts only one --peer"));
        }
        self.peer = Some(parse_peer(&value.to_string_lossy())?);
        Ok(())
    }

    fn push_grant(&mut self, value: &OsString) -> Result<(), CliError> {
        self.grants
            .push(GrantSpec::parse(&value.to_string_lossy())?);
        Ok(())
    }

    fn push_bind(&mut self, value: &OsString) -> Result<(), CliError> {
        self.binds.push(WebBind::parse(&value.to_string_lossy())?);
        Ok(())
    }

    fn finish(self) -> Result<ServeCommand, CliError> {
        // A grant names a capability for a peer; a peer/grant with no raw-9P
        // door is dead config, so require `--p9`. (The websocket door stays
        // unauthenticated; capability gating is the raw-9P door's contract.)
        if !self.grants.is_empty() && self.peer.is_none() {
            return Err(CliError::usage(
                "serve --grant requires --peer HEX to name the authorized peer",
            ));
        }
        if (self.peer.is_some() || !self.grants.is_empty()) && self.p9_addr.is_none() {
            return Err(CliError::usage(
                "serve --peer/--grant gate the raw-9P door; they require --p9 HOST:PORT",
            ));
        }
        Ok(ServeCommand {
            root_path: self.root_path.unwrap_or_else(|| PathBuf::from(".")),
            addr: self.addr.unwrap_or_else(|| DEFAULT_SERVE_ADDR.to_owned()),
            bundle: self.bundle,
            wanix_services: self.wanix_services,
            once: self.once,
            p9_addr: self.p9_addr,
            peer: self.peer,
            grants: self.grants,
            binds: self.binds,
        })
    }
}
