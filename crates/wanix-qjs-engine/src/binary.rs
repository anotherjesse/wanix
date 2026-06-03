use crate::allocation::try_copy_bytes;
use anyhow::{Result, bail};

/// A JavaScript typed-array wrapper kind.
///
/// Values copied through this enum preserve the JavaScript wrapper kind but not
/// element-level typing in Rust: bytes are exposed as the raw visible byte
/// range, in the order QuickJS stores them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum QuickJsTypedArrayKind {
    /// JavaScript `Uint8ClampedArray`.
    Uint8Clamped,
    /// JavaScript `Int8Array`.
    Int8,
    /// JavaScript `Uint8Array`.
    Uint8,
    /// JavaScript `Int16Array`.
    Int16,
    /// JavaScript `Uint16Array`.
    Uint16,
    /// JavaScript `Int32Array`.
    Int32,
    /// JavaScript `Uint32Array`.
    Uint32,
    /// JavaScript `BigInt64Array`.
    BigInt64,
    /// JavaScript `BigUint64Array`.
    BigUint64,
    /// JavaScript `Float16Array`.
    Float16,
    /// JavaScript `Float32Array`.
    Float32,
    /// JavaScript `Float64Array`.
    Float64,
}

impl QuickJsTypedArrayKind {
    /// Returns the wrapper kind from the QuickJS C ABI tag.
    pub(crate) fn from_abi(value: i32) -> Option<Self> {
        match value {
            0 => Some(Self::Uint8Clamped),
            1 => Some(Self::Int8),
            2 => Some(Self::Uint8),
            3 => Some(Self::Int16),
            4 => Some(Self::Uint16),
            5 => Some(Self::Int32),
            6 => Some(Self::Uint32),
            7 => Some(Self::BigInt64),
            8 => Some(Self::BigUint64),
            9 => Some(Self::Float16),
            10 => Some(Self::Float32),
            11 => Some(Self::Float64),
            _ => None,
        }
    }

    /// Returns the QuickJS C ABI tag for this wrapper kind.
    pub(crate) fn abi(self) -> i32 {
        match self {
            Self::Uint8Clamped => 0,
            Self::Int8 => 1,
            Self::Uint8 => 2,
            Self::Int16 => 3,
            Self::Uint16 => 4,
            Self::Int32 => 5,
            Self::Uint32 => 6,
            Self::BigInt64 => 7,
            Self::BigUint64 => 8,
            Self::Float16 => 9,
            Self::Float32 => 10,
            Self::Float64 => 11,
        }
    }

    /// Returns the JavaScript constructor name for this wrapper kind.
    #[must_use]
    pub fn js_name(self) -> &'static str {
        match self {
            Self::Uint8Clamped => "Uint8ClampedArray",
            Self::Int8 => "Int8Array",
            Self::Uint8 => "Uint8Array",
            Self::Int16 => "Int16Array",
            Self::Uint16 => "Uint16Array",
            Self::Int32 => "Int32Array",
            Self::Uint32 => "Uint32Array",
            Self::BigInt64 => "BigInt64Array",
            Self::BigUint64 => "BigUint64Array",
            Self::Float16 => "Float16Array",
            Self::Float32 => "Float32Array",
            Self::Float64 => "Float64Array",
        }
    }

    /// Returns the element width in bytes.
    #[must_use]
    pub fn bytes_per_element(self) -> usize {
        match self {
            Self::Uint8Clamped | Self::Int8 | Self::Uint8 => 1,
            Self::Int16 | Self::Uint16 | Self::Float16 => 2,
            Self::Int32 | Self::Uint32 | Self::Float32 => 4,
            Self::BigInt64 | Self::BigUint64 | Self::Float64 => 8,
        }
    }

    pub(crate) fn validate_byte_len(self, len: usize) -> Result<()> {
        let element_width = self.bytes_per_element();
        if !len.is_multiple_of(element_width) {
            bail!(
                "QuickJS {} byte length {len} is not a multiple of element width {element_width}",
                self.js_name()
            );
        }
        Ok(())
    }
}

/// A small owned binary value copied between Rust and QuickJS.
///
/// This intentionally avoids exposing raw QuickJS handles or guest pointers.
/// Bytes are copied into Rust-owned memory when read from JavaScript and copied
/// into QuickJS-owned memory when written back.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum QuickJsBinaryValue {
    /// A JavaScript `ArrayBuffer` copied as bytes.
    ArrayBuffer(Vec<u8>),
    /// A JavaScript `Uint8Array` copied as its visible byte range.
    Uint8Array(Vec<u8>),
    /// A JavaScript typed array copied as its visible raw byte range.
    ///
    /// `Uint8Array` keeps its dedicated variant for source compatibility. Other
    /// typed-array wrappers use this variant with an exact kind tag.
    TypedArray {
        /// The exact JavaScript typed-array wrapper kind.
        kind: QuickJsTypedArrayKind,
        /// The copied visible raw bytes.
        bytes: Vec<u8>,
    },
    /// A JavaScript `DataView` copied as its visible byte range.
    DataView(Vec<u8>),
}

impl QuickJsBinaryValue {
    /// Copies bytes into an `ArrayBuffer` binary value.
    ///
    /// # Errors
    ///
    /// Returns an error if the byte buffer cannot be copied.
    pub fn array_buffer(bytes: &[u8]) -> Result<Self> {
        Ok(Self::ArrayBuffer(try_copy_bytes(
            bytes,
            "QuickJS ArrayBuffer",
        )?))
    }

    /// Copies bytes into a `Uint8Array` binary value.
    ///
    /// # Errors
    ///
    /// Returns an error if the byte buffer cannot be copied.
    pub fn uint8_array(bytes: &[u8]) -> Result<Self> {
        Ok(Self::Uint8Array(try_copy_bytes(
            bytes,
            "QuickJS Uint8Array",
        )?))
    }

    /// Copies bytes into a typed-array binary value.
    ///
    /// # Errors
    ///
    /// Returns an error if the byte buffer cannot be copied or its length is
    /// not a multiple of the typed-array element width.
    pub fn typed_array(kind: QuickJsTypedArrayKind, bytes: &[u8]) -> Result<Self> {
        kind.validate_byte_len(bytes.len())?;
        if kind == QuickJsTypedArrayKind::Uint8 {
            return Self::uint8_array(bytes);
        }
        Ok(Self::TypedArray {
            kind,
            bytes: try_copy_bytes(bytes, kind.js_name())?,
        })
    }

    /// Copies bytes into a `DataView` binary value.
    ///
    /// # Errors
    ///
    /// Returns an error if the byte buffer cannot be copied.
    pub fn data_view(bytes: &[u8]) -> Result<Self> {
        Ok(Self::DataView(try_copy_bytes(bytes, "QuickJS DataView")?))
    }

    pub(crate) fn typed_array_from_bytes(
        kind: QuickJsTypedArrayKind,
        bytes: Vec<u8>,
    ) -> Result<Self> {
        kind.validate_byte_len(bytes.len())?;
        if kind == QuickJsTypedArrayKind::Uint8 {
            return Ok(Self::Uint8Array(bytes));
        }
        Ok(Self::TypedArray { kind, bytes })
    }

    /// Returns the copied bytes.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        match self {
            Self::ArrayBuffer(bytes)
            | Self::Uint8Array(bytes)
            | Self::TypedArray { bytes, .. }
            | Self::DataView(bytes) => bytes,
        }
    }

    /// Consumes this value and returns its copied bytes.
    #[must_use]
    pub fn into_bytes(self) -> Vec<u8> {
        match self {
            Self::ArrayBuffer(bytes)
            | Self::Uint8Array(bytes)
            | Self::TypedArray { bytes, .. }
            | Self::DataView(bytes) => bytes,
        }
    }
}
