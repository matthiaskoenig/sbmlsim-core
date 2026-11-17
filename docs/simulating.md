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
  "selections": ["time", "y"],
  "settings": {
    "relative_tolerance": 1e-10,
    "absolute_tolerance": { "scalar": 1e-12 },
    "maximum_num_steps": 20000,
    "maximum_time_step": 0.0,
    "initial_time_step": 0.0,
    "stiff": false,
    "linear_solver": "auto",
    "jacobian": "auto"
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
| `selections` | the ids of the columns after `time`; a leading `"time"` is that column (not repeated), a later `"time"` another copy of the time of the row; empty is `time` followed by every selection of the model |
| `settings` | the settings of the integrator, below; the defaults of the engine are the ones of roadrunner |
| `seed` | the seed of the random generator which orders simultaneous events; 1 when it is absent |

The settings of the integrator:

| field | default | what it sets |
|---|---|---|
| `relative_tolerance` | `1e-6` | the relative tolerance |
| `absolute_tolerance` | `{"scalar": 1e-12}` | `{"scalar": x}` one tolerance for every state, `{"per_state": [x, ...]}` one per state in the order of the state vector (the `states` of `model.json`) |
| `maximum_num_steps` | `20000` | the most steps of one call of CVODE; in an output of times (and in a horizon of a steady state) the steps towards one output time, summed over the returns of CVODE at roots without an execution of an event and its restarts at the roots of discontinuities |
| `maximum_time_step` | `0.0` | the largest step in time, `0.0` for none |
| `initial_time_step` | `0.0` | the first step in time, `0.0` for CVODE's choice |
| `stiff` | `true` | BDF (`true`) or Adams (`false`) |
| `linear_solver` | `"auto"` | `"auto"` (dense up to 12 states, KLU above), `"dense"` or `"klu"` |
| `jacobian` | `"auto"` | `"auto"`, `"analytic"` (refused for a model without one) or `"difference_quotient"` (the sparse difference quotient over the colors of the columns of the pattern, also for a model with an analytic Jacobian) |

**Initialization.** The model gets its SBML values, then the pre-initialization values of the request and of its steady state, then its initial assignments and the initial values of its assignment rules are evaluated at the time 0, whatever the start of the simulation (as sbmlsim initializes roadrunner at its time 0). A value which carries the concentration of a species which stores its amount, or the amount of a species which stores its concentration, needs the size of its compartment: it is set after that evaluation, with the size the evaluation settled on, and the initial assignments are evaluated once more, so a pre-initialization holds the concentration of a species also when it changes the size of its compartment, directly or through an initial assignment. The integration runs in absolute time from the start, in segments between the changes.

**Changes.** The values of a change are set at once (the changes of one time are one change): a compartment keeps the concentrations of the species of a concentration in it (not of an amount, not of an assignment rule, not of one the change sets), and the assignment rules follow. A change at the start is applied after the initialization (and after the steady state) and before the output of the start; a change after the start ends a segment of the integration, and the output has the state before it and the state after it at its time (two rows: in the steps of the integrator always, in an output of times when its time is one of them). The events a change triggers execute with it; an event at its time executes after it. An increment adds to the value of its id in the state before the change (an amount `S`, a concentration `[S]`, the value of any other entity), at once with the values: a repeated dose is `{"time": 24, "increments": [["PODOSE_x", 25]]}`, the formula `"PODOSE_x + 25"` of a change of sbmlsim. One change sets or increments an id once.

**Steady state.** The model is initialized once, with the pre-initialization of the request and of the steady state together (an id in both is refused), and integrated from the time 0, events active, until every state has `|dx/dt| <= absolute_tolerance + relative_tolerance * |x|` at the end of a horizon (1, 10, 100, ... time units), at most `max_time`; the simulation starts from that state at its start. A steady state which is not reached is an error of the integration.

## The result

`{columns, n_rows, values}`: the names of the columns (`time` first), the number of rows and `n_rows * columns.length` values, row major. A result does not depend on the simulations before it on the same instance.

An error of the engine has a kind: `load` (the description is refused), `request` (the request is invalid, e.g. a tolerance which is not positive or an output beyond the limit of the values of a result), `unsupported`, `integration` (CVODE failed, with its `flag` and the `time`), or, in the hosts, `internal` (a trap of the engine or the model). The command line prints it, e.g. `integration failed at t = 0 (flag -4): ...`; the JavaScript loader throws an `EngineError`.

The counters of the integrator of the last simulation (steps, calls of the right hand side and of the Jacobian, setups of the linear solver, error test and convergence failures, calls of the root functions, (re)initializations of CVODE, the calls of the probes of the triggers and of the criterion of a steady state) are `Instance::stats` in wasmtime and `instance.stats()` in JavaScript; they are diagnostics, no part of a result.

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
| `compiler` | the compiler and its version, e.g. `"sbmlsim-compiler 0.9.0"` |
| `id` | the id of the model |
| `states` | the states, in the order of the state vector (the order of a `per_state` tolerance) |
| `parameters` | the parameters (with the compartments, the species which are not states and the species references), in the order of the parameter vector |
| `selections` | the ids a request can select, `{"id", "index"}`; `[S]` is the concentration of a species |
| `jacobian` | `{"analytic": pattern}` or `{"colored_difference": {"pattern", "colors", "reason"}}`; a pattern is compressed sparse column (`n`, `col_ptr`, `row_idx`) |
| `events` | per event `id` (empty for an event without one), `has_delay`, `has_priority`, `persistent`, `initial_value`, `use_values_from_trigger_time`, `n_assignments` |
| `n_roots` | the number of root functions of the model, those of the triggers first |
| `n_discontinuity_roots` | the number of root functions of the discontinuities of the math (the branches of `piecewise`), the last of the `n_roots` |
| `n_constants` | the number of constants of the model: the subexpressions of the parameters alone, which `model_init` and `model_assign` compute into the parameter vector behind the parameters, so the right hand side does not compute them in every call |
| `units` | the unit expression of every entity id |

An entity of `states` and `parameters` is `{"id", "kind", "initial", "settable"}`: its SBML id; its kind, `"compartment"`, `"parameter"`, `"species_reference"` or `{"species": {"compartment", "has_only_substance_units", "stores_concentration"}}`; its initial value as loaded (a value which is not finite is the string `"NaN"`, `"inf"` or `"-inf"`, since JSON has no such numbers); and whether a change may set it (the target of an assignment rule may not).

```json
{
  "contract": "model/2",
  "compiler": "sbmlsim-compiler 0.9.0",
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
  "units": {}
}
```

The exports of `model.wasm` (`model_defaults`, `model_init`, `model_rhs`, `model_jac`, `model_roots`, `model_trigger`, `model_priority`, `model_delay`, `model_event_values`, `model_event_apply`, `model_selections`, `model_assign`, `model_sizes`, `model_event_sizes`) and their arguments are documented in `crates/abi/src/lib.rs`; the exports of `engine.wasm` and the layout of its output in `crates/engine/src/exports.rs`.

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

Simulation work releases the GIL. Python signals are checked between slices, so
interrupting a batch discards that call's results while leaving the executor usable.
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
concentration entry. Drop unused Python plans to release their guest slots on the
next prepared operation. Each worker retains at most 32 live plans.

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
