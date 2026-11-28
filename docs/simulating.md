# Simulation

A compiled model (`model.wasm` and `model.json`) is simulated with a request of the contract `simulation/1`. The engine answers with a table: the column `time` and the selections of the request, row major.

## The command line

`sbmlsim-core simulate` runs a model directory in wasmtime with a request and writes the result as CSV (to stdout without `-o`):

```bash
sbmlsim-core simulate crates/host/tests/decay-model crates/host/tests/decay-model/request.json [-o out.csv]
```

The host applies a 30-second WASM execution deadline. `--timeout-ms N` changes it;
`--timeout-ms 0` disables it for trusted long runs. Epoch checks have approximately
10 ms timer granularity. Compilation, serialization and native C work are not directly
interrupted by this deadline.

The CSV has the names of the columns in its first line, then one line per row; a number is written as the shortest decimal which reads back as the same double, without an exponent, and `NaN`, `inf` and `-inf`. `node js/simulate.mjs` writes the same CSV byte for byte in Node (see [JavaScript](javascript.md)).

<svg class="core-diagram" viewBox="0 14 840 290" xmlns="http://www.w3.org/2000/svg" role="img"
     aria-label="One request of the contract simulation/1 is simulated in three stages and nothing is kept between requests. The values of preinit are set before the initialization, which the initial assignments see. The integrator runs a steady state first, from time zero, then one segment per change, firing the events a change makes true. The output is the given times, uniform steps or the steps of the integrator, with two rows at a change after the start. The result carries the columns, the number of rows and the values. Below, the segments of the integrator run from the start to the end, each ending in the state the next one starts from, with a change between them.">
  <defs>
    <marker id="sm-arrow" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="6" markerHeight="6" orient="auto-start-reverse">
      <path d="M 0 0 L 10 5 L 0 10 z" fill="currentColor"/>
    </marker>
  </defs>
  <text class="cd-title" x="420" y="34">one request, three stages, nothing kept between them</text>
  <g class="cd-step">
    <rect x="16" y="78" width="100" height="48" rx="6"/>
    <text x="66" y="99">request</text>
    <text class="cd-sub" x="66" y="115">simulation/1</text>
  </g>
  <line x1="122" y1="102" x2="148" y2="102" marker-end="url(#sm-arrow)"/>
  <g class="cd-step">
    <rect x="154" y="48" width="146" height="108" rx="6"/>
    <text x="227" y="72">preinit</text>
    <text class="cd-sub" x="227" y="98">values set before</text>
    <text class="cd-sub" x="227" y="118">the initialization</text>
    <text class="cd-sub" x="227" y="138">assignments see them</text>
  </g>
  <line x1="306" y1="102" x2="332" y2="102" marker-end="url(#sm-arrow)"/>
  <g class="cd-step cd-active">
    <rect x="338" y="48" width="168" height="108" rx="6"/>
    <text x="422" y="72">integrate</text>
    <text class="cd-sub" x="422" y="98">a steady state first,</text>
    <text class="cd-sub" x="422" y="118">then a segment per change,</text>
    <text class="cd-sub" x="422" y="138">firing the events it makes true</text>
  </g>
  <line x1="512" y1="102" x2="538" y2="102" marker-end="url(#sm-arrow)"/>
  <g class="cd-step">
    <rect x="544" y="48" width="146" height="108" rx="6"/>
    <text x="617" y="72">output</text>
    <text class="cd-sub" x="617" y="98">times, uniform steps</text>
    <text class="cd-sub" x="617" y="118">or the integrator&#8217;s</text>
    <text class="cd-sub" x="617" y="138">two rows at a change</text>
  </g>
  <line x1="696" y1="102" x2="722" y2="102" marker-end="url(#sm-arrow)"/>
  <g class="cd-artifact">
    <rect x="728" y="78" width="96" height="48" rx="4"/>
    <text x="776" y="99">result</text>
    <text class="cd-sub" x="776" y="115">columns, rows</text>
  </g>
  <text class="cd-label" x="16" y="196">the segments of the integrator, each ending in the state the next one starts from</text>
  <line x1="120" y1="240" x2="760" y2="240"/>
  <line x1="120" y1="233" x2="120" y2="247"/>
  <line x1="760" y1="233" x2="760" y2="247"/>
  <text class="cd-sub" x="120" y="266">start</text>
  <text class="cd-sub" x="760" y="266">end</text>
  <line class="cd-call" x1="333" y1="222" x2="333" y2="258"/>
  <text class="cd-contract" x="305" y="284">change</text>
  <line class="cd-call" x1="547" y1="222" x2="547" y2="258"/>
  <text class="cd-contract" x="519" y="284">change</text>
  <text class="cd-sub" x="226" y="218">segment</text>
  <text class="cd-sub" x="440" y="218">segment</text>
  <text class="cd-sub" x="653" y="218">segment</text>
</svg>

## The request, `simulation/1`

```json
{
  "contract": "simulation/1",
  "start": 0.0,
  "end": 10.0,
  "output": { "uniform": { "steps": 10 } },
  "preinit": [["k", 0.7]],
  "changes": [{ "time": 5.0, "values": [["y", 2.0]] }],
  "steady_state": null,
  "steady_state_after": null,
  "before_events": false,
  "selections": ["time", "y"],
  "sensitivities": ["k"],
  "settings": {
    "relative_tolerance": 1e-10,
    "absolute_tolerance": { "scalar": 1e-12 },
    "maximum_num_steps": 20000,
    "maximum_time_step": 0.0,
    "initial_time_step": 0.0,
    "stiff": false,
    "linear_solver": "auto",
    "jacobian": "auto",
    "maximum_bdf_order": 0,
    "maximum_adams_order": 0,
    "minimum_time_step": 0.0,
    "maximum_error_test_failures": 0,
    "maximum_nonlinear_iterations": 0,
    "maximum_convergence_failures": 0,
    "nonlinear_convergence_coefficient": 0.0,
    "stability_limit_detection": false,
    "suppress_algebraic": false
  },
  "seed": 1
}
```

| field | what it holds |
|---|---|
| `contract` | `"simulation/1"` |
| `start` | the start time; negative times are allowed |
| `end` | the end time |
| `output` | the times of the output: `{"times": [t, ...]}` the given times, `{"uniform": {"steps": n}}` `n` equal intervals from the start to the end (`n + 1` rows), `"integrator_steps"` the steps of the integrator |
| `preinit` | values set before the initialization, as `[id, value]` pairs; the ids are selections: `S` is the amount of a species, `[S]` its concentration (converted with the size its compartment has once the initial assignments are evaluated, so a pre-initialization holds the concentration of a species across the initialization, also when it changes that size, directly or through an initial assignment), any other id the value of its entity. The order of the values does not matter. The initial assignments see them |
| `changes` | changes at absolute times in `[start, end]`, in non-decreasing order of the time: `{"time": t, "values": [[id, value], ...], "increments": [[id, increment], ...]}` (`increments` optional, 0.6.0), see below |
| `steady_state` | `null`, or a steady state reached before the start: `{"preinit", "absolute_tolerance", "relative_tolerance", "max_time"}` (defaults `[]`, `1e-8`, `1e-6`, `1e8`), see below |
| `steady_state_after` | `null`, or a steady state reached after the end, whose row the output carries last at the time `inf`: `{"from", "absolute_tolerance", "relative_tolerance", "max_time"}` (defaults `0.0`, `1e-8`, `1e-6`, `1e8`), see below (0.18.0) |
| `before_events` | `false`, or `true` for an output at the end which is the state **before** the events of the end time execute, see below (0.18.0) |
| `sensitivities` | the ids of the values the forward sensitivities of the run are taken in (0.12.0, the native engine only), empty for a run without them; see below |
| `local_errors` | `false`, or `true` for the local error estimates of the integrator at every row (the native engine only), see below (0.26.0) |
| `selections` | the ids of the columns after `time`; a leading `"time"` is that column (not repeated), a later `"time"` another copy of the time of the row; empty is `time` followed by every selection of the model |
| `settings` | the settings of the integrator, below; the defaults of the engine are the ones of roadrunner |
| `seed` | the seed of the random generator which orders simultaneous events; 1 when it is absent |

The settings of the integrator:

| field | default | what it sets |
|---|---|---|
| `relative_tolerance` | `1e-6` | the relative tolerance |
| `absolute_tolerance` | `{"scalar": 1e-12}` | `{"scalar": x}` one tolerance for every state, `{"per_state": [x, ...]}` one per state in the order of the state vector (the `states` of `model.json`) |
| `maximum_num_steps` | `20000` | the most steps of one call of the integrator; in an output of times (and in a horizon of a steady state) the steps towards one output time, including returns at roots without an event execution and restarts at discontinuity roots |
| `maximum_time_step` | `0.0` | the largest step in time, `0.0` for none |
| `initial_time_step` | `0.0` | the first step in time, `0.0` for the integrator's choice |
| `stiff` | `true` | CVODE BDF (`true`) or Adams (`false`); models with algebraic rules always use IDA BDF |
| `linear_solver` | `"auto"` | `"auto"` (dense up to 12 states, KLU above), `"dense"`, `"klu"` or `"gmres"` (stiff ODEs) |
| `jacobian` | `"auto"` | `"auto"`, `"analytic"` (refused for a model without one) or `"difference_quotient"` (the sparse difference quotient over the colors of the columns of the pattern, also for a model with an analytic Jacobian) |
| `maximum_bdf_order` | `0` | the largest order of the BDF, 1 to 5; `0` is not set (CVODE and IDA take 5). It is the order of IDA as well, which integrates a model of algebraic rules with the BDF whatever `stiff` says (0.25.0) |
| `maximum_adams_order` | `0` | the largest order of the Adams method, 1 to 12; `0` is not set (CVODE takes 12). Only `"stiff": false` takes it (0.25.0) |
| `minimum_time_step` | `0.0` | the smallest step in time, `0.0` for none; above `maximum_time_step` it is refused (0.25.0) |
| `maximum_error_test_failures` | `0` | the most failures of the local error test of one step before the integration fails; `0` is not set (CVODE takes 7, IDA the 30 the engine sets for the consistent initial values) (0.25.0) |
| `maximum_nonlinear_iterations` | `0` | the most iterations of the nonlinear solver of one step; `0` is not set (CVODE takes 3, for its Newton iteration and for the fixed point iteration of Adams alike, and IDA 4) (0.25.0) |
| `maximum_convergence_failures` | `0` | the most convergence failures of the nonlinear solver of one step; `0` is not set (10) (0.25.0) |
| `nonlinear_convergence_coefficient` | `0.0` | the safety factor of the convergence test of the nonlinear solver; `0.0` is not set (CVODE takes 0.1, IDA 0.33) (0.25.0) |
| `stability_limit_detection` | `false` | whether CVODE lowers the order of a BDF above 2 at the stability limit, for a model whose dominant eigenvalues lie near the imaginary axis. A setting of the BDF of CVODE: it is refused with `"stiff": false` and for a model of algebraic rules, which IDA integrates (0.25.0) |
| `suppress_algebraic` | `false` | whether IDA leaves the algebraic states out of its error test. A setting of IDA: it is refused for a model without algebraic rules. The engine turns it on of its own where it interpolates over an algebraic jump (0.25.0) |

**The order of the method.** `maximum_bdf_order` bounds the method the run uses, which is the BDF of CVODE (`"stiff": true`) or of IDA (a model of algebraic rules); `maximum_adams_order` bounds the Adams method of `"stiff": false`. The one of the method the run does not use is ignored. A second integration of the same request at a lower order is how a caller bounds the error of the solver itself: the same model, the same tolerances, a method of another order, and the difference of the two runs is an estimate of the error neither of them reports.

With `"linear_solver": "gmres"`, CVODE BDF estimates each Jacobian-vector product by differencing the model's right hand side. GMRES uses a left preconditioner obtained by incomplete LU factorization with zero fill (ILU(0)) of the sparse Newton matrix. The `jacobian` setting controls how the engine evaluates the Jacobian for that preconditioner. Simulation statistics report the linear iterations, linear convergence failures, Jacobian-vector products, preconditioner evaluations and preconditioner solves.

The Krylov subspace has at most 30 vectors and allows two restarts per linear solve, and the linear tolerance is CVODE's default; the request cannot change any of the three. The preconditioner retains the Jacobian pattern plus the diagonal without pivoting; a failed pivot lets CVODE refresh the Jacobian or reduce the step. GMRES requires stiff ODEs; `"gmres"` with an algebraic rule or with `"stiff": false` is refused, and the error names the setting to change. A model without states keeps the existing zero-rate integration path for its events. `"auto"` retains its dense/KLU selection.

**When to choose it.** `"gmres"` keeps the memory of the linear solve linear in the entries of the Jacobian, so it suits a model whose factorization does not fit or whose fill-in dominates. It is not the faster solver of the models measured so far: on the 919-state machine perfusion model it takes 1.29 to 1.43 times as long as KLU, and at `relative_tolerance` 1e-10 its values differ from the reference in a few outputs, as KLU's do. Read `linear_iterations` against `nonlinear_iterations` of the statistics to see how many Krylov steps a Newton step costs, and `linear_convergence_failures` to see that the subspace of 30 vectors or the preconditioner was not enough; a model which reports them is one `"klu"` serves better. See the [machine perfusion evaluation](https://github.com/matthiaskoenig/sbmlsim-core/blob/develop/bench/GMRES.md) for the timing and the accuracy measurements.

**Initialization.** The model gets its SBML values, then the pre-initialization values of the request and of its steady state, then its initial assignments and the initial values of its assignment rules are evaluated at the time 0, whatever the start of the simulation (as sbmlsim initializes roadrunner at its time 0). A value which carries the concentration of a species which stores its amount, or the amount of a species which stores its concentration, needs the size of its compartment: it is set after that evaluation, with the size the evaluation settled on, and the initial assignments are evaluated once more, so a pre-initialization holds the concentration of a species also when it changes the size of its compartment, directly or through an initial assignment. The integration runs in absolute time from the start, in segments between the changes.

For models with algebraic rules, consistent initial conditions also solve the
constraints together with dependent initial assignments and species concentration
conversions. Initial event triggers and initial selections see the consistent
values. A constraint which depends explicitly on time is reconciled at the start
of the simulation as well.

**Changes.** The values of a change are set at once (the changes of one time are one change): a compartment keeps the concentrations of the species of a concentration in it (not of an amount, not of an assignment rule, not of one the change sets), and the assignment rules follow. A change at the start is applied after the initialization (and after the steady state) and before the output of the start; a change after the start ends a segment of the integration, and the output has the state before it and the state after it at its time (two rows: in the steps of the integrator always, in an output of times when its time is one of them). The events a change triggers execute with it; an event at its time executes after it. An increment adds to the value of its id in the state before the change (an amount `S`, a concentration `[S]`, the value of any other entity), at once with the values: a repeated dose is `{"time": 24, "increments": [["PODOSE_x", 25]]}`, the formula `"PODOSE_x + 25"` of a change of sbmlsim. One change sets or increments an id once.

**Steady state.** The model is initialized once, with the pre-initialization of the request and of the steady state together (an id in both is refused), and integrated from the time 0, events active, until every state has `|dx/dt| <= absolute_tolerance + relative_tolerance * |x|` at the end of a horizon (1, 10, 100, ... time units), at most `max_time`; the simulation starts from that state at its start. A steady state which is not reached is an error of the integration.

**Steady state after the end.** `steady_state_after` is the same criterion applied **after** the timecourse, which is a measurement at steady state (PEtab v2's time `inf`) where `steady_state` is the pre-equilibration before the start. The model continues from the state the timecourse ended in, with the triggers of its events as it left them and every pending execution moved from the end to `from`, so an event which executed during the timecourse does not execute again at the same threshold, and it is integrated from `from` over the same growing horizons. Its row is the last of the output, at the time `inf`; the rows of the timecourse are the ones of a request without it, bitwise. `from` is 0 by default, which is where roadrunner integrates the steady state of a model whose math does not read the time; a caller which continues a model which reads the time passes the end of its timecourse, as roadrunner does.

**The state before the events of the end.** A request answers the end with the state **after** the events of the end time, which is what a timecourse reads. A caller which folds the value of a change into a number needs the other one: roadrunner evaluates the triggers and applies a change *before* it executes the events of that time, so the formula of the change reads the state before them. `"before_events": true` is that request: the events of the end are not executed at all, the simulation ending there, and the last row is the state the end was integrated to with the changes and the events before it applied. Only the end is affected, an output time inside the interval seeing the state after the events of its time as it does without the flag. The flag holds for every kind of output, the steps of the integrator included, where the two rows of an event at the end become the one row before it.

## Forward sensitivities

`sensitivities` asks for the derivative of every column of the result in the value every
listed id has at the start of the run, which a least squares fit uses as the Jacobian of its
residuals (`crates/fit`, [Fitting](fitting.md)) and a profile of an identifiability as the
curvature of its cost. The ids are the ones of `preinit`: a parameter of the model, a
compartment, or a species as its amount (`S`) or its concentration (`[S]`).

The result then carries `sensitivities: {parameters, values}`, with `parameters` the ids in
the order they were asked for and `values` of `n_rows * columns.length * parameters.length`
numbers, row major: the sensitivity of the column `c` of the row `r` in the parameter `j` is
`values[(r * columns.length + c) * parameters.length + j]`. The column of the time is 0.

The [python binding](python.md#compile-and-simulate) hands them over as `out.sensitivities`, a
float64 array of the shape `(rows, columns, parameters)`.

The engine integrates one sensitivity vector per parameter next to the states (CVODES,
`CV_STAGGERED`), so a run of `k` parameters is one integration and not `k + 1`. The states of
such a run stay the states of the same run without it: the sensitivities are not in the error
test of a step, and the states of a step are solved before them. `df/dp` is a forward
difference of the right hand side of the model in the parameter and the sensitivity of a
column which is no state is one of its value, so a sensitivity is accurate to about 1e-8 of
itself, against 1e-3 for a difference of whole simulations at the default tolerances.

A step of a target moves everything the initialization of the model derives from it, not the
target alone: `Vli = BW * f_li` of a PBPK model is an initial assignment, so the right hand
side reads `Vli` and never `BW`. The engine therefore takes the difference of two
initializations once per target at the start of a run, which gives both the initial values of
the sensitivities (`dy0/dp`) and the direction `dp/dp_j` the `df/dp` of a step moves the
parameters along. A target whose initialization derives nothing, and which the right hand side
does not read either, has no `df/dp` at all and costs nothing per step.

`df/dp` itself is `model_jac_p` where the model carries it: the compiler differentiates the
rates of the model in its parameters, column by column, and `jacobian_p` of the description
holds the pattern of those columns. The `df/dp` of a target is then the sum of the columns of
its direction, `d f/dp_c * dp_c/dp_j`, which is one call of `model_jac_p` per column the
direction touches and no call of the right hand side at all. A model whose math has an
operator without a derivative, which reads its history, or whose column of a parameter is
above the limits of a WebAssembly function carries no `model_jac_p`; the engine then takes the
forward difference of the right hand side along the direction, one call per target and step.
`sensitivity_jac_p_evals` and `sensitivity_model_rhs_evals` of the counters say which of the
two a run took.

The `changes` of a request are carried across: a change is applied to the state and the
parameters moved along every sensitivity as well, and the difference of the two afterwards is
the sensitivity after it, so a value set to a number has no derivative after the change, one a
number is added to (a dose) keeps it, a form takes the derivatives of its terms, and a
parameter set to a number no longer depends on the target. The `steady_state` before the start
and the `steady_state_after` the end are integrations, and the sensitivities are integrated
with them: the derivatives of the state the integration ends in.

What a run of sensitivities does not carry is refused and not answered wrongly: the events of
a model, a discontinuity of its math, an algebraic rule, a delay and the iterative linear
solver. `engine.wasm` refuses them as well (its
output format carries the values of a table and nothing else): they are answered by the
native engine, which the Rust host, the python bindings and the command line of sbmlsim-core
call as a library.

## Local error estimates

`"local_errors": true` asks for what the integrator estimated of its own error, for a
verification of the numerical error of a simulation next to the counters of its statistics.
Every row then carries two vectors over the states of the model, of the last step the
integrator took before the row: the estimate of the local error of that step
(`CVodeGetEstLocalErrors`, `IDAGetEstLocalErrors` of a DAE model) and the weights of its error
test (`CVodeGetErrWeights`, `IDAGetErrWeights`), `1 / (rtol |y_i| + atol_i)` of the state the
step started from. CVODE accepted the step because the weighted root mean square norm of the
two, `sqrt(mean((e_i w_i)^2))`, is at most 1, so that norm is the share of its tolerance the
step used, and the estimate of a state over its weight is the scale of its error.

A row at an output time is interpolated in the step which reached it or passed it, so two rows
in one step carry the same estimates; a row at a change or an event carries the step before
it. A row which no step precedes, the start and every row of a run which takes no step, is
`NaN`. The result carries `local_errors: {states, errors, weights}`, with `states` the ids of
the states in the order of the state vector and `errors` and `weights` of
`n_rows * states.length` numbers each, row major. The
[python binding](python.md#compile-and-simulate) hands them over as `out.local_errors`, with
`errors` and `weights` float64 arrays of the shape `(rows, states)`.

They cost a copy of two vectors of the states per row and nothing without the flag. The
native engine answers them; `engine.wasm`, whose output has no room for them, refuses a
request which asks for them, as it refuses the forward sensitivities.

## The result

`{columns, n_rows, values}`: the names of the columns (`time` first), the number of rows and `n_rows * columns.length` values, row major. A result does not depend on the simulations before it on the same instance.

An error of the engine has a kind: `load` (the description is refused), `request` (the request is invalid, e.g. a tolerance which is not positive or an output beyond the limit of the values of a result), `unsupported`, `integration` (CVODE or IDA failed, with its `flag` and the `time`), or, in the hosts, `internal` (a trap of the engine or the model). The command line prints it, e.g. `integration failed at t = 0 (flag -4): ...`; the JavaScript loader throws an `EngineError`.

The statistics of the integrator of the last simulation are `Instance::stats` in wasmtime and `instance.stats()` in JavaScript; they are diagnostics, no part of a result. They are the counts of the work (steps, calls of the right hand side and of the Jacobian, setups of the linear solver, error test and convergence failures, calls of the root functions, (re)initializations of the integrator, the calls of the probes of the triggers and of the criterion of a steady state, the calls of the right hand side of the forward sensitivities and, inside it, of the analytic `df/dp` of the model or of the difference quotient which stands in for it, and the reductions of the order at the stability limit), and after them the state the solver ended in (0.25.0): `last_order` and `current_order`, the orders of the last step and of the next one, `actual_initial_time_step`, `last_time_step` and `current_time_step`, and `current_time`, the internal time of the integrator, which is at or past the end of the simulation, whose last values are interpolated. The state is that of the integration which stepped last, not of the projection of the consistent initial values of a model of algebraic rules.

A run which seems to hang is read here: `current_time` says how far the integrator got, `current_time_step` how small its steps became, and `error_test_failures` against `nonlinear_convergence_failures` says which of the two tests drove them down. A run which reaches `maximum_num_steps` answers the same questions.

The engine bounds delayed-event storage independently of the numerical settings:
65,536 pending executions and 1,048,576 retained assignment values (8 MiB) by default.
A single assignment snapshot has the same value limit. Exceeding it returns an
integration error with flag `-1005`; a subsequent request starts with an empty queue.
Native Rust `Engine::with_event_limits` can override this policy. Entity/selection
lookups, compartment relationships and finite-difference coloring/workspace are
cached per engine and reused across compatible simulations.

## The description, `model/2`

`model.json` describes the compiled model next to its `model.wasm`. The engine compares the sizes of the description with the ones `model.wasm` reports when it loads the model, so a description which does not belong to its module is an error.

| field | what it holds |
|---|---|
| `contract` | `"model/2"` |
| `compiler` | the compiler and its version, e.g. `"sbmlsim-compiler 0.44.0"` |
| `id` | the id of the model |
| `states` | the states, in the order of the state vector (the order of a `per_state` tolerance) |
| `parameters` | the parameters (with the compartments, the species which are not states and the species references), in the order of the parameter vector |
| `selections` | the ids a request can select, `{"id", "index"}`; `[S]` is the concentration of a species |
| `jacobian` | `{"analytic": pattern}` or `{"colored_difference": {"pattern", "colors", "reason"}}`; a pattern is compressed sparse column (`n`, `col_ptr`, `row_idx`) |
| `jacobian_p` | the pattern of the Jacobian of the right hand side in the parameters (`rows`, `col_ptr`, `row_idx`: a column per parameter, a row per state), omitted by a model without `model_jac_p` |
| `events` | per event `id` (empty for an event without one), `has_delay`, `has_priority`, `persistent`, `initial_value`, `use_values_from_trigger_time`, `n_assignments` |
| `n_roots` | the number of root functions of the model, those of the triggers first |
| `n_discontinuity_roots` | the number of root functions of math discontinuities and delayed history crossings, the last of the `n_roots` |
| `n_constants` | the number of cached parameter-only subexpressions behind the parameters; a DDE model instead reserves one slot for its history address |
| `n_delays` | optional historical query slot count, omitted for ordinary ODEs; see [Delay equations](delays.md) |
| `units` | the units sbmlsim gives the entities (`UnitsInformation.from_sbml_doc`): `time` (`second` without a time unit), every compartment, species (`S` and `[S]`) and parameter, empty without units (the default units of the model name none), and every reaction with a kinetic law, the unit libsbml derives for its kinetic law; a unit definition as pint reads it (`mmol/min`, `1/min`, `mmol/l`), else by its id |
| `driven` | the driven species, omitted without: per species `id`, `value` and `slope` (the indices of the parameters `X__value` and `X__slope` of its path `value + slope * (t - t_ref)`), `exchange` (the index of the state `X__exchange`, the amount the model gave to it) and `amount` (whether the path is an amount, else a concentration in its compartment) |
| `t_ref` | the index of the parameter `driven__t_ref` of the paths, omitted without driven species |
| `unit_definitions` | the unit definitions `units` names by their id (or `<id>_<n>`) because pint has no unit of them (a factor which is no prefix, `(133.322*newton)/m^2`), the name to its expression of pint, which a registry reads the units with; omitted when there is none |
| `time_unit` | the time unit of the model as sbmlsim names it, `units["time"]` |
| `initial_assignments` | the ids of the parameters, compartments and species references whose value as loaded an initial assignment computes, omitted when there is none |
| `time_dependent` | `true` when a math of the model reads the time or a delay, or an event has a delay, omitted otherwise |

An entity of `states` and `parameters` is `{"id", "kind", "initial", "settable"}`: its SBML id; its kind, `"compartment"`, `"parameter"`, `"species_reference"` or `{"species": {"compartment", "has_only_substance_units", "stores_concentration"}}`; its initial value as loaded (a value which is not finite is the string `"NaN"`, `"inf"` or `"-inf"`, since JSON has no such numbers); and whether a change or the pre-initialization may set it (the target of an assignment rule and the unknown of an algebraic rule may not, and the refusal names the rule which determines the entity).

```json
{
  "contract": "model/2",
  "compiler": "sbmlsim-compiler 0.44.0",
  "id": "toy",
  "states": [
    { "id": "A", "kind": { "species": { "compartment": "c", "has_only_substance_units": false, "stores_concentration": false } }, "initial": 20.0, "settable": true }
  ],
  "parameters": [{ "id": "c", "kind": "compartment", "initial": 2.0, "settable": true }],
  "selections": [{ "id": "c", "index": 0 }, { "id": "A", "index": 1 }, { "id": "[A]", "index": 2 }],
  "jacobian": { "analytic": { "n": 1, "col_ptr": [0, 1], "row_idx": [0] } },
  "events": [],
  "n_roots": 0,
  "n_discontinuity_roots": 0,
  "n_constants": 0,
  "units": { "time": "second", "c": "", "A": "", "[A]": "" },
  "time_unit": "second"
}
```

For a model with algebraic rules, `n_algebraic` is the number of algebraic unknowns
at the end of `states`. The preceding states have differential equations. The
algebraic unknowns contribute to per-state tolerance vectors and to the Jacobian.
An ODE description omits this optional field. Older ODE models continue to load;
a runtime predating DAE support refuses descriptions containing `n_algebraic`.

The exports of `model.wasm` (`model_defaults`, `model_init`, `model_rhs`, `model_jac`, `model_jac_p` (only with `jacobian_p` in the description), `model_roots`, `model_trigger`, `model_priority`, `model_delay`, `model_event_values`, `model_event_apply`, `model_selections`, `model_assign`, `model_sizes`, `model_event_sizes`) and their arguments are documented in `crates/abi/src/lib.rs`; the exports of `engine.wasm` and the layout of its output in `crates/engine/src/exports.rs`.

## Reusing Python batch workers

Reuse one `Runtime`, load the model once, then reuse `model.instance()` for serial
work or `model.batch_executor()` for repeated parallel batches:

```python
with model.batch_executor(threads=4) as executor:
    first = executor.simulate_many(first_requests)
    second = executor.simulate_many(second_requests)
```

Results stay in request order and have the same numerical values as serial runs.
A failed point returns `PointError`; subsequent points and calls remain usable.
Malformed request dictionaries raise before any point in that call runs.
`model.simulate_many(requests, threads=...)` remains available for one-off batches.

An executor starts no threads until its first nonempty submission. Its pool grows
with batch size up to the requested limit; growth replaces the old pool while retaining existing instances. Once sized, both
workers and instances persist between calls. The default
limit is the smaller of available CPUs and eight, bounding retained model memories;
choose fewer workers for large models. Explicit thread counts accept 1 through 1024.
One call at a time is accepted per executor. Concurrent calls, reentry, and closing
while a call is active raise `RuntimeError` instead of accumulating queued work.

Use `sbmlsim_core.simulate_stream(executor, request_iterator, batch_size=32)` to
consume large inputs in bounded chunks and yield ordered results. At most one chunk
is consumed ahead; only that chunk's requests and results are retained by the helper.
Malformed input raises at its chunk, after earlier results may have been yielded.
The caller owns and closes the executor, including when iteration ends early.

Simulation work releases the GIL. Python signals are checked while the pool
simulates, so interrupting a batch stops it before its next points start and
discards that call's results while leaving the executor usable.
`close()` is idempotent, releases retained instances and workers, and prevents new
submissions. The context manager closes on normal exit and exceptions; dropping the
executor also releases its resources.

Python `Runtime(max_memory_bytes=None, timeout_ms=30_000)` also sets the host
execution deadline inherited by loaded models, individual instances, and batch
executors. Set `timeout_ms=0` to disable the deadline for trusted long runs.

For repeated parameter sweeps, prepare the fixed request once and send numeric rows to
an executor. The `varying` IDs define the order of the values in every row:

```python
prepared = model.prepare(request, varying=["k", "y"])
with model.batch_executor(threads=4) as executor:
    outputs = executor.simulate_prepared(prepared, [[0.2, 1.0], [0.5, 2.0]])
    more = executor.simulate_prepared(prepared, [[0.8, 3.0]])
```

Preparation validates the request and input IDs once. Each row supplies initial values
for those IDs; other request settings, changes, selections and steady-state settings
stay fixed. Rows are copied before simulation releases the GIL, and NumPy numeric rows
are accepted. Results retain input order, with a `PointError` for an invalid count,
non-finite value or failed simulation. A malformed numeric row raises `ValueError`
before simulation starts. The executor retains its workers and prepared plans across
submissions and shares workers with `simulate_many`. Prepared requests belong to the
loaded model that created them and can be shared among its instances and executors.
`instance.simulate_prepared(prepared, values)` runs a single point;
`prepared.n_values` gives the required row length. An empty `varying` list supports
repeating a fixed request with empty rows. Executor close and concurrent-call behavior
are the same for both submission methods.

Existing pre-initialization entries retain their order and quantity when overridden.
For example, varying `S` with a template entry for `[S]` replaces the value in that
concentration entry. Drop unused Python plans to release the plans the instances of
the model cached for them on the next prepared operation. The limit of 32 live plans
per instance is the one of a wasm instance, which the hosts of Rust and JavaScript
run; the wheel runs the native engine, whose instances cache the plan of every
request which is alive.

One batch can also hold requests of its own, as a fit does: `simulate_plans` takes
pairs of a prepared request and the values of its varying targets, so the conditions
of one evaluation of an objective, or the whole finite difference Jacobian of one,
are a single fan-out over the pool instead of one call per condition:

```python
first = model.prepare(request, varying=["k"])
second = model.prepare(other_request, varying=["k"])
with model.batch_executor(threads=4) as executor:
    outputs = executor.simulate_plans(
        [(first, [0.2]), (second, [0.2]), (first, [0.5]), (second, [0.5])]
    )
```

Every request of a batch of plans belongs to the loaded model of the executor; one of
another model raises `ValueError` naming its plan (`plans[3]`), as a pair which is
none does. The batch reads its plans while its pool simulates the ones it read before,
so the points before such a plan may have run; the batch stops there and gives no
result. The results keep the order of the plans and a
point which fails is a `PointError` in its own position, as in every other batch.
`bench/OPTIMIZATION.md` measures what one batch of plans is worth to a fit.

## Prepared requests in Rust

The native core exposes `Engine::prepare(&request, &varying)` and
`Engine::simulate_prepared(&plan, &values)`. Core plans belong to the engine that
prepared them. The Wasmtime host also supports plans shared by instances of the
same `LoadedModel`:

```rust,ignore
let plan = model.prepare(&request, &["k".into(), "y".into()])?;
let mut instance = model.instantiate()?;
let result = instance.simulate_prepared(&plan, &[0.5, 2.0])?;
```

Preparation resolves output columns, times, changes and varying targets once.
Every run starts from fresh initial values while retaining solver and scratch
storage. Results own their values independently. Host plans support up to 32 live
plans per instance; drop unused plans to reclaim guest handles on the next prepared
operation. The host sends the request once per instance and only aligned numeric
values afterward. Engines without the optional prepared exports still support
ordinary simulations and return `Unsupported` for preparation.
