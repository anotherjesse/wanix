use crate::QuickJsBinaryValue;
use anyhow::{Result, bail};

/// A small owned scalar value copied between Rust and QuickJS.
///
/// This intentionally avoids exposing raw QuickJS handles or guest pointers. It
/// is a copied value surface: strings are copied into Rust-owned memory and
/// BigInts are copied as exact signed 64-bit integers. Object, array, function,
/// promise, symbol, and larger BigInt handles remain private until a future
/// owned-handle API is designed.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub enum QuickJsValue {
    /// JavaScript `undefined`.
    Undefined,
    /// JavaScript `null`.
    Null,
    /// A JavaScript boolean.
    Bool(bool),
    /// A JavaScript number represented as an `f64`.
    ///
    /// Equality follows Rust `f64` equality: `NaN` is not equal to itself and
    /// `+0.0` compares equal to `-0.0`.
    Number(f64),
    /// A JavaScript string copied into Rust-owned memory.
    String(String),
    /// A JavaScript `BigInt` copied as an exact signed 64-bit integer.
    ///
    /// BigInts outside the signed 64-bit range are rejected instead of being
    /// truncated or wrapped.
    BigIntI64(i64),
}

/// A small owned value copied across Rust and QuickJS boundaries.
///
/// This explicit wrapper lets APIs opt in to accepting or returning copied
/// binary payloads alongside scalar values without
/// exposing raw QuickJS handles.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum QuickJsCopiedValue {
    /// A copied scalar JavaScript value.
    Scalar(QuickJsValue),
    /// A copied binary JavaScript value.
    Binary(QuickJsBinaryValue),
}

impl From<QuickJsValue> for QuickJsCopiedValue {
    fn from(value: QuickJsValue) -> Self {
        Self::Scalar(value)
    }
}

impl From<QuickJsBinaryValue> for QuickJsCopiedValue {
    fn from(value: QuickJsBinaryValue) -> Self {
        Self::Binary(value)
    }
}

/// Compatibility alias for binary-capable callback APIs.
///
/// Prefer [`QuickJsCopiedValue`] for new non-callback APIs. This alias keeps
/// existing callback examples and callers source-compatible.
pub type QuickJsCallbackValue = QuickJsCopiedValue;

/// Compatibility alias for callback APIs.
///
/// Prefer [`QuickJsValue`] for new code. This alias keeps existing callback
/// examples and callers source-compatible while the shared scalar value surface
/// becomes the primary name.
pub type QuickJsHostValue = QuickJsValue;

pub(crate) type HostCallback =
    Box<dyn FnMut(&[QuickJsCopiedValue]) -> Result<QuickJsCopiedValue> + Send + 'static>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HostCallbackMode {
    Scalar,
    BinaryCapable,
}

pub(crate) struct HostCallbackEntry {
    callback: HostCallback,
    mode: HostCallbackMode,
}

impl HostCallbackEntry {
    pub(crate) fn new(callback: HostCallback, mode: HostCallbackMode) -> Self {
        Self { callback, mode }
    }

    pub(crate) fn mode(&self) -> HostCallbackMode {
        self.mode
    }

    pub(crate) fn callback_mut(&mut self) -> &mut HostCallback {
        &mut self.callback
    }
}

pub(crate) fn scalar_host_callback<F>(mut callback: F) -> HostCallback
where
    F: FnMut(&[QuickJsValue]) -> Result<QuickJsValue> + Send + 'static,
{
    Box::new(move |args| {
        let scalar_args = args
            .iter()
            .map(|arg| match arg {
                QuickJsCopiedValue::Scalar(value) => Ok(value.clone()),
                QuickJsCopiedValue::Binary(_) => bail!("unsupported host callback argument type"),
            })
            .collect::<Result<Vec<_>>>()?;
        callback(&scalar_args).map(QuickJsCopiedValue::Scalar)
    })
}
