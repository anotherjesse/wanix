//! Raw WASI Preview 1 `poll_oneoff` (fd_read-only) plus the shell's
//! stdin-watching wait.
//!
//! The Wanix linker (`wanix-wasi-host`) implements exactly the `fd_read`
//! subscription subset, so this hand-rolled binding carries no `wasi` crate
//! dependency. [`watch_fd_or_stdin`] is the cancellation primitive behind the
//! streaming `cat` and the foreground child wait: it parks on
//! `{fd, stdin}`, treats Ctrl-C (`0x03`) on stdin as cancellation, and
//! preserves other typed bytes as type-ahead in `pending`.

use std::collections::VecDeque;
use std::io::Read;

const SUBSCRIPTION_SIZE: usize = 48;
const EVENT_SIZE: usize = 32;
const EVENTTYPE_FD_READ: u8 = 1;
const STDIN_FD: u32 = 0;
const CTRL_C: u8 = 0x03;
const STDIN_DRAIN_BYTES: usize = 8192;

#[link(wasm_import_module = "wasi_snapshot_preview1")]
extern "C" {
    fn poll_oneoff(
        in_ptr: *const u8,
        out_ptr: *mut u8,
        nsubscriptions: usize,
        nevents_ptr: *mut usize,
    ) -> u16;
}

/// What a watched wait observed.
pub enum Watch {
    /// The watched fd is readable (bytes, end-of-stream, or an error a read
    /// will surface).
    Ready,
    /// Ctrl-C arrived on stdin; bytes typed before it are discarded (the
    /// cancelled line), bytes after it stay pending.
    CtrlC,
}

/// Blocks until `fd` is readable or Ctrl-C arrives on interactive stdin.
///
/// Non-Ctrl-C stdin bytes are appended to `pending` (served to the next
/// `read_stdin`), so type-ahead survives a foreground command. Stdin
/// end-of-stream stops the watch half (the wait then parks on `fd` alone).
pub fn watch_fd_or_stdin(fd: u32, pending: &mut VecDeque<u8>) -> Result<Watch, String> {
    if take_ctrl_c(pending) {
        return Ok(Watch::CtrlC);
    }
    let mut stdin_open = true;
    loop {
        let ready = if stdin_open {
            wait_read_ready(&[STDIN_FD, fd])?
        } else {
            wait_read_ready(&[fd])?
        };
        if stdin_open && ready.contains(&STDIN_FD) {
            // Drain what is available; one large read bypasses std's stdin
            // buffering so nothing hides from the next poll.
            let mut buf = [0u8; STDIN_DRAIN_BYTES];
            match std::io::stdin().read(&mut buf) {
                Ok(0) | Err(_) => stdin_open = false,
                Ok(count) => pending.extend(&buf[..count]),
            }
            if take_ctrl_c(pending) {
                return Ok(Watch::CtrlC);
            }
        }
        if ready.iter().any(|ready_fd| *ready_fd == fd) {
            return Ok(Watch::Ready);
        }
    }
}

/// Removes the first Ctrl-C from `pending` (with the cancelled bytes typed
/// before it) and reports whether one was found.
fn take_ctrl_c(pending: &mut VecDeque<u8>) -> bool {
    if let Some(position) = pending.iter().position(|byte| *byte == CTRL_C) {
        pending.drain(..=position);
        return true;
    }
    false
}

/// Blocks until at least one of `fds` is read-ready (or in a readiness-error
/// state a read will surface) and returns the decided fds.
fn wait_read_ready(fds: &[u32]) -> Result<Vec<u32>, String> {
    let mut subscriptions = Vec::with_capacity(fds.len() * SUBSCRIPTION_SIZE);
    for (index, fd) in fds.iter().enumerate() {
        let mut record = [0u8; SUBSCRIPTION_SIZE];
        record[0..8].copy_from_slice(&(index as u64).to_le_bytes());
        record[8] = EVENTTYPE_FD_READ;
        record[16..20].copy_from_slice(&fd.to_le_bytes());
        subscriptions.extend_from_slice(&record);
    }
    let mut events = vec![0u8; fds.len() * EVENT_SIZE];
    let mut nevents: usize = 0;
    let errno = unsafe {
        poll_oneoff(
            subscriptions.as_ptr(),
            events.as_mut_ptr(),
            fds.len(),
            &mut nevents,
        )
    };
    if errno != 0 {
        return Err(format!("poll_oneoff failed: errno {errno}"));
    }
    let mut ready = Vec::with_capacity(nevents);
    for event in events[..nevents * EVENT_SIZE].chunks_exact(EVENT_SIZE) {
        let userdata = u64::from_le_bytes(event[0..8].try_into().expect("8 bytes")) as usize;
        if let Some(fd) = fds.get(userdata) {
            ready.push(*fd);
        }
    }
    Ok(ready)
}
