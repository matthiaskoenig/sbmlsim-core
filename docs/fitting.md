# Fitting

A parameter fit of a compiled model is a request of the contract `fit/1`: the parameters to
vary, the conditions to simulate, the curves they are measured against, an optimizer and the
starts to run. The engine runs the whole fit and answers with the optimum, the counts of the
optimizer and, when the request asks for it, how well the data determines that optimum.

A fit is a batch problem. One evaluation of its objective is one simulation per condition, at
the same parameters, and the Jacobian of a step of a least squares optimizer is that
evaluation once per parameter and once at the parameters themselves, so
`(parameters + 1) * conditions` simulations which do not depend on each other. Every one of
them is one fan out over the threads of the engine
([Reusing Python batch workers](simulating.md#reusing-python-batch-workers) is the same pool),
the residuals of a point are computed on the thread of the point, and a fit in python holds no
GIL while it runs.

## The command line

`sbmlsim-core fit` runs a model directory with a request and writes the answer as JSON (to
stdout without `-o`):

```bash
sbmlsim-core fit crates/host/tests/decay-model fit.json -o answer.json [--threads 8]
```

It prints one line per fit, with the interval of every parameter it profiled:

```
k = 0.300000 [0.283746, 0.316945], y = 1.000000 [0.966070, 1.032069] after 8 evaluations in 0.013 s, cost 9.510899e-18 (SolverConverged): answer.json
```

## The request, `fit/1`

```json
{
  "contract": "fit/1",
  "parameters": [
    { "id": "k", "start": 0.5, "lower": 0.01, "upper": 10.0, "scale": "log10" }
  ],
  "conditions": [
    { "id": "control", "request": { "contract": "simulation/1", "end": 10.0, "output": { "uniform": { "steps": 10 } }, "selections": ["time", "y"] } }
  ],
  "curves": [
    {
      "id": "y of control",
      "condition": "control",
      "column": "y",
      "times": [0.0, 1.0, 2.0],
      "values": [1.0, 0.74, 0.55],
      "noise": [0.02, 0.02, 0.02],
      "weight": 1.0,
      "after_change": [false, false, false]
    }
  ],
  "optimizer": { "algorithm": "trust_region", "max_iterations": 100 },
  "starts": [],
  "threads": 8,
  "identifiability": null
}
```

| field | what it holds |
|---|---|
| `contract` | `"fit/1"` |
| `parameters` | the parameters to vary, below; at least one |
| `conditions` | the conditions to simulate, each an `id` and a `request` of [`simulation/1`](simulating.md#the-request-simulation1); the output of a request cannot be `"integrator_steps"`, because the rows of a condition are resolved once |
| `curves` | the measurements, below; at least one |
| `optimizer` | which optimizer runs and for how long, below |
| `starts` | the starts to run, each a value per parameter on the linear scale; the `start` of every parameter when it is empty. Every start is run, next to the others on the threads of the batch, and the best of them is the answer |
| `threads` | the threads of the batch of an evaluation; the CPUs of the machine when it is absent, and what a caller gives takes precedence |
| `jacobian` | where the jacobian of a step comes from: `"auto"` (the default, below), `"sensitivities"` or `"difference_quotient"` |
| `identifiability` | how well the data determines the optimum, below; nothing of it when it is absent |

The parameters:

| field | what it holds |
|---|---|
| `id` | the id of the parameter, a target a request can set (`preinit`): a parameter of the model, the amount of a species, a compartment |
| `start` | where the fit starts, between the bounds |
| `lower`, `upper` | the bounds, which every point of the search stays inside |
| `scale` | `"lin"` (the default), `"log"` or `"log10"`: the space the optimizer searches in. A parameter of a concentration, a rate or a volume is fitted on a logarithmic scale, where a step is a share of the value and not an absolute amount, and its bounds must be positive |

The conditions:

| field | what it holds |
|---|---|
| `id` | the id of the condition, which the curves read it by |
| `request` | the simulation, a request of `simulation/1` whose output is the times of the data of its curves |
| `derived` | the values of the condition which follow the parameters of the fit, each a linear form over them, absent for a condition of numbers alone (see below) |

A value of a request of `simulation/1` is a number, so a condition whose value is a parameter
the fit estimates, e.g. the initial amount of a species which a PEtab condition assigns an
estimated parameter, cannot carry it as one: a number folded when the request was built holds
that value where the fit started while the optimizer moves the parameter. Such a value is a
`derived` value of its condition instead, which the fit evaluates at the values the optimizer
is at and sets on its target as a pre-initialization of the condition, before every
simulation:

```json
{
  "id": "c0",
  "request": { "contract": "simulation/1", "end": 10.0, "output": { "uniform": { "steps": 10 } }, "selections": ["time", "y"] },
  "derived": [
    { "target": "y", "constant": 0.0, "terms": [{ "parameter": 0, "factor": 1.0 }] }
  ]
}
```

| field | what it holds |
|---|---|
| `target` | the entity of the model the value is set on, a settable selection, which must be no parameter of the fit and no other derived value of the same condition |
| `constant` | the constant term of the linear form, `0` when it is absent |
| `terms` | the terms over the parameters, each the `parameter` (its position in `parameters`) and its `factor`; empty for a value which is a constant |

The value is `constant + Σ factor * parameter` on the linear scale of the parameters, whatever
scale an optimizer searches them in. A fit whose conditions carry derived values takes the
difference quotients of its optimizer for its jacobian: a derived value follows more than one
parameter in general, which the one parameter per varying target of a block of the forward
sensitivities cannot carry.

The curves:

| field | what it holds |
|---|---|
| `id` | a name of the curve for an error message (optional) |
| `condition` | the `id` of the condition it measures |
| `column` | the selection of that condition it measures; the condition has to select it |
| `times` | the times of the measurements, every one an output time of the condition (within 1e-9 relative). A fit interpolates nothing: the condition is asked for the times of its data |
| `values` | the measurements, one per time |
| `noise` | the standard deviation of every measurement, which its residual is divided by; 1 for every one of them when it is absent. A curve of a concentration and one of a volume are weighted against each other by the deviation of their measurements, and the threshold of an identifiability analysis is a rise of the cost which only means something when the residuals are scaled this way |
| `weight` | a weight of the whole curve, 1 by default; a residual is multiplied by its square root, so a weight of 4 counts the curve four times |
| `after_change` | whether a measurement reads the state after a change of its condition rather than the one before it, one flag per time in their order; every measurement reads the state before a change when it is absent. The output of a condition has two rows of the time of a change after its start, the state before the change and the state after it, and a measurement reads the first of them unless its flag says otherwise. A chain of a change and the events it fired has more than two rows of that time, of which a true flag reads the last, the state after the whole chain. This is the `after` of the measurements of PEtab, which a problem read without an extension of its own uses |

The cost of a point of the search space is half the sum of the squares of its residuals, and a
residual is `(simulated - measured) / noise * sqrt(weight)`. A condition whose integration
fails scores every residual of its curves at 1e3 instead, which is finite and large, so an
optimizer walks away from a region it cannot integrate instead of ending the fit; a condition
which cannot be simulated at the start of the fit is an error of the request.

### The optimizer

```json
{ "algorithm": { "differential_evolution": { "seed": 7, "population": 20, "polish": true } }, "max_iterations": 200, "max_seconds": 600.0 }
```

| field | default | what it sets |
|---|---|---|
| `algorithm` | `"trust_region"` | which optimizer runs, below |
| `max_iterations` | 100 | the iterations, or the generations of a differential evolution |
| `max_seconds` | none | a budget of the whole run; the answer is the best point it reached (`"MaxTime"`) |
| `gradient_tolerance` | none | the scaled gradient a least squares run converges at; basin's default when it is absent |
| `step_tolerance` | none | the step a least squares run converges at, relative to the point it steps from, the `xtol` of `scipy.optimize.least_squares`; no test of the step when it is absent |
| `cost_tolerance` | none | the change of the cost a least squares run converges at, relative to the cost before the step, the `ftol` of `scipy.optimize.least_squares`; no test of the cost when it is absent |

A simulation is exact only to the relative tolerance of its integrator, so the cost of a fit carries a noise of about that size, and the last iterations of a run without `step_tolerance` and `cost_tolerance` chase that noise: such a run ends in `NumericalNoProgress` rather than at a step or a change of the cost it calls small enough. On the fits of the HCTZ model of sbmlsim a run without them takes 2.3 times the evaluations of one which stops at `1e-8` and finds the same optimum, so a caller which knows the tolerance of its integrator passes both.

| `algorithm` | what it does |
|---|---|
| `"trust_region"` | trust region reflective, the `trf` of `scipy.optimize.least_squares`, which keeps its steps inside the bounds by reflecting them. The default |
| `"levenberg_marquardt"` | the Levenberg-Marquardt of a box, which solves a damped normal equation per step |
| `{"differential_evolution": {...}}` | the differential evolution of Storn and Price (`DE/rand/1/bin`), the global search of a fit whose optimum is unknown. A generation of its population is one batch of the engine and none of its members needs a derivative |

The block of a differential evolution: `seed` (0 by default, and the same seed gives the same
fit whatever the threads are), `population` (`max(4, 10 * parameters)` by default), `f` and
`cr` (the factor of the mutation, dithered between 0.5 and 1.0 per generation, and the
crossover of 0.9, the ones of `scipy.optimize.differential_evolution`), and `polish` (true: a
trust region run from the best member of the last generation, whose counts are added to the
ones of the evolution).

The Jacobian of a least squares run is a forward difference of the residuals, one fan out of
`(parameters + 1) * conditions` simulations, with a relative step of 1e-3 in the space the
parameter is searched (the `diff_step` of sbmlsim, the square root of the relative tolerance
of the integration); a step at the upper bound is taken backwards.

### The identifiability

```json
{ "parameters": ["k"], "walk": { "points": 50, "step": 0.1, "min_step": 0.01, "max_step": 1.0, "factor": 2.0, "rise": 0.2, "delta": 1.920729410347062 }, "profiles": true, "fisher": true }
```

An optimum is not an answer until the data is known to determine it. Both ways of saying how
well it does run on the batch of the fit itself:

- **The profile of the likelihood** of a parameter holds it at a value and fits the others
  again, stepping away from the optimum in the space the parameter is searched (a decade for
  a parameter of the scale `log10`) until the cost of those fits rises by `delta` above the
  optimum, which is where the profile bounds the parameter; `delta` is
  `chi2(0.95, 1) / 2 = 1.9207...` by default, which is the 95 % interval of a cost that is
  half a sum of squares. The step follows the rise of the cost: the first one is `step`, a
  step which raises the cost by more than `rise` of `delta` is reduced by `factor` and walked
  again, down to `min_step`, and one which raises it by less than that fraction of `rise` is
  enlarged, up to `max_step`, so the points lie where the profile bends and not where it is
  flat, and a side has about `1 / rise` points below the threshold. A side which does not
  reach it walks `points` points or the bound of the parameter, whichever comes first, which
  is what a parameter the data says nothing about does. The fit of a point starts from the
  optimum of the point before it, so a profile of a real model costs about one fit per point
  and not more, the fits of the steps which were walked again included.
  The two sides of a profile and the profiles of the parameters do not depend on each other
  and run next to each other on the threads of the batch, which is what fills a pool a fit of
  few conditions leaves idle: the profiles of three parameters of a fit of two conditions are
  2.2 times faster that way on 20 threads.
A caller which has the optimum of a fit already, e.g. the best run of a fit which ran
before, walks the profiles around that point: the fit of the request starts there and runs
`max_iterations: 0`, so it takes no step and its answer is that point, and the `optimizer`
of the identifiability fits the other parameters at the points of the profiles.

- **The Fisher information** is `JᵀJ` of the residuals at the optimum, one Jacobian and the
  linear algebra of a `parameters x parameters` matrix. Its inverse is the covariance of the
  parameters, its diagonal their standard errors and its scaled form their correlations, in
  the space the parameters are searched. It holds where the cost is a parabola around the
  optimum, and it is singular exactly when the data does not determine every direction.

| field | default | what it sets |
|---|---|---|
| `parameters` | every parameter | the ids to profile, in the order of the profiles |
| `walk` | `{"points": 50, "step": 0.1, "min_step": 0.01, "max_step": 1.0, "factor": 2.0, "rise": 0.2, "delta": 1.9207...}` | what a profile walks, above |
| `optimizer` | the one of the fit | the optimizer of the points of a profile, which fit the other parameters |
| `profiles` | true | whether the profiles are walked at all, which is a fit per point of every one of them |
| `fisher` | true | whether the Fisher information is computed, which is one Jacobian |

## The jacobian of a step

An optimizer of least squares needs the derivative of every residual with respect to every
parameter, and the engine has two ways of giving it:

- **The finite differences** (`"difference_quotient"`) are one simulation per parameter
  besides the one of the residuals: `k + 1` simulations of every condition, which the batch
  runs on every thread at once. A column is a forward difference over a relative step of
  `1e-7` of the parameter, so it carries about 1e-3 of itself in error, and every condition of
  a fit can be differenced, whatever its request holds.
- **The forward sensitivities** (`"sensitivities"`) are one simulation per condition, which
  integrates `dy/dp` along with `y` ([forward
  sensitivities](simulating.md#forward-sensitivities)) and gives the derivative of a residual
  instead of a difference of two of them, accurate to about 1e-8. A condition whose
  sensitivities the engine refuses (events, a delay, an algebraic rule, the changes of a
  request, a steady state, a concentration or a parameter which is no parameter of the model)
  is refused with the condition and the reason, because a jacobian of two accuracies is worse
  than either of them.

`"auto"`, the default, takes what is fastest for the model, which is **the differences**: the
sensitivities of a model are one integration instead of `k + 1`, and `df/dp` inside them is
the analytic `model_jac_p` of the compiler where the model carries it, but a step of the
staggered corrector still solves one system per parameter. On hydrochlorothiazide a jacobian
of the sensitivities is 1.2 times the time of one of the differences, at five, ten, twenty and
forty parameters alike, and a whole fit of them runs 3.7 times fewer simulations in about the
same time
([bench/OPTIMIZATION.md](https://github.com/matthiaskoenig/sbmlsim-core/blob/main/bench/OPTIMIZATION.md)).
Ask for `"sensitivities"` where the accuracy of the jacobian is what matters, which is the
Fisher information of an optimum and a profile of a parameter the data barely determines.

## The answer

```json
{
  "contract": "fit/1",
  "status": "SolverConverged",
  "cost": 9.510899176032252e-18,
  "parameters": [{ "id": "k", "value": 0.30000000000711, "x": -0.5228787452700447 }],
  "iterations": 7,
  "evaluations": 8,
  "jacobians": 8,
  "simulations": 24,
  "jacobian": "difference_quotient",
  "seconds": 0.013,
  "residuals": [0.0, -1.2e-9],
  "runs": [{ "start": [0.5], "values": [0.3], "cost": 9.510899176032252e-18, "status": "SolverConverged", "iterations": 7, "evaluations": 8, "jacobians": 8, "jacobian": "difference_quotient", "seconds": 0.013 }]
}
```

`status` is why the optimizer of the best run stopped: `SolverConverged`, `MaxIter`,
`MaxTime`, `UserRequested` (a caller stopped the fit), `SolverFailed` and the other
termination codes of basin. `parameters` carries the `value` of every parameter on the linear
scale and the `x` it has in the space it was searched, `residuals` the residuals of the
optimum in the order of the curves, and `runs` one entry per start, in the order of the starts.
`seconds` is what the runs took, without the identifiability of the optimum. `simulations` is
what the fit cost the engine, the simulations of every run and of every profile together, and
`jacobian` where the jacobian of a step came from, which is what `"auto"` of the request
resolved to and which every run carries as well.

A fit which asked for the identifiability answers with `profiles` and `fisher` as well:

```json
{
  "profiles": [
    {
      "id": "k",
      "at": 0.30000000000711,
      "cost": 9.510899176032252e-18,
      "threshold": 1.920729410347062,
      "lower": 0.2837458775589689,
      "upper": 0.3169447876882784,
      "identifiable": true,
      "seconds": 0.119870317,
      "points": [{ "value": 0.2672, "x": -0.5728, "cost": 7.72, "values": [0.9647], "status": "SolverConverged", "seconds": 0.0054 }]
    }
  ],
  "fisher": {
    "parameters": ["k", "y"],
    "information": [10484.14, -10585.93, -10585.93, 29405.08],
    "covariance": [1.4985e-4, 5.394e-5, 5.394e-5, 5.3429e-5],
    "errors": [0.012241480428105413, 0.007309525773030912],
    "correlation": [1.0, 0.6029087563225627, 0.6029087563225627, 1.0],
    "singular": null
  }
}
```

`lower` and `upper` are the values where the profile reaches `threshold`, and `identifiable`
says whether it reached it on both sides. The crossing is interpolated on the square root of
the rise of the cost, which is a line in the parameter wherever the cost is a parabola around
the optimum, so a grid of a few points per side bounds a quadratic profile where the Fisher
information bounds it (`sqrt(2 * delta) = 1.96` standard errors; in the run above the profile
gives 0.0239 of a decade and the information 0.0240). On the cost itself the same grid would
report half the interval. A point of
a profile carries the `values` the other parameters reached there. When the information cannot
be inverted there is no `covariance`, no `errors` and no `correlation`, and `singular` says so
and names the parameters whose own information is nothing:

```
the information of the fit cannot be inverted (matrix is not positive definite): the data does not determine ti_hctz, KI__urine_volume, Vfeces
```

## Python

```python
import sbmlsim_core

compiled = sbmlsim_core.compile_sbml(sbml)
model = sbmlsim_core.Runtime().load(compiled.wasm, compiled.description)

answer = model.fit(request, threads=8)
print(answer["status"], answer["cost"])
for parameter in answer["parameters"]:
    print(parameter["id"], parameter["value"])
```

`LoadedModel.fit` runs the whole fit in the engine and holds no GIL while it does, so a thread
of the caller keeps running and `Ctrl-C` ends the fit where it is (the answer of a stopped fit
is `UserRequested`). The request is a dict of `fit/1` and the answer a dict of the JSON above;
`sbmlsim_core.FIT_CONTRACT` is the version of the contract this build carries.

A fit of many starts answers when the last of them finished, which is late for a fit of
minutes, so `on_run` follows it while it runs:

```python
answer = model.fit(
    request, threads=8, on_run=lambda index, run: print(index, run["cost"])
)
```

It is called with the index of a start and the run of it as soon as that run finished, on the
thread of the caller and with the GIL, while the fit runs on. Every run of a fit which
finished is reported before it answers, and the runs of a fit which `Ctrl-C` stopped are
reported before `KeyboardInterrupt` is raised, which is what a caller keeps them by; an error
the callable raises stops the fit and reaches the caller.

A caller whose optimizer stays in python asks for the objective instead. Every call is one
batch of the engine and nothing but the values of the parameters crosses:

```python
problem = model.fit_problem(request, threads=8)
x = numpy.array(problem.start)
for _ in range(20):
    residuals = problem.residuals(x)
    jacobian = problem.jacobian(x)  # (n_residuals, parameters), one fan out
    step, *_ = numpy.linalg.lstsq(jacobian, -residuals, rcond=None)
    x = x + step
print(problem.parameters, problem.values(x), problem.cost(x))
```

`FitProblem` carries `parameters`, `n_residuals`, `threads`, `start` and `bounds` (the space of
the search), and answers `values(x)`, `step(at, x)`, `residuals(x)`, `cost(x)` and
`jacobian(x)`. The conditions are prepared and the rows of every measurement resolved once,
when the problem is built.

## Rust

The crate `sbmlsim-fit` is the fit, and the contract is a thin layer over it:

```rust
use sbmlsim_fit::{Problem, Settings, Stop, Walk, fisher, profile};

let problem = Problem::new(definition, model, 8)?;
let fit = problem.optimize(&Settings::default())?;
let walked = profile(&problem, &Settings::default(), &fit.x, 0, &Walk::default(), &Stop::new())?;
let information = fisher(&problem, &fit.x)?;
```

`Problem::fixing` is what a profile holds a parameter with: the problem it gives shares the
conditions, the curves and the pool of the batch of the fit, so a profile of hundreds of points
prepares and instantiates nothing again. The optimizers are the ones of
[basin](https://crates.io/crates/basin), a pure Rust library with no BLAS, and the whole fit
runs inside `Workers::install` of [`sbmlsim-batch`](rust.md), so the population of a
differential evolution spreads over the threads of the engine and not over a pool of its own.
What a fit holds next to each other on that pool nests: the starts of a fit, the two sides of
a profile and the profiles of the parameters each fan their own evaluations out over the same
threads, and the answer is the same one on one thread and on twenty.

## What a fit costs

One Jacobian of a fit of hydrochlorothiazide (28 states, 20 conditions, 10 parameters, 220
simulations) is 7.7 ms on 20 threads, a fit of ten iterations of that model 0.26 s, and a
profile of one of its parameters, 8 points of a fit of the other nine, 0.91 s. The same
Jacobian out of the forward sensitivities is 20 simulations and 9.7 ms, and the same fit of
them 0.28 s and 800 simulations against 2960. A fit of few conditions does not fill a pool of
20 threads by itself (two conditions keep about five of them busy), which is why the starts and
the profiles run next to each other on it: four starts of such a fit are 1.2 times faster and
the profiles of three of its parameters 2.2 times.
`scripts/fit_timing.py` measures it against the same Jacobian through the python bindings, and
[bench/OPTIMIZATION.md](https://github.com/matthiaskoenig/sbmlsim-core/blob/main/bench/OPTIMIZATION.md)
says what the numbers mean: a fit of many conditions is bound by the integration and by how
well the threads of the machine scale, not by the host it is driven from.
