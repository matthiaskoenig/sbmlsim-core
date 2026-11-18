# Delay differential equations

The compiler supports the SBML MathML `delay` function in expressions, rules,
initial assignments, and events. A model such as

```text
dx/dt = delay(x, 1)
x(t) = 1 for t <= 0
```

has `x(1) = 2`, `x(2) = 3.5`, and `x(3) = 37/6`. Encode `delay` as the SBML
`csymbol`, followed by the expression and duration:

```xml
<apply>
  <csymbol encoding="text"
    definitionURL="http://www.sbml.org/sbml/symbols/delay">delay</csymbol>
  <ci>x</ci>
  <cn>1</cn>
</apply>
```

Simulation requests and results use the same API as ordinary ODE models.
Prepared requests, repeated simulations, native execution, Wasmtime, and
JavaScript execution support delays.

## History and initialization

The first operand is evaluated at the historical time, including historical
state, parameters, assignment rules, and compartment sizes. The duration is
evaluated in the calling context. Nested delays therefore use the historical
context of the outer delay. A zero duration evaluates the current solver state.
Durations must be finite and nonnegative; invalid values return an integration
error with flag `-1006`.

Before time zero, initial values and initial assignments define history according
to the [SBML specification](https://sbml.org/documents/specifications/level-3/version-2/core/release-2/).
Assignment rules still apply at the queried time. The prehistory evaluation
computes only dependencies of the delayed expression. Request pre-initialization
values override the corresponding initial values. `rateOf` is zero before time
zero.

Historical algebraic unknowns are reconstructed by solving their dependency-closed
constraints at the queried time. The projection uses scaled finite differences,
pivoted elimination, and damped Newton iteration. An infeasible constraint or a
failure to converge returns flag `-1006`. Projection code must fit the compiler's
function budgets. Simulations starting before zero use their integrated history
once the requested time lies inside the retained trajectory.

Events and request changes retain both sides of a history discontinuity. A query
exactly at the discontinuity uses the updated value. Steady-state phases retain
their history when the following timecourse resets its clock.

## Integration

The DDE path retains the CVODE or IDA dense polynomial for each accepted solver
step, together with parameter epochs. Output grids do not determine history
resolution. Delayed discontinuities are located with solver roots, including
variable and reversing durations, so a narrow delayed pulse can be resolved even with sparse
requested output times. Discontinuities propagate through derivative order 12.

Queries inside the current trial step use bounded iteration of its continuous
extension. The engine retries or shortens a step when the history has not
converged. It does not impose a step-size ceiling equal to the shortest delay.
Very small delays therefore need not force correspondingly small steps.

DDE models use a colored finite-difference Jacobian because a current state can
also affect a historical query time. Explicitly requesting an analytic Jacobian
for a DDE model is rejected. Solver step budgets and execution deadlines still
apply. History storage is bounded to 1 GiB, and nested evaluation to 128 frames;
exceeding a history limit returns flag `-1006`.

## Retained history

The compiler bounds how far back a model can read when every duration of a
`delay` is a finite nonnegative constant: the bound is the longest chain of
nested durations, `max_delay` of the description. The engine then drops the
accepted steps older than twice that bound and rebuilds its arena without them,
so the memory of a run grows with the longest delay and not with the horizon.
The arena has to double before it is rebuilt again, so the work of the
compactions of a run stays proportional to its steps.

A duration which depends on the state or on the time has no bound, so such a
model keeps every accepted step and its history grows with the horizon. Give the
delays constant durations where the model allows it.

The measured effect on a chain of 20 Mackey-Glass equations with a delay of 2,
integrated to t = 6400 at `relative_tolerance` 1e-9, is a peak resident size of
17 MB instead of 1638 MB, and 2.65 s instead of 3.51 s. The same model without
the bound fails with flag `-1006` once its history outgrows its room.

## Ordinary ODEs

Models without `delay` keep their existing compiled module and description
bytes. The solver selects the DDE path once per model. Ordinary ODE integrators
use a zero-sized history type, without history allocation, recording, or lookup
in solver callbacks. The DDE history ABI is an optional model custom section.

In the fixed comparison against the preceding engine, 10 of 48 ordinary ODE
workloads showed a slowdown, seven showed a speedup, and 31 were inconclusive.
The largest measured slowdowns were 3.90% for native prepared Robertson with
KLU and 2.30% for prepared Adams on WASM. Three baseline controls also showed
timing differences. The retained implementation did not meet the zero-overhead
criterion. [Performance results and simultaneous confidence intervals](https://github.com/matthiaskoenig/sbmlsim-core/blob/develop/bench/dde-performance/results-019e226.md)

## Validation

All 53 prepared SBML Test Suite delay cases pass with optimization enabled and
disabled and with the difference Jacobian option. The complete suite passes
1,754 of 1,789 cases. The remaining 35 require unsupported fast reactions.
Additional integration tests cover nested, zero, state-dependent, and tiny
delays, prehistory, concentrations, events, narrow pulses, prepared replay,
steady-state phases, negative starts, algebraic prehistory, invalid durations,
and step budgets, including rejected overlapping trials.
