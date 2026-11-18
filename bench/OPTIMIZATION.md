# The cost of a fit on the engine, and what one batch of plans is worth

A parameter fit is a batch problem. One evaluation of its objective is one simulation per condition, every one of them at the same parameters, and the Jacobian of a step of a least squares optimizer is that evaluation once per parameter and once at the parameters themselves, so `(parameters + 1) * conditions` simulations which do not depend on each other. A profile of the identifiability of one parameter is hundreds of such fits. None of this is new work for the engine: it is the work of a scan, with the points of the scan named differently.

This file measures what the python bindings cost per simulation on each of the paths they offer for that work, which is the question [sbmlsim#556](https://github.com/matthiaskoenig/sbmlsim/issues/556) asks, and estimates what is left for a fit which runs in Rust. [bench/SBMLSIM.md](SBMLSIM.md) is the same kind of measurement for one simulation.

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
- **Half of the threads are left.** 0.035 ms per simulation against 0.0185 ms of linear scaling on 20 threads is about 53 % efficiency at 220 points. What is left is python: a numpy table per point built under the GIL (220 of them per Jacobian), the GIL taken back between two slices of 160 points, and the list of objects the call answers with.

One-off costs are not in the table: compiling the model once, and `prepare` per condition, which instantiates the model once per call (0.33 ms, 17 ms for 50 conditions) and is paid once per fit and not per evaluation.

What the table does not measure is the rest of a fit: the residuals, the interpolation to the times of the data, the weighting and the pandas work of sbmlsim, the optimizer itself and the startup of its workers. The share of the simulation in a fit decides how much of these factors a fit sees, which is the point of the estimates below.

## Estimates, not measurements

1. **Within the python optimizer of sbmlsim, today.** Replacing the dispatch of one point per task by one `simulate_plans` per evaluation, or per Jacobian, is the `plans, one batch` column against the `dicts, a process pool` column: 1.9x to 5.4x of the simulation part of a fit, 4.6x to 10.6x against a serial run. A fit of few conditions and an identifiability profile, which re-fits one parameter at a time, are the small shapes, where the factor against a pool is 2.6x to 5.4x. This needs no new algorithm and no port: the prepared requests of the conditions are built once per fit and every evaluation is one call.
2. **A fit which evaluates in Rust.** `Workers::map` of `sbmlsim-batch` runs the whole work of a point on the thread of the point, so an objective which computes its residuals there builds no table of python at all. The measured gap to linear scaling is the python objects and the GIL, so the simulation part is estimated at 1.9x beyond item 1 (0.035 ms to about 0.0185 ms per simulation), and the residual, interpolation and weighting work leaves python with it. Together with item 1 that is an estimated 3.5x to 10x of what the fit of the same machine does today, before any change of algorithm.
3. **Exact sensitivities instead of a finite difference Jacobian.** A forward sensitivity run of `k` parameters costs about one integration plus a right hand side and a linear solve per parameter and step, against `k + 1` integrations for the finite differences, which is an estimated 2x to 4x on a Jacobian of ten parameters, and the gradients become exact: sbmlsim steps at 1e-3 relative today because a simulation is only accurate to the tolerance of the integrator, so an exact Jacobian is expected to converge in fewer iterations as well. Neither part is measured here, and both need work the engine has not got: CVODES is not compiled in `crates/csolvers` (its sources are in the submodule) and the code generator emits df/dy only, no df/dp.
4. **Differential evolution is the same arithmetic.** A generation of a population of `5k` members over `G` conditions is `5kG` simulations which do not depend on each other, so the Jacobian column is its column: at ten parameters and 20 conditions a generation is 1000 simulations, 36 ms in one batch of plans against 0.37 s of a serial loop. Differential evolution gains nothing from item 3, which is why the sensitivities are the last item and not the first.
