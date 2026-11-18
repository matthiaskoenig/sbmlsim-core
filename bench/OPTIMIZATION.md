# The cost of a fit on the engine, and what one batch of plans is worth

A parameter fit is a batch problem. One evaluation of its objective is one simulation per condition, every one of them at the same parameters, and the Jacobian of a step of a least squares optimizer is that evaluation once per parameter and once at the parameters themselves, so `(parameters + 1) * conditions` simulations which do not depend on each other. A profile of the identifiability of one parameter is hundreds of such fits. None of this is new work for the engine: it is the work of a scan, with the points of the scan named differently.

This file measures what the python bindings cost per simulation on each of the paths they offer for that work, which is the question [sbmlsim#556](https://github.com/matthiaskoenig/sbmlsim/issues/556) asks, and then the same Jacobian inside the engine, which is the fit of [`fit/1`](../docs/fitting.md). [bench/SBMLSIM.md](SBMLSIM.md) is the same kind of measurement for one simulation.

## The measurement

`scripts/objective_timing.py` measures one Jacobian of a fit of hydrochlorothiazide (28 states, the model of `bench/sbmlsim.toml`, 0.37 ms per simulation on this machine), with a condition an oral dose of 12.5 to 100 mg over 24 or 48 hours, 101 output points and one observable, the venous plasma concentration. The parameters are the ones a fit of that model varies first (`Ka_dis_hctz`, `ti_hctz`, `KI__HCTZEX_k`, `KI__urine_volume`, `f_renal_function` and so on), stepped by 1e-3 relative, which is the `diff_step` of sbmlsim; a shape of more than nine parameters fills up with the settable parameters after them, so the per-simulation time of such a shape is its own and only the paths within a shape compare.

```bash
.venv/bin/python scripts/objective_timing.py --conditions 20 --parameters 10 --rounds 9
```

The paths take turns within a round and the table is the median of nine rounds after a warm up, with the smallest and the largest round next to it, so that a load of the machine which comes and goes is spread over the paths instead of landing on the one which happens to run while it lasts. Every path is checked against `dicts, one instance` bitwise, as a multiset: a batch per condition answers its points in another order. The machine is the one of `bench/RESULTS.md` with 20 CPUs, and the batch paths run on 20 threads.

- `dicts, one instance`: a request of `simulation/1` per point on one kept instance, which is what an executor that simulates one point per call does.
- `prepared, one instance`: the same points as prepared requests, serially, which tells building a request and reading a table from the integration.
- `prepared, a batch per condition`: `BatchExecutor.simulate_prepared`, one call per condition, which is as far as one batch reached before `simulate_plans`.
- `plans, one batch`: `BatchExecutor.simulate_plans`, the whole Jacobian in one fan out over the pool.
- `dicts, a process pool`: a worker process per thread, each with an instance and the conditions of its own, one task per point and the values pickled back, which is the shape of the parallel fit of sbmlsim.

## One Jacobian, four shapes of a fit

| shape | simulations | `dicts, one instance` | `prepared, one instance` | `prepared, a batch per condition` | `plans, one batch` | `dicts, a process pool` |
|---|---:|---:|---:|---:|---:|---:|
| 20 conditions, no parameter (one evaluation) | 20 | 7.59 ms | 7.40 ms | 7.89 ms | **1.65 ms** | 8.86 ms |
| 8 conditions, 5 parameters | 48 | 18.14 ms | 17.28 ms | 5.63 ms | **2.17 ms** | 6.86 ms |
| 20 conditions, 10 parameters | 220 | 81.50 ms | 79.47 ms | 14.29 ms | **7.68 ms** | 15.02 ms |
| 50 conditions, 17 parameters | 900 | 332.95 ms | 323.93 ms | 36.64 ms | **31.96 ms** | 59.60 ms |

The same numbers as the speedup over `dicts, one instance`, and per simulation:

| shape | `prepared, one instance` | `prepared, a batch per condition` | `plans, one batch` | `dicts, a process pool` | `plans` ms per simulation |
|---|---:|---:|---:|---:|---:|
| 20 conditions, no parameter | 1.03x | 0.96x | **4.61x** | 0.86x | 0.082 |
| 8 conditions, 5 parameters | 1.05x | 3.22x | **8.37x** | 2.64x | 0.045 |
| 20 conditions, 10 parameters | 1.03x | 5.70x | **10.62x** | 5.43x | 0.035 |
| 50 conditions, 17 parameters | 1.03x | 9.09x | **10.42x** | 5.59x | 0.036 |

## What the numbers say

- **A request costs nothing on this model.** Prepared requests are 1.03 to 1.05 times a request of `simulation/1` per point, serially: building the dict, reading it into the types of the contract and giving the table back is a few microseconds next to 0.37 ms of integration. A prepared request is worth having because of what crosses into a worker (a vector of values and nothing else, and a plan resolved once per instance), not because the request was the cost.
- **One batch of plans is 1.9 to 5.4 times a process pool of the same width.** The pool pays a task of its own per point: 0.44 ms per simulation at 20 points, where it is slower than a serial loop (0.86x), and 0.066 ms at 900 points. One fan out pays 0.082 ms at 20 points and 0.035 ms above 200. The process pool is slower than a serial loop at 20 points and faster at 48, so a fit of few conditions, and a profile which re-fits one parameter at a time, gain the most from one fan out.
- **A batch per condition is not a batch.** One evaluation of an objective is one point per condition, so a call per condition is a serial run with a pool attached (0.96x). The plans of one batch are what makes an evaluation parallel at all, and on the Jacobian shapes they are still 1.2 to 2.6 times a call per condition, which cannot spread the conditions of a point over the threads.
- **Half of the threads are left, and it is not python.** 0.035 ms per simulation against 0.0185 ms of linear scaling on 20 threads is about 53 % efficiency at 220 points. The section below measures the same Jacobian inside the engine, where no table of python is built at all, and it reaches 0.0353 ms: the 220 numpy tables and the GIL cost 7 % of a Jacobian of this model, not half of it. What is left is the machine, which has 12 cores and not 20 (eight of them with two hyperthreads, four of them efficiency cores) and one memory system for 20 integrators.

One-off costs are not in the table: compiling the model once, and `prepare` per condition, which instantiates the model once per call (0.33 ms, 17 ms for 50 conditions) and is paid once per fit and not per evaluation.

What the table does not measure is the rest of a fit: the residuals, the interpolation to the times of the data, the weighting and the pandas work of sbmlsim, the optimizer itself and the startup of its workers. The share of the simulation in a fit decides how much of these factors a fit sees, which is the point of the estimates below.

## The same Jacobian inside the engine

`scripts/fit_timing.py` builds the same fit as a request of [`fit/1`](../docs/fitting.md) (20 conditions, 10 parameters, the data of the model at parameters a third above the ones the fit starts from, a standard deviation of 5 % of the largest value of a curve) and times one Jacobian of it three ways in one process, taking turns: the residuals in python over one batch of plans, the objective in Rust out of the finite differences, and the objective in Rust out of the [forward sensitivities](../docs/simulating.md#forward-sensitivities) of the conditions, which is one simulation per condition instead of `(k + 1)` of them:

```bash
.venv/bin/python scripts/fit_timing.py --conditions 20 --parameters 10 --rounds 9 [--threads 8] [--profile]
```

| the Jacobian of one step | simulations | 20 threads | ms per simulation | 8 threads | ms per simulation |
|---|---:|---:|---:|---:|---:|
| plans of one batch, the residuals in python | 220 | 8.47 ms | 0.0385 | 12.68 ms | 0.0576 |
| the objective in rust, the finite differences | 220 | **7.71 ms** | **0.0350** | **11.40 ms** | **0.0518** |
| the objective in rust, the forward sensitivities | 20 | 9.73 ms | 0.4864 | 13.18 ms | 0.6590 |

And the same fit run to its end both ways, at most ten iterations of the trust region each, both at a cost of 1e-9 or below and at parameters within 23 % of the ones of the data (the differences stop at `MaxIter`, the sensitivities at `NumericalNoProgress` after nine iterations):

| the same fit, 10 iterations | evaluations | Jacobians | simulations | 20 threads | 8 threads |
|---|---:|---:|---:|---:|---:|
| the finite differences | 26 | 11 | 2960 | **0.262 s** | 0.344 s |
| the forward sensitivities | 29 | 10 | 800 | 0.275 s | **0.337 s** |

The rounds of a path are within 10 % of each other once the pools do not overlap, and the table is the fastest round of each.

- **The host was not the cost.** One batch of plans with the residuals in numpy is 7 % off the rate the engine reaches on its own threads, on this model and at this width. A simulation of hydrochlorothiazide over 24 hours is 0.37 ms of integration, so the python object per point, the GIL and the list of results are a few per cent of it. A model of fewer states and shorter horizons shifts that share, which is why the script reports both.
- **What the fit in Rust is for is not the rate.** 0.134 s for a fit of six iterations (1700 simulations) is the same arithmetic as the python path would do; what the port adds is that nothing of it is in python: a fit is one call which holds no GIL and is interruptible, the optimizer and the residuals do not cross a thread, the hundreds of fits of an identifiability profile never return to the host (8 points of a profile of this model, each a fit of the other nine parameters, is 1.065 s), a differential evolution is bitwise the same fit on 1 thread and on 20, and the analysis the fit answers with has no python to be written in. On the fit above, which has ten parameters against one observable, it is the analysis which earns its keep: the Fisher information of the optimum is singular and names `ti_hctz`, `KI__urine_volume` and `Vfeces`, the parameters the plasma concentration of the drug says nothing about.
- **Fewer simulations are not yet less work.** The sensitivities do what they promise on the count: one simulation per condition instead of eleven, 800 simulations of a whole fit instead of 2960, and a derivative accurate to about 1e-8 instead of a difference accurate to 1e-3. They are 1.26 times the time of the differences per Jacobian on 20 threads and 1.16 on 8, and a whole fit of them is within 5 % of one of the differences either way (faster on 8 threads, slower on 20). What is left of the gap is not `df/dp` any more: with the analytic `model_jac_p` of the compiler a Jacobian of the sensitivities is 9.5 ms where the difference quotient of the right hand side made it 17.6 ms, 1.86 times less for the same derivative (the two agree to 3e-7 of themselves, with 111 parameters and 307 constants of the model in the chains). It is the staggered corrector, which solves one system per parameter and step: a run of `k` parameters costs about `1.2 (k + 1)` plain simulations, and the ratio is the same at five, ten, twenty and forty parameters. So `"auto"` of a fit takes the differences and `"sensitivities"` is the choice of a caller who wants the accuracy, which is what a Fisher information and a profile are measured in.

## Estimates, not measurements

1. **Within the python optimizer of sbmlsim, today.** Replacing the dispatch of one point per task by one `simulate_plans` per evaluation, or per Jacobian, is the `plans, one batch` column against the `dicts, a process pool` column: 1.9x to 5.4x of the simulation part of a fit, 4.6x to 10.6x against a serial run. A fit of few conditions and an identifiability profile, which re-fits one parameter at a time, are the small shapes, where the factor against a pool is 2.6x to 5.4x. This needs no new algorithm and no port: the prepared requests of the conditions are built once per fit and every evaluation is one call.
2. **A fit which evaluates in Rust.** This was estimated here at 1.9x beyond item 1, by reading the gap to linear scaling as the cost of the python objects and the GIL. It is measured in the section above at 1.07x: the gap is the machine, not the host. The port of the fit (`crates/fit`, [Fitting](../docs/fitting.md)) is worth having for what it does instead of for its rate per simulation, and the factor of a fit of sbmlsim against its fit of today stays the one of item 1.
3. **An analytic `df/dp`, which the sensitivities have now.** This was estimated here as the factor the sensitivities were missing: a run of ten parameters costing about seven plain simulations instead of the two to three it would cost with `df/dp` in hand, which would have made the Jacobian of the sensitivities 4x the differences the other way around. It is built (`model_jac_p` of the contract `model/2`, which the compiler differentiates out of the rates of the model column by column and the engine sums over the chain of a target) and measured in the section above at 1.86x: the Jacobian of the sensitivities is 9.5 ms instead of 17.6 ms, and 1.2 times the differences instead of 2.2 times. The estimate was the right factor on `df/dp` itself and wrong about what was left over, which is the one linear system per parameter and step of the staggered corrector. Beating the differences on time needs that cost, not the derivative: the adjoint of the cost (one backward integration per condition for the whole gradient, which a trust region cannot use as it stands) or a corrector which solves the sensitivities as one block.
4. **Differential evolution is the same arithmetic.** A generation of a population of `5k` members over `G` conditions is `5kG` simulations which do not depend on each other, so the Jacobian column is its column: at ten parameters and 20 conditions a generation is 1000 simulations, 36 ms in one batch of plans against 0.37 s of a serial loop. Differential evolution gains nothing from item 3, which is why the sensitivities are the last item and not the first.
