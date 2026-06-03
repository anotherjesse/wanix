(module
  (import "wasi_snapshot_preview1" "fd_write" (func $fd_write (param i32 i32 i32 i32) (result i32)))

  (memory (export "memory") 1)
  (global $__stack_pointer (export "__stack_pointer") (mut i32) (i32.const 65536))
  (global $heap (mut i32) (i32.const 4096))

  (data (i32.const 1024) "G:4096\n")
  (data (i32.const 1032) "G:4103\n")
  (data (i32.const 1040) "G:4111\n")
  (data (i32.const 1048) "G:4127\n")
  (data (i32.const 1056) "G:other\n")
  (data (i32.const 1064) "G:4109\n")
  (data (i32.const 1280) "G:4113\n")
  (data (i32.const 1288) "B:4127\n")
  (data (i32.const 1072) "C:1216\n")
  (data (i32.const 1080) "C:other\n")
  (data (i32.const 1096) "V:eval\n")
  (data (i32.const 1112) "V:global\n")
  (data (i32.const 1128) "V:function\n")
  (data (i32.const 1144) "V:undefined\n")
  (data (i32.const 1160) "V:string\n")
  (data (i32.const 1176) "V:call\n")
  (data (i32.const 1192) "V:other\n")
  (data (i32.const 1200) "V:exception\n")
  (data (i32.const 1216) "ok\00")
  (data (i32.const 1232) "J:run\n")
  (data (i32.const 1240) "G:4134\n")
  (data (i32.const 1248) "V:bigint\n")

  (func $emit (param $ptr i32) (param $len i32)
    i32.const 64
    local.get $ptr
    i32.store
    i32.const 68
    local.get $len
    i32.store
    i32.const 1
    i32.const 64
    i32.const 1
    i32.const 80
    call $fd_write
    drop)

  (func (export "_initialize"))

  (func (export "qjs_init") (result i32)
    i32.const 0)

  (func (export "qjs_destroy"))

  (func (export "qjs_eval")
    (param $code i32)
    (param $code_len i32)
    (param $filename i32)
    (param $flags i32)
    (result i32)
    i32.const 4)
  (func (export "qjs_compile")
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
    i32.const 4127)
  (func (export "qjs_eval_bytecode") (param $buf i32) (param $buf_len i32) (result i32)
    i32.const 50)
  (func (export "qjs_free_bytecode") (param $buf i32)
    local.get $buf
    i32.const 4127
    i32.eq
    if
      i32.const 1288
      i32.const 7
      call $emit
    end)
  (func (export "qjs_run_gc"))
  (func (export "qjs_set_gc_threshold") (param $threshold i32))
  (func (export "qjs_get_gc_threshold") (result i32)
    i32.const 0)
  (func (export "qjs_compute_memory_usage") (param $out i32))

  (func (export "qjs_new_string") (param $ptr i32) (param $len i32) (result i32)
    i32.const 40)
  (func (export "qjs_new_array_buffer") (param $ptr i32) (param $len i32) (result i32)
    i32.const 4)
  (func (export "qjs_new_uint8_array") (param $ptr i32) (param $len i32) (result i32)
    i32.const 4)
  (func (export "qjs_new_typed_array") (param $kind i32) (param $ptr i32) (param $len i32) (result i32)
    i32.const 4)
  (func (export "qjs_new_data_view") (param $ptr i32) (param $len i32) (result i32)
    i32.const 4)
  (func (export "qjs_get_undefined") (result i32)
    i32.const 30)
  (func (export "qjs_get_null") (result i32)
    i32.const 70)
  (func (export "qjs_get_true") (result i32)
    i32.const 80)
  (func (export "qjs_get_false") (result i32)
    i32.const 90)
  (func (export "qjs_get_global") (result i32)
    i32.const 10)
  (func (export "qjs_get_prop_string") (param $global i32) (param $name i32) (result i32)
    i32.const 20)
  (func (export "qjs_set_prop_string") (param $global i32) (param $name i32) (param $value i32) (result i32)
    i32.const 0)
  (func (export "qjs_call")
    (param $function i32)
    (param $this_value i32)
    (param $argc i32)
    (param $argv i32)
    (result i32)
    i32.const 50)
  (func (export "qjs_is_exception") (param $value i32) (result i32)
    i32.const 0)
  (func (export "qjs_is_number") (param $value i32) (result i32)
    i32.const 0)
  (func (export "qjs_is_string") (param $value i32) (result i32)
    i32.const 1)
  (func (export "qjs_is_array_buffer") (param $value i32) (result i32)
    i32.const 1)
  (func (export "qjs_is_uint8_array") (param $value i32) (result i32)
    i32.const 0)
  (func (export "qjs_is_undefined") (param $value i32) (result i32)
    i32.const 0)
  (func (export "qjs_is_null") (param $value i32) (result i32)
    i32.const 0)
  (func (export "qjs_is_bool") (param $value i32) (result i32)
    i32.const 0)
  (func (export "qjs_get_bool") (param $value i32) (result i32)
    i32.const 0)
  (func (export "qjs_get_exception") (result i32)
    i32.const 60)
  (func (export "qjs_get_float64") (param $value i32) (result f64)
    f64.const 0)
  (func (export "qjs_new_number") (param $value f64) (result i32)
    i32.const 70)
  (func (export "qjs_new_big_int64") (param $lo i32) (param $hi i32) (result i32)
    i32.const 100)
  (func (export "qjs_get_string") (param $value i32) (result i32)
    i32.const 1216)
  (func (export "qjs_is_big_int") (param $value i32) (result i32)
    i32.const 0)
  (func (export "qjs_get_big_int64")
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
    i32.const 0)
  (func (export "qjs_get_array_buffer") (param $value i32) (param $len_out i32) (result i32)
    local.get $len_out
    i32.const 2
    i32.store
    i32.const 4127)
  (func (export "qjs_get_uint8_array") (param $value i32) (param $len_out i32) (result i32)
    local.get $len_out
    i32.const 2
    i32.store
    i32.const 4127)

  (func (export "qjs_free_cstring") (param $ptr i32)
    local.get $ptr
    i32.const 1216
    i32.eq
    if
      i32.const 1072
      i32.const 7
      call $emit
      return
    end
    i32.const 1080
    i32.const 8
    call $emit)

  (func (export "qjs_free_value") (param $value i32)
    local.get $value
    i32.const 4
    i32.eq
    if
      i32.const 1096
      i32.const 7
      call $emit
      return
    end
    local.get $value
    i32.const 10
    i32.eq
    if
      i32.const 1112
      i32.const 9
      call $emit
      return
    end
    local.get $value
    i32.const 20
    i32.eq
    if
      i32.const 1128
      i32.const 11
      call $emit
      return
    end
    local.get $value
    i32.const 30
    i32.eq
    if
      i32.const 1144
      i32.const 12
      call $emit
      return
    end
    local.get $value
    i32.const 40
    i32.eq
    if
      i32.const 1160
      i32.const 9
      call $emit
      return
    end
    local.get $value
    i32.const 50
    i32.eq
    if
      i32.const 1176
      i32.const 7
      call $emit
      return
    end
    local.get $value
    i32.const 60
    i32.eq
    if
      i32.const 1200
      i32.const 12
      call $emit
      return
    end
    local.get $value
    i32.const 100
    i32.eq
    if
      i32.const 1248
      i32.const 9
      call $emit
      return
    end
    i32.const 1192
    i32.const 8
    call $emit)

  (func (export "qjs_is_job_pending") (result i32)
    i32.const 0)
  (func (export "qjs_execute_pending_job") (result i32)
    i32.const 0)
  (func (export "qjs_get_runtime_ptr") (result i32)
    i32.const 256)
  (func (export "qjs_get_context_ptr") (result i32)
    i32.const 512)
  (func (export "qjs_set_runtime_and_context") (param $runtime i32) (param $context i32))

  (func (export "wasm_malloc") (param $size i32) (result i32)
    (local $ptr i32)
    global.get $heap
    local.set $ptr
    global.get $heap
    local.get $size
    i32.add
    global.set $heap
    local.get $ptr)

  (func (export "wasm_free") (param $ptr i32)
    local.get $ptr
    i32.const 4096
    i32.eq
    if
      i32.const 1024
      i32.const 7
      call $emit
      return
    end
    local.get $ptr
    i32.const 4103
    i32.eq
    if
      i32.const 1032
      i32.const 7
      call $emit
      return
    end
    local.get $ptr
    i32.const 4109
    i32.eq
    if
      i32.const 1064
      i32.const 7
      call $emit
      return
    end
    local.get $ptr
    i32.const 4111
    i32.eq
    if
      i32.const 1040
      i32.const 7
      call $emit
      return
    end
    local.get $ptr
    i32.const 4113
    i32.eq
    if
      i32.const 1280
      i32.const 7
      call $emit
      return
    end
    local.get $ptr
    i32.const 4134
    i32.eq
    if
      i32.const 1240
      i32.const 7
      call $emit
      return
    end
    local.get $ptr
    i32.const 4127
    i32.eq
    if
      i32.const 1048
      i32.const 7
      call $emit
      return
    end
    i32.const 1056
    i32.const 8
    call $emit)
)
