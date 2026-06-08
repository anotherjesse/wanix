use std::ffi::OsString;

use crate::command_args::named_arg;

pub(super) const SERVE_VALUE_OPTIONS: &[(&str, ServeValueOption)] = &[
    ("--root", ServeValueOption::Root),
    ("--addr", ServeValueOption::Addr),
    ("--listen", ServeValueOption::Listen),
    ("--bundle", ServeValueOption::Bundle),
    ("--p9", ServeValueOption::P9),
    ("--peer", ServeValueOption::Peer),
    ("--grant", ServeValueOption::Grant),
];

pub(super) const SERVE_FLAG_OPTIONS: &[(&str, ServeFlagOption)] = &[
    ("--once", ServeFlagOption::Once),
    ("--wanix-services", ServeFlagOption::WanixServices),
];

#[derive(Clone, Copy)]
pub(super) enum ServeArg {
    Value(ServeValueOption),
    Flag(ServeFlagOption),
    PositionalRoot,
}

impl ServeArg {
    pub(super) fn from_arg(arg: &OsString) -> Option<Self> {
        named_arg(arg, SERVE_VALUE_OPTIONS)
            .map(Self::Value)
            .or_else(|| named_arg(arg, SERVE_FLAG_OPTIONS).map(Self::Flag))
    }
}

#[derive(Clone, Copy)]
pub(super) enum ServeValueOption {
    Root,
    Addr,
    Listen,
    Bundle,
    P9,
    Peer,
    Grant,
}

impl ServeValueOption {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Root => "serve --root",
            Self::Addr => "serve --addr",
            Self::Listen => "serve --listen",
            Self::Bundle => "serve --bundle",
            Self::P9 => "serve --p9",
            Self::Peer => "serve --peer",
            Self::Grant => "serve --grant",
        }
    }

    pub(super) fn value_name(self) -> &'static str {
        match self {
            Self::Root => "DIR",
            Self::Addr | Self::Listen | Self::P9 => "HOST:PORT",
            Self::Bundle => "NAME",
            Self::Peer => "HEX",
            Self::Grant => "ANAME:PREFIX:RIGHTS",
        }
    }
}

#[derive(Clone, Copy)]
pub(super) enum ServeFlagOption {
    Once,
    WanixServices,
}

pub(super) fn normalize_listen_addr(addr: &str) -> String {
    if let Some(port) = addr.strip_prefix(':') {
        format!("0.0.0.0:{port}")
    } else {
        addr.to_owned()
    }
}
