use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::Arc;

use wanix_9p::P9Server;
use wanix_fs::{FileSystem, LocalFs};

use crate::{CliError, CliOutput, write_process_output};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct P9StdioCommand {
    root_path: PathBuf,
}

pub(super) fn parse_p9_stdio_command(args: &[OsString]) -> Result<P9StdioCommand, CliError> {
    let mut root_path = None;
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--root" {
            i += 1;
            let value = args
                .get(i)
                .ok_or_else(|| CliError::usage("p9-stdio --root expects DIR"))?;
            if root_path.is_some() {
                return Err(CliError::usage("p9-stdio accepts only one --root"));
            }
            root_path = Some(PathBuf::from(value));
            i += 1;
        } else {
            return Err(CliError::usage(format!(
                "unexpected p9-stdio argument: {}",
                args[i].to_string_lossy()
            )));
        }
    }

    let root_path = root_path.ok_or_else(|| CliError::usage("p9-stdio requires --root DIR"))?;
    Ok(P9StdioCommand { root_path })
}

pub(super) fn run_p9_stdio(
    command: P9StdioCommand,
    process_stdin: &mut dyn Read,
) -> Result<CliOutput, CliError> {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let exit_code = run_p9_stdio_streaming(command, process_stdin, &mut stdout, &mut stderr)?;
    Ok(CliOutput::new(stdout, stderr, exit_code))
}

pub(super) fn run_p9_stdio_streaming(
    command: P9StdioCommand,
    process_stdin: &mut dyn Read,
    process_stdout: &mut dyn Write,
    process_stderr: &mut dyn Write,
) -> Result<i32, CliError> {
    let root = LocalFs::new(&command.root_path).map_err(|error| {
        CliError::new(
            format!(
                "failed to open p9-stdio root {}: {error}",
                command.root_path.display()
            ),
            1,
        )
    })?;
    let root: Arc<dyn FileSystem> = Arc::new(root);
    let mut server = P9Server::new(root);

    match server.serve_stream(process_stdin, process_stdout) {
        Ok(_) => Ok(0),
        Err(error) => {
            write_process_output(
                process_stderr,
                "stderr",
                format!("wanix-rust p9-stdio: {error}\n").as_bytes(),
            )?;
            Ok(1)
        }
    }
}
