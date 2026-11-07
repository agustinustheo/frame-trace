;; A test runtime that imports a host function polkadot-frame-trace does not provide.
;; It exists only to show the missing host function error.
(module
  (import "env" "ext_frame_trace_missing_version_1" (func))
  (memory (export "memory") 1)
  (global (export "__heap_base") i32 (i32.const 1024))
  (func (export "Core_execute_block") (param i32 i32) (result i64) i64.const 0))
