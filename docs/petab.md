# PEtab

[PEtab](https://petab.readthedocs.io) describes a parameter estimation problem as tables: the conditions an experiment applies at its times, the observables and the noise of the measurements, the measurements themselves and the parameters to estimate with their bounds and priors. sbmlsim-core reads a problem of PEtab v2 (since 0.38.0), simulates it, evaluates its likelihood and fits it in Rust (the crate `sbmlsim-petab`), with no python and no sympy on the way: the math of the tables is read by the parser of the crate into the math of SBML, which the compiler and the engine evaluate as they evaluate a model.

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

`Executor::problem` is the problem of [`sbmlsim-fit`](fitting.md), so its optimizers, its Fisher information and its profiles fit a problem of PEtab. Every measurement is a curve of a [likelihood](fitting.md#the-likelihood-of-a-curve), the column of its observable (of its logarithm for a logarithmic noise) with the noise of its number or of its column, and every prior (since 0.40.0), of any of the eleven kinds of PEtab v2, a [prior](fitting.md#the-priors) truncated at the bounds, whose row is zero at the mode of the prior and whose derivative is analytic. A value of a first period which reads estimated parameters is a [derived value](fitting.md#the-request-fit1): a linear form of them as the form itself, any other formula (`initial_A1 * initial_A2`, `exp(log_a)`) as its formula, which the fit evaluates where the optimizer is and whose partial derivatives, the ones of the operators of the engine's analytic jacobian, are the chain rule of the sensitivities of its target. Half the sum of the squares of the residuals is the objective of PEtab plus a constant, `cost_offset`: the negative log-likelihood, and the negative log posterior for a problem whose parameters carry a prior.

A parameter is searched between its bounds, within the support of its prior, outside which its posterior is zero (a uniform prior narrower than the bounds is the box of the search). A bound may be infinite (since 0.40.0; `-inf`, `inf` or an empty cell of the parameter table), which the least squares optimizers search as they search a box; a differential evolution draws its population in the box and refuses a bound which is not finite. The uniform prior PEtab gives a parameter without one has no density between bounds which are not finite, so PEtab's log posterior of such a problem is `-inf`: the objective of the fit leaves the improper prior out, a density of one. A parameter is searched on its scale of the [extension](#the-extension-of-sbmlsim), else on the scale `log10` when the bounds of its box are positive and finite and on the linear scale otherwise.

What the fit of the engine cannot carry is refused with the reason, while the evaluation runs: a prior whose density is not bounded between the bounds of its parameter, so that the posterior has no largest value and a fit no optimum. That is a gamma prior of a shape below one, a chi-squared prior of fewer than two degrees of freedom and a log-Laplace prior of a scale above one, at a lower bound of zero, e.g. the `log-laplace(3, 5)` of `p_log_laplace` of the cases 0024 and 0025 of the test suite, whose density grows as `x^(-0.8)` at zero.

## The extension of sbmlsim

sbmlsim writes a problem with the extension `sbmlsim`, which carries what the tables cannot (`sbmlsim.fit.petab_v2`). Most of it is the objective of sbmlsim, its weighted least squares, which is not PEtab's: the plan does not read it and evaluates the likelihood of PEtab. What it says about the simulation of the measurements is read (`ExtensionPolicy::Read`, the default), so a problem of sbmlsim is the same problem in Rust and in python:

- `settings.data_at_change`: `before` reads a measurement at a change after the start in the state before the change, except at the first time of an observable of an experiment which is measured later as well, the start of a collection,
- `settings.relative_tolerance` and `settings.absolute_tolerance` (the smallest of its kinds), the tolerances of the integrator,
- `parameters.<id>.scale`, else `settings.parameter_scale` (since 0.40.0): the scale a [fit](#the-fit) searches a parameter on, `LINEAR`, `LOG` or `LOG10` (the names of sbmlsim's `ParameterScaleType`); any other name is refused,
- `observables.<id>.kind`: only the training data enter the likelihood; validation data and outliers are simulated and reported. The validation problem of a fit (`role: validation`, the problem of sbmlsim's `to_petab_problems` which holds the validation data at the fitted parameters) has its validation data as the data of its likelihood, so its log-likelihood is the one of the validation data, as in sbmlsim.

`ExtensionPolicy::Ignore` (`extension="ignore"`) reads a problem as a tool which does not know the extension does. Any other extension which the problem marks as required is refused, as PEtab asks of a tool which does not support it.

## The PEtab test suite

The [PEtab test suite](https://github.com/PEtab-dev/petab_test_suite) runs at the commit sbmlsim pins (`d09985e7`), in `crates/petab/tests/testsuite.rs` against `crates/petab/tests/data/petab_baseline.json`, which lists the cases which do not pass. A case is evaluated at the nominal values of its parameter table with tolerances of the integrator of `1e-12`, and its simulations, `chi2`, `llh`, log priors and log posterior are compared with its solution at its own tolerances; the test fails for a case which breaks and for a case of the baseline which passes. The fit of every case is held to it as well: the cost of the fit at its start is the objective of PEtab plus `cost_offset`, to `1e-6`, but for 0024 and 0025, whose posterior has no largest value (see [the fit](#the-fit)), and whose refusal is held to that reason. The two cases of the priors are fitted in `the_cases_of_every_prior_are_fitted`, with the lower bound of `p_log_laplace` raised to 1: the cost is the negative log posterior and its gradient the central difference of the cost at points of the box, and a fit finds the mode of every prior, which no data informs.

```bash
python scripts/petab_testsuite.py download   # the cases into ~/.cache/sbmlsim/petab-test-suite/<commit>
cargo test -p sbmlsim-petab --test testsuite -- --nocapture
```

| group | cases | pass | the fit of the engine |
| --- | --- | --- | --- |
| PEtab v2, SBML (`v2.0.0/sbml`) | 31 | 31 | 29, and 0024 and 0025 refused for an unbounded prior, fitted with it bounded |
| PEtab v1, SBML, converted to v2 (`v1.0.0-converted/sbml`) | 20 | 20 | 20 |
| math (`v2.0.0/math`) | 98 | 98 | |

All 149 cases pass, the pass rate of sbmlsim in python (`tests/data/petab_baseline.json` of sbmlsim, 149 of 149). The cases of PEtab v1 are the ones sbmlsim converts with `petab.v2.petab1to2` of python into the same cache; the script of this repository downloads the cases of PEtab v2 and of the math, which CI runs. `SBMLSIM_PETAB_SUITE_PATH` names the directory `petabtests/cases` of a checkout instead, and `SBMLSIM_PETAB_BASELINE=write` writes the baseline again.

## The PEtab benchmark collection

The problems of the [PEtab benchmark collection](https://github.com/Benchmarking-Initiative/Benchmark-Models-PEtab) run at the commit sbmlsim pins (`c44d5ede`), in `crates/petab/tests/benchmark.rs` against `crates/petab/tests/data/benchmark_baseline.json`, which lists the problems which do not pass with the reason. The input are the problems sbmlsim downloads and converts to PEtab v2 (`sbmlsim.fit.petab_v2.benchmark`): the collection has every problem in PEtab v1, which `petab.v2.petab1to2` of python converts together with the simulations of the collection, so `scripts/petab_benchmark.py` runs the download of sbmlsim into its cache. A problem is read, compiled and evaluated at the nominal values of its parameter table as sbmlsim evaluates it in the core: the integrator at the tolerances `1e-8` (relative) and `1e-10` (absolute), `1e-12` and `1e-14` for `Crauste_CellSystems2017`, and a measurement at a change reads the state after it. The simulation of every measurement is compared with the simulations of the collection at `1e-3` absolute and relative, the log-likelihood with the one of AMICI at `1e-3` absolute and `1e-6` relative; the test fails for a problem which breaks, for a problem of the baseline which passes and for a problem which fails otherwise than the baseline says. `Froehlich_CellSystems2018` is left out, as in sbmlsim (9169 experiments with a pre-equilibration each).

```bash
uv run scripts/petab_benchmark.py download   # the problems into ~/.cache/sbmlsim/petab-benchmark/<commit>
cargo test --release -p sbmlsim-petab --test benchmark -- --nocapture
```

28 of the 34 problems pass, the pass rate of sbmlsim in python (`tests/data/benchmark_baseline.json` of sbmlsim, 28 of 34), and the six which do not are the same six for the same reasons: in four the simulations of the collection are wrong (`Fujita_SciSignal2010`, `Perelson_Science1996`, `Sneyd_PNAS2002`, `Zheng_PNAS2012`), `Chen_MSB2009` agrees with AMICI 1.2.0 while its reference is stale, and the log-likelihood of `Crauste_CellSystems2017` differs from the one of AMICI by `1.2e-3`. Three of the 28 need the reader of the core to do what libSBML does: `Armistead_CellDeathDis2024` requires `comp`, which the reader flattens (see [Compiling a model](compiling.md)), `Beer_MolBioSystems2014` has an `<apply>` whose only child is an `<apply>`, which is that child, and every measurement of `Blasi_CellSystems2016` is at `inf` without an experiment, the steady state the model reaches from its initial state. The test prints the seconds every problem takes to read, to plan and compile, and to evaluate: in a release build the whole collection takes 0.1 s to read, 0.6 s to compile and 0.6 s to evaluate (sbmlsim measures 15.9 s for the 31 problems the core ran before, read and evaluated in python, and 1.4 s for the core called from python), and 9 s in a debug build. CI downloads the collection once and runs it with the other tests; `SBMLSIM_BENCHMARK_PATH` names another directory of the collection, and `SBMLSIM_PETAB_BENCHMARK_BASELINE=write` writes the baseline again, keeping the reasons of the problems which fail as they did.

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

The reader, the plan and the evaluation are pure Rust without a file system, which builds for `wasm32-unknown-unknown` (`sbmlsim-petab` without its feature `native`). `compiler.wasm` carries them (the feature `petab` of `sbmlsim-wasm`, on by default since 0.39.0) as `petab_open` (the problem from the JSON `{"yaml": name, "files": {name: text}}`, with the summary of its parameters), `petab_model` (a model of the plan compiled as `compile` compiles one), `petab_requests` (the requests of the simulations at a point), `petab_evaluate` (the evaluation from the results the host simulated, their values as the bytes of their `f64`s since 0.39.0) and `petab_close`, which `crates/wasm/tests/petab.rs` drives with the results of a host.

`js/petab.mjs` (since 0.39.0) is the host of JavaScript of these calls, which simulates the requests with `engine.wasm` under the loader or on the workers of a pool, so a page evaluates a problem with the bits of the command line:

```js
import { openPetab } from "sbmlsim-core/petab";

const problem = await openPetab({ compiler, runtime, files }); // {name: text or bytes}
const { llh, chi2, logPrior, simulationTable } = await problem.evaluate();
```

A page fits a problem as well (since 0.40.0), `problem.fit()`: the definition of the fit (`Plan::definition`, the one of `Executor::fit`) and the optimizers of `sbmlsim-fit` are in `compiler.wasm` (the feature `fit` of `sbmlsim-wasm`, on by default), which ask for the simulations of every step (`petab_fit_open`, `petab_fit_ask`, `petab_fit_tell`, `petab_fit_stop` and `petab_fit_close`), and the page simulates them on its engine, see [`sbmlsim_fit::Asking`](fitting.md#a-fit-whose-host-simulates). Its jacobian is the finite differences of the command line.

See [JavaScript](javascript.md#petab-problems) for its interface, the sizes and the times of the evaluation and of the fit of hydrochlorothiazide in Node and in a browser.

## Not supported yet

- models of other languages than SBML (the PySB cases of the test suite), and a problem of PEtab v1, which is converted first (`petab.v2.petab1to2`),
- the fit of a posterior which has no largest value, see [the fit](#the-fit),
- the sensitivities in a page: `engine.wasm` writes no sensitivities into its output, so the fit of a page takes the finite differences, as the fit of the command line does by default ([#156](https://github.com/matthiaskoenig/sbmlsim-core/issues/156)),
- PEtab SciML.
