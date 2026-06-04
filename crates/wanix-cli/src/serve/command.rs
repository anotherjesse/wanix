use std::ffi::OsString;
use std::path::PathBuf;

use crate::CliError;

pub(crate) const DEFAULT_SERVE_ADDR: &str = "127.0.0.1:7654";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ServeCommand {
    pub(super) root_path: PathBuf,
    pub(super) addr: String,
    pub(super) bundle: Option<String>,
    pub(super) wanix_services: bool,
    pub(super) once: bool,
}

pub(crate) fn parse_serve_command(args: &[OsString]) -> Result<ServeCommand, CliError> {
    let mut parts = ServeCommandParts::default();
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--root" {
            i = parts.parse_root(args, i)?;
        } else if args[i] == "--addr" || args[i] == "--listen" {
            i = parts.parse_addr(args, i)?;
        } else if args[i] == "--bundle" {
            i = parts.parse_bundle(args, i)?;
        } else if args[i] == "--once" {
            parts.set_once()?;
            i += 1;
        } else if args[i] == "--wanix-services" {
            parts.set_wanix_services()?;
            i += 1;
        } else if args[i].to_string_lossy().starts_with('-') {
            return Err(CliError::usage(format!(
                "unexpected serve argument: {}",
                args[i].to_string_lossy()
            )));
        } else {
            parts.set_positional_root(&args[i])?;
            i += 1;
        }
    }

    Ok(parts.finish())
}

#[derive(Default)]
struct ServeCommandParts {
    root_path: Option<PathBuf>,
    addr: Option<String>,
    bundle: Option<String>,
    wanix_services: bool,
    once: bool,
}

impl ServeCommandParts {
    fn parse_root(&mut self, args: &[OsString], option_index: usize) -> Result<usize, CliError> {
        let value_index = option_index + 1;
        let value = args
            .get(value_index)
            .ok_or_else(|| CliError::usage("serve --root expects DIR"))?;
        if self.root_path.is_some() {
            return Err(CliError::usage("serve accepts only one --root"));
        }
        self.root_path = Some(PathBuf::from(value));
        Ok(value_index + 1)
    }

    fn parse_addr(&mut self, args: &[OsString], option_index: usize) -> Result<usize, CliError> {
        let option = args[option_index].to_string_lossy();
        let value_index = option_index + 1;
        let raw_value = args
            .get(value_index)
            .ok_or_else(|| CliError::usage(format!("serve {option} expects HOST:PORT")))?;
        if self.addr.is_some() {
            return Err(CliError::usage("serve accepts only one --addr or --listen"));
        }
        let value = raw_value.to_string_lossy();
        self.addr = Some(normalize_listen_addr(&value));
        Ok(value_index + 1)
    }

    fn parse_bundle(&mut self, args: &[OsString], option_index: usize) -> Result<usize, CliError> {
        let value_index = option_index + 1;
        let value = args
            .get(value_index)
            .ok_or_else(|| CliError::usage("serve --bundle expects NAME"))?;
        if self.bundle.is_some() {
            return Err(CliError::usage("serve accepts only one --bundle"));
        }
        self.bundle = Some(value.to_string_lossy().into_owned());
        Ok(value_index + 1)
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

    fn set_positional_root(&mut self, value: &OsString) -> Result<(), CliError> {
        if self.root_path.is_some() {
            return Err(CliError::usage("serve accepts only one directory"));
        }
        self.root_path = Some(PathBuf::from(value));
        Ok(())
    }

    fn finish(self) -> ServeCommand {
        ServeCommand {
            root_path: self.root_path.unwrap_or_else(|| PathBuf::from(".")),
            addr: self.addr.unwrap_or_else(|| DEFAULT_SERVE_ADDR.to_owned()),
            bundle: self.bundle,
            wanix_services: self.wanix_services,
            once: self.once,
        }
    }
}

fn normalize_listen_addr(addr: &str) -> String {
    if let Some(port) = addr.strip_prefix(':') {
        format!("0.0.0.0:{port}")
    } else {
        addr.to_owned()
    }
}
