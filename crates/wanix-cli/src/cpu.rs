//! `wanix-rust cpu`: Plan 9 cpu over the mesh — send the job to the data node.
//!
//! The caller dials a remote node's cpu exec plane (ALPN `wanix/cpu/1`),
//! reverse-exports a **scoped, read-only-by-default** sub-namespace built from
//! the local working directory, and asks the node to run a task whose world *is*
//! that exported namespace. The task runs on the remote node against the caller's
//! files over the reverse 9P session; its captured stdout/stderr and exit status
//! return on the control stream and are written to the process here.
//!
//! Demo: `wanix-rust cpu --node iroh://<PEER>[?addr=IP:PORT] -- qjs build.js`.
//!
//! The export root is the caller's `--cwd` (default `.`), scoped read-only with
//! [`wanix_cpu::ExportScope`]; `--write` opts the job subtree into read-write so
//! the remote run can write outputs back. The job command after `--` is
//! `<kind> <program> [args...]`, mirroring the local launch surface.

mod parse;
mod run;

pub(crate) use parse::parse_cpu_command;
pub(crate) use run::run_cpu_streaming;
