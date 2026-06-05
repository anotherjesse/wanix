use std::os::fd::BorrowedFd;

pub(crate) fn with_borrowed_fd<T>(fd: libc::c_int, f: impl FnOnce(BorrowedFd<'_>) -> T) -> T {
    // SAFETY: Callers pass live process-owned fds and the borrowed fd cannot
    // escape this function's closure.
    unsafe { f(BorrowedFd::borrow_raw(fd)) }
}
