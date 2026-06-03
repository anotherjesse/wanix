pub(super) const WASM_PAGE_SIZE: u64 = 65_536;
/// Snapshot metadata version for the QuickJS WebAssembly ABI contract.
///
/// Reconsider this value when required exports, memory, global, or runtime
/// pointer assumptions change.
pub(crate) const QUICKJS_WASM_ABI_VERSION: u32 = 1;

#[derive(Clone, Copy)]
pub(super) enum AbiVal {
    I32,
    F64,
}

pub(super) struct RequiredFunc {
    pub(super) name: &'static str,
    pub(super) params: &'static [AbiVal],
    pub(super) results: &'static [AbiVal],
}

const NONE: &[AbiVal] = &[];
const I32: &[AbiVal] = &[AbiVal::I32];
const I32_I32: &[AbiVal] = &[AbiVal::I32, AbiVal::I32];
const I32_I32_I32_I32: &[AbiVal] = &[AbiVal::I32, AbiVal::I32, AbiVal::I32, AbiVal::I32];
const I32_F64: &[AbiVal] = &[AbiVal::F64];

pub(super) const REQUIRED_FUNCS: &[RequiredFunc] = &[
    RequiredFunc {
        name: "_initialize",
        params: NONE,
        results: NONE,
    },
    RequiredFunc {
        name: "qjs_init",
        params: NONE,
        results: I32,
    },
    RequiredFunc {
        name: "qjs_destroy",
        params: NONE,
        results: NONE,
    },
    RequiredFunc {
        name: "qjs_eval",
        params: I32_I32_I32_I32,
        results: I32,
    },
    RequiredFunc {
        name: "qjs_new_string",
        params: I32_I32,
        results: I32,
    },
    RequiredFunc {
        name: "qjs_get_undefined",
        params: NONE,
        results: I32,
    },
    RequiredFunc {
        name: "qjs_get_global",
        params: NONE,
        results: I32,
    },
    RequiredFunc {
        name: "qjs_get_prop_string",
        params: I32_I32,
        results: I32,
    },
    RequiredFunc {
        name: "qjs_call",
        params: I32_I32_I32_I32,
        results: I32,
    },
    RequiredFunc {
        name: "qjs_is_exception",
        params: I32,
        results: I32,
    },
    RequiredFunc {
        name: "qjs_is_number",
        params: I32,
        results: I32,
    },
    RequiredFunc {
        name: "qjs_is_string",
        params: I32,
        results: I32,
    },
    RequiredFunc {
        name: "qjs_get_exception",
        params: NONE,
        results: I32,
    },
    RequiredFunc {
        name: "qjs_get_float64",
        params: I32,
        results: I32_F64,
    },
    RequiredFunc {
        name: "qjs_get_string",
        params: I32,
        results: I32,
    },
    RequiredFunc {
        name: "qjs_free_cstring",
        params: I32,
        results: NONE,
    },
    RequiredFunc {
        name: "qjs_free_value",
        params: I32,
        results: NONE,
    },
    RequiredFunc {
        name: "qjs_is_job_pending",
        params: NONE,
        results: I32,
    },
    RequiredFunc {
        name: "qjs_execute_pending_job",
        params: NONE,
        results: I32,
    },
    RequiredFunc {
        name: "qjs_get_runtime_ptr",
        params: NONE,
        results: I32,
    },
    RequiredFunc {
        name: "qjs_get_context_ptr",
        params: NONE,
        results: I32,
    },
    RequiredFunc {
        name: "qjs_set_runtime_and_context",
        params: I32_I32,
        results: NONE,
    },
    RequiredFunc {
        name: "wasm_malloc",
        params: I32,
        results: I32,
    },
    RequiredFunc {
        name: "wasm_free",
        params: I32,
        results: NONE,
    },
];

impl AbiVal {
    pub(super) fn name(self) -> &'static str {
        match self {
            Self::I32 => "i32",
            Self::F64 => "f64",
        }
    }
}
