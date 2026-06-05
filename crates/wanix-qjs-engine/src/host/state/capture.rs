use super::HostState;

impl HostState {
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

    pub(in crate::host) fn append_captured_stdout(&mut self, bytes: &[u8]) -> wasmtime::Result<()> {
        self.reserve_captured_stdout(bytes.len())?;
        self.captured_stdout.extend_from_slice(bytes);
        Ok(())
    }

    pub(in crate::host) fn append_captured_stderr(&mut self, bytes: &[u8]) -> wasmtime::Result<()> {
        self.reserve_captured_stderr(bytes.len())?;
        self.captured_stderr.extend_from_slice(bytes);
        Ok(())
    }

    pub(in crate::host) fn reserve_captured_stdout(
        &mut self,
        additional: usize,
    ) -> wasmtime::Result<()> {
        reserve_captured_buffer(
            &mut self.captured_stdout,
            additional,
            self.config.stdout_capture_byte_limit(),
            "stdout",
        )
    }

    pub(in crate::host) fn reserve_captured_stderr(
        &mut self,
        additional: usize,
    ) -> wasmtime::Result<()> {
        reserve_captured_buffer(
            &mut self.captured_stderr,
            additional,
            self.config.stderr_capture_byte_limit(),
            "stderr",
        )
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
