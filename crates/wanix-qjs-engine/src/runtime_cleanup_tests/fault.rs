const EVAL_VALUE_FREE_RETURN: &str = r#"      i32.const 1096
      i32.const 7
      call $emit
      return"#;
const EVAL_VALUE_FREE_TRAP: &str = r#"      i32.const 1096
      i32.const 7
      call $emit
      unreachable"#;

const C_STRING_FREE_RETURN: &str = r#"      i32.const 1072
      i32.const 7
      call $emit
      return"#;
const C_STRING_FREE_TRAP: &str = r#"      i32.const 1072
      i32.const 7
      call $emit
      unreachable"#;

const C_STRING_DATA_VALID: &str = r#"  (data (i32.const 1216) "ok\00")"#;
const C_STRING_DATA_INVALID_UTF8: &str = r#"  (data (i32.const 1216) "\ff\00")"#;

const FUNCTION_VALUE_FREE_RETURN: &str = r#"      i32.const 1128
      i32.const 11
      call $emit
      return"#;
const FUNCTION_VALUE_FREE_TRAP: &str = r#"      i32.const 1128
      i32.const 11
      call $emit
      unreachable"#;

const QJS_EVAL_RETURN: &str = r#"  (func (export "qjs_eval")
    (param $code i32)
    (param $code_len i32)
    (param $filename i32)
    (param $flags i32)
    (result i32)
    i32.const 4)"#;
const QJS_EVAL_TRAP: &str = r#"  (func (export "qjs_eval")
    (param $code i32)
    (param $code_len i32)
    (param $filename i32)
    (param $flags i32)
    (result i32)
    unreachable)"#;
const QJS_EVAL_RETURN_NULL: &str = r#"  (func (export "qjs_eval")
    (param $code i32)
    (param $code_len i32)
    (param $filename i32)
    (param $flags i32)
    (result i32)
    i32.const 0)"#;

const QJS_COMPILE_RETURN: &str = r#"  (func (export "qjs_compile")
    (param $code i32)
    (param $code_len i32)
    (param $filename i32)
    (param $eval_flags i32)
    (param $write_flags i32)
    (param $out_len i32)
    (result i32)
    local.get $out_len
    i32.const 2
    i32.store
    i32.const 4127)"#;
const QJS_COMPILE_RETURN_OVERSIZED_BUFFER: &str = r#"  (func (export "qjs_compile")
    (param $code i32)
    (param $code_len i32)
    (param $filename i32)
    (param $eval_flags i32)
    (param $write_flags i32)
    (param $out_len i32)
    (result i32)
    local.get $out_len
    i32.const 650000
    i32.store
    i32.const 4127)"#;
const QJS_COMPILE_TRAP: &str = r#"  (func (export "qjs_compile")
    (param $code i32)
    (param $code_len i32)
    (param $filename i32)
    (param $eval_flags i32)
    (param $write_flags i32)
    (param $out_len i32)
    (result i32)
    unreachable)"#;
const QJS_COMPILE_RETURN_NULL: &str = r#"  (func (export "qjs_compile")
    (param $code i32)
    (param $code_len i32)
    (param $filename i32)
    (param $eval_flags i32)
    (param $write_flags i32)
    (param $out_len i32)
    (result i32)
    local.get $out_len
    i32.const 0
    i32.store
    i32.const 0)"#;

const QJS_FREE_BYTECODE_RETURN: &str = r#"  (func (export "qjs_free_bytecode") (param $buf i32)
    local.get $buf
    i32.const 4127
    i32.eq
    if
      i32.const 1288
      i32.const 7
      call $emit
    end)"#;
const QJS_FREE_BYTECODE_TRAP: &str = r#"  (func (export "qjs_free_bytecode") (param $buf i32)
    local.get $buf
    i32.const 4127
    i32.eq
    if
      i32.const 1288
      i32.const 7
      call $emit
      unreachable
    end)"#;

const QJS_CALL_RETURN: &str = r#"  (func (export "qjs_call")
    (param $function i32)
    (param $this_value i32)
    (param $argc i32)
    (param $argv i32)
    (result i32)
    i32.const 50)"#;
const QJS_CALL_TRAP: &str = r#"  (func (export "qjs_call")
    (param $function i32)
    (param $this_value i32)
    (param $argc i32)
    (param $argv i32)
    (result i32)
    unreachable)"#;

const QJS_IS_EXCEPTION_RETURN_NONE: &str = r#"  (func (export "qjs_is_exception") (param $value i32) (result i32)
    i32.const 0)"#;
const QJS_IS_EXCEPTION_FOR_EVAL: &str = r#"  (func (export "qjs_is_exception") (param $value i32) (result i32)
    local.get $value
    i32.const 4
    i32.eq)"#;
const QJS_IS_EXCEPTION_FOR_CALL: &str = r#"  (func (export "qjs_is_exception") (param $value i32) (result i32)
    local.get $value
    i32.const 50
    i32.eq)"#;
const QJS_IS_EXCEPTION_TRAP: &str = r#"  (func (export "qjs_is_exception") (param $value i32) (result i32)
    unreachable)"#;

const QJS_IS_NUMBER_RETURN_FALSE: &str = r#"  (func (export "qjs_is_number") (param $value i32) (result i32)
    i32.const 0)"#;
const QJS_IS_NUMBER_TRAP: &str = r#"  (func (export "qjs_is_number") (param $value i32) (result i32)
    unreachable)"#;
const QJS_IS_NUMBER_RETURN_TRUE: &str = r#"  (func (export "qjs_is_number") (param $value i32) (result i32)
    i32.const 1)"#;

const QJS_IS_STRING_RETURN_TRUE: &str = r#"  (func (export "qjs_is_string") (param $value i32) (result i32)
    i32.const 1)"#;
const QJS_IS_STRING_RETURN_FALSE: &str = r#"  (func (export "qjs_is_string") (param $value i32) (result i32)
    i32.const 0)"#;
const QJS_IS_STRING_TRAP: &str = r#"  (func (export "qjs_is_string") (param $value i32) (result i32)
    unreachable)"#;

const QJS_GET_EXCEPTION_RETURN: &str = r#"  (func (export "qjs_get_exception") (result i32)
    i32.const 60)"#;
const QJS_GET_EXCEPTION_TRAP: &str = r#"  (func (export "qjs_get_exception") (result i32)
    unreachable)"#;

const QJS_GET_STRING_RETURN: &str = r#"  (func (export "qjs_get_string") (param $value i32) (result i32)
    i32.const 1216)"#;
const QJS_GET_STRING_TRAP: &str = r#"  (func (export "qjs_get_string") (param $value i32) (result i32)
    unreachable)"#;
const QJS_GET_STRING_RETURN_NULL: &str = r#"  (func (export "qjs_get_string") (param $value i32) (result i32)
    i32.const 0)"#;

const QJS_NEW_ARRAY_BUFFER_RETURN: &str = r#"  (func (export "qjs_new_array_buffer") (param $ptr i32) (param $len i32) (result i32)
    i32.const 4)"#;
const QJS_NEW_ARRAY_BUFFER_TRAP: &str = r#"  (func (export "qjs_new_array_buffer") (param $ptr i32) (param $len i32) (result i32)
    unreachable)"#;

const QJS_NEW_TYPED_ARRAY_RETURN: &str = r#"  (func (export "qjs_new_typed_array") (param $kind i32) (param $ptr i32) (param $len i32) (result i32)
    i32.const 4)"#;
const QJS_NEW_TYPED_ARRAY_TRAP: &str = r#"  (func (export "qjs_new_typed_array") (param $kind i32) (param $ptr i32) (param $len i32) (result i32)
    unreachable)"#;

const QJS_NEW_DATA_VIEW_RETURN: &str = r#"  (func (export "qjs_new_data_view") (param $ptr i32) (param $len i32) (result i32)
    i32.const 4)"#;
const QJS_NEW_DATA_VIEW_TRAP: &str = r#"  (func (export "qjs_new_data_view") (param $ptr i32) (param $len i32) (result i32)
    unreachable)"#;

const QJS_NEW_BIG_INT64_RETURN: &str = r#"  (func (export "qjs_new_big_int64") (param $lo i32) (param $hi i32) (result i32)
    i32.const 100)"#;
const QJS_NEW_BIG_INT64_TRAP: &str = r#"  (func (export "qjs_new_big_int64") (param $lo i32) (param $hi i32) (result i32)
    unreachable)"#;

const QJS_IS_BIG_INT_RETURN_FALSE: &str = r#"  (func (export "qjs_is_big_int") (param $value i32) (result i32)
    i32.const 0)"#;
const QJS_IS_BIG_INT_RETURN_TRUE: &str = r#"  (func (export "qjs_is_big_int") (param $value i32) (result i32)
    i32.const 1)"#;
const QJS_GET_BIG_INT64_RETURN: &str = r#"  (func (export "qjs_get_big_int64")
    (param $value i32)
    (param $lo_out i32)
    (param $hi_out i32)
    (result i32)
    local.get $lo_out
    i32.const 42
    i32.store
    local.get $hi_out
    i32.const 1
    i32.store
    i32.const 0)"#;
const QJS_GET_BIG_INT64_TRAP: &str = r#"  (func (export "qjs_get_big_int64")
    (param $value i32)
    (param $lo_out i32)
    (param $hi_out i32)
    (result i32)
    unreachable)"#;
const QJS_GET_BIG_INT64_RETURN_FAILURE: &str = r#"  (func (export "qjs_get_big_int64")
    (param $value i32)
    (param $lo_out i32)
    (param $hi_out i32)
    (result i32)
    i32.const -1)"#;

const QJS_GET_ARRAY_BUFFER_RETURN: &str = r#"  (func (export "qjs_get_array_buffer") (param $value i32) (param $len_out i32) (result i32)
    local.get $len_out
    i32.const 2
    i32.store
    i32.const 4127)"#;
const QJS_GET_ARRAY_BUFFER_TRAP: &str = r#"  (func (export "qjs_get_array_buffer") (param $value i32) (param $len_out i32) (result i32)
    unreachable)"#;
const QJS_GET_ARRAY_BUFFER_RETURN_NULL: &str = r#"  (func (export "qjs_get_array_buffer") (param $value i32) (param $len_out i32) (result i32)
    local.get $len_out
    i32.const 0
    i32.store
    i32.const 0)"#;

const QJS_GET_FLOAT64_RETURN: &str = r#"  (func (export "qjs_get_float64") (param $value i32) (result f64)
    f64.const 0)"#;
const QJS_GET_FLOAT64_TRAP: &str = r#"  (func (export "qjs_get_float64") (param $value i32) (result f64)
    unreachable)"#;

const QJS_COMPUTE_MEMORY_USAGE_RETURN: &str =
    r#"  (func (export "qjs_compute_memory_usage") (param $out i32))"#;
const QJS_COMPUTE_MEMORY_USAGE_TRAP: &str = r#"  (func (export "qjs_compute_memory_usage") (param $out i32)
    unreachable)"#;

const JOB_PENDING_RETURN_EMPTY: &str = r#"  (func (export "qjs_is_job_pending") (result i32)
    i32.const 0)"#;
const JOB_PENDING_RETURN_ONE: &str = r#"  (func (export "qjs_is_job_pending") (result i32)
    i32.const 1)"#;

const EXECUTE_JOB_RETURN_OK: &str = r#"  (func (export "qjs_execute_pending_job") (result i32)
    i32.const 0)"#;
const EXECUTE_JOB_LOG_AND_RETURN_OK: &str = r#"  (func (export "qjs_execute_pending_job") (result i32)
    i32.const 1232
    i32.const 6
    call $emit
    i32.const 0)"#;
const EXECUTE_JOB_RETURN_FAILURE: &str = r#"  (func (export "qjs_execute_pending_job") (result i32)
    i32.const -1)"#;

#[derive(Clone, Copy)]
pub(super) enum CleanupFault {
    EvalValueFreeTrap,
    CStringFreeTrap,
    CStringDataInvalidUtf8,
    FunctionValueFreeTrap,
    QjsEvalTrap,
    QjsEvalReturnsNull,
    QjsCompileTrap,
    QjsCompileReturnsNull,
    QjsCompileReturnsOversizedBuffer,
    QjsFreeBytecodeTrap,
    QjsCallTrap,
    QjsIsExceptionTrap,
    QjsIsNumberTrap,
    QjsIsNumberReturnsTrue,
    QjsIsStringTrap,
    QjsIsStringReturnsFalse,
    QjsGetFloat64Trap,
    QjsComputeMemoryUsageTrap,
    QjsGetExceptionTrap,
    QjsGetStringTrap,
    QjsGetStringReturnsNull,
    QjsIsBigIntReturnsTrue,
    QjsNewBigInt64Trap,
    QjsGetBigInt64Trap,
    QjsGetBigInt64ReturnsFailure,
    QjsNewArrayBufferTrap,
    QjsNewTypedArrayTrap,
    QjsNewDataViewTrap,
    QjsGetArrayBufferTrap,
    QjsGetArrayBufferReturnsNull,
    QjsEvalReturnsException,
    QjsCallReturnsException,
    JobQueueAlwaysPending,
    PendingJobLogsAndSucceeds,
    PendingJobFails,
}

impl CleanupFault {
    pub(super) fn replacement(self) -> (&'static str, &'static str) {
        match self {
            Self::EvalValueFreeTrap => (EVAL_VALUE_FREE_RETURN, EVAL_VALUE_FREE_TRAP),
            Self::CStringFreeTrap => (C_STRING_FREE_RETURN, C_STRING_FREE_TRAP),
            Self::CStringDataInvalidUtf8 => (C_STRING_DATA_VALID, C_STRING_DATA_INVALID_UTF8),
            Self::FunctionValueFreeTrap => (FUNCTION_VALUE_FREE_RETURN, FUNCTION_VALUE_FREE_TRAP),
            Self::QjsEvalTrap => (QJS_EVAL_RETURN, QJS_EVAL_TRAP),
            Self::QjsEvalReturnsNull => (QJS_EVAL_RETURN, QJS_EVAL_RETURN_NULL),
            Self::QjsCompileTrap => (QJS_COMPILE_RETURN, QJS_COMPILE_TRAP),
            Self::QjsCompileReturnsNull => (QJS_COMPILE_RETURN, QJS_COMPILE_RETURN_NULL),
            Self::QjsCompileReturnsOversizedBuffer => {
                (QJS_COMPILE_RETURN, QJS_COMPILE_RETURN_OVERSIZED_BUFFER)
            }
            Self::QjsFreeBytecodeTrap => (QJS_FREE_BYTECODE_RETURN, QJS_FREE_BYTECODE_TRAP),
            Self::QjsCallTrap => (QJS_CALL_RETURN, QJS_CALL_TRAP),
            Self::QjsIsExceptionTrap => (QJS_IS_EXCEPTION_RETURN_NONE, QJS_IS_EXCEPTION_TRAP),
            Self::QjsIsNumberTrap => (QJS_IS_NUMBER_RETURN_FALSE, QJS_IS_NUMBER_TRAP),
            Self::QjsIsNumberReturnsTrue => (QJS_IS_NUMBER_RETURN_FALSE, QJS_IS_NUMBER_RETURN_TRUE),
            Self::QjsIsStringTrap => (QJS_IS_STRING_RETURN_TRUE, QJS_IS_STRING_TRAP),
            Self::QjsIsStringReturnsFalse => {
                (QJS_IS_STRING_RETURN_TRUE, QJS_IS_STRING_RETURN_FALSE)
            }
            Self::QjsGetFloat64Trap => (QJS_GET_FLOAT64_RETURN, QJS_GET_FLOAT64_TRAP),
            Self::QjsComputeMemoryUsageTrap => (
                QJS_COMPUTE_MEMORY_USAGE_RETURN,
                QJS_COMPUTE_MEMORY_USAGE_TRAP,
            ),
            Self::QjsGetExceptionTrap => (QJS_GET_EXCEPTION_RETURN, QJS_GET_EXCEPTION_TRAP),
            Self::QjsGetStringTrap => (QJS_GET_STRING_RETURN, QJS_GET_STRING_TRAP),
            Self::QjsGetStringReturnsNull => (QJS_GET_STRING_RETURN, QJS_GET_STRING_RETURN_NULL),
            Self::QjsIsBigIntReturnsTrue => {
                (QJS_IS_BIG_INT_RETURN_FALSE, QJS_IS_BIG_INT_RETURN_TRUE)
            }
            Self::QjsNewBigInt64Trap => (QJS_NEW_BIG_INT64_RETURN, QJS_NEW_BIG_INT64_TRAP),
            Self::QjsGetBigInt64Trap => (QJS_GET_BIG_INT64_RETURN, QJS_GET_BIG_INT64_TRAP),
            Self::QjsGetBigInt64ReturnsFailure => {
                (QJS_GET_BIG_INT64_RETURN, QJS_GET_BIG_INT64_RETURN_FAILURE)
            }
            Self::QjsNewArrayBufferTrap => (QJS_NEW_ARRAY_BUFFER_RETURN, QJS_NEW_ARRAY_BUFFER_TRAP),
            Self::QjsNewTypedArrayTrap => (QJS_NEW_TYPED_ARRAY_RETURN, QJS_NEW_TYPED_ARRAY_TRAP),
            Self::QjsNewDataViewTrap => (QJS_NEW_DATA_VIEW_RETURN, QJS_NEW_DATA_VIEW_TRAP),
            Self::QjsGetArrayBufferTrap => (QJS_GET_ARRAY_BUFFER_RETURN, QJS_GET_ARRAY_BUFFER_TRAP),
            Self::QjsGetArrayBufferReturnsNull => (
                QJS_GET_ARRAY_BUFFER_RETURN,
                QJS_GET_ARRAY_BUFFER_RETURN_NULL,
            ),
            Self::QjsEvalReturnsException => {
                (QJS_IS_EXCEPTION_RETURN_NONE, QJS_IS_EXCEPTION_FOR_EVAL)
            }
            Self::QjsCallReturnsException => {
                (QJS_IS_EXCEPTION_RETURN_NONE, QJS_IS_EXCEPTION_FOR_CALL)
            }
            Self::JobQueueAlwaysPending => (JOB_PENDING_RETURN_EMPTY, JOB_PENDING_RETURN_ONE),
            Self::PendingJobLogsAndSucceeds => {
                (EXECUTE_JOB_RETURN_OK, EXECUTE_JOB_LOG_AND_RETURN_OK)
            }
            Self::PendingJobFails => (EXECUTE_JOB_RETURN_OK, EXECUTE_JOB_RETURN_FAILURE),
        }
    }
}
