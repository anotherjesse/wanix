//! Running a parsed `wanix cpu` job over the mesh.
//!
//! Binds a fresh dialer [`MeshNode`] from an ephemeral identity (dialing out
//! needs only an endpoint), dials the data node's cpu exec plane, reverse-exports
//! the scoped working-directory namespace, runs the job to its terminal exit, and
//! writes its captured stdout/stderr and exit code to the process streams.

use std::io::Write;
use std::path::Path;
use std::sync::Arc;

use wanix_cpu::{CpuJobSpec, ExportScope};
use wanix_fs::{FileSystem, LocalFs};
use wanix_id::NodeIdentity;
use wanix_mesh::{CpuJobReport, MeshNode};

use super::parse::CpuCommand;
use crate::mesh::MeshTicket;
use crate::{CliError, write_process_output};

/// Runs a `cpu` job, writing its stdout/stderr to the process and returning the
/// remote task's exit code.
///
/// # Errors
///
/// Returns a CLI error when the ticket is malformed, the dialer endpoint cannot
/// bind, the export scope cannot be built, the QUIC dial fails, or the remote run
/// reports a transport/session failure (a non-zero guest exit is **not** an
/// error — it is returned as the exit code).
pub(crate) fn run_cpu_streaming(
    command: CpuCommand,
    process_stdout: &mut dyn Write,
    process_stderr: &mut dyn Write,
) -> Result<i32, CliError> {
    let ticket = MeshTicket::parse(&command.node)?;
    let export_root = build_export_root(&command.cwd, command.writable)?;
    let spec = build_spec(&command)?;
    let node = bind_dialer_node()?;
    let report = run_job(&node, ticket, &spec, export_root)?;
    write_report(process_stdout, process_stderr, &report)?;
    Ok(report.exit_code)
}

/// Builds the scoped, read-only-by-default reverse export from `cwd`.
fn build_export_root(cwd: &Path, writable: bool) -> Result<Arc<dyn FileSystem>, CliError> {
    let backing = LocalFs::new(cwd).map_err(|error| {
        CliError::new(
            format!("failed to open cpu export root {}: {error}", cwd.display()),
            1,
        )
    })?;
    let mut scope = ExportScope::new(Arc::new(backing), ".");
    if writable {
        scope = scope.writable();
    }
    scope
        .into_root()
        .map_err(|error| CliError::new(format!("failed to build cpu export scope: {error}"), 1))
}

/// Builds the [`CpuJobSpec`] from the parsed command.
fn build_spec(command: &CpuCommand) -> Result<CpuJobSpec, CliError> {
    CpuJobSpec::new(&command.kind, &command.program)
        .map(|spec| {
            spec.with_args(command.args.clone())
                .with_env(command.env.clone())
        })
        .map_err(|error| CliError::new(format!("invalid cpu job spec: {error}"), 1))
}

/// Binds a dialer-only mesh node from an ephemeral identity.
fn bind_dialer_node() -> Result<MeshNode, CliError> {
    let identity = NodeIdentity::generate().map_err(|error| CliError::new(error.to_string(), 1))?;
    MeshNode::bind(&identity)
        .map_err(|error| CliError::new(format!("failed to bind mesh endpoint: {error}"), 1))
}

/// Dials the data node's cpu plane and runs the job to its terminal exit.
fn run_job(
    node: &MeshNode,
    ticket: MeshTicket,
    spec: &CpuJobSpec,
    export_root: Arc<dyn FileSystem>,
) -> Result<CpuJobReport, CliError> {
    let dialer = node
        .dial_cpu(ticket.endpoint_addr())
        .map_err(|error| CliError::new(format!("failed to dial cpu node: {error}"), 1))?;
    dialer
        .run(spec, export_root)
        .map_err(|error| CliError::new(format!("cpu job failed: {error}"), 1))
}

/// Writes the job's captured stdout/stderr to the process streams.
fn write_report(
    process_stdout: &mut dyn Write,
    process_stderr: &mut dyn Write,
    report: &CpuJobReport,
) -> Result<(), CliError> {
    write_process_output(process_stdout, "stdout", &report.output.stdout)?;
    write_process_output(process_stderr, "stderr", &report.output.stderr)
}
