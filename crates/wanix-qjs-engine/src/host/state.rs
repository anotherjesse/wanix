use super::PromiseRejectionHandler;
use super::QuickJsHostConfig;
use super::callback::{HostCallbackEntry, HostCallbackMode, QuickJsCopiedValue};
use super::fs::{FIRST_VIRTUAL_FILE_FD, PREOPEN_ROOT_FD, VirtualFileHandle};
use super::module_loader::ModuleLoader;
use super::promise_rejection::QuickJsPromiseRejection;
use super::wasi_host::{QuickJsWasiErrno, QuickJsWasiHostHandle};
use crate::allocation::try_copy_str;
use anyhow::{Result, bail};
use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::time::Duration;
use wanix_wasi::{Errno, WasiCtx, WasiFd};
use wasmtime::Memory;

const RIGHT_FD_READ: u64 = 1 << 1;
const RIGHT_FD_WRITE: u64 = 1 << 6;

/// Outcome of a `poll_oneoff` readiness probe over either WASI backing.
pub(super) enum PollFdReady {
    /// The fd is ready now; emit a success event.
    Ready,
    /// The fd is not ready; report it as pending.
    Pending,
    /// Emit an event carrying this errno.
    Errno(QuickJsWasiErrno),
    /// No live WASI provider can answer this probe.
    Unsupported,
}

fn errno_to_quickjs(errno: Errno) -> QuickJsWasiErrno {
    match errno {
        Errno::Badf => QuickJsWasiErrno::Badf,
        Errno::Inval => QuickJsWasiErrno::Inval,
        Errno::Nametoolong => QuickJsWasiErrno::Nametoolong,
        Errno::Noent => QuickJsWasiErrno::Noent,
        Errno::Exist => QuickJsWasiErrno::Exist,
        Errno::Notdir => QuickJsWasiErrno::Notdir,
        Errno::Isdir => QuickJsWasiErrno::Isdir,
        Errno::Notempty => QuickJsWasiErrno::Notempty,
        Errno::Nosys => QuickJsWasiErrno::Nosys,
        Errno::Notcapable => QuickJsWasiErrno::Notcapable,
        Errno::Success | Errno::Io => QuickJsWasiErrno::Io,
    }
}

/// The live WASI provider backing a runtime's Preview 1 imports.
///
/// `Ctx` runs on the shared [`wanix_wasi_host`] linker (the single
/// guest-memory <-> [`WasiCtx`] marshalling). `Trait` is the engine's
/// host-owned hook surface, used by embedders that supply their own provider
/// (and by the engine's generic-host tests). `None` leaves stdio capture and
/// read-only virtual files on their deterministic engine behavior.
pub(crate) enum WasiBacking {
    None,
    Trait(QuickJsWasiHostHandle),
    Ctx(Box<WasiCtx>),
}

impl WasiBacking {
    pub(crate) const fn is_ctx(&self) -> bool {
        matches!(self, Self::Ctx(_))
    }
}

/// A hook the embedder installs to observe `proc_exit` for a `Ctx` backing.
pub(crate) type ProcExitHook = Box<dyn FnMut(i32) + Send + 'static>;

pub(crate) struct HostState {
    memory: Option<Memory>,
    config: QuickJsHostConfig,
    captured_stdout: Vec<u8>,
    captured_stderr: Vec<u8>,
    host_callbacks: HashMap<String, HostCallbackEntry>,
    host_callback_depth: usize,
    module_loader: Option<ModuleLoader>,
    module_loader_depth: usize,
    interrupt_handler: Option<Box<dyn FnMut() -> bool + Send + 'static>>,
    interrupt_handler_depth: usize,
    promise_rejection_handler: Option<PromiseRejectionHandler>,
    promise_rejection_handler_depth: usize,
    wasi_backing: WasiBacking,
    proc_exit_hook: Option<ProcExitHook>,
    process_exited: bool,
    virtual_file_fds: BTreeMap<i32, VirtualFileHandle>,
    next_virtual_file_fd: i32,
}

impl HostState {
    #[cfg(test)]
    pub(crate) fn new(config: QuickJsHostConfig) -> Self {
        Self::new_with_backing(config, WasiBacking::None, None)
    }

    #[cfg(test)]
    pub(crate) fn new_with_wasi_host(
        config: QuickJsHostConfig,
        wasi_host: Option<QuickJsWasiHostHandle>,
    ) -> Self {
        let backing = match wasi_host {
            Some(host) => WasiBacking::Trait(host),
            None => WasiBacking::None,
        };
        Self::new_with_backing(config, backing, None)
    }

    pub(crate) fn new_with_backing(
        config: QuickJsHostConfig,
        wasi_backing: WasiBacking,
        proc_exit_hook: Option<ProcExitHook>,
    ) -> Self {
        Self {
            memory: None,
            config,
            captured_stdout: Vec::new(),
            captured_stderr: Vec::new(),
            host_callbacks: HashMap::new(),
            host_callback_depth: 0,
            module_loader: None,
            module_loader_depth: 0,
            interrupt_handler: None,
            interrupt_handler_depth: 0,
            promise_rejection_handler: None,
            promise_rejection_handler_depth: 0,
            wasi_backing,
            proc_exit_hook,
            process_exited: false,
            virtual_file_fds: BTreeMap::new(),
            next_virtual_file_fd: FIRST_VIRTUAL_FILE_FD,
        }
    }

    pub(crate) fn set_memory(&mut self, memory: Memory) {
        self.memory = Some(memory);
    }

    pub(crate) fn captured_stdout(&self) -> &[u8] {
        &self.captured_stdout
    }

    pub(crate) fn captured_stderr(&self) -> &[u8] {
        &self.captured_stderr
    }

    pub(crate) fn take_captured_stdout(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.captured_stdout)
    }

    pub(crate) fn take_captured_stderr(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.captured_stderr)
    }

    pub(super) fn config(&self) -> &QuickJsHostConfig {
        &self.config
    }

    pub(crate) fn advance_clock_time_by(&mut self, duration: Duration) -> Result<()> {
        let delta_ns = u64::try_from(duration.as_nanos())
            .map_err(|_| anyhow::anyhow!("clock advance duration does not fit in u64 ns"))?;
        let next = self
            .config
            .clock_time_ns()
            .checked_add(delta_ns)
            .ok_or_else(|| anyhow::anyhow!("clock advance overflowed"))?;
        self.config.set_clock_time_ns(next);
        Ok(())
    }

    pub(super) fn wasi_host(&self) -> Option<QuickJsWasiHostHandle> {
        match &self.wasi_backing {
            WasiBacking::Trait(host) => Some(Arc::clone(host)),
            WasiBacking::None | WasiBacking::Ctx(_) => None,
        }
    }

    /// Resolves a `poll_oneoff` fd-read/-write readiness probe over whichever
    /// live WASI backing is attached, normalizing both to [`PollFdReady`].
    pub(super) fn poll_fd_ready(
        &mut self,
        fd: u32,
        want_read: bool,
    ) -> wasmtime::Result<PollFdReady> {
        match &mut self.wasi_backing {
            WasiBacking::None => Ok(PollFdReady::Unsupported),
            WasiBacking::Ctx(ctx) => {
                let ready = if want_read {
                    ctx.fd_read_ready(WasiFd::new(fd))
                } else {
                    ctx.fd_write_ready(WasiFd::new(fd))
                };
                Ok(match ready {
                    Ok(true) => PollFdReady::Ready,
                    Ok(false) => PollFdReady::Pending,
                    Err(errno) => PollFdReady::Errno(errno_to_quickjs(errno)),
                })
            }
            WasiBacking::Trait(host) => {
                let mut host = host
                    .lock()
                    .map_err(|_| wasmtime::Error::msg("QuickJS WASI host lock poisoned"))?;
                let required_right = if want_read {
                    RIGHT_FD_READ
                } else {
                    RIGHT_FD_WRITE
                };
                let stat = match host.fd_fdstat_get(fd) {
                    Ok(stat) => stat,
                    Err(errno) => return Ok(PollFdReady::Errno(errno)),
                };
                if stat.rights_base() & required_right == 0 {
                    return Ok(PollFdReady::Errno(QuickJsWasiErrno::Notcapable));
                }
                let ready = if want_read {
                    host.fd_read_ready(fd)
                } else {
                    host.fd_write_ready(fd)
                };
                Ok(match ready {
                    Ok(true) => PollFdReady::Ready,
                    Ok(false) => PollFdReady::Pending,
                    Err(errno) => PollFdReady::Errno(errno),
                })
            }
        }
    }

    pub(crate) fn wasi_host_snapshot_blockers(&self) -> Result<Vec<String>> {
        match &self.wasi_backing {
            WasiBacking::None => Ok(Vec::new()),
            WasiBacking::Ctx(ctx) => {
                // Reproduce the prior adapter semantics from the shared context.
                let open_fds = ctx.open_dynamic_fd_count();
                if open_fds == 0 {
                    Ok(Vec::new())
                } else {
                    Ok(vec![format!("{open_fds} open dynamic WASI fd(s)")])
                }
            }
            WasiBacking::Trait(host) => host
                .lock()
                .map_err(|_| anyhow::anyhow!("WASI host lock poisoned"))?
                .snapshot_blockers()
                .map_err(|errno| {
                    anyhow::anyhow!("WASI host snapshot blocker check failed: {errno:?}")
                }),
        }
    }

    pub(crate) fn mark_process_exited(&mut self) {
        self.process_exited = true;
    }

    pub(crate) fn process_exited(&self) -> bool {
        self.process_exited
    }

    pub(super) fn memory(&self) -> Option<Memory> {
        self.memory
    }

    pub(super) fn has_virtual_filesystem(&self) -> bool {
        self.config.has_read_only_virtual_files()
    }

    pub(super) fn is_virtual_preopen_fd(&self, fd: i32) -> bool {
        fd == PREOPEN_ROOT_FD && self.has_virtual_filesystem()
    }

    pub(super) fn open_virtual_file(&mut self, path: Vec<u8>, rights_base: u64) -> Option<i32> {
        let bytes = self.config.read_only_virtual_file(&path)?;
        let fd = self.next_virtual_file_fd;
        self.next_virtual_file_fd = self.next_virtual_file_fd.checked_add(1)?;
        self.virtual_file_fds.insert(
            fd,
            VirtualFileHandle {
                bytes,
                offset: 0,
                rights_base,
            },
        );
        Some(fd)
    }

    pub(super) fn virtual_file(&self, fd: i32) -> Option<&VirtualFileHandle> {
        self.virtual_file_fds.get(&fd)
    }

    pub(super) fn virtual_file_mut(&mut self, fd: i32) -> Option<&mut VirtualFileHandle> {
        self.virtual_file_fds.get_mut(&fd)
    }

    pub(super) fn close_virtual_file(&mut self, fd: i32) -> bool {
        self.virtual_file_fds.remove(&fd).is_some()
    }

    pub(crate) fn open_virtual_file_count(&self) -> usize {
        self.virtual_file_fds.len()
    }

    pub(crate) fn contains_host_callback(&self, name: &str) -> bool {
        self.host_callbacks.contains_key(name)
    }

    pub(crate) fn insert_host_callback(
        &mut self,
        name: String,
        callback: HostCallbackEntry,
    ) -> Result<()> {
        if self.host_callbacks.contains_key(&name) {
            bail!("host callback '{name}' is already registered");
        }
        self.host_callbacks.insert(name, callback);
        Ok(())
    }

    pub(crate) fn host_callback_mode(&self, name: &str) -> Result<HostCallbackMode> {
        self.host_callbacks
            .get(name)
            .map(HostCallbackEntry::mode)
            .ok_or_else(|| anyhow::anyhow!("host callback '{name}' is not registered"))
    }

    pub(crate) fn host_callback_depth(&self) -> usize {
        self.host_callback_depth
    }

    pub(crate) fn set_module_loader(&mut self, module_loader: ModuleLoader) {
        self.module_loader = Some(module_loader);
    }

    pub(crate) fn module_loader_depth(&self) -> usize {
        self.module_loader_depth
    }

    pub(crate) fn set_interrupt_handler(
        &mut self,
        interrupt_handler: Box<dyn FnMut() -> bool + Send + 'static>,
    ) {
        self.interrupt_handler = Some(interrupt_handler);
    }

    pub(crate) fn clear_interrupt_handler(&mut self) {
        self.interrupt_handler = None;
    }

    pub(crate) fn interrupt_handler_depth(&self) -> usize {
        self.interrupt_handler_depth
    }

    pub(crate) fn set_promise_rejection_handler(&mut self, handler: PromiseRejectionHandler) {
        self.promise_rejection_handler = Some(handler);
    }

    pub(crate) fn clear_promise_rejection_handler(&mut self) {
        self.promise_rejection_handler = None;
    }

    pub(crate) fn has_promise_rejection_handler(&self) -> bool {
        self.promise_rejection_handler.is_some()
    }

    pub(crate) fn promise_rejection_handler_depth(&self) -> usize {
        self.promise_rejection_handler_depth
    }

    pub(super) fn interrupt_requested(&mut self) -> bool {
        if self.interrupt_handler.is_none() {
            return false;
        }
        let Some(next_depth) = self.interrupt_handler_depth.checked_add(1) else {
            return true;
        };
        self.interrupt_handler_depth = next_depth;
        let result = {
            let _guard = InterruptHandlerDepthGuard {
                depth: &mut self.interrupt_handler_depth,
            };
            if let Some(interrupt_handler) = self.interrupt_handler.as_mut() {
                catch_unwind(AssertUnwindSafe(interrupt_handler))
            } else {
                return false;
            }
        };
        result.unwrap_or(true)
    }

    pub(super) fn handle_promise_rejection(&mut self, event: QuickJsPromiseRejection) {
        if self.promise_rejection_handler.is_none() {
            return;
        }
        let Some(next_depth) = self.promise_rejection_handler_depth.checked_add(1) else {
            return;
        };
        self.promise_rejection_handler_depth = next_depth;
        let _guard = PromiseRejectionHandlerDepthGuard {
            depth: &mut self.promise_rejection_handler_depth,
        };
        if let Some(handler) = self.promise_rejection_handler.as_mut() {
            let _ = catch_unwind(AssertUnwindSafe(|| handler(event)));
        }
    }

    pub(super) fn normalize_module(&mut self, base_name: &str, name: &str) -> Result<String> {
        self.with_module_loader("module normalizer", |module_loader| {
            match module_loader.normalize.as_mut() {
                Some(normalize) => normalize(base_name, name),
                None => try_copy_str(name, "module specifier"),
            }
        })
    }

    pub(super) fn load_module(&mut self, name: &str) -> Result<String> {
        self.with_module_loader("module loader", |module_loader| (module_loader.load)(name))
    }

    pub(super) fn call_host_callback(
        &mut self,
        name: &str,
        args: &[QuickJsCopiedValue],
    ) -> Result<QuickJsCopiedValue> {
        if !self.host_callbacks.contains_key(name) {
            bail!("host callback '{name}' is not registered");
        }
        self.host_callback_depth = self
            .host_callback_depth
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("host callback depth overflowed"))?;
        let result = {
            let _guard = HostCallbackDepthGuard {
                depth: &mut self.host_callback_depth,
            };
            if let Some(entry) = self.host_callbacks.get_mut(name) {
                catch_unwind(AssertUnwindSafe(|| (entry.callback_mut())(args)))
            } else {
                return Err(anyhow::anyhow!("host callback '{name}' is not registered"));
            }
        };
        match result {
            Ok(result) => result,
            Err(_payload) => Err(anyhow::anyhow!("host callback panicked")),
        }
    }

    fn with_module_loader<T>(
        &mut self,
        callback_label: &'static str,
        callback: impl FnOnce(&mut ModuleLoader) -> Result<T>,
    ) -> Result<T> {
        if self.module_loader.is_none() {
            bail!("module loader is not registered");
        }
        self.module_loader_depth = self
            .module_loader_depth
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("module loader depth overflowed"))?;
        let result = {
            let _guard = ModuleLoaderDepthGuard {
                depth: &mut self.module_loader_depth,
            };
            if let Some(module_loader) = self.module_loader.as_mut() {
                catch_unwind(AssertUnwindSafe(|| callback(module_loader)))
            } else {
                return Err(anyhow::anyhow!("module loader is not registered"));
            }
        };
        match result {
            Ok(result) => result,
            Err(_payload) => Err(anyhow::anyhow!("{callback_label} panicked")),
        }
    }

    pub(super) fn append_captured_stdout(&mut self, bytes: &[u8]) -> wasmtime::Result<()> {
        self.reserve_captured_stdout(bytes.len())?;
        self.captured_stdout.extend_from_slice(bytes);
        Ok(())
    }

    pub(super) fn append_captured_stderr(&mut self, bytes: &[u8]) -> wasmtime::Result<()> {
        self.reserve_captured_stderr(bytes.len())?;
        self.captured_stderr.extend_from_slice(bytes);
        Ok(())
    }

    pub(super) fn reserve_captured_stdout(&mut self, additional: usize) -> wasmtime::Result<()> {
        reserve_captured_buffer(
            &mut self.captured_stdout,
            additional,
            self.config.stdout_capture_byte_limit(),
            "stdout",
        )
    }

    pub(super) fn reserve_captured_stderr(&mut self, additional: usize) -> wasmtime::Result<()> {
        reserve_captured_buffer(
            &mut self.captured_stderr,
            additional,
            self.config.stderr_capture_byte_limit(),
            "stderr",
        )
    }
}

impl wanix_wasi_host::WasiHost for HostState {
    fn wasi(&mut self) -> &mut WasiCtx {
        match &mut self.wasi_backing {
            WasiBacking::Ctx(ctx) => ctx,
            WasiBacking::None | WasiBacking::Trait(_) => {
                // The shared linker is only attached for a `Ctx` backing, so the
                // marshalling never reaches a non-`Ctx` state here.
                unreachable!("shared WASI linker invoked without a WasiCtx backing")
            }
        }
    }

    fn clock_time_ns(&self) -> u64 {
        self.config.clock_time_ns()
    }

    fn on_proc_exit(&mut self, code: i32) {
        self.process_exited = true;
        if let Some(hook) = self.proc_exit_hook.as_mut() {
            hook(code);
        }
    }
}

struct HostCallbackDepthGuard<'a> {
    depth: &'a mut usize,
}

impl Drop for HostCallbackDepthGuard<'_> {
    fn drop(&mut self) {
        *self.depth -= 1;
    }
}

struct ModuleLoaderDepthGuard<'a> {
    depth: &'a mut usize,
}

struct InterruptHandlerDepthGuard<'a> {
    depth: &'a mut usize,
}

struct PromiseRejectionHandlerDepthGuard<'a> {
    depth: &'a mut usize,
}

impl Drop for InterruptHandlerDepthGuard<'_> {
    fn drop(&mut self) {
        *self.depth -= 1;
    }
}

impl Drop for PromiseRejectionHandlerDepthGuard<'_> {
    fn drop(&mut self) {
        *self.depth -= 1;
    }
}

impl Drop for ModuleLoaderDepthGuard<'_> {
    fn drop(&mut self) {
        *self.depth -= 1;
    }
}

impl fmt::Debug for HostState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HostState")
            .field("memory", &self.memory)
            .field("config", &self.config)
            .field("captured_stdout_len", &self.captured_stdout.len())
            .field("captured_stderr_len", &self.captured_stderr.len())
            .field("host_callback_count", &self.host_callbacks.len())
            .field("host_callback_depth", &self.host_callback_depth)
            .field("has_module_loader", &self.module_loader.is_some())
            .field("module_loader_depth", &self.module_loader_depth)
            .field("has_interrupt_handler", &self.interrupt_handler.is_some())
            .field("interrupt_handler_depth", &self.interrupt_handler_depth)
            .field(
                "has_promise_rejection_handler",
                &self.promise_rejection_handler.is_some(),
            )
            .field(
                "promise_rejection_handler_depth",
                &self.promise_rejection_handler_depth,
            )
            .field("open_virtual_file_count", &self.virtual_file_fds.len())
            .finish()
    }
}

fn reserve_captured_buffer(
    buffer: &mut Vec<u8>,
    additional: usize,
    byte_limit: Option<usize>,
    stream: &str,
) -> wasmtime::Result<()> {
    let needed = buffer.len().checked_add(additional).ok_or_else(|| {
        wasmtime::Error::msg(format!(
            "captured {stream} byte limit exceeded: retained byte count overflow"
        ))
    })?;
    if let Some(byte_limit) = byte_limit
        && needed > byte_limit
    {
        return Err(wasmtime::Error::msg(format!(
            "captured {stream} byte limit exceeded: {needed} bytes would exceed {byte_limit}-byte limit"
        )));
    }
    buffer.try_reserve(additional).map_err(|err| {
        wasmtime::Error::msg(format!("captured {stream} buffer is too large: {err}"))
    })
}
