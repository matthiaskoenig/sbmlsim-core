# PEtab

[PEtab](https://petab.readthedocs.io) describes a parameter estimation problem as tables: the conditions an experiment applies at its times, the observables and the noise of the measurements, the measurements themselves and the parameters to estimate with their bounds and priors. sbmlsim-core reads a problem of PEtab v2, simulates it, evaluates its likelihood and fits it in Rust (the crate `sbmlsim-petab`), with no python and no sympy on the way: the math of the tables is read by the parser of the crate into the math of SBML, which the compiler and the engine evaluate as they evaluate a model.

## Running a problem

The command line evaluates a problem at the nominal values of its parameter table, writes its simulation table and fits it:

```bash
sbmlsim-core petab problem.yaml --simulations simulations.tsv
sbmlsim-core petab problem.yaml --fit --threads 8
```

```
777 measurements, 59 simulations of 1 models: llh 1.6202615718892766e3, chi2 1.6960511165260075e2, log prior -1.380354136700037e1 (read and compiled in 0.078 s, evaluated in 0.038 s)
```

The command is part of the build with the native engine (`cargo build -p sbmlsim-cli --features native`). In python, `sbmlsim_core.petab.Problem` is the same problem:

```python
from sbmlsim_core import petab

problem = petab.Problem.from_yaml("problem.yaml")
evaluation = problem.evaluate()  # at the nominal values
print(evaluation["llh"], evaluation["chi2"], evaluation["log_priors"])
print(evaluation["simulation_table"])

answer = problem.fit({"algorithm": "trust_region", "max_iterations": 200}, threads=8)
print(dict(zip(problem.parameters, answer["x"])), answer["objective"])
```

| member | what it is |
| --- | --- |
| `Problem.from_yaml(yaml, ...)`, `Problem.from_files(files, yaml, ...)` | a problem read from its directory, or from the contents of its files (`bytes` by name); `relative_tolerance` and `absolute_tolerance` of the integrator, `data_at_change` (`"after"`, `"before"`) and `extension` (`"read"`, `"ignore"`, see [the extension of sbmlsim](#the-extension-of-sbmlsim)) |
| `parameters`, `nominal`, `bounds` | the estimated parameters, in the order of a point, their nominal values and their bounds |
| `n_measurements`, `n_simulations` | the measurements and the simulations of an evaluation, one simulation per experiment of a model |
| `requests(x=None)` | the requests of [`simulation/1`](simulating.md#the-request-simulation1) of the simulations at a point |
| `evaluate(x=None)` | `llh`, `chi2`, `log_prior`, `log_priors` (by parameter), `unnorm_log_posterior`, `simulated` and `sigma` (per measurement), `simulation_table` and `failures` |
| `llh(x=None)` | the log-likelihood alone |
| `fit(settings=None, threads=None)` | a fit from the nominal values with an optimizer of the engine; `settings` is the `optimizer` of [`fit/1`](fitting.md#the-optimizer); gives `x`, the `objective` of PEtab, the `status` and the counts of the optimizer |
| `fit_problem(threads=None)`, `cost_offset` | the [`FitProblem`](fitting.md#python) of an optimizer of python, whose cost is the objective plus `cost_offset` |

In Rust:

```rust
use sbmlsim_petab::{Executor, Options, PetabProblem, Plan};

let problem = PetabProblem::from_path("problem.yaml")?;
let plan = Plan::new(&problem, &Options::default())?;
let executor = Executor::new(plan)?;
let evaluation = executor.evaluate(&executor.plan().nominal());
let answer = executor.fit(&sbmlsim_fit::Settings::default(), 8)?;
```

## How a problem runs on the engine

`PetabProblem::read` reads the YAML file, the models and the tables through `Files`, the contents of the files by their names, so the reader touches no file system. `Plan::new` translates the problem into what the engine simulates:

| PEtab v2 | the engine |
| --- | --- |
| a model | its SBML document, with a parameter added for every parameter of the parameter table it has not and a parameter of an assignment rule for every observable, every noise formula which reads the simulation, the logarithm of an observable of a logarithmic noise and every value of a condition after the start (ids beginning with `__petab_`), compiled as one model |
| an experiment of a model | one request of `simulation/1`; a measurement without an experiment is one of the model as it is, from the time 0 |
| the parameter table | the pre-initialization of the request: the estimated parameters at the point of the evaluation, the others at their nominal values; the engine skips the initial assignment of a value it pre-initializes, as PEtab asks |
| the condition of the first period | the pre-initialization as well, its values evaluated from the parameters of the table |
| a first period at `-inf` | the steady state of the request before its start, which is the time of the next period |
| a condition of a later period | a change at its time: a number is set as one, every other value is a form which reads the assignment rule of its value in the state before the change, so it reads the state, the time and the parameters where the change happens |
| a measurement at `inf` | the steady state after the end of the request, continued from the end |
| a measurement at the time of a change | the state after the change |
| the observable of a measurement | its column, the placeholders replaced by the values of the measurement; one column per distinct set of values |
| the noise | a number, or the column of its formula, which reads the observable, the placeholders, the parameters and the model |

A species in the math of PEtab is its amount for a species of `hasOnlySubstanceUnits` and its concentration otherwise, which is what the math of SBML reads it as; a condition sets the concentration of a species of a concentration. A change keeps the concentration of a species of a concentration in a compartment whose size it sets and the amount of a species of an amount, and an event the change triggers executes after it, both of them semantics of a change of the engine which are the ones of PEtab. The mapping table renames the ids of PEtab into the ids of the models.

`Plan::evaluate` reads the simulated observables and the noise of every measurement off the results of the requests and gives the densities of the specification:

| `noiseDistribution` | log density of a measurement `m` of the simulation `y` and the noise `sigma` |
| --- | --- |
| `normal` | `-ln(2 pi sigma^2) / 2 - ((m - y) / sigma)^2 / 2` |
| `log-normal` | `-ln(2 pi sigma^2 m^2) / 2 - ((ln m - ln y) / sigma)^2 / 2` |
| `laplace` | `-ln(2 sigma) - abs(m - y) / sigma` |
| `log-laplace` | `-ln(2 sigma m) - abs(ln m - ln y) / sigma` |

`chi2` is the sum of the squares of `(m - y) / sigma`, of the logarithms for a logarithmic noise. The log prior of an estimated parameter is the density of its prior, every one of the eleven distributions of PEtab v2, truncated at the bounds of the parameter where its support reaches beyond them; a parameter without a prior has the uniform prior between its bounds, as PEtab defaults it. A simulation which fails gives `NaN` for its measurements, and so a log-likelihood of `NaN`.

## The fit

`Executor::problem` is the problem of [`sbmlsim-fit`](fitting.md), so its optimizers, its Fisher information and its profiles fit a problem of PEtab. Every measurement is a curve of a [likelihood](fitting.md#the-likelihood-of-a-curve), the column of its observable (of its logarithm for a logarithmic noise) with the noise of its number or of its column, every prior of a normal, log-normal, uniform, log-uniform, Laplace or log-Laplace kind a [prior](fitting.md#the-priors) truncated at the bounds, and a value of a first period which is a linear form of the estimated parameters a [derived value](fitting.md#the-request-fit1). Half the sum of the squares of the residuals is the objective of PEtab plus a constant, `cost_offset`: the negative log-likelihood, and the negative log posterior for a problem whose parameters carry a prior. A parameter whose bounds are positive is searched on the scale `log10`, any other one on the linear scale.

What the fit of the engine cannot carry is refused with the reason, while the evaluation runs:

- a prior of the kinds `cauchy`, `chisquare`, `exponential`, `gamma` or `rayleigh`, which the rows of the engine do not evaluate (the cases 0024 and 0025 of the test suite),
- a value of a first period which is no linear form of the estimated parameters, e.g. their product,
- a bound which is not finite.

## The extension of sbmlsim

sbmlsim writes a problem with the extension `sbmlsim`, which carries what the tables cannot (`sbmlsim.fit.petab_v2`). Most of it is the objective of sbmlsim, its weighted least squares, which is not PEtab's: the plan does not read it and evaluates the likelihood of PEtab. What it says about the simulation of the measurements is read (`ExtensionPolicy::Read`, the default), so a problem of sbmlsim is the same problem in Rust and in python:

- `settings.data_at_change`: `before` reads a measurement at a change after the start in the state before the change, except at the first time of an observable of an experiment which is measured later as well, the start of a collection,
- `settings.relative_tolerance` and `settings.absolute_tolerance` (the smallest of its kinds), the tolerances of the integrator,
- `observables.<id>.kind`: only the training data enter the likelihood; validation data and outliers are simulated and reported.

`ExtensionPolicy::Ignore` (`extension="ignore"`) reads a problem as a tool which does not know the extension does. Any other extension which the problem marks as required is refused, as PEtab asks of a tool which does not support it.

## The PEtab test suite

The [PEtab test suite](https://github.com/PEtab-dev/petab_test_suite) runs at the commit sbmlsim pins (`d09985e7`), in `crates/petab/tests/testsuite.rs` against `crates/petab/tests/data/petab_baseline.json`, which lists the cases which do not pass. A case is evaluated at the nominal values of its parameter table with tolerances of the integrator of `1e-12`, and its simulations, `chi2`, `llh`, log priors and log posterior are compared with its solution at its own tolerances; the test fails for a case which breaks and for a case of the baseline which passes. Every case whose problem the fit carries is held to it as well: the cost of the fit at its start is the objective of PEtab plus `cost_offset`, to `1e-6`.

```bash
python scripts/petab_testsuite.py download   # the cases into ~/.cache/sbmlsim/petab-test-suite/<commit>
cargo test -p sbmlsim-petab --test testsuite -- --nocapture
```

| group | cases | pass | the fit of the engine |
| --- | --- | --- | --- |
| PEtab v2, SBML (`v2.0.0/sbml`) | 31 | 31 | 29, without 0024 and 0025 (priors) |
| PEtab v1, SBML, converted to v2 (`v1.0.0-converted/sbml`) | 20 | 20 | 20 |
| math (`v2.0.0/math`) | 98 | 98 | |

All 149 cases pass, the pass rate of sbmlsim in python (`tests/data/petab_baseline.json` of sbmlsim, 149 of 149). The cases of PEtab v1 are the ones sbmlsim converts with `petab.v2.petab1to2` of python into the same cache; the script of this repository downloads the cases of PEtab v2 and of the math, which CI runs. `SBMLSIM_PETAB_SUITE_PATH` names the directory `petabtests/cases` of a checkout instead, and `SBMLSIM_PETAB_BASELINE=write` writes the baseline again.

## A fit of sbmlsim as PEtab

The fit of the pharmacokinetics of hydrochlorothiazide of pkdb_models (`PK`, 6 parameters, 59 experiments, 777 measurements, a steady state before every experiment and repeated doses as changes of `PODOSE_hctz + 25`), written by sbmlsim with its fitted parameters (`python -m sbmlsim.run run petab`), evaluates as in python:

| | the tolerances of the problem (`1e-6`) | tolerances of `1e-10` and `1e-12` |
| --- | --- | --- |
| llh, Rust | 1620.2615719 | 1620.2609451 |
| llh, `sbmlsim.fit.petab_v2.log_likelihood` | 1620.2612695 | 1620.2609451 |
| chi2, Rust | 169.60511165 | 169.60636519 |
| chi2, sbmlsim | 169.60571636 | 169.60636529 |

At the tolerances of the problem the two differ by the error of their integrators (`2e-7` of the log-likelihood); at tight tolerances they agree to `1e-12` of it. Reading, compiling and evaluating the problem takes 0.1 s, and a fit of the likelihood of PEtab from the parameters of sbmlsim's least squares 1.7 s on 8 threads.

## In a browser

The reader, the plan and the evaluation are pure Rust without a file system, which builds for `wasm32-unknown-unknown` (`sbmlsim-petab` without its feature `native`). `compiler.wasm` carries them (the feature `petab` of `sbmlsim-wasm`, on by default) as `petab_open` (the problem from the JSON `{"yaml": name, "files": {name: text}}`, with the summary of its parameters), `petab_model` (a model of the plan compiled as `compile` compiles one), `petab_requests` (the requests of the simulations at a point), `petab_evaluate` (the evaluation from the results the host simulated, their values as the bytes of their `f64`s) and `petab_close`, which `crates/wasm/tests/petab.rs` drives with the results of a host.

`js/petab.mjs` is the host of JavaScript of these calls, which simulates the requests with `engine.wasm` under the loader or on the workers of a pool, so a page evaluates a problem with the bits of the command line:

```js
import { openPetab } from "sbmlsim-core/petab";

const problem = await openPetab({ compiler, runtime, files }); // {name: text or bytes}
const { llh, chi2, logPrior, simulationTable } = await problem.evaluate();
```

See [JavaScript](javascript.md#petab-problems) for its interface, the sizes and the times of the fit of hydrochlorothiazide in Node and in a browser.

## Not supported yet

- models of other languages than SBML (the PySB cases of the test suite), and a problem of PEtab v1, which is converted first (`petab.v2.petab1to2`),
- the fit of the problems listed under [the fit](#the-fit),
- the fit of a problem in a page: `js/petab.mjs` evaluates a problem, and the optimizers are `sbmlsim-fit`, which runs on the native host and its batch of threads ([#147](https://github.com/matthiaskoenig/sbmlsim-core/issues/147)),
- the problems of the [PEtab benchmark collection](https://github.com/Benchmarking-Initiative/Benchmark-Models-PEtab), which sbmlsim runs in python and which are not run in Rust yet,
- PEtab SciML.
