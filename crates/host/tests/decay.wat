;; A hand-written model of the contract model/1: y' = -k y, y(0) = 1, k = 0.5, with the
;; assignment rule t_half = ln(2) / k. It is the native test model `Decay { k: 0.5 }` of the
;; engine: the parameters are [k, t_half] (t_half is not settable), the state is [y], the
;; selections are [y, t_half]; it has no events.
;;
;; As every compiled model it imports the memory of the engine and its math from mathlib, and
;; has no memory, data, globals or stack of its own: every pointer is an offset into the
;; imported memory, given by the engine. `math_exp` is imported as a model imports the math it
;; may use (the instance of mathlib must satisfy it); `math_log` computes ln(2).
(module
  (import "env" "memory" (memory 0))
  (import "math" "math_exp" (func $exp (param f64) (result f64)))
  (import "math" "math_log" (func $log (param f64) (result f64)))

  ;; t_half = ln(2) / k, from p[0] into p[1]
  (func $t_half (param $p i32)
    (f64.store offset=8 (local.get $p)
      (f64.div (call $log (f64.const 2)) (f64.load (local.get $p)))))

  (func (export "model_defaults") (param $p i32) (param $y i32)
    (f64.store (local.get $p) (f64.const 0.5))
    (call $t_half (local.get $p))
    (f64.store (local.get $y) (f64.const 1)))

  ;; fixed: a byte per entity, the parameters then the states; t_half is entity 1
  (func (export "model_init") (param $t f64) (param $p i32) (param $y i32) (param $fixed i32)
    (if (i32.eqz (i32.load8_u offset=1 (local.get $fixed)))
      (then (call $t_half (local.get $p)))))

  (func (export "model_rhs") (param $t f64) (param $y i32) (param $p i32) (param $ydot i32)
    (f64.store (local.get $ydot)
      (f64.mul (f64.neg (f64.load (local.get $p))) (f64.load (local.get $y)))))

  (func (export "model_jac") (param $t f64) (param $y i32) (param $p i32) (param $values i32)
    (f64.store (local.get $values) (f64.neg (f64.load (local.get $p)))))

  (func (export "model_roots") (param $t f64) (param $y i32) (param $p i32) (param $g i32))

  (func (export "model_trigger") (param $i i32) (param $t f64) (param $y i32) (param $p i32)
    (result i32)
    (i32.const 0))

  (func (export "model_priority") (param $i i32) (param $t f64) (param $y i32) (param $p i32)
    (result f64)
    (f64.const 0))

  (func (export "model_delay") (param $i i32) (param $t f64) (param $y i32) (param $p i32)
    (result f64)
    (f64.const 0))

  (func (export "model_event_values")
    (param $i i32) (param $t f64) (param $y i32) (param $p i32) (param $out i32))

  (func (export "model_event_apply")
    (param $i i32) (param $t f64) (param $values i32) (param $y i32) (param $p i32))

  (func (export "model_selections") (param $t f64) (param $y i32) (param $p i32) (param $out i32)
    (f64.store (local.get $out) (f64.load (local.get $y)))
    (f64.store offset=8 (local.get $out) (f64.load offset=8 (local.get $p))))

  (func (export "model_assign") (param $t f64) (param $y i32) (param $p i32)
    (call $t_half (local.get $p)))

  ;; the sizes: 1 state, 2 parameters, 2 selections, 1 entry of the Jacobian, 0 root functions,
  ;; 0 events (so model_event_sizes writes nothing)
  (func (export "model_sizes") (param $out i32)
    (i32.store (local.get $out) (i32.const 1))
    (i32.store offset=4 (local.get $out) (i32.const 2))
    (i32.store offset=8 (local.get $out) (i32.const 2))
    (i32.store offset=12 (local.get $out) (i32.const 1))
    (i32.store offset=16 (local.get $out) (i32.const 0))
    (i32.store offset=20 (local.get $out) (i32.const 0)))

  (func (export "model_event_sizes") (param $out i32))
)
