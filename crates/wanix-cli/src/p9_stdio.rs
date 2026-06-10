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
                format!("wanix p9-stdio: {error}\n").as_bytes(),
            )?;
            Ok(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use wanix_protocol::{P9_VERSION_9P2000_L, p9_tversion};

    use super::{P9StdioCommand, parse_p9_stdio_command, run_p9_stdio, run_p9_stdio_streaming};

    #[test]
    fn parse_p9_stdio_accepts_root_option() {
        let command =
            parse_p9_stdio_command(&[OsString::from("--root"), OsString::from("/tmp/wanix-root")])
                .unwrap();

        assert_eq!(command.root_path, PathBuf::from("/tmp/wanix-root"));
    }

    #[test]
    fn parse_p9_stdio_reports_option_errors() {
        let cases = [
            (vec![], "p9-stdio requires --root DIR"),
            (
                vec![OsString::from("--root")],
                "p9-stdio --root expects DIR",
            ),
            (
                vec![
                    OsString::from("--root"),
                    OsString::from("."),
                    OsString::from("--root"),
                    OsString::from("."),
                ],
                "p9-stdio accepts only one --root",
            ),
            (
                vec![OsString::from("--bad")],
                "unexpected p9-stdio argument: --bad",
            ),
        ];

        for (args, expected) in cases {
            let error = parse_p9_stdio_command(&args).unwrap_err();
            assert!(
                error.to_string().contains(expected),
                "{error} did not contain {expected}"
            );
        }
    }

    #[test]
    fn p9_stdio_streaming_reports_missing_root() {
        let command = P9StdioCommand {
            root_path: temp_dir_path("wanix-cli-p9-stdio-missing-root"),
        };
        let mut stdin = std::io::empty();
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        let error =
            run_p9_stdio_streaming(command, &mut stdin, &mut stdout, &mut stderr).unwrap_err();

        assert_eq!(error.exit_code(), 1);
        assert!(error.to_string().contains("failed to open p9-stdio root"));
        assert!(stdout.is_empty());
        assert!(stderr.is_empty());
    }

    #[test]
    fn p9_stdio_collects_transport_errors() {
        let root = temp_dir("wanix-cli-p9-stdio-unit-error");
        let command = P9StdioCommand { root_path: root };
        let mut input = p9_tversion(1, 8192, P9_VERSION_9P2000_L)
            .unwrap()
            .encode()
            .unwrap();
        input.truncate(input.len() - 1);

        let output = run_p9_stdio(command, &mut input.as_slice()).unwrap();

        assert_eq!(output.exit_code(), 1);
        assert!(output.stdout().is_empty());
        assert!(String::from_utf8_lossy(output.stderr()).contains("wanix p9-stdio"));
    }

    fn temp_dir(name: &str) -> PathBuf {
        let path = temp_dir_path(name);
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn temp_dir_path(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("{name}-{}-{nanos}", std::process::id()))
    }
}
