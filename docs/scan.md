# Scans

A scan of sbmlsim is a simulation and its dimensions with the observables of the run: `Simulator.run(model, scan, observables, time=, keep=, on_error=, points=)`. Since 0.45.0 its planning runs here, in the crate `sbmlsim-scan`: the request `scan/1` is planned into the requests `simulation/1` of the engine from the description of the compiled model alone, the formulas at the changes are folded, the observables are evaluated and the result `scanresult/1` is assembled, as sbmlsim's `Simulator.run` does with the engine core. Python (`sbmlsim_core.scan`) and a page (`js/scan.mjs` on `compiler.wasm`) run the same planner, so a scan has the numbers of sbmlsim in either.

| | Rust | Python | JavaScript |
|---|---|---|---|
| run a scan | the loop of `Scan` with a host | `scan.run(request, model, models=, threads=, observe=, on_batch=)` | `await runScan(request, { compiler, runtime \| pool, model, models, observe, onBatch })` |
| plan a scan | `Scan::open(request, description)`, `Scan::open_with_models(request, None, models)` | `scan.Scan(request, model, models=)` | `openScan(request, { compiler, model, models })` |
| next round | `ask()` | `ask()` | `ask()` |
| answers of simulations | `tell(answers)` | `tell(results)` | `tell(results)` |
| values of observables | `tell_observed(values)` | `tell_observed(values)` | `tellObserved(values)` |
| result | `result()` | `result()` | `result()` |
| error | `ScanError` | `sbmlsim_core.ScanError` | `ScanError` |

The results are the ones of sbmlsim: every oracle case of `testdata/scan` (written by sbmlsim from its own runs of the engine core) gives sbmlsim's result through the planner in Rust (`crates/scan/tests/fixtures.rs`), in python (`python/tests/test_scan.py`) and in a page (`js/test/scan.test.mjs`), the native values by their bits and the values interpolated onto a grid and the observables within 4 units in the last place, and the error of the case which raises with sbmlsim's message. In python every value is sbmlsim's by its bits; in a page so is every value but one parameter of a PK observable (`pk.clast_pred` of the case `tutorial_scan`), 1 unit in the last place off, the kernel of pkpdutils running in `compiler.wasm`.

## Python

```python
import sbmlsim_core
from sbmlsim_core import scan

compiled = sbmlsim_core.compile_sbml(sbml)
model = sbmlsim_core.Runtime().load(compiled.wasm, compiled.description)

request = {
    "contract": "scan/1",
    "scan": scan_of_sbmlsim.to_dict(),  # the simulation and its dimensions
    "observables": [o.to_dict() for o in observables],
    "keep": None,
    "time": None,
    "on_error": "raise",
    "points": None,
    "settings": integrator_settings,  # the settings of simulation/1
    "selections": selections,
}
result = scan.run(
    request, model, threads=4, on_batch=lambda done, total: print(done, total)
)
for variable in result["variables"]:
    print(variable["name"], variable["unit"], variable["values"].shape)
```

`scan.run` drives the rounds of the planner on `model.batch_executor(threads=threads)` (`threads=None` is the default of the executor, every CPU of the process and at most eight): the requests of a round never become objects of python, every worker of the executor simulates on an instance of its own, which it keeps across the rounds, and a request carries everything of its simulation, so the result does not depend on `threads`, and the thread of the caller holds no GIL while the batch runs (Ctrl-C stops it between the points). `on_batch(done, total)` is called with the points of the block which ran whenever a batch of them finished.

A request is a dict, written with `json.dumps` and read by the planner from that text, so every mapping keeps the order of the caller (a pre-initialization of several targets, the simulations of a dimension), the order in which sbmlsim applies and runs them; arrays and numbers of numpy are written as their lists and numbers, and the JSON text itself is taken as well.

`scan.Scan(request, model)` is the planner itself, for a host which simulates elsewhere: `ask()` gives `("simulate", requests)` (the requests of `simulation/1` as dicts, of the model `round_model`), `("observe", question)` or `("done", None)`, `tell(results)` takes what `simulate_many` gives (a `SimulationOutput` or a `PointError` per request, in their order), `tell_observed(values)` the values of the observables of the host, `progress` the points which ran and `result()` the result, once.

The result is a dict of the fields of `scanresult/1` whose arrays are arrays of numpy of float64 in the shape of their variable, built from the vectors of the planner without JSON, `NaN` and the infinities as the values they are: `variables` (`name`, `dims`, `shape`, `unit`, `values`), `coords` (`name`, `dims`, `unit`, `values` as `{"numbers": array}`, `{"labels": [...]}` or `{"indices": array}` of int64), `sizes` (`[{"name", "size"}]`), `dims`, `layout`, `units` (pairs of the name and the unit), `changes` (an array), `errors` and `block`.

## Dimensions of models

A dimension of models (sbmlsim's `Dimension(models=...)`) gives every point the model of its key, which replaces the model of the run, as sbmlsim's `Simulator.run(None, scan)`. The request carries the dimension as `Scan.to_dict()` writes it (`models: {key: SBML}`); the host compiles and loads every model and gives them by their keys, and no model of the run:

```python
result = scan.run(request, None, models={"fast": fast, "slow": slow}, threads=4)
```

```js
const result = await runScan(request, { compiler, runtime, models: { fast, slow } });
```

The planner plans every simulation on every model and runs the points in the C order of the scan, a chunk of the points of one simulation and one model at a time, whose round of simulations names its model (`Ask::Simulate { model, requests }` in Rust, `round_model` of `scan.Scan` in python, `{ simulate, model }` of `openScan` in a page; `None` or `null` for the model of the run). `scan.run` runs the requests of a model on a batch executor of that model and `runScan` on an engine of it, each made at the first round of the model and closed once every point of the model ran (`Scan::finished_models`, `finished_models()` of `scan.Scan`, `finished` of a round of `openScan`), so with the dimension of models first one executor is open at a time. A block of `points` takes the points of every model in that order.

The models are checked against the first one before a point runs, as sbmlsim's `_check_model` does: a model has every selection of the requests and every dose target of a PK observable, each with the unit it has in the first model (`The models of the dimension have different units of 'time': 'second' and 'dimensionless' of the model 'unitless'.`); the observables, the units of the result and the grid of `time` are the ones of the first model. What differs by model is in the request by the key of the model: `model_settings`, the settings of its requests (the absolute tolerance of every state of the model, `settings` without), and `model_changes`, its changes (sbmlsim's `AbstractModel.changes`), the defaults of the pre-initialization of its simulations.

## JavaScript

```js
import { createCompiler } from "sbmlsim-core/compile";
import { createRuntime } from "sbmlsim-core";
import { runScan } from "sbmlsim-core/scan";

const compiler = await createCompiler({ compiler: compilerWasm });
const runtime = await createRuntime({ engine, mathlib }); // or a pool of `pool.mjs`
const model = compiler.compile(sbml); // { wasm, description }
const result = await runScan(request, { compiler, runtime, model, onBatch: (done, total) => {} });
for (const { name, shape, unit, values } of result.variables) console.log(name, shape, unit, values);
```

The planner is the Rust of `sbmlsim-scan` in `compiler.wasm` (the feature `scan` of `sbmlsim-wasm`, on by default), and the module only simulates the requests of its rounds with the engine of the page, one after the other with a `runtime` or next to each other on the workers of a `pool` (at most `poolWindow` of them queued at once, 256), whose number changes no bit of the result; an error of the host which is no failure of a point (an abort, a pool which was closed) ends the round, `runScan` rejects with it once the simulations in flight settled, and no further simulation of the scan starts. A request is an object (`JSON.stringify` keeps the order of its keys) or its JSON text. The values of the variables, the numbers of the coordinates and the changes of the result are `Float64Array`s, `NaN` and the infinities as the values they are, and the `indices` of a coordinate are an array of integers (numbers of JavaScript, int64 in python).

`openScan(request, { compiler, model })` is the planner for a host which simulates itself (`openScan(request, { compiler, models })` of a dimension of models): `ask()` gives `{ simulate: [request, ...], model }` (the key of the model of the requests, `null` for the model of the run), `{ observe: question }` or `{ done: true }`, `tell(results)` takes the tables of the loader (`{ columns, nRows, values }`) or the `EngineError`s of the requests in their order, `tellObserved(values)` the values of the observables of the host, `progress` is `[done, total]`, `result()` the result, and `close()` drops the scan.

The calls of `compiler.wasm` are `scan_open`, `scan_ask`, `scan_tell`, `scan_observed`, `scan_result` and `scan_close`, `scan_open` taking the request, the description of the model of the run and the descriptions of the models of a dimension of models (`{key: description}`, an empty buffer for none), every text as UTF-8 JSON and every array of numbers as the little-endian bytes of its doubles in a buffer of its own next to the JSON (the tables of the engine, the time and the columns of the observables, their values and the values of the variables of the result), so nothing large is written into text and parsed again; a call which fails answers the error as `{"kind", "message", "point"}`.

## Observables of the host

sbmlsim's `Custom` observable is a function of the host, `function(time, values)`, which the planner cannot call. A scan with such observables asks a round of them once the points of a chunk ran: the question names the observables (`observables`, in the order they are evaluated) and has per point its flat `index`, its finite time points (`time`, the steady state after the end is none) and the values of the symbols the functions read (`columns`, a value per time point of a timecourse, one value of a value per simulation), and `scalars` names the symbols which are a value per simulation, so a host tells them from a timecourse of one time point. `observe(question)` answers

```python
{"values": {id: [value of every point]}, "errors": [{"observable": id, "index": i, "message": "ValueError: ..."}]}
```

a value a number for an observable of the kind `scalar` and a value per time point for one of the kind `timecourse`, in the order of the points of the question; `errors` (optional) are the functions which failed, whose points fail alone (flagged, or raised with `on_error: raise`). A scan with such observables and no `observe` is a `ScanError` of the kind `request`.

## Errors

`ScanError` has a `kind`, a message and a `point`:

| kind | |
|---|---|
| `request` | a request which does not fit itself or its models (an unknown field, a target the model does not have, a time outside of the simulation, a coordinate which would hide or be hidden by another name of the result, a model of a dimension of models without a selection of the first one), an answer which does not fit its round, a call out of turn |
| `units` | a time or a value which cannot be converted into the units of the model |
| `unsupported` | a simulation which is not supported, or a point the engine does not simulate as sbmlsim's reference does |
| `point` | a point of a scan of `on_error: raise` which failed, with its flat index as `point`; the message names its labels and values as sbmlsim does (`The point blowup=1, kb=2.0 of the scan failed: RuntimeError: ...`) |

Every check which needs no simulation runs when the scan is planned, so a request which does not fit the model fails before a point runs; the messages are sbmlsim's, in its python wording also for a host of JavaScript (the names of a list as python writes them, `['status']`, and sbmlsim's names such as `None` and `SBMLModel.set_selections`). With `on_error: raise` the scan fails with the first failed point in the C order of the scan: the chunks of other simulations or models which start before a failed point still run, as in sbmlsim. With `on_error: flag` a failed point is `NaN` with the variable `status` and the first 10 failed points in `errors`.

## The request `scan/1`

| field | |
|---|---|
| `contract` | `"scan/1"` |
| `scan` | sbmlsim's `Scan.to_dict()`: the `simulation` (start, end, time unit, pre-initialization, changes at absolute times, presimulation, output times or steps, time shift) and the `dimensions` (`values`, `simulations`, `models`; labels, values at a time `at`, the record of a design, covariates) |
| `observables` | `Observable.to_dict()` of each: `Formula`, `PK` and `Custom` |
| `keep` | the selections the result keeps next to the observables, `null` for every selection without observables |
| `time` | a grid of the timecourses, numbers in the time unit of the model or `{"value": [...], "unit": u}`; `null` for the output times of the simulations |
| `on_error` | `raise` or `flag` |
| `points` | `[start, stop)` of the flat indices of a block of the scan, `null` for the whole scan |
| `settings` | the integrator settings of `simulation/1` |
| `model_settings` | the integrator settings of the models of a dimension of models by their key, `settings` for a model without; optional |
| `changes` | the changes of the model of the run (target to value, sbmlsim's `AbstractModel.changes`), the defaults of the pre-initialization of its simulations; optional, none with a dimension of models |
| `model_changes` | the changes of the models of a dimension of models by their key (target to value), the defaults of the pre-initialization of their simulations; optional |
| `selections` | the selections of the model |

A number which is not finite is the text `"inf"`, `"-inf"` or `"NaN"` (or `null` for NaN, see [Numbers which are not finite](#numbers-which-are-not-finite)), also as the value of a change, which is then that number and no formula; `request_text` of python writes them so. The times and the values of a request are given in the units it names (a quantity `{"value": ..., "unit": ...}`) or in the units of the model, and converted into the units of the model with the registry of [Units](units.md), pint's numbers to the last bit. The changes of a model (sbmlsim's `AbstractModel.changes`) are the defaults of the pre-initialization of its simulations (`Simulation.with_preinit_defaults`): `changes` of the model of the run and `model_changes` of the models of a dimension of models.

## The plans `plan/1`

`scan.compile_plans(request, model, *, models=None)` compiles the plans of a request instead of running it, for a host which simulates them itself (sbmlsim's executors of roadrunner, COPASI and AMICI and its fit): every simulation of the scan (or every simulation of its dimension of simulations) against the model of the run (or every model of its dimension of models), as the planner compiles them before it plans the requests of the engine. A model is a `CompiledModel` (nothing has to be loaded to plan), a `LoadedModel` or the description `model/2` as a dict; of the request only `scan`, `changes` and `model_changes` decide the plans, so `{"contract": "scan/1", "scan": ...}` is a request:

```python
compiled = sbmlsim_core.compile_sbml(sbml)
for entry in scan.compile_plans(request, compiled):
    print(entry["simulation"], entry["model"], entry["plan"]["times"])
```

It returns one plan per simulation and model, every model of a simulation and then the next simulation (`compile_plans` of `sbmlsim-scan` in Rust gives `CompiledPlans`), each with

| field | |
|---|---|
| `simulation` | the index of the simulation in the dimension of simulations, 0 without one |
| `model` | the key of the model in the dimension of models, `null` for the model of the run |
| `plan` | the plan: `start`, `end` and `time_shift` in the time unit of the model, `preinit` (the assignments before the initialization), `events` (`{"time", "assignments"}`, the changes merged into one event per time, sorted by time), `steady_state` (the steady state before the start, `null` without one), `output` (`integrator` for the steps of the integrator, `times` for the output `times`, merged within the resolution of the time) and `steady_state_output` (the steady state after the end, the output at the time `inf`, `null` without one) |
| `values` | per dimension the values of its targets in the units of the model, `[{"target", "values"}]` in the order of the request (empty for a dimension of simulations or of models) |
| `at` | per dimension the time of its values in the time unit of the model, `null` for a dimension without `at` |

An assignment is `{"target", "kind", "value"}` of a number in the unit of its target in the model or `{"target", "kind", "formula"}` of a formula of the math of PEtab, which stays text; `kind` is `parameter`, `compartment`, `species amount` or `species concentration`. A steady state is `{"preinit", "absolute_tolerance", "relative_tolerance", "max_time"}`. A number is a number, `NaN` is `null` and the infinities are `"inf"` and `"-inf"`, as in `scanresult/1`, so the JSON reads back as the plans, every number by its bits. The plan of a point of the scan is the plan of its simulation and model with the values of the point applied: the values of the dimensions without `at` replace the assignment of their target wherever the plan has one and are added before the initialization otherwise, the values of a dimension with `at` are a change at its time (sbmlsim's `Plan.with_values`, `Plan::with_values` in Rust). The errors are the ones of a scan which does not fit its models (`ScanError` of the kind `request` or `units`).

## The units of `model/2`

The planner needs nothing of the model but its description, and reads from `model/2`: `units` and `time_unit`, the units sbmlsim gives the entities of the model (`time`, every compartment, species `S` and `[S]`, parameter and reaction), which convert the request and are the units of the result, read with `unit_definitions`, the unit definitions the units name by their id because pint has no unit of them; the kinds of the entities and which ones a change may set; their values as loaded; the states; `initial_assignments`, the parameters, compartments and species references whose value an initial assignment computes; whether the model has events; and `time_dependent`, whether a math reads the time or a delay or an event has a delay. A description of a release before 0.45.0 has no `time_unit`, `initial_assignments` and `time_dependent`, and the planner refuses it: compile the model again.

## The result `scanresult/1`

The arrays of sbmlsim's `ScanResult` (an `xarray.Dataset`), from which a host rebuilds the dataset with its names, coordinates and attributes:

| field | |
|---|---|
| `contract` | `"scanresult/1"` |
| `sizes` | the dimensions of the arrays with their sizes, in the order of the dataset: the dimensions of the scan (or `_index`, the flat indices of the points of a block), then `time` on a grid or `_point` of a ragged result |
| `dims` | the dimensions of the scan, sbmlsim's attribute `dims` (also of a block) |
| `layout` | `grid` (one grid of times, the coordinate `time`) or `ragged` (the output points of every point over `_point`, padded with `NaN`, the variable `time`) |
| `coords` | the coordinates in the order of the dataset (`_index` of a block, per dimension its labels, the values of its targets, under `<dimension>.<target>` where a variable has the name of the target, and its covariates, then `time` of a grid), each with `name`, `dims`, `unit` and `values`: `numbers`, `labels` (numbers or texts as the request gives them) or `indices` |
| `variables` | the variables in the order of the dataset (`time` of a ragged result, the kept timecourses, the values per simulation, `status` of `on_error: flag`), each with `name`, `dims`, `shape`, `unit` and `values` flat in C order |
| `units` | the unit of every variable and coordinate, in the order of sbmlsim's attribute `units` |
| `changes` | the times of the changes after the start in the time of the result |
| `errors` | the first 10 failed points (`index`, `point`, the labels and values of the point, and `message`) of `on_error: flag`, `null` for `on_error: raise` |
| `block` | `start`, `stop`, `shape`, `dims` and `coords` (the dimension of every coordinate along `_index`) of a run of `points`, else `null` |

The timecourses are on the grid of `time` (interpolated with the rules of sbmlsim's `grid_weights`), on the output times every simulation shares, on the union of the output times which differ only at their changes, or ragged. As JSON (the serialization of the Rust type) a number of an array is a number, `NaN` is `null` and the infinities are `"inf"` and `"-inf"`, so a serialized result reads back as itself, every value by its bits; the bindings carry the arrays as arrays of numbers of their language (numpy in python, `Float64Array` in JavaScript), with `NaN` and the infinities as values.

## Numbers which are not finite

JSON has no number for `NaN` and the infinities, so every contract writes them as text, and a reader takes what the others write:

| Where | `NaN` written | infinities written | also read |
|---|---|---|---|
| `scan/1`, the request (times, values and changes) | `"NaN"` | `"inf"`, `"-inf"` | `null`, `"nan"`, `"Infinity"`, `"-Infinity"` |
| `model/2`, the `initial` of an entity | `"NaN"` | `"inf"`, `"-inf"` | |
| `plan/1`, the plans | `null` | `"inf"`, `"-inf"` | `"NaN"` |
| `scanresult/1` as JSON, the result | `null` | `"inf"`, `"-inf"` | `"NaN"` |
| the arrays of python and JavaScript (`scan.run`, `runScan`) | `NaN` | `inf`, `-inf` (`Infinity`) | |

`plan/1` and `scanresult/1` write `NaN` as `null`, as the fixtures of sbmlsim write a result; the request and the description write the text, which the engine reads.
