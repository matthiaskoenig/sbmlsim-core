# Benchmark and agreement with roadrunner

The owner's threshold: at most 2x the simulation time of roadrunner native (LLJIT `O2`), dense and with KLU, in wasmtime and in Chrome. This file reports the measurement on the models of `bench/models.toml`, the choice of `KLU_THRESHOLD` and the agreement with roadrunner at tight tolerances. The method is the one of the spike (`spike/RESULTS.md`), so the numbers are comparable.

## Summary

- No ratio is above 2x. The largest is 1.56 (the event model `repeated_dose`, dense, output of 201 times, wasmtime; Node and Chrome 1.05). On the large models the ratio is 1.11 to 1.46 (Chen dense 1.15 to 1.32, KLU 1.11 to 1.20; mp_flat dense 1.27 to 1.46, KLU 1.30 to 1.34), on the PBPK models 0.81 to 0.97, on mp_core 0.02 (dense) and 0.29 to 0.33 (KLU), because roadrunner restarts CVODE at the roots of its piecewise conditions and the engine does not.
- Chrome (headless Chrome 157 in a module worker, pinned to the performance cores) measured directly; Node and Chrome agree within a few percent and are 2 % to 13 % faster than wasmtime on the large models.
- The counters of the integrator are the same in wasmtime, Node and Chrome for every configuration (one `engine.wasm`, the same bits), and every timed simulation repeated the bits of the first.
- `KLU_THRESHOLD` is now 16 (was 64): KLU with the analytic Jacobian is as fast as the dense solver at 11 to 20 states and faster above (0.65 of the dense time at 64 states).
- Agreement at rtol 1e-10, atol 1e-12: midazolam, repressilator, demo and the event model pass the criterion of 1e-7 relative; hctz, mp_flat and mp_core do not. For hctz and mp_flat the difference is the integration error at rtol 1e-10: roadrunner at 1e-10 fails the same criterion against roadrunner at 1e-12, and the engine at 1e-12 passes it against roadrunner at 1e-12 (hctz 3.8e-9, mp_flat 3.8e-6 largest relative difference of a value above 1e-9). mp_core is ill-conditioned at the threshold of a piecewise (the oxygen of the cell sits at 0 within 1e-29 for the whole perfusion, `piecewise(1, o2_cell <= 0, ...)`), where roadrunner restarts at 161 roots; even at rtol 1e-12 its states differ by 1.8e-7 relative. No engine or compiler defect was found.

## Setup

- Machine: Intel Core i7-12700 (8 performance cores with 2 threads, 4 efficiency cores; 20 logical CPUs), Linux 7.0, governor `powersave`, the same machine as the spike. The command line hosts are pinned with `taskset -c 2` (a performance core), Chrome with `taskset -c 4-15`.
- sbmlsim-core: rustc 1.100.0, wasi-sdk 34 (clang 23.1.0), SUNDIALS 7.9.0, SuiteSparse 7.14.1 (KLU 2.3.6), the C solvers with `-O3 -msimd128 -ffp-contract=off`; wasmtime 50.0.1 (the crate of `sbmlsim-host`, Cranelift, defaults); Node 24.21.0; Chrome 157.0.8103.78 headless (`--headless=new`), the page served cross-origin isolated (`bench/web/serve.mjs`) so `performance.now()` in the worker is fine grained, driven with `chrome-devtools-axi`.
- roadrunner: dense is libroadrunner 2.12.0.dev683 (the newest build of the index of the sbmlsim scripts, python 3.14), set explicitly to `linear_solver = dense`, `jacobian = difference_quotient` (CVODE's dense difference quotient); KLU is the `roadrunner-sparsedq` build of the spike (`~/rr-enzyme/build-sparsedq`, libroadrunner 2.11.1, KLU with its colored sparse difference quotient); both LLJIT at `O2` (`DEFAULT`, as sbmlsim loads models). libroadrunner 2.12.0.dev683 has KLU with the sparse difference quotient as its default (`linear_solver = klu`, `jacobian = sparse_difference_quotient`); it is measured as a third roadrunner (`roadrunner 2.12.0 KLU` in the tables), within 0 to 8 % of the sparsedq build.
- Settings everywhere: BDF, relative tolerance 1e-6, absolute tolerance 1e-12 scaled per state as roadrunner applies it (the engine gets the very vector of roadrunner, `<model>.atol.json` of the preparation, checked equal in every roadrunner run), at most 20000 steps, 201 uniform output points (roadrunner `simulate(0, end, 201)`, the engine `Output::Uniform { steps: 200 }`), the states as the selections, and for `repeated_dose` also the output of the steps of the integrator (roadrunner `variable_step_size`).
- A measurement is the median of 20 runs after 3 warm ups; the configurations take turns within a round, 3 rounds, the tables have the median of the round medians. The time of the engine is the wall time of `Instance::simulate` (wasmtime) or `instance.simulate` (Node, Chrome): the request to JSON, its parse in the engine, the simulation and the copy of the result. An instance is reused, so its CVODE memory is re-initialized and not created per run (the spike created it per run). roadrunner's time is `np.array(r.simulate(...))` after `resetAll` and the pre-initialization.
- The engine uses the analytic Jacobian and, as a second configuration, the colored sparse difference quotient (`jacobian = difference`); the ratios are of the analytic Jacobian, dense against roadrunner dense, KLU against roadrunner KLU.

### Workflow

```bash
uv run scripts/prepare_model.py --bench bench/models.toml            # target/bench-models/: L3V2, comp flattened, distrib stripped, *.atol.json
taskset -c 2 target/release/sbmlsim-core bench bench/models.toml --json target/bench/engine.json --export target/bench/export
target/release/sbmlsim-core runtime -o js/runtime
taskset -c 2 node js/bench.mjs target/bench/export --json target/bench/node.json
node bench/web/serve.mjs target/bench/chrome.json &                  # then open http://127.0.0.1:8765/bench/web/index.html
uv run scripts/roadrunner_reference.py bench/models.toml --json target/bench/roadrunner.json
uv run scripts/roadrunner_reference.py bench/models.toml --compare target/bench/roadrunner.json \
    target/bench/engine.json target/bench/node.json target/bench/chrome.json
uv run scripts/scaling_models.py && taskset -c 2 target/release/sbmlsim-core bench \
    target/bench-models/scaling/scaling.toml --jacobians analytic --json target/bench/scaling-wasmtime.json
uv run scripts/agreement.py --convergence
```

`scripts/prepare_model.py` (ruling R39) converts a model to L3V2, flattens `comp`, strips the elements and annotations of `distrib` (the uncertainties of the fitted parameters of the machine perfusion models, metadata) and refuses, with a non-zero exit naming the element, a model whose math uses a distribution function of `distrib` (checked: an initial assignment `normal(0, 1)` is refused as `initialAssignment of x (normal)`); the compiler keeps refusing required packages. `bench/models.toml` references the prepared files in the gitignored `target/bench-models/`, and `prepared.json` there records every source with its sha256 (and the commit of a checkout).

### Models

| model | source | states | Jacobian entries | events | end | pre-initialization |
|---|---|---:|---:|---:|---:|---|
| hctz | `sbmlsim:hctz_body_flat.xml` (sbmlsim 0.24.0) | 28 | 82 | 0 | 1440 min | `PODOSE_hctz = 25` |
| midazolam | `sbmlsim:midazolam_body_flat.xml` | 38 | 93 | 0 | 1440 min | `PODOSE_mid = 7.5` |
| repeated_dose | `bench/models/repeated_dose.xml` (an oral dose every 8 h, two compartments and a metabolite) | 4 | 8 | 1 (21 executions) | 168 h | |
| mp_core | `machine_perfusion_liver_core.xml` of pkdb_models 1a8d30a (comp) | 43 | 184 | 0 | 360 min | |
| chen | Chen_MSB2009 of the PEtab benchmark collection at c44d5ed (L2V4) | 500 | 4400 | 0 | 9000 s | `c1 = 5e-9` (condition model1_data1) |
| mp_flat | `machine_perfusion_liver_flat.xml` of pkdb_models 1a8d30a (distrib) | 919 | 4242 | 0 | 360 min | |

The brief's list has no model with events; `repeated_dose` was added to measure the events and the probe of the triggers (Task 13). Chen has `piecewise` functions of the time (a stimulation at 1800 s), which roadrunner finds as roots (4 restarts) and the engine integrates through.

## Ratios against roadrunner

The analytic Jacobian of the engine; ms the median time of a simulation; ratio the time over roadrunner's (dense against roadrunner dense, KLU against roadrunner KLU).

| model | states | solver | output | roadrunner ms | wasmtime ms | ratio | Node ms | ratio | Chrome ms | ratio |
|---|---:|---|---|---:|---:|---:|---:|---:|---:|---:|
| hctz | 28 | dense | times | 1.12 | 0.999 | 0.89 | 0.956 | 0.85 | 0.935 | 0.84 |
| hctz | 28 | klu | times | 0.922 | 0.896 | 0.97 | 0.864 | 0.94 | 0.827 | 0.90 |
| midazolam | 38 | dense | times | 0.766 | 0.714 | 0.93 | 0.638 | 0.83 | 0.642 | 0.84 |
| midazolam | 38 | klu | times | 0.61 | 0.557 | 0.91 | 0.493 | 0.81 | 0.5 | 0.82 |
| repeated_dose | 4 | dense | times | 1.43 | 2.22 | 1.56 | 1.5 | 1.05 | 1.49 | 1.04 |
| repeated_dose | 4 | klu | times | 1.68 | 2.49 | 1.49 | 1.66 | 0.99 | 1.59 | 0.95 |
| repeated_dose | 4 | dense | steps | 5.01 | 2.89 | 0.58 | 2.16 | 0.43 | 2.12 | 0.42 |
| repeated_dose | 4 | klu | steps | 5.47 | 3.22 | 0.59 | 2.28 | 0.42 | 2.31 | 0.42 |
| mp_core | 43 | dense | times | 134 | 3.31 | 0.02 | 3.01 | 0.02 | 3.04 | 0.02 |
| mp_core | 43 | klu | times | 7.14 | 2.35 | 0.33 | 2.11 | 0.30 | 2.11 | 0.29 |
| chen | 500 | dense | times | 1127 | 1484 | 1.32 | 1323 | 1.17 | 1295 | 1.15 |
| chen | 500 | klu | times | 460 | 553 | 1.20 | 513 | 1.11 | 510 | 1.11 |
| mp_flat | 919 | dense | times | 2945 | 4312 | 1.46 | 3827 | 1.30 | 3734 | 1.27 |
| mp_flat | 919 | klu | times | 84.4 | 113 | 1.34 | 111 | 1.31 | 110 | 1.30 |

Where the ratios above 1 come from:

- mp_flat and Chen: the work differs, the cost per step barely. The engine takes more steps than roadrunner (mp_flat KLU 2694 against 2306, dense 2798 against 2060; Chen KLU 2901 against 2641, dense 2988 against 2812). Per step the engine in wasmtime is at 1.08 of roadrunner on mp_flat dense (the dense LU of SUNDIALS on 919 x 919 in both), 1.14 on mp_flat KLU, 1.24 on Chen dense and 1.10 on Chen KLU; in Node and Chrome at 0.93 to 0.96, 1.11 to 1.13, 1.08 to 1.10 and 1.01 to 1.02. The step counts depend on the last bits of the right hand side and the Jacobian (the analytic Jacobian against roadrunner's difference quotients, the order of the terms), as in the spike (2184 / 2304 steps of the spike's engine against 2060 / 2306); the solutions are in the same accuracy class (below). Chen dense with the analytic Jacobian is slower than with the difference quotient (1484 against 1083 ms, 2988 against 2691 steps).
- repeated_dose in wasmtime: 3594 steps of a model of 4 states, 0.62 µs per step against 0.42 in Node and Chrome and 0.40 in roadrunner; the engine is bound by its own overhead per step (events, root functions, output) on such a small model, which Cranelift compiles less well than V8. The output of the steps of the integrator is 0.4 to 0.6 of roadrunner's.

## Compilation and instantiation

compile: SBML to `model.wasm` and `model.json` (`sbmlsim_compiler::compile`, the median of 5). instantiate: `Runtime::instantiate` in wasmtime (the compilation of `model.wasm` by Cranelift, the instances, the load of the description) and `runtime.instantiate` in Node and Chrome (V8 compiles lazily), the median of 5; `engine.wasm` and `mathlib.wasm` are compiled once per runtime. roadrunner's load is `RoadRunner(sbml, options)` (LLJIT `O2`).

| model | states | Jacobian entries | model.wasm kB | model.json kB | compile ms | instantiate wasmtime ms | instantiate Node ms | instantiate Chrome ms | roadrunner load s |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| hctz | 28 | 82 | 16.4 | 39 | 2.25 | 8.5 | 2.11 | 0.91 | 0.097 |
| midazolam | 38 | 93 | 21.1 | 54 | 2.24 | 10.1 | 1.54 | 0.6 | 0.135 |
| repeated_dose | 4 | 8 | 1.7 | 4 | 0.148 | 2.5 | 0.21 | 0.175 | 0.018 |
| mp_core | 43 | 184 | 20.9 | 50 | 4.16 | 10.9 | 0.604 | 0.54 | 0.073 |
| chen | 500 | 4400 | 216.5 | 375 | 46.5 | 122 | 3.68 | 2.9 | 0.907 |
| mp_flat | 919 | 4242 | 495.8 | 704 | 95.6 | 307 | 4.85 | 5.59 | 1.73 |

Compile and instantiate in wasmtime together are 4 to 11 times faster than roadrunner's load (mp_flat 0.40 s against 1.73 s); in V8 the instantiation is milliseconds, and the code is compiled lazily during the warm ups.

## Full tables

rhs: the calls of the right hand side by CVODE (roadrunner: `num_rhs_evals`); DQ rhs: those of a difference quotient (roadrunner dense: CVODE's `num_lin_rhs_evals`; roadrunner's sparse difference quotient is not counted); integrations: the (re)initializations of CVODE (roadrunner: its restarts at roots plus 1); roots: the calls of the root functions; probe rhs: the calls of the right hand side by the probe of the triggers (below). wasmtime, Node and Chrome have the same counters in every configuration.

#### hctz

| host | solver | Jacobian | output | median ms | µs per step | steps | rhs | Jacobians | DQ rhs | lin setups | err test fails | nonlin iters | conv fails | roots | integrations | probe rhs |
|---|---|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| roadrunner dense | dense | difference_quotient | times | 1.118 | 2.33 | 479 | 538 | 9 | 252 | 63 | 7 | 537 | 0 | - | 1 | - |
| roadrunner 2.12.0 KLU | klu | sparse_difference_quotient | times | 0.9046 | 1.89 | 479 | 538 | 9 | - | 63 | 7 | 537 | 0 | - | 1 | - |
| roadrunner KLU | klu | sparse_difference_quotient | times | 0.9215 | 1.92 | 479 | 538 | 9 | - | 63 | 7 | 537 | 0 | - | 1 | - |
| wasmtime | dense | analytic | times | 0.9991 | 2.09 | 479 | 536 | 9 | 0 | 63 | 7 | 535 | 0 | 0 | 1 | 0 |
| wasmtime | dense | difference | times | 1.019 | 2.13 | 479 | 538 | 9 | 54 | 64 | 7 | 537 | 0 | 0 | 1 | 0 |
| wasmtime | klu | analytic | times | 0.896 | 1.87 | 479 | 536 | 9 | 0 | 63 | 7 | 535 | 0 | 0 | 1 | 0 |
| wasmtime | klu | difference | times | 0.9116 | 1.9 | 479 | 538 | 9 | 54 | 64 | 7 | 537 | 0 | 0 | 1 | 0 |
| Node | dense | analytic | times | 0.956 | 2 | 479 | 536 | 9 | 0 | 63 | 7 | 535 | 0 | 0 | 1 | 0 |
| Node | dense | difference | times | 0.9732 | 2.03 | 479 | 538 | 9 | 54 | 64 | 7 | 537 | 0 | 0 | 1 | 0 |
| Node | klu | analytic | times | 0.8637 | 1.8 | 479 | 536 | 9 | 0 | 63 | 7 | 535 | 0 | 0 | 1 | 0 |
| Node | klu | difference | times | 0.8679 | 1.81 | 479 | 538 | 9 | 54 | 64 | 7 | 537 | 0 | 0 | 1 | 0 |
| Chrome | dense | analytic | times | 0.935 | 1.95 | 479 | 536 | 9 | 0 | 63 | 7 | 535 | 0 | 0 | 1 | 0 |
| Chrome | dense | difference | times | 0.945 | 1.97 | 479 | 538 | 9 | 54 | 64 | 7 | 537 | 0 | 0 | 1 | 0 |
| Chrome | klu | analytic | times | 0.8275 | 1.73 | 479 | 536 | 9 | 0 | 63 | 7 | 535 | 0 | 0 | 1 | 0 |
| Chrome | klu | difference | times | 0.8175 | 1.71 | 479 | 538 | 9 | 54 | 64 | 7 | 537 | 0 | 0 | 1 | 0 |

#### midazolam

| host | solver | Jacobian | output | median ms | µs per step | steps | rhs | Jacobians | DQ rhs | lin setups | err test fails | nonlin iters | conv fails | roots | integrations | probe rhs |
|---|---|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| roadrunner dense | dense | difference_quotient | times | 0.7659 | 2.57 | 298 | 330 | 6 | 228 | 38 | 1 | 327 | 0 | - | 1 | - |
| roadrunner 2.12.0 KLU | klu | sparse_difference_quotient | times | 0.5747 | 1.93 | 298 | 330 | 6 | - | 38 | 1 | 327 | 0 | - | 1 | - |
| roadrunner KLU | klu | sparse_difference_quotient | times | 0.6096 | 2.05 | 298 | 330 | 6 | - | 38 | 1 | 327 | 0 | - | 1 | - |
| wasmtime | dense | analytic | times | 0.7137 | 2.39 | 298 | 329 | 6 | 0 | 39 | 1 | 326 | 0 | 0 | 1 | 0 |
| wasmtime | dense | difference | times | 0.7147 | 2.4 | 298 | 329 | 6 | 42 | 39 | 1 | 326 | 0 | 0 | 1 | 0 |
| wasmtime | klu | analytic | times | 0.5567 | 1.87 | 298 | 329 | 6 | 0 | 39 | 1 | 326 | 0 | 0 | 1 | 0 |
| wasmtime | klu | difference | times | 0.5634 | 1.89 | 298 | 329 | 6 | 42 | 39 | 1 | 326 | 0 | 0 | 1 | 0 |
| Node | dense | analytic | times | 0.6381 | 2.14 | 298 | 329 | 6 | 0 | 39 | 1 | 326 | 0 | 0 | 1 | 0 |
| Node | dense | difference | times | 0.6311 | 2.12 | 298 | 329 | 6 | 42 | 39 | 1 | 326 | 0 | 0 | 1 | 0 |
| Node | klu | analytic | times | 0.4934 | 1.66 | 298 | 329 | 6 | 0 | 39 | 1 | 326 | 0 | 0 | 1 | 0 |
| Node | klu | difference | times | 0.5 | 1.68 | 298 | 329 | 6 | 42 | 39 | 1 | 326 | 0 | 0 | 1 | 0 |
| Chrome | dense | analytic | times | 0.6425 | 2.16 | 298 | 329 | 6 | 0 | 39 | 1 | 326 | 0 | 0 | 1 | 0 |
| Chrome | dense | difference | times | 0.64 | 2.15 | 298 | 329 | 6 | 42 | 39 | 1 | 326 | 0 | 0 | 1 | 0 |
| Chrome | klu | analytic | times | 0.5 | 1.68 | 298 | 329 | 6 | 0 | 39 | 1 | 326 | 0 | 0 | 1 | 0 |
| Chrome | klu | difference | times | 0.4975 | 1.67 | 298 | 329 | 6 | 42 | 39 | 1 | 326 | 0 | 0 | 1 | 0 |

#### repeated_dose

| host | solver | Jacobian | output | median ms | µs per step | steps | rhs | Jacobians | DQ rhs | lin setups | err test fails | nonlin iters | conv fails | roots | integrations | probe rhs |
|---|---|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| roadrunner dense | dense | difference_quotient | times | 1.427 | 0.397 | 3594 | 4091 | 64 | 256 | 431 | 40 | 4029 | 0 | - | 22 | - |
| roadrunner dense | dense | difference_quotient | steps | 5.008 | 1.39 | 3595 | 4092 | 64 | 256 | 431 | 40 | 4031 | 0 | - | 21 | - |
| roadrunner 2.12.0 KLU | klu | sparse_difference_quotient | times | 1.549 | 0.431 | 3594 | 4091 | 64 | - | 431 | 40 | 4029 | 0 | - | 22 | - |
| roadrunner 2.12.0 KLU | klu | sparse_difference_quotient | steps | 5.049 | 1.4 | 3595 | 4092 | 64 | - | 431 | 40 | 4031 | 0 | - | 21 | - |
| roadrunner KLU | klu | sparse_difference_quotient | times | 1.678 | 0.467 | 3594 | 4091 | 64 | - | 431 | 40 | 4029 | 0 | - | 22 | - |
| roadrunner KLU | klu | sparse_difference_quotient | steps | 5.472 | 1.52 | 3595 | 4092 | 64 | - | 431 | 40 | 4031 | 0 | - | 21 | - |
| wasmtime | dense | analytic | times | 2.221 | 0.618 | 3594 | 4091 | 65 | 0 | 432 | 40 | 4029 | 0 | 3834 | 21 | 221 |
| wasmtime | dense | difference | times | 2.227 | 0.62 | 3594 | 4091 | 65 | 195 | 432 | 40 | 4029 | 0 | 3834 | 21 | 221 |
| wasmtime | klu | analytic | times | 2.495 | 0.694 | 3594 | 4091 | 65 | 0 | 432 | 40 | 4029 | 0 | 3834 | 21 | 221 |
| wasmtime | klu | difference | times | 2.484 | 0.691 | 3594 | 4091 | 65 | 195 | 432 | 40 | 4029 | 0 | 3834 | 21 | 221 |
| wasmtime | dense | analytic | steps | 2.891 | 0.804 | 3595 | 4092 | 65 | 0 | 432 | 40 | 4031 | 0 | 3636 | 21 | 3596 |
| wasmtime | dense | difference | steps | 2.902 | 0.807 | 3595 | 4092 | 65 | 195 | 432 | 40 | 4031 | 0 | 3636 | 21 | 3596 |
| wasmtime | klu | analytic | steps | 3.22 | 0.896 | 3595 | 4092 | 65 | 0 | 432 | 40 | 4031 | 0 | 3636 | 21 | 3596 |
| wasmtime | klu | difference | steps | 3.239 | 0.901 | 3595 | 4092 | 65 | 195 | 432 | 40 | 4031 | 0 | 3636 | 21 | 3596 |
| Node | dense | analytic | times | 1.504 | 0.419 | 3594 | 4091 | 65 | 0 | 432 | 40 | 4029 | 0 | 3834 | 21 | 221 |
| Node | dense | difference | times | 1.514 | 0.421 | 3594 | 4091 | 65 | 195 | 432 | 40 | 4029 | 0 | 3834 | 21 | 221 |
| Node | klu | analytic | times | 1.663 | 0.463 | 3594 | 4091 | 65 | 0 | 432 | 40 | 4029 | 0 | 3834 | 21 | 221 |
| Node | klu | difference | times | 1.706 | 0.475 | 3594 | 4091 | 65 | 195 | 432 | 40 | 4029 | 0 | 3834 | 21 | 221 |
| Node | dense | analytic | steps | 2.157 | 0.6 | 3595 | 4092 | 65 | 0 | 432 | 40 | 4031 | 0 | 3636 | 21 | 3596 |
| Node | dense | difference | steps | 2.123 | 0.591 | 3595 | 4092 | 65 | 195 | 432 | 40 | 4031 | 0 | 3636 | 21 | 3596 |
| Node | klu | analytic | steps | 2.284 | 0.635 | 3595 | 4092 | 65 | 0 | 432 | 40 | 4031 | 0 | 3636 | 21 | 3596 |
| Node | klu | difference | steps | 2.302 | 0.64 | 3595 | 4092 | 65 | 195 | 432 | 40 | 4031 | 0 | 3636 | 21 | 3596 |
| Chrome | dense | analytic | times | 1.49 | 0.415 | 3594 | 4091 | 65 | 0 | 432 | 40 | 4029 | 0 | 3834 | 21 | 221 |
| Chrome | dense | difference | times | 1.468 | 0.408 | 3594 | 4091 | 65 | 195 | 432 | 40 | 4029 | 0 | 3834 | 21 | 221 |
| Chrome | klu | analytic | times | 1.593 | 0.443 | 3594 | 4091 | 65 | 0 | 432 | 40 | 4029 | 0 | 3834 | 21 | 221 |
| Chrome | klu | difference | times | 1.635 | 0.455 | 3594 | 4091 | 65 | 195 | 432 | 40 | 4029 | 0 | 3834 | 21 | 221 |
| Chrome | dense | analytic | steps | 2.125 | 0.591 | 3595 | 4092 | 65 | 0 | 432 | 40 | 4031 | 0 | 3636 | 21 | 3596 |
| Chrome | dense | difference | steps | 2.09 | 0.581 | 3595 | 4092 | 65 | 195 | 432 | 40 | 4031 | 0 | 3636 | 21 | 3596 |
| Chrome | klu | analytic | steps | 2.305 | 0.641 | 3595 | 4092 | 65 | 0 | 432 | 40 | 4031 | 0 | 3636 | 21 | 3596 |
| Chrome | klu | difference | steps | 2.338 | 0.65 | 3595 | 4092 | 65 | 195 | 432 | 40 | 4031 | 0 | 3636 | 21 | 3596 |

#### mp_core

| host | solver | Jacobian | output | median ms | µs per step | steps | rhs | Jacobians | DQ rhs | lin setups | err test fails | nonlin iters | conv fails | roots | integrations | probe rhs |
|---|---|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| roadrunner dense | dense | difference_quotient | times | 134.4 | 9.37 | 14345 | 37690 | 4658 | 200294 | 11978 | 1871 | 23743 | 0 | - | 4649 | - |
| roadrunner 2.12.0 KLU | klu | sparse_difference_quotient | times | 7.093 | 3.12 | 2274 | 3755 | 175 | - | 1094 | 192 | 3269 | 0 | - | 162 | - |
| roadrunner KLU | klu | sparse_difference_quotient | times | 7.14 | 3.14 | 2274 | 3755 | 175 | - | 1094 | 192 | 3269 | 0 | - | 162 | - |
| wasmtime | dense | analytic | times | 3.312 | 3.32 | 999 | 1369 | 19 | 0 | 174 | 70 | 1366 | 1 | 0 | 1 | 0 |
| wasmtime | dense | difference | times | 3.409 | 3.21 | 1061 | 1404 | 19 | 209 | 172 | 60 | 1401 | 1 | 0 | 1 | 0 |
| wasmtime | klu | analytic | times | 2.348 | 2.4 | 979 | 1288 | 18 | 0 | 160 | 55 | 1285 | 1 | 0 | 1 | 0 |
| wasmtime | klu | difference | times | 2.556 | 2.53 | 1012 | 1397 | 19 | 209 | 182 | 68 | 1394 | 3 | 0 | 1 | 0 |
| Node | dense | analytic | times | 3.006 | 3.01 | 999 | 1369 | 19 | 0 | 174 | 70 | 1366 | 1 | 0 | 1 | 0 |
| Node | dense | difference | times | 3.037 | 2.86 | 1061 | 1404 | 19 | 209 | 172 | 60 | 1401 | 1 | 0 | 1 | 0 |
| Node | klu | analytic | times | 2.113 | 2.16 | 979 | 1288 | 18 | 0 | 160 | 55 | 1285 | 1 | 0 | 1 | 0 |
| Node | klu | difference | times | 2.297 | 2.27 | 1012 | 1397 | 19 | 209 | 182 | 68 | 1394 | 3 | 0 | 1 | 0 |
| Chrome | dense | analytic | times | 3.04 | 3.04 | 999 | 1369 | 19 | 0 | 174 | 70 | 1366 | 1 | 0 | 1 | 0 |
| Chrome | dense | difference | times | 3.093 | 2.91 | 1061 | 1404 | 19 | 209 | 172 | 60 | 1401 | 1 | 0 | 1 | 0 |
| Chrome | klu | analytic | times | 2.105 | 2.15 | 979 | 1288 | 18 | 0 | 160 | 55 | 1285 | 1 | 0 | 1 | 0 |
| Chrome | klu | difference | times | 2.293 | 2.27 | 1012 | 1397 | 19 | 209 | 182 | 68 | 1394 | 3 | 0 | 1 | 0 |

#### chen

| host | solver | Jacobian | output | median ms | µs per step | steps | rhs | Jacobians | DQ rhs | lin setups | err test fails | nonlin iters | conv fails | roots | integrations | probe rhs |
|---|---|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| roadrunner dense | dense | difference_quotient | times | 1127 | 401 | 2812 | 4383 | 86 | 43000 | 606 | 242 | 4373 | 48 | - | 5 | - |
| roadrunner 2.12.0 KLU | klu | sparse_difference_quotient | times | 458.1 | 173 | 2641 | 4048 | 88 | - | 578 | 221 | 4035 | 55 | - | 5 | - |
| roadrunner KLU | klu | sparse_difference_quotient | times | 460.5 | 174 | 2641 | 4048 | 88 | - | 578 | 221 | 4035 | 55 | - | 5 | - |
| wasmtime | dense | analytic | times | 1484 | 497 | 2988 | 4631 | 91 | 0 | 649 | 268 | 4630 | 47 | 0 | 1 | 0 |
| wasmtime | dense | difference | times | 1083 | 403 | 2691 | 4087 | 89 | 12816 | 563 | 215 | 4086 | 57 | 0 | 1 | 0 |
| wasmtime | klu | analytic | times | 553.2 | 191 | 2901 | 4507 | 86 | 0 | 648 | 280 | 4505 | 47 | 0 | 1 | 0 |
| wasmtime | klu | difference | times | 545.2 | 190 | 2867 | 4422 | 91 | 13104 | 613 | 246 | 4421 | 54 | 0 | 1 | 0 |
| Node | dense | analytic | times | 1323 | 443 | 2988 | 4631 | 91 | 0 | 649 | 268 | 4630 | 47 | 0 | 1 | 0 |
| Node | dense | difference | times | 984.9 | 366 | 2691 | 4087 | 89 | 12816 | 563 | 215 | 4086 | 57 | 0 | 1 | 0 |
| Node | klu | analytic | times | 513 | 177 | 2901 | 4507 | 86 | 0 | 648 | 280 | 4505 | 47 | 0 | 1 | 0 |
| Node | klu | difference | times | 500.2 | 174 | 2867 | 4422 | 91 | 13104 | 613 | 246 | 4421 | 54 | 0 | 1 | 0 |
| Chrome | dense | analytic | times | 1295 | 434 | 2988 | 4631 | 91 | 0 | 649 | 268 | 4630 | 47 | 0 | 1 | 0 |
| Chrome | dense | difference | times | 955.6 | 355 | 2691 | 4087 | 89 | 12816 | 563 | 215 | 4086 | 57 | 0 | 1 | 0 |
| Chrome | klu | analytic | times | 510.2 | 176 | 2901 | 4507 | 86 | 0 | 648 | 280 | 4505 | 47 | 0 | 1 | 0 |
| Chrome | klu | difference | times | 492 | 172 | 2867 | 4422 | 91 | 13104 | 613 | 246 | 4421 | 54 | 0 | 1 | 0 |

#### mp_flat

| host | solver | Jacobian | output | median ms | µs per step | steps | rhs | Jacobians | DQ rhs | lin setups | err test fails | nonlin iters | conv fails | roots | integrations | probe rhs |
|---|---|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| roadrunner dense | dense | difference_quotient | times | 2945 | 1430 | 2060 | 3054 | 40 | 36760 | 342 | 146 | 3051 | 13 | - | 1 | - |
| roadrunner 2.12.0 KLU | klu | sparse_difference_quotient | times | 84.98 | 36.9 | 2306 | 3177 | 46 | - | 334 | 135 | 3174 | 12 | - | 1 | - |
| roadrunner KLU | klu | sparse_difference_quotient | times | 84.44 | 36.6 | 2306 | 3177 | 46 | - | 334 | 135 | 3174 | 12 | - | 1 | - |
| wasmtime | dense | analytic | times | 4312 | 1540 | 2798 | 3780 | 51 | 0 | 344 | 135 | 3777 | 6 | 0 | 1 | 0 |
| wasmtime | dense | difference | times | 3723 | 1460 | 2555 | 3506 | 51 | 612 | 348 | 139 | 3503 | 13 | 0 | 1 | 0 |
| wasmtime | klu | analytic | times | 112.9 | 41.9 | 2694 | 3618 | 53 | 0 | 350 | 134 | 3615 | 15 | 0 | 1 | 0 |
| wasmtime | klu | difference | times | 120.4 | 42.5 | 2834 | 3719 | 54 | 648 | 353 | 130 | 3716 | 12 | 0 | 1 | 0 |
| Node | dense | analytic | times | 3827 | 1370 | 2798 | 3780 | 51 | 0 | 344 | 135 | 3777 | 6 | 0 | 1 | 0 |
| Node | dense | difference | times | 3274 | 1280 | 2555 | 3506 | 51 | 612 | 348 | 139 | 3503 | 13 | 0 | 1 | 0 |
| Node | klu | analytic | times | 111 | 41.2 | 2694 | 3618 | 53 | 0 | 350 | 134 | 3615 | 15 | 0 | 1 | 0 |
| Node | klu | difference | times | 118.4 | 41.8 | 2834 | 3719 | 54 | 648 | 353 | 130 | 3716 | 12 | 0 | 1 | 0 |
| Chrome | dense | analytic | times | 3734 | 1330 | 2798 | 3780 | 51 | 0 | 344 | 135 | 3777 | 6 | 0 | 1 | 0 |
| Chrome | dense | difference | times | 3213 | 1260 | 2555 | 3506 | 51 | 612 | 348 | 139 | 3503 | 13 | 0 | 1 | 0 |
| Chrome | klu | analytic | times | 109.6 | 40.7 | 2694 | 3618 | 53 | 0 | 350 | 134 | 3615 | 15 | 0 | 1 | 0 |
| Chrome | klu | difference | times | 116.9 | 41.3 | 2834 | 3719 | 54 | 648 | 353 | 130 | 3716 | 12 | 0 | 1 | 0 |

### Accuracy at the benchmark settings

The largest error against a reference of roadrunner dense at rtol 1e-12 and atol 1e-16 (scaled), in units of `atol_i + 1e-6 |x_ref|` (pointwise) / `atol_i + 1e-6 max_t |x_ref|` (scaled), as in the spike: the engine is in the accuracy class of roadrunner on every model.

| model | roadrunner dense | roadrunner KLU (2.12.0) | engine dense | engine KLU |
|---|---|---|---|---|
| hctz | 40.2 / 2.61 | 40.2 / 2.61 | 38 / 2.53 | 38 / 2.53 |
| midazolam | 100 / 3.03 | 100 / 3.03 | 100 / 3.03 | 100 / 3.03 |
| repeated_dose | 36.4 / 3.16 | 36.4 / 3.16 | 36.4 / 3.16 | 36.4 / 3.16 |
| mp_core | 2460 / 121 | 1550 / 66.8 | 182 / 6.58 | 222 / 4.56 |
| chen | 394 / 67 | 312 / 77.8 | 251 / 205 | 527 / 75.8 |
| mp_flat | 903 / 23.5 | 1530 / 188 | 2180 / 312 | 514 / 101 |

## KLU threshold

`LinearSolver::Auto` takes KLU above `KLU_THRESHOLD` states. Measured on two families of generated models (`scripts/scaling_models.py`, 201 points from 0 to 50, rtol 1e-6, atol 1e-12, the analytic Jacobian, 3 rounds of 20 after 3 warm ups): `chain_<n>`, a chain of units as the sinusoids of mp_flat (banded, about 3 entries per row), and `star_<n>`, a whole body model whose venous and arterial pools couple to every organ (an arrow head, as the PBPK models).

| model | states | entries | wasmtime dense ms | wasmtime KLU ms | KLU / dense | Node dense ms | Node KLU ms | KLU / dense |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| chain_4 | 4 | 11 | 0.171 | 0.184 | 1.07 | 0.158 | 0.165 | 1.05 |
| chain_8 | 8 | 24 | 0.249 | 0.27 | 1.08 | 0.21 | 0.221 | 1.05 |
| chain_12 | 12 | 37 | 0.225 | 0.236 | 1.05 | 0.195 | 0.203 | 1.04 |
| chain_16 | 16 | 50 | 0.302 | 0.298 | 0.99 | 0.259 | 0.269 | 1.04 |
| chain_20 | 20 | 63 | 0.349 | 0.339 | 0.97 | 0.299 | 0.29 | 0.97 |
| chain_24 | 24 | 76 | 0.363 | 0.34 | 0.93 | 0.328 | 0.309 | 0.94 |
| chain_28 | 28 | 89 | 0.551 | 0.492 | 0.89 | 0.484 | 0.428 | 0.88 |
| chain_32 | 32 | 102 | 0.599 | 0.515 | 0.86 | 0.546 | 0.453 | 0.83 |
| chain_40 | 40 | 128 | 0.622 | 0.489 | 0.79 | 0.575 | 0.457 | 0.80 |
| chain_48 | 48 | 154 | 0.718 | 0.532 | 0.74 | 0.624 | 0.472 | 0.76 |
| chain_64 | 64 | 206 | 1 | 0.647 | 0.65 | 0.899 | 0.567 | 0.63 |
| chain_80 | 80 | 258 | 1.5 | 0.819 | 0.55 | 1.38 | 0.754 | 0.55 |
| chain_96 | 96 | 310 | 2.11 | 1.04 | 0.49 | 1.94 | 0.916 | 0.47 |
| chain_128 | 128 | 414 | 3.63 | 1.4 | 0.39 | 3.25 | 1.23 | 0.38 |
| chain_192 | 192 | 622 | 6.17 | 1.76 | 0.28 | 5.79 | 1.63 | 0.28 |
| chain_256 | 256 | 830 | 13 | 2.88 | 0.22 | 12.6 | 2.7 | 0.21 |
| chain_384 | 384 | 1246 | 20.7 | 3.61 | 0.17 | 21 | 3.54 | 0.17 |
| chain_512 | 512 | 1662 | 54.4 | 6.17 | 0.11 | 54.8 | 6.08 | 0.11 |
| star_5 | 5 | 12 | 0.172 | 0.19 | 1.10 | 0.131 | 0.141 | 1.08 |
| star_8 | 8 | 20 | 0.224 | 0.235 | 1.05 | 0.171 | 0.171 | 1.00 |
| star_11 | 11 | 28 | 0.286 | 0.289 | 1.01 | 0.225 | 0.223 | 0.99 |
| star_14 | 14 | 36 | 0.341 | 0.325 | 0.95 | 0.268 | 0.251 | 0.94 |
| star_17 | 17 | 44 | 0.354 | 0.338 | 0.95 | 0.309 | 0.281 | 0.91 |
| star_20 | 20 | 52 | 0.396 | 0.36 | 0.91 | 0.342 | 0.302 | 0.88 |
| star_23 | 23 | 60 | 0.455 | 0.413 | 0.91 | 0.407 | 0.348 | 0.86 |
| star_26 | 26 | 68 | 0.519 | 0.438 | 0.84 | 0.451 | 0.373 | 0.83 |
| star_32 | 32 | 84 | 0.619 | 0.482 | 0.78 | 0.535 | 0.424 | 0.79 |
| star_38 | 38 | 100 | 0.74 | 0.547 | 0.74 | 0.642 | 0.482 | 0.75 |
| star_44 | 44 | 116 | 0.884 | 0.621 | 0.70 | 0.748 | 0.545 | 0.73 |
| star_50 | 50 | 132 | 1.08 | 0.705 | 0.65 | 0.968 | 0.653 | 0.67 |
| star_62 | 62 | 164 | 1.5 | 0.88 | 0.59 | 1.32 | 0.802 | 0.61 |
| star_74 | 74 | 196 | 2.05 | 1.09 | 0.53 | 1.71 | 0.946 | 0.55 |
| star_98 | 98 | 260 | 3.53 | 1.59 | 0.45 | 2.87 | 1.39 | 0.48 |
| star_128 | 128 | 340 | 5.83 | 2.23 | 0.38 | 4.78 | 2.04 | 0.43 |
| star_194 | 194 | 516 | 15.7 | 4.9 | 0.31 | 13.4 | 4.49 | 0.34 |
| star_254 | 254 | 676 | 28.1 | 8.13 | 0.29 | 24.4 | 7.46 | 0.31 |
| star_386 | 386 | 1028 | 88.3 | 21.8 | 0.25 | 77.9 | 20.3 | 0.26 |
| star_512 | 512 | 1364 | 190 | 45.1 | 0.24 | 168 | 40.7 | 0.24 |

The crossover is at 11 to 14 states for the arrow head and 16 to 20 for the chain, in both hosts; the real models agree (repeated_dose with 4 states dense 0.89 of KLU, hctz 28, midazolam 38 and mp_core 43 states KLU 0.70 to 0.90 of dense). `KLU_THRESHOLD = 16`: every model above 16 states measured is faster with KLU in both hosts, at 16 the two are within 1 % to 4 % (chain_16) and below it the dense solver is faster, but for star_14 (KLU 5 % to 6 % faster, not taken). With the threshold of 64 the models of 17 to 64 states ran dense, where KLU takes 0.63 to 0.97 of the time. With 16, about 8 cases of the SBML Test Suite (17 to 104 states) now take KLU; the suite gives 1581 pass and no difference to the baseline with the repeat checks under `auto`, `--jacobian difference` and `--no-optimize`.

## The probe of the triggers (carried from Task 13)

After a stop of the integration with a trigger which is false, the engine evaluates the triggers right after the time (`PROBE_RTOL` times the scale of the times, the state moved along its rate), one call of the right hand side per stop (`probe_rhs_evals`): per output time (201 outputs and 20 events: 221) and, for the output of the steps of the integrator, per step (3596 calls against CVODE's 4092). Measured directly on `repeated_dose` in wasmtime with a throwaway build whose probe skips the right hand side (identical output, the trigger `time >= tnext` reads no state; 3 rounds of 20 each, alternating):

| output | solver | with the probe ms | without ms | cost |
|---|---|---:|---:|---:|
| times | dense | 2.245 | 2.176 | 3 % |
| times | KLU | 2.408 | 2.402 | 0 % |
| steps | dense | 2.871 | 2.835 | 1 % |
| steps | KLU | 3.189 | 3.037 | 5 % |

The cost is one right hand side per step, which on this model of 4 states is about 0.01 to 0.04 µs against 0.8 µs per step. On a larger model with events it scales with the share of the right hand side in a step: CVODE evaluates 1.1 to 1.4 right hand sides per step (hctz 1.1, Chen 1.6 with its difference quotient), so the probe adds 65 % to 90 % to the calls of the right hand side of the output of the steps of the integrator of a model with a false trigger, and its cost is that share of the time of the right hand side (an upper bound for a model whose step is dominated by the right hand side, small where the linear solver dominates). The output of times costs one right hand side per output time and event (221 here, negligible). The output of the steps of the integrator is still 0.4 to 0.6 of roadrunner's time on this model.

## Agreement with roadrunner

`scripts/agreement.py`: the packaged models of sbmlsim (`HCTZ_SBML`, `MIDAZOLAM_SBML`, `REPRESSILATOR_SBML`, `DEMO_SBML`), the machine perfusion models and, beyond the brief, the event model, prepared with `scripts/prepare_model.py`, simulated with sbmlsim's `Simulator` (roadrunner 2.12.0, its defaults KLU and the sparse difference quotient, sbmlsim's pre-initialization, which the engine follows) and with `sbmlsim-core simulate` (`auto`: KLU above 16 states), both at rtol 1e-10 and atol 1e-12 scaled by sbmlsim per state (the engine gets the very vector), 201 uniform times, every column both have compared with `DataSetsComparison` (roadrunner the reference) with `tol_rel = 1e-7` and `tol_abs = 1e-12` (the absolute tolerance of the integrator: a value at that level is resolved by neither integrator). `--convergence` also simulates both at rtol 1e-12 and atol 1e-14 and compares each with roadrunner at 1e-12, which separates the integration error from a difference of the models.

| model | columns | result | max relative (all values) | max relative (values above 1e-9) | max of \|c - u\| / (1e-12 + 1e-7 \|c\|) | failing values | roadrunner 1e-10 against roadrunner 1e-12: failing / max excess | sbmlsim-core 1e-10 against roadrunner 1e-12 | sbmlsim-core 1e-12 against roadrunner 1e-12 |
|---|---:|---|---:|---:|---:|---:|---:|---:|---:|
| hctz | 191 | FAIL | 2 (GU__application_hctz) | 7.66e-07 (transport_lu_hctz) | 2.24 | 4 | 4 / 1.78 | 0 / 0.467 | 0 / 0.00439 |
| midazolam | 268 | pass | 0.000196 (dissolution_mid) | 0.000139 (GUTDOSE_mid) | 0.947 | 0 | 307 / 9.25 | 307 / 9.25 | 0 / 0.0038 |
| repressilator | 42 | pass | 5.44e-11 (Reaction10) | 5.44e-11 (Reaction10) | 0.000544 | 0 | 0 / 0.125 | 0 / 0.126 | 0 / 0.0012 |
| demo | 36 | pass | 0 (time) | 0 (time) | 0 | 0 | 0 / 0 | 0 / 0 | 0 / 0 |
| mp_core | 237 | FAIL | 2 (BUF_ext) | 2 (LACEX) | 3530 | 1509 | 1704 / 2440 | 764 / 1780 | 221 / 40.7 |
| mp_flat | 3299 | FAIL | 2 (BUF_in) | 4.35e-05 ([nadp_z20_cell]) | 7.28 | 56 | 122 / 7.21 | 101 / 5.23 | 0 / 0.125 |
| repeated_dose | 26 | pass | 5.33e-09 (absorption) | 5.33e-09 (absorption) | 0.0533 | 0 | 1 / 1.74 | 1 / 1.74 | 0 / 0.000227 |

The largest relative difference over all values is 2 where one result is 0 or below the absolute tolerance and the other is not (e.g. a rate of 1e-23 against an exact 0).

- hctz: 4 values of transport rates (e.g. `transport_lu_hctz` -3.1166e-8, relative 1.5e-7) differ by up to 2.2 times the criterion. Roadrunner at 1e-10 is farther from roadrunner at 1e-12 (4 failing values, 1.78 times) than the engine at 1e-10 is (0 failing, 0.47 times), and at 1e-12 the two agree to 3.8e-9. The difference is roadrunner's integration error at rtol 1e-10. (With `KLU_THRESHOLD = 64` hctz used the dense solver and passed with 0.14 times the criterion; with KLU the steps of the engine differ.)
- mp_flat: the 56 failing values are states and rates of 1e-4 and below whose integration error at rtol 1e-10 exceeds 1e-7 relative (`[fa_z1_cell]` 2.2e-4, relative 7.6e-7; `BUF_out`, the net rate of a fast equilibrium, h·hco3/Ka - co2, of 1e-10 against its terms). Roadrunner at 1e-10 fails the same criterion against itself at 1e-12 (122 values, 7.2 times); at 1e-12 the engine passes against roadrunner (0 failing, 0.125 times the criterion). The term order of the rates and the libm of wasm are not the cause: they would not vanish at 1e-12.
- mp_core: the oxygen of the cell is 0 within 1e-29 (roadrunner -3.6e-221, the engine -8.2e-29) for the whole perfusion, exactly at the threshold of `s_hyp = piecewise(1, o2_cell <= 0, K^n / (K^n + o2_cell^n))`, and the ATP of the cell falls to 1e-13 at the threshold of `s_atp`. Roadrunner finds the roots of these conditions and restarts CVODE 161 times (dense: 4648 times); the engine does not locate the roots of piecewise conditions, the expressions are continuous there. The solution is ill-conditioned at that point: roadrunner's own solutions at 1e-10 and 1e-12 differ in 1704 values by up to 2440 times the criterion, more than the engine and roadrunner at 1e-10 (1509 values, 3530 times). At 1e-12, 221 values of 16 states and their rates fail (the necrosis and the enzymes released by it, `dmg`, `alt_dead`, `ldh_ext`, `k_dead`, with 1.8e-7 relative, and the exchange of lactate `LACEX`, a rate crossing 0 at 1e-10 to 1e-12). The agreement of 1e-7 is not reachable for this model at these tolerances by either integrator; whether the engine should find the roots of piecewise conditions as roadrunner does is a question for the owner (it would add restarts and cost: roadrunner dense takes 134 ms for 4648 restarts against 3.3 ms of the engine).

Exit code of the script: 1 (three models fail the strict criterion), with the evidence above.
