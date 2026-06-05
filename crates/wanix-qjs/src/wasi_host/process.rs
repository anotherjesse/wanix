use rust_wasi_quickjs::QuickJsWasiErrno;

use super::WanixQuickJsWasiHost;

const MAX_WASI_EXIT_STATUS: i32 = 255;

pub(super) fn snapshot_blockers(
    host: &mut WanixQuickJsWasiHost,
) -> Result<Vec<String>, QuickJsWasiErrno> {
    let open_fds = host.ctx.open_dynamic_fd_count();
    if open_fds == 0 {
        Ok(Vec::new())
    } else {
        Ok(vec![format!("{open_fds} open dynamic WASI fd(s)")])
    }
}

pub(super) fn args(host: &mut WanixQuickJsWasiHost) -> Result<Vec<String>, QuickJsWasiErrno> {
    Ok(host.ctx.args().to_vec())
}

pub(super) fn env(host: &mut WanixQuickJsWasiHost) -> Result<Vec<String>, QuickJsWasiErrno> {
    Ok(host.ctx.env().to_vec())
}

pub(super) fn proc_exit(
    host: &mut WanixQuickJsWasiHost,
    code: u32,
) -> Result<(), QuickJsWasiErrno> {
    let code = i32::try_from(code).map_err(|_| QuickJsWasiErrno::Inval)?;
    if !(0..=MAX_WASI_EXIT_STATUS).contains(&code) {
        return Err(QuickJsWasiErrno::Inval);
    }
    let Some(exit_state) = &host.exit_state else {
        return Err(QuickJsWasiErrno::Nosys);
    };
    exit_state
        .request_exit(code)
        .map_err(|_| QuickJsWasiErrno::Io)
}
