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

## Ordinary ODEs

Models without `delay` keep their existing compiled module and description
bytes. The solver selects the DDE path once per model. Ordinary ODE integrators
use a zero-sized history type, without history allocation, recording, or lookup
in solver callbacks. The DDE history ABI is an optional model custom section.

The [performance protocol](https://github.com/matthiaskoenig/sbmlsim-core/blob/develop/bench/dde-performance/protocol.md)
compares ordinary ODE execution against the preceding engine, including native
and Wasmtime execution, events, prepared requests, and both direct solvers.

## Validation

All 53 prepared SBML Test Suite delay cases pass with optimization enabled and
disabled and with the difference Jacobian option. The complete suite passes
1,754 of 1,789 cases. The remaining 35 require unsupported fast reactions.
Additional integration tests cover nested, zero, state-dependent, and tiny
delays, prehistory, concentrations, events, narrow pulses, prepared replay,
steady-state phases, negative starts, algebraic prehistory, invalid durations,
and step budgets, including rejected overlapping trials.
