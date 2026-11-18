# Semantics and known differences

The semantics follow the simulation core of sbmlsim (PEtab v2): the pre-initialization, the changes at times and the steady state of [Simulating](simulating.md), the events, and the root finding of the conditions of `piecewise`. ODE models use CVODE (BDF or Adams); models with algebraic rules use IDA (BDF). Both use the dense linear solver or KLU, an analytic sparse Jacobian or the colored sparse difference quotient.

[Delay differential equations](delays.md) use the same solvers with retained dense history and the colored difference Jacobian. The SBML `delay` function evaluates its operand in the historical context, including before time zero.

## Bitwise identical hosts

wasmtime, Node and the browsers run one `engine.wasm`, one `mathlib.wasm` and one `model.wasm`, so a request gives the same bits on every host. The compiler emits the math as written, without fast math, reassociation or folding of constants, with the meaning libroadrunner gives every operator; every math function of a model is the one of `mathlib.wasm` (the functions of the Rust crate `libm`), never the builtin math of a host. The C solvers are compiled with `-ffp-contract=off` and without fast math. The tests check it: every passing case of the SBML Test Suite gives in Node the CSV of the wasmtime host byte for byte, a model posted to workers gives the bits of the main thread, and every repeated simulation of the test suite and of the benchmark repeats the bits of the first.

## The events and the discontinuities

CVODE stops where the branch of a `piecewise` of a rule, a kinetic law or a trigger changes and starts again from the state there, as roadrunner does at its piecewise triggers: the root functions of the discontinuities come after those of the triggers (`n_discontinuity_roots` of `model.json`). The events execute in the order of their priorities; simultaneous events of equal priority are ordered by the random generator of the request (`seed`). The assignments of an event are applied as libroadrunner 2.11 applies them: the targets in document order, the compartments last.

One behavior follows sbmlsim rather than SBML, so it is no difference to sbmlsim: an event with a delay whose trigger a change of the request makes true (also through the events the change executes) does not fire. Its trigger stays armed, so the event fires only after the trigger falls and rises again during the integration. A trigger which the pre-initialization makes true is scheduled as usual.

## The differences a user meets first

- Events which accumulate end with an error of the integration (`CV_TOO_MUCH_WORK`, flag -1): the executions of events within the relative tolerance (as a distance in time) of the last root share the budget of `maximum_num_steps` steps towards one output time, and in the output of the steps of the integrator `maximum_num_steps` consecutive stops within the resolution of the time (1e-9 of the magnitude of the times) end the run. The bouncing ball (`x' = v`, `v' = -10`, the event `x < 0`: `v = -0.5 v`, `x = 0`) fails at its accumulation point t = 1.3416 within milliseconds; roadrunner lets the ball fall through and continues.
- Roots which chatter share that budget too, so a model which roadrunner finishes after millions of steps (a sliding mode at a loose tolerance) may end with `CV_TOO_MUCH_WORK` here.
- Three differences to the roadrunner executor of sbmlsim: a change which sets a state exactly to the threshold of a strict comparison triggers its event right after the change (as the executors of COPASI and AMICI of sbmlsim; roadrunner's CVODE ignores a root which is 0 at the start of a segment); the variable of an assignment rule without math can be changed (the rule is ignored, as roadrunner does; sbmlsim refuses to change it); an entity named `time` is set as an entity by a change (sbmlsim sets the clock of roadrunner).
- A limitation shared with roadrunner: CVODE finds a root where a root function changes its sign across a step, so two roots of one function within one step cancel and are missed. A condition which switches faster than the steps needs `maximum_time_step`, e.g. `piecewise(1, sin(time) > 0, 0)` over many periods with a `maximum_time_step` of 0.5.

## Known differences to libroadrunner

Besides the differences above, which are libroadrunner's as well where sbmlsim keeps them (from the documentation of the engine, `crates/engine/src/lib.rs`):

- The roots of the conditions of `piecewise` are located with continuous root functions of their operands (`a - b`, `min` and `max` for `and` and `or`), libroadrunner's with the booleans of the branches (`1` or `-1`); both stop where the selected branch changes. The kinetic laws with local parameters have them too, libroadrunner's none.
- The steps towards one output time are bounded by `maximum_num_steps` across the returns of CVODE at roots and its restarts at the roots of discontinuities; libroadrunner counts the steps of one call of CVODE, which every return at a root ends, so a model whose roots chatter may run far longer there (the sliding mode `x' = piecewise(-1, x > 0, 1)` at rtol 1e-3, atol 1e-6: 28.6 million steps in libroadrunner, `CV_TOO_MUCH_WORK` within 20000 steps of one output interval here).
- The output of the steps of the integrator ends with `CV_TOO_MUCH_WORK` after `maximum_num_steps` consecutive stops (steps, roots, stop times) within the resolution of the time, 1e-9 times the magnitude of the times of the simulation. libroadrunner has no such bound. A chatter whose stops are further apart than the resolution is bounded by the limit of the output only.
- libroadrunner's count of roots close together ignores the events of an ignored root as well; here it decides the restarts at the roots of discontinuities only, and the events execute whatever it counts.
- The output of the steps of the integrator has the two rows of a root (of an event, or a root of a discontinuity) at its exact time; libroadrunner writes them at `t - tol` and `t + tol` (`tol` its root tolerance, `100 * 32 * eps * (|t| + |h|)`), which sbmlsim moves to the time half way between them for a model with events.
- In the output of the steps of the integrator the triggers at a step are evaluated at its time only: an event executes at its root, as in roadrunner.

The modules `changes` and `events` of the engine document the details of the changes and the events, and `bench/RESULTS.md` the measurements behind them.
