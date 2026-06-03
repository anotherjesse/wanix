use super::PromiseRejectionHandler;
use super::QuickJsHostConfig;
use super::callback::{HostCallbackEntry, HostCallbackMode, QuickJsCopiedValue};
use super::fs::{FIRST_VIRTUAL_FILE_FD, PREOPEN_ROOT_FD, VirtualFileHandle};
use super::module_loader::ModuleLoader;
use super::promise_rejection::QuickJsPromiseRejection;
use super::wasi_host::QuickJsWasiHostHandle;
use crate::allocation::try_copy_str;
use anyhow::{Result, bail};
use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::time::Duration;
use wasmtime::Memory;

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
    wasi_host: Option<QuickJsWasiHostHandle>,
    process_exited: bool,
    virtual_file_fds: BTreeMap<i32, VirtualFileHandle>,
    next_virtual_file_fd: i32,
}

impl HostState {
    #[cfg(test)]
    pub(crate) fn new(config: QuickJsHostConfig) -> Self {
        Self::new_with_wasi_host(config, None)
    }

    pub(crate) fn new_with_wasi_host(
        config: QuickJsHostConfig,
        wasi_host: Option<QuickJsWasiHostHandle>,
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
            wasi_host,
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
        self.wasi_host.as_ref().map(Arc::clone)
    }

    pub(crate) fn wasi_host_snapshot_blockers(&self) -> Result<Vec<String>> {
        let Some(wasi_host) = &self.wasi_host else {
            return Ok(Vec::new());
        };
        wasi_host
            .lock()
            .map_err(|_| anyhow::anyhow!("WASI host lock poisoned"))?
            .snapshot_blockers()
            .map_err(|errno| anyhow::anyhow!("WASI host snapshot blocker check failed: {errno:?}"))
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
