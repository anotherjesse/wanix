use std::ops::{BitAnd, BitAndAssign, BitOr, BitOrAssign, Not};

/// QuickJS intrinsic flags used when creating a fresh runtime.
///
/// Base objects such as `Object`, `Array`, `Number`, `String`, `Boolean`, and
/// `Error` are always installed by the reference adapter and cannot be
/// disabled. Other built-ins can be selected before QuickJS context
/// initialization through [`QuickJsCreateOptions`](crate::QuickJsCreateOptions).
///
/// The bit values match the reference `Intrinsics` constants and `qjs_init2`
/// ABI. Unknown bits are preserved so callers can pass masks understood by a
/// newer compatible QuickJS WASM adapter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QuickJsIntrinsics {
    bits: u32,
}

impl QuickJsIntrinsics {
    /// No optional intrinsics beyond the always-on base objects.
    pub const NONE: Self = Self::from_bits(0);
    /// `Date` constructor and prototype methods.
    pub const DATE: Self = Self::from_bits(1 << 0);
    /// `eval()` and `Function()` constructor.
    pub const EVAL: Self = Self::from_bits(1 << 1);
    /// `RegExp` constructor, prototype methods, and regex literals.
    pub const REGEXP: Self = Self::from_bits(1 << 2);
    /// `JSON.parse()` and `JSON.stringify()`.
    pub const JSON: Self = Self::from_bits(1 << 3);
    /// `Proxy` and related proxy behavior.
    pub const PROXY: Self = Self::from_bits(1 << 4);
    /// `Map`, `Set`, `WeakMap`, and `WeakSet`.
    pub const MAP_SET: Self = Self::from_bits(1 << 5);
    /// `ArrayBuffer`, typed array variants, and `DataView`.
    pub const TYPED_ARRAYS: Self = Self::from_bits(1 << 6);
    /// `Promise` plus `async` and `await` behavior.
    pub const PROMISE: Self = Self::from_bits(1 << 7);
    /// `BigInt`.
    ///
    /// QuickJS-NG currently includes some BigInt behavior in base objects, so
    /// omitting this flag may not remove every BigInt surface.
    pub const BIG_INT: Self = Self::from_bits(1 << 8);
    /// `WeakRef` and `FinalizationRegistry`.
    pub const WEAK_REF: Self = Self::from_bits(1 << 9);
    /// `performance.now()`.
    pub const PERFORMANCE: Self = Self::from_bits(1 << 10);
    /// `DOMException`.
    pub const DOM_EXCEPTION: Self = Self::from_bits(1 << 11);
    /// `atob()` and `btoa()`.
    ///
    /// The reference adapter also installs `DOMException` when this intrinsic
    /// is enabled because the base64 helpers use it for errors.
    pub const ATOB_BTOA: Self = Self::from_bits(1 << 12);
    /// All intrinsics supported by the reference adapter.
    ///
    /// This is `u32::MAX`, matching `qjs_init2`'s all-intrinsics sentinel. It
    /// also preserves future bits for newer compatible adapters, so use an
    /// allow-list mask such as `EVAL | JSON` when policy must enable only known
    /// intrinsics.
    pub const ALL: Self = Self::from_bits(u32::MAX);

    /// Returns a bitmask from raw bits.
    ///
    /// Unknown bits are preserved for newer compatible adapters.
    #[must_use]
    pub const fn from_bits(bits: u32) -> Self {
        Self { bits }
    }

    /// Returns the raw bitmask passed to the QuickJS WASM adapter.
    #[must_use]
    pub const fn bits(self) -> u32 {
        self.bits
    }

    /// Returns a mask with `intrinsics` removed.
    #[must_use]
    pub const fn without(self, intrinsics: Self) -> Self {
        Self::from_bits(self.bits & !intrinsics.bits)
    }

    /// Returns whether all bits in `intrinsics` are present in this mask.
    #[must_use]
    pub const fn contains(self, intrinsics: Self) -> bool {
        (self.bits & intrinsics.bits) == intrinsics.bits
    }
}

impl Default for QuickJsIntrinsics {
    fn default() -> Self {
        Self::ALL
    }
}

impl BitOr for QuickJsIntrinsics {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self::from_bits(self.bits | rhs.bits)
    }
}

impl BitOrAssign for QuickJsIntrinsics {
    fn bitor_assign(&mut self, rhs: Self) {
        self.bits |= rhs.bits;
    }
}

impl BitAnd for QuickJsIntrinsics {
    type Output = Self;

    fn bitand(self, rhs: Self) -> Self::Output {
        Self::from_bits(self.bits & rhs.bits)
    }
}

impl BitAndAssign for QuickJsIntrinsics {
    fn bitand_assign(&mut self, rhs: Self) {
        self.bits &= rhs.bits;
    }
}

impl Not for QuickJsIntrinsics {
    type Output = Self;

    fn not(self) -> Self::Output {
        Self::from_bits(!self.bits)
    }
}

/// Fresh-runtime creation options.
///
/// These options are used only for creating a new QuickJS runtime. Restoring a
/// snapshot bypasses QuickJS initialization and resumes the already-created
/// context stored in the snapshot, so restore APIs continue to accept only the
/// Rust host configuration that should be reattached.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct QuickJsCreateOptions {
    host_config: crate::QuickJsHostConfig,
    intrinsics: Option<QuickJsIntrinsics>,
}

impl QuickJsCreateOptions {
    /// Returns default create options.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Uses explicit host import configuration for the fresh runtime.
    #[must_use]
    pub fn with_host_config(mut self, config: crate::QuickJsHostConfig) -> Self {
        self.host_config = config;
        self
    }

    /// Selects the QuickJS built-ins installed in the fresh runtime.
    ///
    /// Setting this option makes runtime creation call the optional `qjs_init2`
    /// export. Omit it to preserve ABI-v1 compatibility through the required
    /// `qjs_init` export.
    #[must_use]
    pub fn with_intrinsics(mut self, intrinsics: QuickJsIntrinsics) -> Self {
        self.intrinsics = Some(intrinsics);
        self
    }

    /// Returns the host import configuration for this fresh runtime.
    #[must_use]
    pub fn host_config(&self) -> &crate::QuickJsHostConfig {
        &self.host_config
    }

    /// Returns the selected intrinsic mask, if explicit intrinsics were set.
    #[must_use]
    pub const fn intrinsics(&self) -> Option<QuickJsIntrinsics> {
        self.intrinsics
    }

    pub(crate) fn into_parts(self) -> (crate::QuickJsHostConfig, Option<QuickJsIntrinsics>) {
        (self.host_config, self.intrinsics)
    }
}
