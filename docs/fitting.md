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
| `priors` | a prior per parameter, below; empty or absent for a fit which has none |
| `prior_correlation` | the correlation of a group of priors, below; nothing of it when it is absent |
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
| `model` | the id of the model the condition simulates, one of the models of the fit; absent for a fit of one model, which every condition simulates (see below) |
| `derived` | the values of the condition which follow the parameters of the fit, each a linear form over them, absent for a condition of numbers alone (see below) |
| `pinned` | the parameters of the fit whose targets the condition holds at a value of its own, each the `parameter` (its position in `parameters`) and its `value`; absent for a condition every parameter reaches (see [the points of a scan](#the-points-of-a-scan)) |

A fit of one model needs no `model` of its conditions, and a fit of several, e.g. of the data
of two builds of a model or of a model of two species, names the model of every condition:
`sbmlsim_core.fit_models` and `Problem::of_models` take the models by the id their conditions
name them by, a condition which names a model the fit has not or names none although the fit
has several is refused before anything is simulated, and a thread of the fit holds an instance
of every model its points met, so a fit of two models is twice the memory of a fit of one. The
parameters of a fit are the same in every model it reads: a parameter is one value of the
search, applied to its target in the model of every condition it reaches.

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
| `condition` | the `id` of the condition it measures; none for a curve over a `dimension` |
| `column` | the selection of that condition it measures; the condition has to select it |
| `times` | the times of the measurements, every one an output time of the condition (within 1e-9 relative). A fit interpolates nothing: the condition is asked for the times of its data |
| `values` | the measurements, one per time |
| `noise` | the noise of every measurement, which its residual is divided by; 1 for every one of them when it is absent. It is the standard deviation of the measurement for a curve without a `likelihood` and for the `normal` and `log-normal` kinds, and the scale of its distribution for the `laplace` and `log-laplace` kinds, whose standard deviation is `sqrt(2)` times it. A curve of a concentration and one of a volume are weighted against each other by the deviation of their measurements, and the threshold of an identifiability analysis is a rise of the cost which only means something when the residuals are scaled this way. A positive infinity is a measurement which carries no information (a caller whose weight of a point is 0 gives `reference / sqrt(0)`): its residual is `0.0` and its row of the jacobian is zeros, so the residuals and the jacobian keep the alignment of the problem. Zero, a negative value and `NaN` are refused |
| `likelihood` | which likelihood the curve is, below; absent is the weighted least squares of `noise`. A curve with one has **two** residuals per measurement |
| `weight` | a weight of the whole curve, 1 by default; a residual is multiplied by its square root, so a weight of 4 counts the curve four times |
| `reduction` | a value of the whole column the measurements are compared with instead of the column at their rows, below; absent for a curve of rows |
| `dimension` | the points of a dimension of a scan the curve interpolates over, each a condition, see [the points of a scan](#the-points-of-a-scan); absent for a curve of its `condition` |
| `after_change` | whether a measurement reads the state after a change of its condition rather than the one before it, one flag per time in their order; every measurement reads the state before a change when it is absent. The output of a condition has two rows of the time of a change after its start, the state before the change and the state after it, and a measurement reads the first of them unless its flag says otherwise. A chain of a change and the events it fired has more than two rows of that time, of which a true flag reads the last, the state after the whole chain. This is the `after` of the measurements of PEtab, which a problem read without an extension of its own uses |

The cost of a point of the search space is half the sum of the squares of its residuals, and a
residual is `(simulated - measured) / noise * sqrt(weight)`. A condition whose integration
fails scores every residual of its curves at 1e3 instead, which is finite and large, so an
optimizer walks away from a region it cannot integrate instead of ending the fit; a condition
which cannot be simulated at the start of the fit is an error of the request.

### The reduction of a curve

The data of a study is often a value per simulation rather than a timecourse: the largest concentration, the mean over a dosing interval, the amount at the end of a collection or a parameter of the non-compartmental analysis of a concentration (the area under the curve, the half-life). Such a value depends on every row of the column and on the times of the rows, so a curve which reads it carries a `reduction` and is compared with the reduced column:

```json
{ "condition": "control", "column": "C", "values": [2.6], "noise": [0.3], "reduction": { "kind": "max" } }
```

| `kind` | the value | its fields |
|---|---|---|
| `max`, `min` | the largest or the smallest value of the column over its rows of a finite time, ignoring `NaN` | `factor` |
| `mean` | the trapezoid integral of the column over its rows of a finite time divided by the time between the first and the last of them, the value of the last row for a simulation of one time point; the two rows of a change at its time are an interval without time | `factor`, `time_shift` |
| `at` | the column interpolated linearly at the `times` of the curve, one per measurement, with the jumps of sbmlsim (`grid_weights` of `sbmlsim.result.timecourse`): rows of one time are a jump, a time before a jump is interpolated toward the state before it, a time at a jump sees the state after it unless its flag of `before` says otherwise, and the rows after a jump at most `tolerance` apart are a chain of jumps; a time outside of the rows is refused when the problem is built | `factor`, `time_shift`, `tolerance`, `changes`, `before` |
| `pk` | the `parameter` of the non-compartmental analysis of pkpdutils (the kernel `pkpdutils-core`, which the python package runs as well) of the column: the `batch` of one sample of its contract `batch/1` with the units, the doses and the route of the analysis, whose time points and values the engine replaces with the rows of every simulation, and the `options` of the analysis; the state before a change at one of `changes` is left out, the sample is padded with `NaN` to the next power of two of its time points and to at least 16, as sbmlsim pads it, and a parameter the analysis does not derive is refused when the problem is built | `factor`, `time_shift`, `changes`, `parameter`, `batch`, `options` |

`factor` multiplies the reduced value (1 when it is absent), as a host multiplies the value of a formula by the factor of the unit it declares. Every time of a reduction is the time of the table plus `time_shift` (0 when it is absent): a host whose data is on a shifted axis reduces on the shifted times, and gives `changes` (the times of the changes after the start), the `times` of `at` and the times of the doses of a `pk` batch on that axis as well. A reduction other than `at` has no `times`, and every one of its measurements is compared with the one reduced value. A curve of a reduction reads no `after_change` and no `noise_column`, which are values at a row, and a request which carries them is refused; a `likelihood` with a `noise` is the likelihood of the reduced value.

The rows of a curve of a reduction are not resolved when the problem is built, so a condition all of whose curves are reductions may output the steps of the integrator, which is the output a host keeps for a reduction: its value depends on the rows it is computed over.

The jacobian of a reduction is the reduction of the forward sensitivities of the column: the sensitivity at the row of the extremum of `max` and `min` (the first of a tie), the trapezoid mean of the sensitivities, the sensitivities interpolated with the weights of `at`. The analysis of `pk` is no linear function of the column, so its row of the jacobian is the central difference of the analysis along the sensitivity of every parameter, `(P(c + h s) - P(c - h s)) / 2h`, with a step `h` which moves the column by `1e-6` of its largest absolute value: two analyses per parameter and no simulation, where a difference quotient of the condition costs a simulation per parameter. On the steps of the integrator a sensitivity is the better derivative of the two, since the steps move with the parameters, which a difference of the residuals sees as well.

`sbmlsim_core.FIT_REDUCTIONS` names the kinds a build fits. A release before 0.32.0 has no such constant and does not deny the field, so it would read the column at the rows of the times instead: a host reads the constant before it sends a reduction.

### The points of a scan

A host fits a scan as one condition per point, and two things of a scan are no curve of one condition.

The data of a study is not always measured at a point of a scan: the cmax of a dose between two doses of a scan, the amount in the urine of a subject whose renal function lies between two points of a scan of the renal function. sbmlsim compares such a measurement with the line between the values of the two points at the x of the measurement, `numpy.interp`, and a curve over a `dimension` is that line between two conditions:

```json
{
  "column": "C",
  "values": [2.1, 3.0, 4.4],
  "noise": [0.1, 0.1, 0.2],
  "reduction": { "kind": "max" },
  "dimension": {
    "conditions": ["dose 50", "dose 100", "dose 200"],
    "values": [50.0, 100.0, 200.0],
    "x": [75.0, 100.0, 150.0]
  }
}
```

| field | what it holds |
|---|---|
| `conditions` | the `id` of the condition of every point, in the order of their values; at least two, every one of them once |
| `values` | the value of the dimension at every point, finite and strictly increasing |
| `x` | the value of the dimension of every measurement, between the first and the last of `values` |
| `reductions` | the reduction of the curve in the condition of every point, one per point, for a curve whose reduction is not the same at every point, e.g. a `pk` whose batch carries the dose of the point; the curve then carries no `reduction` of its own. Absent, every point reads the `reduction` of the curve, or its column at the rows of its `times` |

A measurement between two points is compared with `slope * (x - x0) + y0` of the slope `(y1 - y0) / (x1 - x0)` of what the curve reads in the conditions of the two points, the arithmetic of `numpy.interp` operation for operation, so the residuals of a host which interpolates with numpy are the ones of the engine bitwise. A measurement at the value of a point, and one at the last point, is compared with what the curve reads in the condition of that point alone. What a curve reads in a condition is what a curve of that condition alone would read: the column at the row of the time of a measurement, or its reduction. The row of the jacobian of a measurement is the same line between the derivatives of the two points. A curve over a dimension names no `condition` and carries no `noise_column`, which is a value of one condition; the conditions of the points are only the points a curve reads, and two points next to each other on the dimension stay next to each other among them.

A dimension of a scan may set the target of a parameter of the fit. The engine sets a parameter on its target in every condition, over the values of the request of the condition, while a host whose dimension wins over the parameter, as sbmlsim's does, simulates the point with the value of the point. Such a condition pins the parameter:

```json
{ "id": "rate 0.2", "request": { "...": "..." }, "pinned": [{ "parameter": 0, "value": 0.2 }] }
```

The target of a pinned parameter holds the `value` of the pin in that condition, on the linear scale, so the parameter has no effect on the condition and its column of the jacobian of the curves of the condition is zero. A parameter is pinned at most once per condition, at a finite value. A `derived` value of the condition reads the value of the parameter and not the pin.

`sbmlsim_core.FIT_FIELDS` names the fields a build reads which an older release would not refuse: `"curve.dimension"` and `"condition.pinned"`. A release before them has no such constant, does not deny `pinned` and would set the parameter over the value of the point, and refuses a curve without a `condition` like any other request: a host reads the constant before it sends either field.

### The likelihood of a curve

A curve without a `likelihood` is a weighted least squares, which says nothing about the noise
of its measurements beyond dividing by it. A fit which estimates that noise needs more: the
scale of a density normalizes it as well as scaling the deviation, so an objective which only
divides by the scale is smaller the larger the scale, and an optimizer free to move it walks it
to infinity. The `likelihood` of a curve is that normalization.

```json
{
  "condition": "control",
  "column": "y",
  "times": [1.0, 2.0],
  "values": [0.5, 0.4],
  "noise": [0.02, 0.02],
  "likelihood": { "distribution": "log-normal", "noise_column": "sigma" }
}
```

| field | what it holds |
|---|---|
| `distribution` | which likelihood it is, one of `"normal"`, `"log-normal"`, `"laplace"` and `"log-laplace"`; required, and any other string is refused when the request is read |
| `noise_column` | the selection of the condition whose value in the row of a measurement is the noise of that measurement, which the condition has to select; absent means the noise is the `noise` of the curve. A noise which follows the parameters or the simulation is one more selection and no second simulation. A `noise_column` together with a `noise` is refused, and so is a likelihood with neither: a standard deviation of 1 is no noise model |

A curve with a likelihood has **twice** its measurements as residuals, laid out
`[data..., scale...]` per curve, with the curves in the order of the request, in the
`residuals` of the answer and in the rows of the jacobian alike. With `sigma` the noise of a
measurement and `value` its measured value:

```text
r      = (column - value) / sigma
data   = (normal | log-normal) ? r : r.signum() * sqrt(2 |r|)
term   = ln(sigma) + (log ? value : 0) + (laplace ? ln(2) : ln(2 pi) / 2)
offset = 100 + (log ? max(-2 value, 0) : 0)
scale  = sqrt(max(2 term + offset, 0))
```

both multiplied by the square root of the weight of the curve. Half the sum of the squares of
the two rows is the negative log-likelihood of the measurement **plus `0.5 * offset`**, which
the host subtracts: the engine never does, because the offset of a measurement of a log kind
depends on that measurement's own value. The constant 100 keeps the term under the root
positive for the noise a real fit meets, and a noise so small that `2 term + offset` is still
negative gives a scale row of `0.0`.

The rows of the jacobian are the chain rule of the two, with `dsigma` the sensitivity of the
noise column, which is zero for a fixed `noise`:

```text
ddata  = (dcolumn - r dsigma) / sigma      // and / sqrt(2 |r|) for a Laplace kind, 0 where r == 0
dscale = scale > 0 ? (dsigma / sigma) / scale : 0
```

A measurement whose noise is a positive infinity carries no information under a likelihood
exactly as it does without one: both of its residuals are `±0.0` and both of its rows of the
jacobian are zeros, so it keeps its place in the vector. A noise which is zero, negative or
`NaN` is a failed measurement, which scores 1e3 in both of its rows.

### The priors

A prior turns a fit into a maximum a posteriori estimate: the engine appends a residual row per
prior after every curve row, so that half the sum of the squares of the whole vector is the
negative log posterior of the fit up to a constant per prior which the host subtracts. A prior
is a function of the values of the parameters alone, so none of these rows costs a simulation.

```json
{
  "priors": [
    { "parameter": 0, "kind": "normal", "parameters": [1.0, 0.5], "offset": 2.0, "normalizer": -0.03659170390601009, "lower": 0.1, "upper": 5.0 }
  ]
}
```

| field | what it holds |
|---|---|
| `parameter` | the position of the parameter in `parameters`, which carries at most one prior and must be no member of `prior_correlation` |
| `kind` | one of `"normal"`, `"log-normal"`, `"uniform"`, `"log-uniform"`, `"laplace"` and `"log-laplace"` |
| `parameters` | exactly two numbers, the ones of PEtab for that kind, below |
| `offset` | the `c` of the row, the constant the host wants the cost measured from |
| `normalizer` | the log of the mass of the prior between `lower` and `upper`, which the engine subtracts from the density of the kind to get the truncated one; `0` for a prior the host does not truncate. The engine integrates nothing: a truncation is this one number |
| `lower`, `upper` | finite and increasing, and they have to cover the bounds of the parameter, since the row is `NaN` outside them and an optimizer searches the whole box. Wider bounds than the parameter's are accepted |

| `kind` | `parameters` | the density |
|---|---|---|
| `normal` | the mean and the standard deviation | the normal of the value |
| `log-normal` | the mean and the standard deviation **of the logarithm** | the log-normal of the value |
| `uniform` | the bounds of the value | `1 / (b - a)` inside them and zero outside |
| `log-uniform` | the bounds **of the value**, both positive, as PEtab writes them | `1 / (x (ln b - ln a))` |
| `laplace` | the location and the scale | the Laplace of the value |
| `log-laplace` | the location and the scale **of the logarithm** | the Laplace of the logarithm of the value |

The row of a prior is `sqrt(max(offset - 2 (log_density(x) - normalizer), 0))`, and its one
non-zero column of the jacobian is `-(d log_density / dx) / row` times the derivative of the
scale the parameter is searched in, or `0` where the maximum clipped the square to zero. The
row is `NaN`, with a zero column, for a value outside `[lower, upper]`, for a `NaN` value and
for a density of zero (a uniform prior outside its own support, a log kind at a value which is
not positive). A value **at** a bound is a value of the prior, since an optimizer sits at a
bound.

Priors which are not independent are one `prior_correlation`, a Gaussian copula over the normal
scores of its members:

```json
{
  "prior_correlation": {
    "parameters": [0, 1],
    "log": [false, true],
    "location": [1.0, 0.0],
    "scale": [0.5, 0.7],
    "inverse": [[1.0, 0.0], [-0.25, 0.5]],
    "offsets": [3.0],
    "lower": [0.1, 0.1],
    "upper": [5.0, 5.0]
  }
}
```

| field | what it holds |
|---|---|
| `parameters` | the `k` positions of its members in `parameters`, each at most once and none of them a `parameter` of `priors`; one member is allowed, none is refused |
| `log` | whether a member is read as its logarithm, one flag per member |
| `location`, `scale` | the location and the scale of the normal score of every member, `scale` positive and finite |
| `inverse` | the inverse of the lower Cholesky factor of the correlation, `k` rows of `k` values. A wrong row count and a ragged row are refused |
| `offsets` | one offset per **logarithmic** member, in the order of those members; `[]` when none of them is logarithmic |
| `lower`, `upper` | the bounds of every member, finite, increasing and positive where `log` is true. They are checked and are no term of the rows: a whitened score outside them is an ordinary number of the multivariate normal, and an optimizer searches a parameter between its own bounds |

With `v_i` the value of a member or its logarithm where `log[i]`, its score is
`z_i = (v_i - location_i) / scale_i`, and the group adds `k` whitened rows
`w_i = Σ_j inverse[i][j] z_j` followed by one row `sqrt(max(offsets_k + 2 ln(x_i), 0))` per
logarithmic member in ascending order of `i`. All of the whitened rows are `NaN` when any score
is not finite. The `Σ ln(scale) + ln(det L) + k ln(2 pi) / 2` of the density is not subtracted
by the engine and belongs in the offsets of the host, and the engine normalizes no truncation
of a correlated group.

The rows of the whole problem are therefore, in this order:

```text
[ every curve row, curve by curve ]    (a curve with a likelihood: its data rows, then its scale rows)
[ one row per entry of `priors`, in the order of the request ]
[ the k whitened rows of `prior_correlation` ]
[ one row per logarithmic member of `prior_correlation` ]
```

A host which wants the rows of its data alone takes the ones before the priors, whose number
its own request determines.

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
  flat, and a side has about `1 / rise` points below the threshold. A profile is locally
  quadratic in the step, so a step which is walked again is not reduced by `factor` but by
  the curvature its own rise gives: the step whose rise is half of what the walk allows is
  `step * sqrt(rise * delta / (2 * paid))` of the rise it paid, which reaches the scale of
  the profile in one correction where a fixed factor pays a fit of the other parameters for
  every factor of two it comes down; `factor` is what a step which rises slowly is enlarged
  by. A side which does not reach the threshold walks `points` points or the bound of the
  parameter, whichever comes first, which is what a parameter the data says nothing about
  does. The fit of a point starts from the
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
  Its residuals are **every** residual of the problem, the prior rows included, so for a fit
  which carries priors it is the precision of the posterior, the information of the data plus
  the curvature of the priors, and an interval of its errors is a credible interval of the
  posterior and not a confidence interval of the estimate; for a fit without priors the two are
  one number and it is the confidence interval. A host which wants the information of the data
  alone asks for the `jacobian` at the optimum and drops its trailing prior rows, whose number
  its own request determines.

| field | default | what it sets |
|---|---|---|
| `parameters` | every parameter | the ids to profile, in the order of the profiles |
| `walk` | `{"points": 50, "step": 0.1, "min_step": 0.01, "max_step": 1.0, "factor": 2.0, "rise": 0.2, "delta": 1.9207...}` | what a profile walks, above |
| `steps` | the `step` of the walk | the first step of every parameter of `parameters`, in its order and in the space it is searched: a caller which knows the scale of a parameter, e.g. from the standard error of the Fisher information of the optimum, starts its walk there instead of at one step for every parameter |
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
  sensitivities the engine refuses (events, a delay, an algebraic rule, a concentration or a
  parameter which is no parameter of the model)
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
optimum in the order of the rows of the problem (the curves, then the priors), and `runs` one
entry per start, in the order of the starts.
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
a profile carries the `values` the other parameters reached there. The `fisher` of a fit which
carries priors is the precision of its posterior, see [the
identifiability](#the-identifiability) above. When the information cannot
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

`sbmlsim_core.fit_models(models, request, threads=8)` is the same fit of several models, with
the loaded models by the id the conditions of the request name them by, and
`sbmlsim_core.fit_problem_of_models(models, request)` their problem for an optimizer which
stays in python.

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

`Problem::of_models` is the same problem of several models, which takes them by the id the
conditions of the definition name them by; `Problem::new` is the fit of one model, whose
conditions need no id of their own.

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
