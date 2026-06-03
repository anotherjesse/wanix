use crate::allocation::try_copy_bytes;
use anyhow::{Result, bail};
use std::fmt;

const JS_EVAL_TYPE_MODULE: i32 = 1;
const JS_WRITE_OBJ_STRIP_SOURCE: i32 = 1 << 4;
const JS_WRITE_OBJ_STRIP_DEBUG: i32 = 1 << 5;

/// Trusted QuickJS bytecode tied to the exact QuickJS WASM module identity.
///
/// QuickJS bytecode is not a portable or untrusted interchange format. Treat
/// bytecode bytes as trusted data produced for the same `QuickJsModule` build.
#[derive(Clone, PartialEq, Eq)]
pub struct QuickJsBytecode {
    wasm_sha256: [u8; 32],
    bytes: Vec<u8>,
}

impl QuickJsBytecode {
    pub(crate) fn new(wasm_sha256: [u8; 32], bytes: Vec<u8>) -> Result<Self> {
        if bytes.is_empty() {
            bail!("QuickJS bytecode must not be empty");
        }
        Ok(Self { wasm_sha256, bytes })
    }

    /// Copies trusted bytecode bytes and binds them to the stored module identity.
    ///
    /// This does not validate that the bytes are well-formed QuickJS bytecode;
    /// evaluation remains a trusted operation performed by QuickJS. Persist the
    /// module SHA-256 alongside the bytes and pass the stored SHA-256 here when
    /// reconstructing bytecode.
    ///
    /// # Errors
    ///
    /// Returns an error if the byte buffer is empty or cannot be copied.
    pub fn from_trusted_parts(wasm_sha256: [u8; 32], bytes: &[u8]) -> anyhow::Result<Self> {
        Self::new(wasm_sha256, try_copy_bytes(bytes, "QuickJS bytecode")?)
    }

    /// Returns the exact QuickJS WASM module SHA-256 this bytecode is bound to.
    #[must_use]
    pub fn wasm_sha256(&self) -> [u8; 32] {
        self.wasm_sha256
    }

    /// Returns the serialized QuickJS bytecode bytes.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Consumes this bytecode and returns the module SHA-256 plus bytecode bytes.
    #[must_use]
    pub fn into_parts(self) -> ([u8; 32], Vec<u8>) {
        (self.wasm_sha256, self.bytes)
    }
}

impl fmt::Debug for QuickJsBytecode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("QuickJsBytecode")
            .field("wasm_sha256", &format_sha256_hex(&self.wasm_sha256))
            .field("byte_len", &self.bytes.len())
            .finish_non_exhaustive()
    }
}

fn format_sha256_hex(bytes: &[u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut hex = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        hex.push(char::from(HEX[usize::from(byte >> 4)]));
        hex.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    hex
}

/// Options for compiling JavaScript source to QuickJS bytecode.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct QuickJsBytecodeCompileOptions {
    eval_flags: i32,
    write_flags: i32,
}

impl QuickJsBytecodeCompileOptions {
    /// Returns default script bytecode compilation options.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Compiles the source as an ES module.
    #[must_use]
    pub fn as_module(mut self) -> Self {
        self.eval_flags |= JS_EVAL_TYPE_MODULE;
        self
    }

    /// Omits source text from the serialized bytecode when QuickJS supports it.
    #[must_use]
    pub fn strip_source(mut self) -> Self {
        self.write_flags |= JS_WRITE_OBJ_STRIP_SOURCE;
        self
    }

    /// Omits debug metadata from the serialized bytecode when QuickJS supports it.
    #[must_use]
    pub fn strip_debug(mut self) -> Self {
        self.write_flags |= JS_WRITE_OBJ_STRIP_DEBUG;
        self
    }

    pub(crate) fn eval_flags(self) -> i32 {
        self.eval_flags
    }

    pub(crate) fn write_flags(self) -> i32 {
        self.write_flags
    }
}
