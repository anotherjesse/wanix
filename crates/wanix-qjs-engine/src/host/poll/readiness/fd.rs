use wasmtime::Caller;

use super::{EVENTTYPE_FD_READ, event_with_errno};
use crate::host::poll::{events::PollReadiness, layout};
use crate::host::{ERRNO_NOSYS, HostState, QuickJsWasiErrno, QuickJsWasiFdStat};

const RIGHT_FD_READ: u64 = 1 << 1;
const RIGHT_FD_WRITE: u64 = 1 << 6;

pub(super) fn fd_event(
    caller: &Caller<'_, HostState>,
    subscription: &layout::Subscription,
    event_type: u8,
) -> wasmtime::Result<PollReadiness> {
    let Some(host) = caller.data().wasi_host() else {
        return Ok(PollReadiness::Unsupported(ERRNO_NOSYS));
    };
    let fd = layout::subscription_fd(subscription);
    let mut host = host
        .lock()
        .map_err(|_| wasmtime::Error::msg("QuickJS WASI host lock poisoned"))?;
    let errno = match host.fd_fdstat_get(fd) {
        Ok(stat) => readiness_errno(stat, required_right(event_type)),
        Err(errno) => Some(errno),
    };
    if let Some(errno) = errno {
        return Ok(PollReadiness::Ready(event_with_errno(
            subscription,
            event_type,
            Some(errno),
        )));
    }
    Ok(fd_readiness_event(
        host.as_mut(),
        fd,
        subscription,
        event_type,
    ))
}

fn required_right(event_type: u8) -> u64 {
    if event_type == EVENTTYPE_FD_READ {
        RIGHT_FD_READ
    } else {
        RIGHT_FD_WRITE
    }
}

fn fd_readiness_event(
    host: &mut dyn crate::host::QuickJsWasiHost,
    fd: u32,
    subscription: &layout::Subscription,
    event_type: u8,
) -> PollReadiness {
    let ready = if event_type == EVENTTYPE_FD_READ {
        host.fd_read_ready(fd)
    } else {
        host.fd_write_ready(fd)
    };
    fd_ready_event(subscription, event_type, ready)
}

fn fd_ready_event(
    subscription: &layout::Subscription,
    event_type: u8,
    ready: Result<bool, QuickJsWasiErrno>,
) -> PollReadiness {
    match ready {
        Ok(true) => PollReadiness::Ready(event_with_errno(subscription, event_type, None)),
        Ok(false) => PollReadiness::PendingFd,
        Err(errno) => PollReadiness::Ready(event_with_errno(subscription, event_type, Some(errno))),
    }
}

fn readiness_errno(stat: QuickJsWasiFdStat, required_right: u64) -> Option<QuickJsWasiErrno> {
    if stat.rights_base() & required_right == 0 {
        Some(QuickJsWasiErrno::Notcapable)
    } else {
        None
    }
}
