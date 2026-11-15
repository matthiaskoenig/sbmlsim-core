# Benchmark and agreement with roadrunner

The threshold of the project: at most 2x the simulation time of roadrunner native (LLJIT `O2`), dense and with KLU, in wasmtime and in Chrome. This file reports the measurement on the models of `bench/models.toml`, the choice of `KLU_THRESHOLD` and the agreement with roadrunner at tight tolerances. The method is the one of the spike (`spike/RESULTS.md`), so the numbers are comparable.

## Summary

- The performance round (below, "Performance round"): every model of the benchmark is now faster than roadrunner native in Node and Chrome but Chen and mp_flat dense (1.10 to 1.14) and the event model (0.93 to 1.04); the largest ratio of any host is 1.38 (`repeated_dose`, dense, wasmtime; was 1.60). The PBPK models take 0.45 to 0.61 of roadrunner's time natively, 0.57 to 0.83 in wasmtime, Node and Chrome; Chen with KLU 0.16 to 0.19 (KLU in the ordering AMD). The native engine (`sbmlsim-core bench --host native`) is a new column of every table. The sections after "Performance round" are the measurements before it.

- No ratio is above 2x. The largest is 1.56 (the event model `repeated_dose`, dense, output of 201 times, wasmtime; Node and Chrome 1.05) (1.60 in the rerun after the bound of events which accumulate, below). On the large models the ratio is 1.04 to 1.45 (Chen dense 1.25 to 1.42, KLU 1.04 to 1.16; mp_flat dense 1.27 to 1.45, KLU 1.36 to 1.39), on the PBPK models 0.80 to 0.94, on mp_core 0.91 to 1.04 (dense) and 1.13 to 1.30 (KLU). The engine finds the roots of the conditions of `piecewise` and restarts CVODE there as roadrunner does, with roadrunner's guard against roots which chatter (below), so mp_core and Chen now do roadrunner's work (Chen 4 restarts in both, mp_core 158 against 161 with KLU and 6456 against 4648 dense).
- Chrome (headless Chrome 157 in a module worker, pinned to the performance cores) measured directly; Node and Chrome agree within a few percent and are 2 % to 13 % faster than wasmtime on the large models.
- The counters of the integrator are the same in wasmtime, Node and Chrome for every configuration (one `engine.wasm`, the same bits), and every timed simulation repeated the bits of the first.
- `KLU_THRESHOLD` was 16 (was 64): KLU with the analytic Jacobian is as fast as the dense solver at 11 to 20 states and faster above (0.65 of the dense time at 64 states). Since the performance round it is 12.
- Agreement (the engine at rtol 1e-10 against roadrunner at rtol 1e-12, within 1e-12 + 1e-7 |c|, or its largest excess at most 1 % above the one of roadrunner at rtol 1e-10 with the same linear solver): every model passes with the dense solver and with KLU, mp_core since the root finding of the conditions of `piecewise` (dense 371.5 against roadrunner's 383.1, was 1401). No engine or compiler defect was found.

## Performance round

A round of optimizations of the integrator and of the code of the models, measured on every host: roadrunner native, the native engine, wasmtime, Node and Chrome. The method is the one of the tables below (BDF, rtol 1e-6, atol 1e-12 scaled per state as roadrunner does, 201 points, the median of 20 runs after 3 warm ups, 3 rounds, the configurations taking turns; one performance core, `taskset -c 2`, Chrome on 4-15), the analytic Jacobian. "before" is the commit before the round (bc3dae2), "after" the round; roadrunner is libroadrunner 2.12.0.dev683 dense and the `roadrunner-sparsedq` build (KLU), measured in the same session.

### Profile

`perf` is not available on the machine (`perf_event_paranoid` 4), so the profile is V8's sampling profiler (`node --cpu-prof`, 50 µs) on `engine.wasm`, `mathlib.wasm` and `model.wasm`, whose functions it names (the name sections of the engine and of mathlib). Before the round:

| model | where the time went |
|---|---|
| hctz (KLU, 201 points) | `libm::pow` (mathlib, the FreeBSD `e_pow.c` of the `libm` crate) 35 %, of it 16 % in `model_rhs`, 14 % in `model_selections`, 9 % in `model_assign`; the rows of the output 19 % (`model_selections` 11 %, `model_assign` 7 %); `N_VLinearSum` 12 %; KLU solve 13 % |
| hctz, 7 daily doses (changes), 2001 points | `pow` 39 %, the rows of the output 29 % |
| repeated_dose (dense) | the integrator itself: `N_VLinearSum` 23 %, `CVode` 20 %, dense solve 10 % |
| mp_core (KLU, 159 restarts) | KLU 30 % (its symbolic analysis 8 %, redone at every restart), `pow` 12 % |
| Chen (KLU) | `klu_l_refactor` 74 %: COLAMD, the ordering `SUNLinSol_KLU` sets, fills the factors of the Jacobian |

After it, hctz with 7 daily doses: `N_VLinearSum` 18 %, KLU solve 20 %, `CVode` 8 %, the right hand side 7 %, the rows 8 %, `pow` 6 % (and 3 % in the C `pow` of CVODE's step sizes). What is left is the work of CVODE itself.

### Changes

| change | effect |
|---|---|
| **Constants of the model** (`model/2`, `sbmlsim_ir::optimize::Constants`): the largest subtrees of the math which read only parameters which no assignment rule assigns (and the rules of such parameters: `pow(BW, 0.75)`, a body surface area) are computed by `model_init` and `model_assign` into `p` behind the parameters, and `model_rhs`, `model_jac`, `model_roots`, the functions of the events and `model_selections` read them. The engine calls `model_assign` after every change of a parameter (a change, an event). | the bits of the code as written (the SBML Test Suite with and without the optimizer, `optimized_bits_equal`; a test of changes and events, `the_constants_follow_the_changes_and_the_events`); hctz 100 constants, midazolam 138, mp_flat 1290 |
| **The output**: a row no longer calls `model_assign` (the selections compute the rules themselves) nor copies the state and the parameters; `model_selections` takes a mask and computes a selection which is more than a load or two (a rate of a reaction) only when a column asks for it. | the rows of hctz from 29 % to 8 % of the time |
| **`pow`, `exp`, `log` of ARM optimized-routines** (the code of glibc and musl), ported to Rust in mathlib without fused multiply add, so native and wasm give the same bits | `pow` 38.4 to 11.0 ns in wasm (Node), `exp` 11.1 to 1.9 ns, `log` 4.2 to 2.3 ns; 99.9 % of the results bitwise those of glibc, the others within 1 ulp; the results of the models change in the last bits |
| **KLU in the ordering AMD** (KLU's own default; `SUNLinSol_KLU` sets COLAMD) | Chen KLU 6.0x faster in wasmtime (534 to 89 ms), mp_flat 1.47x |
| **The symbolic analysis of KLU kept across the restarts of CVODE** (`sbmlsim_SUNLinSol_KLUKept`, `crates/csolvers/csrc`; analyzed again only when the pattern changes) | the bits as before; mp_core KLU -10 % |
| **32-bit indices** in SUNDIALS and KLU (`SUNDIALS_INT32_T`, the `int` KLU), a model whose sizes do not fit refused | the bits as before; -3 % to -6 % with KLU |
| **`KLU_THRESHOLD` 12** (was 16), measured again on the scaling models with the KLU above: KLU as fast as dense at 11 to 12 states, 0.9 of the dense time at 14 to 28 | models of 13 to 16 states take KLU |

### Ratios against roadrunner, after

`uv run scripts/roadrunner_reference.py bench/models.toml --compare roadrunner.json native.json engine.json node.json chrome.json`:

| model | states | solver | output | roadrunner ms | native ms | ratio | wasmtime ms | ratio | Node ms | ratio | Chrome ms | ratio |
|---|---:|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| hctz | 28 | dense | times | 1.12 | 0.546 | 0.49 | 0.712 | 0.64 | 0.676 | 0.61 | 0.67 | 0.60 |
| hctz | 28 | klu | times | 0.927 | 0.418 | 0.45 | 0.617 | 0.67 | 0.542 | 0.58 | 0.532 | 0.57 |
| midazolam | 38 | dense | times | 0.788 | 0.482 | 0.61 | 0.656 | 0.83 | 0.599 | 0.76 | 0.59 | 0.75 |
| midazolam | 38 | klu | times | 0.613 | 0.317 | 0.52 | 0.468 | 0.76 | 0.418 | 0.68 | 0.423 | 0.69 |
| repeated_dose | 4 | dense | times | 1.45 | 1.39 | 0.96 | 2.01 | 1.38 | 1.51 | 1.04 | 1.47 | 1.01 |
| repeated_dose | 4 | klu | times | 1.73 | 1.42 | 0.82 | 2.17 | 1.26 | 1.63 | 0.95 | 1.61 | 0.93 |
| repeated_dose | 4 | dense | steps | 5.13 | 2.1 | 0.41 | 2.47 | 0.48 | 1.95 | 0.38 | 1.98 | 0.39 |
| repeated_dose | 4 | klu | steps | 5.66 | 2.13 | 0.38 | 2.76 | 0.49 | 2.09 | 0.37 | 2.15 | 0.38 |
| mp_core | 43 | dense | times | 136 | 104 | 0.77 | 127 | 0.94 | 109 | 0.80 | 107 | 0.79 |
| mp_core | 43 | klu | times | 7.27 | 5.28 | 0.73 | 7.21 | 0.99 | 6.31 | 0.87 | 6.29 | 0.86 |
| chen | 500 | dense | times | 1132 | 1050 | 0.93 | 1412 | 1.25 | 1274 | 1.13 | 1245 | 1.10 |
| chen | 500 | klu | times | 464 | 72.4 | 0.16 | 89.2 | 0.19 | 75.9 | 0.16 | 75.6 | 0.16 |
| mp_flat | 919 | dense | times | 3009 | 2896 | 0.96 | 3948 | 1.31 | 3424 | 1.14 | 3307 | 1.10 |
| mp_flat | 919 | klu | times | 84.6 | 70.2 | 0.83 | 81.9 | 0.97 | 79.3 | 0.94 | 78.9 | 0.93 |

The native engine (`Runtime::instantiate_native`: `sbmlsim-engine` built for the machine with the CVODE and KLU of the C compiler of the host, the model in wasmtime) is the bound of the integrator without WebAssembly; the hosts of `engine.wasm` take 1.0 to 1.5 of its time. Its results are not bitwise those of `engine.wasm` (the C library of the host in the step sizes of CVODE), within the tolerances (`crates/host/tests/native.rs`).

### Before and after

ms, the median as above:

| model | solver | output | wasmtime before | after | speedup | Node before | after | speedup | Chrome before | after | speedup |
|---|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| hctz | dense | times | 0.994 | 0.712 | 1.40 | 0.967 | 0.676 | 1.43 | 0.948 | 0.670 | 1.41 |
| hctz | klu | times | 0.854 | 0.617 | 1.38 | 0.859 | 0.542 | 1.59 | 0.855 | 0.532 | 1.61 |
| midazolam | dense | times | 0.692 | 0.656 | 1.06 | 0.679 | 0.599 | 1.13 | 0.648 | 0.590 | 1.10 |
| midazolam | klu | times | 0.532 | 0.468 | 1.14 | 0.523 | 0.418 | 1.25 | 0.513 | 0.423 | 1.21 |
| repeated_dose | dense | times | 2.31 | 2.01 | 1.15 | 1.51 | 1.51 | 1.00 | 1.52 | 1.47 | 1.03 |
| repeated_dose | klu | times | 2.49 | 2.17 | 1.15 | 1.66 | 1.63 | 1.02 | 1.67 | 1.61 | 1.04 |
| repeated_dose | dense | steps | 2.79 | 2.47 | 1.13 | 2.06 | 1.95 | 1.05 | 2.07 | 1.98 | 1.05 |
| repeated_dose | klu | steps | 3.01 | 2.76 | 1.09 | 2.21 | 2.09 | 1.06 | 2.30 | 2.15 | 1.06 |
| mp_core | dense | times | 141 | 127 | 1.11 | 129 | 109 | 1.19 | 127 | 107 | 1.18 |
| mp_core | klu | times | 9.40 | 7.21 | 1.30 | 8.65 | 6.31 | 1.37 | 8.46 | 6.29 | 1.35 |
| chen | dense | times | 1526 | 1412 | 1.08 | 1382 | 1274 | 1.08 | 1345 | 1245 | 1.08 |
| chen | klu | times | 534 | 89.2 | 5.98 | 493 | 75.9 | 6.50 | 504 | 75.6 | 6.67 |
| mp_flat | dense | times | 4635 | 3948 | 1.17 | 3948 | 3424 | 1.15 | 3923 | 3307 | 1.19 |
| mp_flat | klu | times | 121 | 81.9 | 1.47 | 115 | 79.3 | 1.45 | 116 | 78.9 | 1.48 |

The steps differ in the last bits of `pow`, `exp` and `log` and with the ordering of KLU (Chen KLU 2853 to 2675, mp_flat KLU 2694 to 2585, dense 2798 to 2011); the PBPK models and the event model take the same steps. A PBPK simulation of the kind of the web apps, hctz and midazolam with 7 daily oral doses as changes of the request (2001 points over a week, KLU), in Node: hctz 5.27 to 3.04 ms (1.73x), midazolam 3.80 to 3.06 ms (1.24x).

The compilation of a model takes 5 % to 15 % longer and its `model.wasm` is 10 % to 23 % larger (the constants in `model_init` and `model_assign`, the blocks of the mask); the instantiation in wasmtime (Cranelift) 6 % to 35 % longer (Chen 127 to 172 ms), in V8 (lazy) not. A block of the mask around every selection made the compilation of mp_flat by Cranelift 5x slower (0.75 against 0.16 s), so only the selections which are more than a load or two have one.

### Linear algebra

Libraries of the kind of LAPACK or BLAS (in C or Rust) do not make the linear algebra of these models faster. Measured on the Newton matrix `I - gamma J` of every model at the state of the middle of its simulation (ns per factorization of new values and per solve, the best of 5; `faer` 0.24, a pure Rust library of dense and sparse linear algebra with SIMD kernels, which builds for wasm32 with `simd128`):

| model | n | entries | KLU (AMD) factor / solve | SUNDIALS dense | faer dense LU | faer sparse LU |
|---|---:|---:|---|---|---|---|
| **native** | | | | | | |
| hctz | 28 | 88 | 301 / 151 | 634 / 341 | 1916 / 465 | 2097 / 141 |
| midazolam | 38 | 97 | 390 / 157 | 1097 / 527 | 3473 / 704 | 2232 / 144 |
| mp_core | 43 | 190 | 750 / 246 | 1857 / 661 | 4487 / 787 | 3697 / 208 |
| chen | 500 | 4400 | 38177 / 4864 | 1156641 / 48606 | 1741372 / 35024 | 2080389 / 16468 |
| mp_flat | 919 | 4262 | 24328 / 6581 | 5917336 / 171271 | 9979774 / 132315 | 241520 / 6466 |
| **wasm (wasmtime)** | | | | | | |
| hctz | 28 | 88 | 494 / 192 | 977 / 392 | 3874 / 576 | 3120 / 227 |
| mp_core | 43 | 190 | 1124 / 320 | 2905 / 725 | 11873 / 1042 | 5482 / 352 |
| chen | 500 | 4400 | 65078 / 5692 | 1651912 / 73161 | 9244396 / 68299 | 2614927 / 26138 |
| mp_flat | 919 | 4262 | 35646 / 7981 | 8367478 / 232023 | 55024493 / 215544 | 312413 / 9600 |

The matrices are small (PBPK) or very sparse (Chen, mp_flat): a factorization of a PBPK model takes 0.3 to 1.1 µs and a solve 0.15 to 0.3 µs with KLU, and the factors of KLU in the ordering AMD of the large models are 30 to 250 times faster than a dense LU, which a blocked LU with optimized kernels (LAPACK's `getrf` over an optimized BLAS) speeds up by a constant factor of a few at most (and an optimized BLAS is assembly for the CPU, which wasm does not run). The linear algebra was the ordering of KLU (above).

### Tried and not taken

- The fused vector operations of SUNDIALS (`N_VEnableFusedOps_Serial`): the same steps, no difference of the time (hctz 0.40 to 0.42 against 0.42 ms, repeated_dose 1.34 to 1.36 against 1.37 to 1.39 ms natively).
- The discontinuities of the time (a `piecewise` or a trigger of the time alone) as stop times of CVODE instead of root functions: the root functions take 1.5 % of the event model (`cvRcheck3`), Chen restarts 4 times; the restart at a discontinuity is needed in either case.
- One model with events instead of a request with changes: an event restarts CVODE as a change does; hctz with 7 daily doses as changes takes 342 steps per day against 458 of the single dose, so the restarts cost little (and less with the kept analysis of KLU).

## Setup

- Machine: Intel Core i7-12700 (8 performance cores with 2 threads, 4 efficiency cores; 20 logical CPUs), Linux 7.0, governor `powersave`, the same machine as the spike. The command line hosts are pinned with `taskset -c 2` (a performance core), Chrome with `taskset -c 4-15`.
- sbmlsim-core: rustc 1.100.0, wasi-sdk 34 (clang 23.1.0), SUNDIALS 7.9.0, SuiteSparse 7.14.1 (KLU 2.3.6), the C solvers with `-O3 -msimd128 -ffp-contract=off`; wasmtime 50.0.1 (the crate of `sbmlsim-host`, Cranelift, defaults); Node 24.21.0; Chrome 157.0.8103.78 headless (`--headless=new`), the page served cross-origin isolated (`bench/web/serve.mjs`) so `performance.now()` in the worker is fine grained, driven with `chrome-devtools-axi`.
- roadrunner: dense is libroadrunner 2.12.0.dev683 (the newest build of the index of the sbmlsim scripts, python 3.14), set explicitly to `linear_solver = dense`, `jacobian = difference_quotient` (CVODE's dense difference quotient); KLU is the `roadrunner-sparsedq` build of the spike (a local build of libroadrunner 2.11.1 with KLU and its colored sparse difference quotient, named by `SBMLSIM_CORE_SPARSEDQ_BUILD`; without it the script takes the KLU of libroadrunner 2.12 as the baseline); both LLJIT at `O2` (`DEFAULT`, as sbmlsim loads models). libroadrunner 2.12.0.dev683 has KLU with the sparse difference quotient as its default (`linear_solver = klu`, `jacobian = sparse_difference_quotient`); it is measured as a third roadrunner (`roadrunner 2.12.0 KLU` in the tables), within 0 to 8 % of the sparsedq build.
- Settings everywhere: BDF, relative tolerance 1e-6, absolute tolerance 1e-12 scaled per state as roadrunner applies it (the engine gets the very vector of roadrunner for the pre-initialized model, `<model>.atol.json` of the preparation, checked equal in every roadrunner run), at most 20000 steps, 201 uniform output points (roadrunner `simulate(0, end, 201)`, the engine `Output::Uniform { steps: 200 }`), the states as the selections, and for `repeated_dose` also the output of the steps of the integrator (roadrunner `variable_step_size`).
- A measurement is the median of 20 runs after 3 warm ups; the configurations take turns within a round, 3 rounds, the tables have the median of the round medians. The time of the engine is the wall time of `Instance::simulate` (wasmtime) or `instance.simulate` (Node, Chrome): the request to JSON, its parse in the engine, the simulation and the copy of the result. An instance is reused, so its CVODE memory is re-initialized and not created per run (the spike created it per run). roadrunner's time is `np.array(r.simulate(...))` after `resetAll`; the pre-initialization is set as initial values (`init(x)`) once, which `resetAll` keeps and evaluates the initial assignments with (the engine's and sbmlsim's semantics), and the state at the start is checked to be the engine's (see the agreement).
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
uv run scripts/agreement.py
```

`scripts/prepare_model.py` converts a model to L3V2, flattens `comp`, strips the elements and annotations of `distrib` (the uncertainties of the fitted parameters of the machine perfusion models, metadata) and refuses, with a non-zero exit naming the element, a model whose math uses a distribution function of `distrib` (checked: an initial assignment `normal(0, 1)` is refused as `initialAssignment of x (normal)`); the compiler keeps refusing required packages. `bench/models.toml` references the prepared files in the gitignored `target/bench-models/`, and `prepared.json` there records every source with its sha256 (and the commit of a checkout).

### Models

| model | source | states | Jacobian entries | events | end | pre-initialization |
|---|---|---:|---:|---:|---:|---|
| hctz | `sbmlsim:hctz_body_flat.xml` (sbmlsim 0.24.0) | 28 | 82 | 0 | 1440 min | `PODOSE_hctz = 25` |
| midazolam | `sbmlsim:midazolam_body_flat.xml` | 38 | 93 | 0 | 1440 min | `PODOSE_mid = 7.5` |
| repeated_dose | `bench/models/repeated_dose.xml` (an oral dose every 8 h, two compartments and a metabolite) | 4 | 8 | 1 (21 executions) | 168 h | |
| mp_core | `machine_perfusion_liver_core.xml` of pkdb_models 1a8d30a (comp) | 43 | 184 | 0 | 360 min | |
| chen | Chen_MSB2009 of the PEtab benchmark collection at c44d5ed (L2V4) | 500 | 4400 | 0 | 9000 s | `c1 = 5e-9` (condition model1_data1) |
| mp_flat | `machine_perfusion_liver_flat.xml` of pkdb_models 1a8d30a (distrib) | 919 | 4242 | 0 | 360 min | |

The other models have no events; `repeated_dose` measures the events and the probe of the triggers. Chen has `piecewise` functions of the time (a stimulation at 1800 s), whose roots roadrunner and the engine find (4 restarts each); mp_core has two `piecewise` of states at 0 (`o2_cell <= 0`, `atp_cell <= 0`), mp_flat 63 root functions of `piecewise` conditions whose signs do not change in the perfusion.

## Ratios against roadrunner

The analytic Jacobian of the engine; ms the median time of a simulation; ratio the time over roadrunner's (dense against roadrunner dense, KLU against roadrunner KLU). The rows of hctz, midazolam and mp_flat (every host and roadrunner) were measured again after the root finding of the conditions of `piecewise`, the rows of mp_core and Chen once more after its guard against roots which chatter; the rows of `repeated_dose`, which has no `piecewise`, are of the first measurement.

| model | states | solver | output | roadrunner ms | wasmtime ms | ratio | Node ms | ratio | Chrome ms | ratio |
|---|---:|---|---|---:|---:|---:|---:|---:|---:|---:|
| hctz | 28 | dense | times | 1.09 | 0.969 | 0.89 | 0.921 | 0.85 | 0.968 | 0.89 |
| hctz | 28 | klu | times | 0.926 | 0.851 | 0.92 | 0.807 | 0.87 | 0.868 | 0.94 |
| midazolam | 38 | dense | times | 0.771 | 0.686 | 0.89 | 0.641 | 0.83 | 0.625 | 0.81 |
| midazolam | 38 | klu | times | 0.612 | 0.531 | 0.87 | 0.487 | 0.80 | 0.49 | 0.80 |
| repeated_dose | 4 | dense | times | 1.43 | 2.22 | 1.56 | 1.5 | 1.05 | 1.49 | 1.04 |
| repeated_dose | 4 | klu | times | 1.68 | 2.49 | 1.49 | 1.66 | 0.99 | 1.59 | 0.95 |
| repeated_dose | 4 | dense | steps | 5.01 | 2.89 | 0.58 | 2.16 | 0.43 | 2.12 | 0.42 |
| repeated_dose | 4 | klu | steps | 5.47 | 3.22 | 0.59 | 2.28 | 0.42 | 2.31 | 0.42 |
| mp_core | 43 | dense | times | 134 | 140 | 1.04 | 127 | 0.95 | 122 | 0.91 |
| mp_core | 43 | klu | times | 7.18 | 9.29 | 1.30 | 8.36 | 1.17 | 8.08 | 1.13 |
| chen | 500 | dense | times | 1049 | 1492 | 1.42 | 1331 | 1.27 | 1313 | 1.25 |
| chen | 500 | klu | times | 455 | 530 | 1.16 | 472 | 1.04 | 493 | 1.08 |
| mp_flat | 919 | dense | times | 2967 | 4289 | 1.45 | 3841 | 1.29 | 3755 | 1.27 |
| mp_flat | 919 | klu | times | 83.4 | 116 | 1.39 | 113 | 1.36 | 114 | 1.37 |

Rerun after the changes of the event path (the budget of steps at an execution, the bound of chatter in the output of the steps, the delayed events a change triggers), the event model and mp_core, wasmtime, Node and roadrunner (not Chrome), the same counts of steps as before:

| model | solver | output | roadrunner ms | wasmtime ms | ratio | Node ms | ratio |
|---|---|---|---:|---:|---:|---:|---:|
| repeated_dose | dense | times | 1.45 | 2.32 | 1.60 | 1.53 | 1.06 |
| repeated_dose | klu | times | 1.68 | 2.46 | 1.47 | 1.63 | 0.97 |
| repeated_dose | dense | steps | 4.98 | 2.69 | 0.54 | 2.11 | 0.42 |
| repeated_dose | klu | steps | 5.64 | 2.96 | 0.53 | 2.15 | 0.38 |
| mp_core | dense | times | 135 | 139 | 1.03 | 129 | 0.95 |
| mp_core | klu | times | 7.22 | 9.17 | 1.27 | 8.24 | 1.14 |

The ratio of the event model in wasmtime, dense, moved from 1.56 to 1.60 (the engine 2.22 against 2.32 ms, roadrunner 1.43 against 1.45 ms; the steps, 3594, are the same), within the noise of the sessions; every ratio stays below 2x. The agreement (below) gives the same numbers after these changes.

Where the ratios above 1 come from:

- mp_flat and Chen: the work differs, the cost per step barely. The engine takes more steps than roadrunner (mp_flat KLU 2694 against 2306, dense 2798 against 2060; Chen KLU 2853 against 2641, dense 2930 against 2812). Per step the engine in wasmtime is at 1.06 of roadrunner on mp_flat dense (the dense LU of SUNDIALS on 919 x 919 in both), 1.19 on mp_flat KLU, 1.36 on Chen dense and 1.08 on Chen KLU; in Node and Chrome at 0.93 to 0.95, 1.16 to 1.17, 1.20 to 1.22 and 0.96 to 1.0. The step counts depend on the last bits of the right hand side and the Jacobian (the analytic Jacobian against roadrunner's difference quotients, the order of the terms), as in the spike (2184 / 2304 steps of the spike's engine against 2060 / 2306); the solutions are in the same accuracy class (below). Chen dense with the analytic Jacobian is slower than with the difference quotient (1498 against 1146 ms, 2930 against 2837 steps). roadrunner dense took 1047 ms on Chen in this session against 1127 ms in the first run (the engine 1498 against 1484), so the ratio of Chen dense in wasmtime moved from 1.32 to 1.42.
- mp_core: the oxygen of the cell sits at 0 within the absolute tolerance for the whole perfusion, exactly at the threshold of `s_hyp = piecewise(1, o2_cell <= 0, ...)`, and its sign changes with the noise of the integration; every change is a root and a restart, in roadrunner and in the engine (but the roots roadrunner's guard ignores, the third and later root within the relative tolerance, as a time, of the last restart in one output interval; the guard took the restarts of the engine dense from 6850 to 6456). With KLU both do nearly the same work (2313 against 2274 steps, 158 against 161 restarts), and the engine is at 1.27 of roadrunner per step in wasmtime (the root functions are evaluated after every step and in the root finding: 4605 calls). With the dense solver the engine restarts more often than roadrunner (6456 against 4648 restarts, 19368 against 14345 steps), and is at 0.77 of roadrunner per step (roadrunner's dense difference quotient costs 43 right hand sides per Jacobian, 4658 of them).
- repeated_dose in wasmtime: 3594 steps of a model of 4 states, 0.62 µs per step against 0.42 in Node and Chrome and 0.40 in roadrunner; the engine is bound by its own overhead per step (events, root functions, output) on such a small model, which Cranelift compiles less well than V8. The output of the steps of the integrator is 0.4 to 0.6 of roadrunner's.

## Compilation and instantiation

compile: SBML to `model.wasm` and `model.json` (`sbmlsim_compiler::compile`, the median of 5). instantiate: `Runtime::instantiate` in wasmtime (the compilation of `model.wasm` by Cranelift, the instances, the load of the description) and `runtime.instantiate` in Node and Chrome (V8 compiles lazily), the median of 5; `engine.wasm` and `mathlib.wasm` are compiled once per runtime. roadrunner's load is `RoadRunner(sbml, options)` (LLJIT `O2`).

| model | states | Jacobian entries | model.wasm kB | model.json kB | compile ms | instantiate wasmtime ms | instantiate Node ms | instantiate Chrome ms | roadrunner load s |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| hctz | 28 | 82 | 16.4 | 39 | 2.33 | 8.47 | 2.14 | 0.895 | 0.097 |
| midazolam | 38 | 93 | 21.1 | 54 | 2.18 | 10.2 | 1.59 | 0.6 | 0.14 |
| repeated_dose | 4 | 8 | 1.7 | 4 | 0.148 | 2.5 | 0.21 | 0.175 | 0.018 |
| mp_core | 43 | 184 | 20.9 | 50 | 4.16 | 10.9 | 0.604 | 0.54 | 0.072 |
| chen | 500 | 4400 | 216.5 | 375 | 46.5 | 122 | 3.68 | 2.9 | 0.91 |
| mp_flat | 919 | 4242 | 495.8 | 704 | 95.6 | 307 | 4.85 | 5.59 | 1.7 |

Compile and instantiate in wasmtime together are 4 to 11 times faster than roadrunner's load (mp_flat 0.40 s against 1.73 s); in V8 the instantiation is milliseconds, and the code is compiled lazily during the warm ups.

## Full tables

rhs: the calls of the right hand side by CVODE (roadrunner: `num_rhs_evals`); DQ rhs: those of a difference quotient (roadrunner dense: CVODE's `num_lin_rhs_evals`; roadrunner's sparse difference quotient is not counted); integrations: the (re)initializations of CVODE (roadrunner: its restarts at roots plus 1); roots: the calls of the root functions (of the triggers of the events and of the conditions of `piecewise`); probe rhs: the calls of the right hand side by the probe of the triggers (below). wasmtime, Node and Chrome have the same counters in every configuration.

#### hctz

| host | solver | Jacobian | output | median ms | µs per step | steps | rhs | Jacobians | DQ rhs | lin setups | err test fails | nonlin iters | conv fails | roots | integrations | probe rhs |
|---|---|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| roadrunner dense | dense | difference_quotient | times | 1.088 | 2.44 | 446 | 511 | 9 | 252 | 65 | 9 | 510 | 0 | - | 1 | - |
| roadrunner 2.12.0 KLU | klu | sparse_difference_quotient | times | 0.8518 | 1.91 | 446 | 511 | 9 | - | 65 | 9 | 510 | 0 | - | 1 | - |
| roadrunner KLU | klu | sparse_difference_quotient | times | 0.926 | 2.08 | 446 | 511 | 9 | - | 65 | 9 | 510 | 0 | - | 1 | - |
| wasmtime | dense | analytic | times | 0.969 | 2.12 | 458 | 518 | 8 | 0 | 62 | 8 | 517 | 0 | 0 | 1 | 0 |
| wasmtime | dense | difference | times | 0.9645 | 2.16 | 446 | 511 | 9 | 54 | 66 | 9 | 510 | 0 | 0 | 1 | 0 |
| wasmtime | klu | analytic | times | 0.8512 | 1.86 | 458 | 518 | 8 | 0 | 62 | 8 | 517 | 0 | 0 | 1 | 0 |
| wasmtime | klu | difference | times | 0.8721 | 1.96 | 446 | 511 | 9 | 54 | 66 | 9 | 510 | 0 | 0 | 1 | 0 |
| Node | dense | analytic | times | 0.9208 | 2.01 | 458 | 518 | 8 | 0 | 62 | 8 | 517 | 0 | 0 | 1 | 0 |
| Node | dense | difference | times | 0.9299 | 2.08 | 446 | 511 | 9 | 54 | 66 | 9 | 510 | 0 | 0 | 1 | 0 |
| Node | klu | analytic | times | 0.8067 | 1.76 | 458 | 518 | 8 | 0 | 62 | 8 | 517 | 0 | 0 | 1 | 0 |
| Node | klu | difference | times | 0.8273 | 1.85 | 446 | 511 | 9 | 54 | 66 | 9 | 510 | 0 | 0 | 1 | 0 |
| Chrome | dense | analytic | times | 0.9675 | 2.11 | 458 | 518 | 8 | 0 | 62 | 8 | 517 | 0 | 0 | 1 | 0 |
| Chrome | dense | difference | times | 0.935 | 2.1 | 446 | 511 | 9 | 54 | 66 | 9 | 510 | 0 | 0 | 1 | 0 |
| Chrome | klu | analytic | times | 0.8675 | 1.89 | 458 | 518 | 8 | 0 | 62 | 8 | 517 | 0 | 0 | 1 | 0 |
| Chrome | klu | difference | times | 0.845 | 1.89 | 446 | 511 | 9 | 54 | 66 | 9 | 510 | 0 | 0 | 1 | 0 |

#### midazolam

| host | solver | Jacobian | output | median ms | µs per step | steps | rhs | Jacobians | DQ rhs | lin setups | err test fails | nonlin iters | conv fails | roots | integrations | probe rhs |
|---|---|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| roadrunner dense | dense | difference_quotient | times | 0.7709 | 2.59 | 298 | 330 | 6 | 228 | 38 | 1 | 327 | 0 | - | 1 | - |
| roadrunner 2.12.0 KLU | klu | sparse_difference_quotient | times | 0.5793 | 1.94 | 298 | 330 | 6 | - | 38 | 1 | 327 | 0 | - | 1 | - |
| roadrunner KLU | klu | sparse_difference_quotient | times | 0.6121 | 2.05 | 298 | 330 | 6 | - | 38 | 1 | 327 | 0 | - | 1 | - |
| wasmtime | dense | analytic | times | 0.6865 | 2.3 | 298 | 329 | 6 | 0 | 39 | 1 | 326 | 0 | 0 | 1 | 0 |
| wasmtime | dense | difference | times | 0.6898 | 2.31 | 298 | 329 | 6 | 42 | 39 | 1 | 326 | 0 | 0 | 1 | 0 |
| wasmtime | klu | analytic | times | 0.5312 | 1.78 | 298 | 329 | 6 | 0 | 39 | 1 | 326 | 0 | 0 | 1 | 0 |
| wasmtime | klu | difference | times | 0.5329 | 1.79 | 298 | 329 | 6 | 42 | 39 | 1 | 326 | 0 | 0 | 1 | 0 |
| Node | dense | analytic | times | 0.6415 | 2.15 | 298 | 329 | 6 | 0 | 39 | 1 | 326 | 0 | 0 | 1 | 0 |
| Node | dense | difference | times | 0.6254 | 2.1 | 298 | 329 | 6 | 42 | 39 | 1 | 326 | 0 | 0 | 1 | 0 |
| Node | klu | analytic | times | 0.4872 | 1.63 | 298 | 329 | 6 | 0 | 39 | 1 | 326 | 0 | 0 | 1 | 0 |
| Node | klu | difference | times | 0.4874 | 1.64 | 298 | 329 | 6 | 42 | 39 | 1 | 326 | 0 | 0 | 1 | 0 |
| Chrome | dense | analytic | times | 0.625 | 2.1 | 298 | 329 | 6 | 0 | 39 | 1 | 326 | 0 | 0 | 1 | 0 |
| Chrome | dense | difference | times | 0.62 | 2.08 | 298 | 329 | 6 | 42 | 39 | 1 | 326 | 0 | 0 | 1 | 0 |
| Chrome | klu | analytic | times | 0.49 | 1.64 | 298 | 329 | 6 | 0 | 39 | 1 | 326 | 0 | 0 | 1 | 0 |
| Chrome | klu | difference | times | 0.485 | 1.63 | 298 | 329 | 6 | 42 | 39 | 1 | 326 | 0 | 0 | 1 | 0 |

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
| roadrunner dense | dense | difference_quotient | times | 134.1 | 9.35 | 14345 | 37690 | 4658 | 200294 | 11978 | 1871 | 23743 | 0 | - | 4649 | - |
| roadrunner 2.12.0 KLU | klu | sparse_difference_quotient | times | 6.97 | 3.07 | 2274 | 3755 | 175 | - | 1094 | 192 | 3269 | 0 | - | 162 | - |
| roadrunner KLU | klu | sparse_difference_quotient | times | 7.176 | 3.16 | 2274 | 3755 | 175 | - | 1094 | 192 | 3269 | 0 | - | 162 | - |
| wasmtime | dense | analytic | times | 139.9 | 7.22 | 19368 | 51021 | 6464 | 0 | 15549 | 2353 | 31623 | 0 | 88361 | 6457 | 0 |
| wasmtime | dense | difference | times | 125.4 | 7.76 | 16154 | 42038 | 5128 | 56408 | 13396 | 2122 | 26678 | 0 | 73447 | 5120 | 0 |
| wasmtime | klu | analytic | times | 9.294 | 4.02 | 2313 | 3768 | 173 | 0 | 1095 | 189 | 3291 | 1 | 4605 | 159 | 0 |
| wasmtime | klu | difference | times | 9.321 | 4.06 | 2295 | 3759 | 173 | 1903 | 1091 | 190 | 3279 | 0 | 4629 | 160 | 0 |
| Node | dense | analytic | times | 127.3 | 6.57 | 19368 | 51021 | 6464 | 0 | 15549 | 2353 | 31623 | 0 | 88361 | 6457 | 0 |
| Node | dense | difference | times | 118.6 | 7.34 | 16154 | 42038 | 5128 | 56408 | 13396 | 2122 | 26678 | 0 | 73447 | 5120 | 0 |
| Node | klu | analytic | times | 8.364 | 3.62 | 2313 | 3768 | 173 | 0 | 1095 | 189 | 3291 | 1 | 4605 | 159 | 0 |
| Node | klu | difference | times | 8.543 | 3.72 | 2295 | 3759 | 173 | 1903 | 1091 | 190 | 3279 | 0 | 4629 | 160 | 0 |
| Chrome | dense | analytic | times | 122.4 | 6.32 | 19368 | 51021 | 6464 | 0 | 15549 | 2353 | 31623 | 0 | 88361 | 6457 | 0 |
| Chrome | dense | difference | times | 115.7 | 7.16 | 16154 | 42038 | 5128 | 56408 | 13396 | 2122 | 26678 | 0 | 73447 | 5120 | 0 |
| Chrome | klu | analytic | times | 8.085 | 3.5 | 2313 | 3768 | 173 | 0 | 1095 | 189 | 3291 | 1 | 4605 | 159 | 0 |
| Chrome | klu | difference | times | 8.28 | 3.61 | 2295 | 3759 | 173 | 1903 | 1091 | 190 | 3279 | 0 | 4629 | 160 | 0 |

#### chen

| host | solver | Jacobian | output | median ms | µs per step | steps | rhs | Jacobians | DQ rhs | lin setups | err test fails | nonlin iters | conv fails | roots | integrations | probe rhs |
|---|---|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| roadrunner dense | dense | difference_quotient | times | 1049 | 373 | 2812 | 4383 | 86 | 43000 | 606 | 242 | 4373 | 48 | - | 5 | - |
| roadrunner 2.12.0 KLU | klu | sparse_difference_quotient | times | 452.6 | 171 | 2641 | 4048 | 88 | - | 578 | 221 | 4035 | 55 | - | 5 | - |
| roadrunner KLU | klu | sparse_difference_quotient | times | 455.3 | 172 | 2641 | 4048 | 88 | - | 578 | 221 | 4035 | 55 | - | 5 | - |
| wasmtime | dense | analytic | times | 1492 | 509 | 2930 | 4583 | 92 | 0 | 658 | 265 | 4570 | 51 | 3140 | 5 | 0 |
| wasmtime | dense | difference | times | 1149 | 405 | 2837 | 4314 | 93 | 13392 | 597 | 229 | 4301 | 53 | 3047 | 5 | 0 |
| wasmtime | klu | analytic | times | 530.1 | 186 | 2853 | 4446 | 90 | 0 | 599 | 241 | 4433 | 51 | 3063 | 5 | 0 |
| wasmtime | klu | difference | times | 510.5 | 187 | 2731 | 4134 | 90 | 12960 | 564 | 212 | 4121 | 53 | 2941 | 5 | 0 |
| Node | dense | analytic | times | 1331 | 454 | 2930 | 4583 | 92 | 0 | 658 | 265 | 4570 | 51 | 3140 | 5 | 0 |
| Node | dense | difference | times | 1043 | 368 | 2837 | 4314 | 93 | 13392 | 597 | 229 | 4301 | 53 | 3047 | 5 | 0 |
| Node | klu | analytic | times | 472.2 | 166 | 2853 | 4446 | 90 | 0 | 599 | 241 | 4433 | 51 | 3063 | 5 | 0 |
| Node | klu | difference | times | 456.2 | 167 | 2731 | 4134 | 90 | 12960 | 564 | 212 | 4121 | 53 | 2941 | 5 | 0 |
| Chrome | dense | analytic | times | 1313 | 448 | 2930 | 4583 | 92 | 0 | 658 | 265 | 4570 | 51 | 3140 | 5 | 0 |
| Chrome | dense | difference | times | 1015 | 358 | 2837 | 4314 | 93 | 13392 | 597 | 229 | 4301 | 53 | 3047 | 5 | 0 |
| Chrome | klu | analytic | times | 493.2 | 173 | 2853 | 4446 | 90 | 0 | 599 | 241 | 4433 | 51 | 3063 | 5 | 0 |
| Chrome | klu | difference | times | 467.6 | 171 | 2731 | 4134 | 90 | 12960 | 564 | 212 | 4121 | 53 | 2941 | 5 | 0 |

#### mp_flat

| host | solver | Jacobian | output | median ms | µs per step | steps | rhs | Jacobians | DQ rhs | lin setups | err test fails | nonlin iters | conv fails | roots | integrations | probe rhs |
|---|---|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| roadrunner dense | dense | difference_quotient | times | 2967 | 1440 | 2060 | 3054 | 40 | 36760 | 342 | 146 | 3051 | 13 | - | 1 | - |
| roadrunner 2.12.0 KLU | klu | sparse_difference_quotient | times | 84.16 | 36.5 | 2306 | 3177 | 46 | - | 334 | 135 | 3174 | 12 | - | 1 | - |
| roadrunner KLU | klu | sparse_difference_quotient | times | 83.38 | 36.2 | 2306 | 3177 | 46 | - | 334 | 135 | 3174 | 12 | - | 1 | - |
| wasmtime | dense | analytic | times | 4289 | 1530 | 2798 | 3780 | 51 | 0 | 344 | 135 | 3777 | 6 | 2998 | 1 | 0 |
| wasmtime | dense | difference | times | 3688 | 1440 | 2555 | 3506 | 51 | 612 | 348 | 139 | 3503 | 13 | 2755 | 1 | 0 |
| wasmtime | klu | analytic | times | 116 | 43.1 | 2694 | 3618 | 53 | 0 | 350 | 134 | 3615 | 15 | 2894 | 1 | 0 |
| wasmtime | klu | difference | times | 123.5 | 43.6 | 2834 | 3719 | 54 | 648 | 353 | 130 | 3716 | 12 | 3034 | 1 | 0 |
| Node | dense | analytic | times | 3841 | 1370 | 2798 | 3780 | 51 | 0 | 344 | 135 | 3777 | 6 | 2998 | 1 | 0 |
| Node | dense | difference | times | 3297 | 1290 | 2555 | 3506 | 51 | 612 | 348 | 139 | 3503 | 13 | 2755 | 1 | 0 |
| Node | klu | analytic | times | 113.3 | 42.1 | 2694 | 3618 | 53 | 0 | 350 | 134 | 3615 | 15 | 2894 | 1 | 0 |
| Node | klu | difference | times | 120.8 | 42.6 | 2834 | 3719 | 54 | 648 | 353 | 130 | 3716 | 12 | 3034 | 1 | 0 |
| Chrome | dense | analytic | times | 3755 | 1340 | 2798 | 3780 | 51 | 0 | 344 | 135 | 3777 | 6 | 2998 | 1 | 0 |
| Chrome | dense | difference | times | 3219 | 1260 | 2555 | 3506 | 51 | 612 | 348 | 139 | 3503 | 13 | 2755 | 1 | 0 |
| Chrome | klu | analytic | times | 114.1 | 42.4 | 2694 | 3618 | 53 | 0 | 350 | 134 | 3615 | 15 | 2894 | 1 | 0 |
| Chrome | klu | difference | times | 121.5 | 42.9 | 2834 | 3719 | 54 | 648 | 353 | 130 | 3716 | 12 | 3034 | 1 | 0 |

### Accuracy at the benchmark settings

The largest error against a reference of roadrunner dense at rtol 1e-12 and atol 1e-16 (scaled), in units of `atol_i + 1e-6 |x_ref|` (pointwise) / `atol_i + 1e-6 max_t |x_ref|` (scaled), as in the spike: the engine is in the accuracy class of roadrunner on every model. This table is of the first run, before the root finding of the conditions of `piecewise` (not measured again; the agreement below is).

| model | roadrunner dense | roadrunner KLU (2.12.0) | engine dense | engine KLU |
|---|---|---|---|---|
| hctz | 46.9 / 3.95 | 46.9 / 3.95 | 17.9 / 1.33 | 17.9 / 1.33 |
| midazolam | 81 / 3.03 | 81 / 3.03 | 81 / 3.03 | 81 / 3.03 |
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

## The probe of the triggers

The triggers at a step of the integrator in the output of its steps are now evaluated at its time only, without the probe (an event then executes at its root): the probe of the steps measured below (3596 calls on `repeated_dose`) is gone, the probe of the output times, the stop times and the roots stays. The rows of `repeated_dose` in the tables are of the first measurement, before that change.

After a stop of the integration with a trigger which is false, the engine evaluates the triggers right after the time (`PROBE_RTOL` times the scale of the times, the state moved along its rate), one call of the right hand side per stop (`probe_rhs_evals`): per output time (201 outputs and 20 events: 221) and, for the output of the steps of the integrator, per step (3596 calls against CVODE's 4092). Measured directly on `repeated_dose` in wasmtime with a throwaway build whose probe skips the right hand side (identical output, the trigger `time >= tnext` reads no state; 3 rounds of 20 each, alternating):

| output | solver | with the probe ms | without ms | cost |
|---|---|---:|---:|---:|
| times | dense | 2.245 | 2.176 | 3 % |
| times | KLU | 2.408 | 2.402 | 0 % |
| steps | dense | 2.871 | 2.835 | 1 % |
| steps | KLU | 3.189 | 3.037 | 5 % |

The cost is one right hand side per step, which on this model of 4 states is about 0.01 to 0.04 µs against 0.8 µs per step. On a larger model with events it scales with the share of the right hand side in a step: CVODE evaluates 1.1 to 1.4 right hand sides per step (hctz 1.1, Chen 1.6 with its difference quotient), so the probe adds 65 % to 90 % to the calls of the right hand side of the output of the steps of the integrator of a model with a false trigger, and its cost is that share of the time of the right hand side (an upper bound for a model whose step is dominated by the right hand side, small where the linear solver dominates). The output of times costs one right hand side per output time and event (221 here, negligible). The output of the steps of the integrator is still 0.4 to 0.6 of roadrunner's time on this model.

## Root finding of the conditions of piecewise

roadrunner (its LLVM model generator, sys-bio/roadrunner#1383) makes a root function of CVODE of every branch of every `piecewise` in the rules, the kinetic laws and the event triggers (`c1`, `c2 && !c1`, ..., the value `1` or `-1`), and restarts CVODE wherever one changes, without a change of the state. The engine does the same with continuous root functions of the operands (`a - b` of `a > b`, `min` and `max` for `and` and `or`, one per branch but the `otherwise`, none for a condition of constants and parameters only; `model/1` field `n_discontinuity_roots`), which CVODE locates as the roots of the triggers. The cost on the models of the benchmark, wasmtime, before and after:

| model | root functions | solver | ms before | ms after | integrations before / after | roadrunner integrations |
|---|---:|---|---:|---:|---|---:|
| mp_core | 2 | dense | 3.31 | 140 | 1 / 6457 | 4649 |
| mp_core | 2 | klu | 2.35 | 9.29 | 1 / 159 | 162 |
| chen | 4 | dense | 1484 | 1492 | 1 / 5 | 5 |
| chen | 4 | klu | 553 | 530 | 1 / 5 | 5 |
| mp_flat | 63 | dense | 4312 | 4289 | 1 / 1 | 1 |
| mp_flat | 63 | klu | 113 | 116 | 1 / 1 | 1 |

(The rows of mp_core and Chen are measured with the guard below.) On mp_core the engine now does roadrunner's work (its earlier 0.02 and 0.33 of roadrunner were the work it skipped); on Chen the 4 restarts at the stimulation change little; on mp_flat, whose 63 root functions never change their sign, the evaluation of the root functions after every step (2998 calls) costs nothing measurable. 27 cases of the SBML Test Suite which pass have root functions of `piecewise` conditions; the suite gives 1581 pass and no difference to the baseline with the repeat checks under `auto`, `--jacobian difference` and `--no-optimize`, and Node equals wasmtime byte for byte on every case.

**Roots which chatter.** roadrunner ignores, in one output interval of an output of times, the third and later root within its relative tolerance (as a distance in time) of the last root it applied (`strikes` in `CVODEIntegrator::integrate`: neither its events nor a restart; every step of its output of the integrator steps is a call of its own, so there it ignores none). The engine applies the same count to its restarts at the roots of discontinuities (every return at a root counts; events execute whatever the count, sbmlsim's semantics), in the output of times and in the horizons of a steady state; besides, a root of a discontinuity at the time of the last restart restarts nothing (CVODE started at a zero of a root function with a first step below the rounding of the time returns that zero again), and in the output of times the steps towards one output time are bounded by `maximum_num_steps` across the returns at roots and the restarts (CVODE counts one call, which every root ends), so a chattering model ends in a result or with CVODE's flag `CV_TOO_MUCH_WORK` (-1) within the step limit; the output of the integrator steps is bounded by the limit of the output. On two chattering models (host tests `a_sliding_mode_ends_as_in_roadrunner`, `a_guard_at_noise_level_agrees_with_roadrunner`):

- `x' = piecewise(-1, x > 0, 1)` (a sliding mode at 0 from t = 1): at rtol 1e-6, atol 1e-12 CVODE never accepts a step across 0 and creeps towards it; roadrunner fails with `CV_TOO_MUCH_WORK` at t = 1.00000000157277, the engine at 1.00000000157206. In the steps of the integrator roadrunner stops at its `max_output_rows` (100000 rows, a warning); the engine stopped at the limit of its output (flag -1000, 33.5 million rows, 14 s) before the bound of chatter, and ends now with `CV_TOO_MUCH_WORK` after 20000 consecutive stops within the resolution of the time, close to 1.0000000016. At rtol 1e-3, atol 1e-4 up to 2 both end within the absolute tolerance of the exact solution `max(1 - t, 0)` (roadrunner 6.7e-6, the engine 5.5e-6) and of each other (1.1e-5); the engine restarts at about 3000 roots, roadrunner at none (the trajectories chatter differently). At rtol 1e-3, atol 1e-6 roadrunner ends after 28.6 million steps (25 s, 5981 restarts: every root return starts its count of steps again), the engine ends with `CV_TOO_MUCH_WORK` after at most 20000 steps of one output interval (6 ms). The sliding mode is chaotic at noise level: the engine at end 1.1 and at end 3 takes other roots.
- `piecewise(0, S < 0, k * S)` as the rate of a decay (a guard at noise level): one restart in roadrunner and in the engine (times and steps), and against `exp(-t)` at rtol 1e-10, atol 1e-12 the largest excess of the engine is 1.67745 against roadrunner's 1.67732 dense and 1.67745 KLU (times) and 1.90227 against 1.90231 (steps): within 1 % of roadrunner's excess, the agreement passes.

### Known differences to roadrunner

The engine's documentation (`crates/engine/src/lib.rs`, "Known differences to libroadrunner") lists them; the ones of the root finding:

- Root functions: continuous in the operands of the conditions (`a - b`, `min`, `max`), roadrunner's the booleans of the branches; the same stops. Kinetic laws with local parameters have them too (roadrunner none, a slip of its generator).
- Chattering roots: the steps towards one output time are bounded by `maximum_num_steps` across the returns at roots and the restarts; roadrunner counts one call of CVODE, which every root return ends. The sliding mode at rtol 1e-3, atol 1e-6 ends in roadrunner after 28.6 million steps (25 s), in the engine with `CV_TOO_MUCH_WORK` within 20000 steps of one output interval.
- roadrunner's count of roots close together also skips the events of an ignored root; the engine's events execute whatever the count.
- The output of the integrator steps has the two rows of a root (an event or a discontinuity) at its exact time; roadrunner writes them at `t - tol` and `t + tol` (its root tolerance), which sbmlsim moves to the time half way between them for a model with events.
- In the output of the integrator steps the triggers at a step are evaluated at its time only: an event executes at its root, as in roadrunner, not up to the window of the probe before it (the probe stays at the roots, the stop times and the output times).
- Events which accumulate: an execution within the relative tolerance (as a distance in time) of the last applied root does not start the budget of steps towards one output time again, so a Zeno model ends with `CV_TOO_MUCH_WORK`. The bouncing ball (`x' = v`, `v' = -10`, the event `x < 0`: `v = -0.5 v`, `x = 0`, rtol 1e-8), whose bounces accumulate at t = 1.3416, did not return within 60 s before (every execution started the budget again, and each bounce advanced the time by 67 units of rounding); it ends there now in 0.1 s. roadrunner lets the ball fall through and continues.
- The bound of chatter in the output of the integrator steps: `maximum_num_steps` consecutive stops within the resolution of the time (1e-9 of the magnitude of the times) end the run with `CV_TOO_MUCH_WORK`. The bouncing ball ran 14 s to the limit of the output (22.4 million rows) before, the sliding mode 33.5 million rows; both end within 0.1 s now. roadrunner has no such bound. A chatter whose stops are further apart than the resolution (the sliding mode at rtol 1e-3, atol 1e-6, about 1e-7 apart) is bounded by the limit of the output only, as before.
- An event with a delay whose trigger a change of the request makes true is skipped, as sbmlsim skips it: it is not scheduled, assigns nothing, and its trigger stays armed, so it fires only after the trigger falls and rises again during the integration. SBML would schedule it.

## Agreement with roadrunner

`scripts/agreement.py`: the packaged models of sbmlsim (`HCTZ_SBML`, `MIDAZOLAM_SBML`, `REPRESSILATOR_SBML`, `DEMO_SBML`), the machine perfusion models and the event model, prepared with `scripts/prepare_model.py`. The reference is roadrunner through sbmlsim's `Simulator` (sbmlsim's pre-initialization, which the engine follows; libroadrunner 2.12.0 with its defaults, KLU and the sparse difference quotient) at rtol 1e-12 and atol 1e-14, the converged solution. The engine (`sbmlsim-core simulate`) runs at rtol 1e-10 and atol 1e-12 (scaled per state by sbmlsim; the engine gets the very vector roadrunner integrates with) with the dense solver and with KLU, both forced whatever `KLU_THRESHOLD`; 201 uniform times. Every column both have is compared with `DataSetsComparison` (`tol_rel = 1e-7`, `tol_abs = 1e-12`); the excess of a value is `|c - u| / (1e-12 + 1e-7 |c|)`. A solver passes when every value is within the criterion, or when its largest excess is at most 1 % above (`EXCESS_MARGIN = 1.01`) the one of roadrunner at rtol 1e-10 and atol 1e-12 with the same linear solver (dense with CVODE's difference quotient, KLU with the sparse difference quotient) against the same reference. A model passes when both solvers pass. The last column, the engine against roadrunner at the same tolerances, is informative only.

| model | solver | result | engine max excess | roadrunner 1e-10 max excess (same solver) | engine failing values | roadrunner failing values | engine max relative above 1e-9 | engine against roadrunner 1e-10 (informative) |
|---|---|---|---:|---:|---:|---:|---:|---:|
| hctz | dense | pass | 1.7491 | 1.7789 | 4 | 4 | 6.1e-07 (transport_lu_hctz) | 0.167 |
| hctz | klu | pass | 0.4667 | 1.7771 | 0 | 4 | 3.98e-07 (transport_re_hctz) | 2.24 |
| midazolam | dense | pass | 9.2542 | 9.2542 | 307 | 307 | 0.000623 (GUTDOSE_mid) | 0.947 |
| midazolam | klu | pass | 9.2542 | 9.2542 | 307 | 307 | 0.000623 (GUTDOSE_mid) | 0.947 |
| repressilator | dense | pass | 0.12572 | 0.14883 | 0 | 0 | 1.26e-08 (Reaction2) | 0.0239 |
| repressilator | klu | pass | 0.12579 | 0.12527 | 0 | 0 | 1.26e-08 (Reaction5) | 0.000637 |
| demo | dense | pass | 0 | 0 | 0 | 0 | 0 | 0 |
| demo | klu | pass | 0 | 0 | 0 | 0 | 0 | 0 |
| mp_core | dense | pass | 371.5 | 383.1 | 8116 | 7558 | 0.000566 ([nadp_cell]) | 13.1 |
| mp_core | klu | pass | 1396 | 2436.9 | 1717 | 1704 | 0.0368 (LACEX) | 2420 |
| mp_flat | dense | pass | 7.885 | 9.8605 | 200 | 147 | 0.000879 ([nadp_z20_cell]) | 16.2 |
| mp_flat | klu | pass | 5.2276 | 7.2111 | 101 | 122 | 0.000752 ([nadp_z20_cell]) | 7.28 |
| repeated_dose | dense | pass | 1.7386 | 1.7379 | 1 | 1 | 1.74e-07 (distribution) | 0.0176 |
| repeated_dose | klu | pass | 1.7383 | 1.7383 | 1 | 1 | 1.74e-07 (distribution) | 0.0618 |

Every model passes with both solvers; the script exits with 0. (Run again after the guard against roots which chatter: the same numbers.) Only the rows of mp_core changed with the root finding of the conditions of `piecewise`; the other models have no root of such a condition in their simulation (mp_flat has 63 root functions whose signs do not change, its rows are the same bits).

- hctz: with both solvers the engine is at least as close to the converged solution as roadrunner (dense 1.75 against 1.78, KLU 0.47 against 1.78); the largest excesses are transport rates and concentrations at 7.2 min, e.g. `[Clu_hctz]` 1.1303e-5 relative 3.3e-7, where roadrunner at 1e-10 misses the converged solution by as much. KLU is closer than dense, so the KLU path is not less accurate; the informative excess of 2.24 against roadrunner at 1e-10 is the distance between two solutions of the same error class, not an error of KLU. With `KLU_THRESHOLD = 64` hctz ran dense in the criterion of the first round (against roadrunner at 1e-10), which is why it passed there.
- mp_flat: the engine dense (7.9) is closer to the converged solution than roadrunner dense (9.9), KLU (5.2) closer than roadrunner KLU (7.2). Against roadrunner's default KLU at 1e-10 (7.21) the engine dense would be above it, which is why the criterion pairs the linear solvers, as the benchmark does.
- repeated_dose: the largest excess of every configuration is the same value (`distribution` at 42 h, 0.1486298706 converged, 0.1486298447 at 1e-10), roadrunner dense 1.73789, roadrunner KLU 1.73829, the engine KLU 1.73828 and the engine dense 1.73855, 0.04 % above roadrunner dense, where the engine dense and roadrunner dense differ by 2.6e-14 relative (the informative excess of 0.018). Such a tie is why the criterion allows the engine 1 % above roadrunner's excess; without the margin the engine dense failed.
- mp_core: the oxygen of the cell is 0 within 1e-29 for the whole perfusion, exactly at the threshold of `s_hyp = piecewise(1, o2_cell <= 0, K^n / (K^n + o2_cell^n))`, and the ATP of the cell falls to 1e-13 at the threshold of `s_atp`. Before the root finding of the conditions of `piecewise` the engine integrated through them and failed with the dense solver (1401 against roadrunner's 383, 1050 failing values); now it finds their roots and restarts CVODE there as roadrunner does, and its dense excess is 371.5 against roadrunner's 383.1, the KLU excess 1396 against 2437 (was 1783). Its largest excesses are now the enzymes released by the dead cells (`[ast_ext]`, `[ldh_ext]`, `[alt_ext]` at 21.6 min, 3.7e-5 relative) with the dense solver and the lactate exchange rate `LACEX` with KLU (a value of 1e-9 against 5e-10, at the absolute tolerance), where roadrunner at 1e-10 misses the converged solution by as much or more; the engine against roadrunner at the same tolerances is 13.1 dense (was 1400).

The largest relative difference over all values is 2 where one result is 0 or below the absolute tolerance and the other is not (e.g. a rate of 1e-23 against an exact 0).

The benchmark itself simulates the same problem with both: `scripts/roadrunner_reference.py` applies the pre-initialization as the initial values (`init(x)`, kept by every `resetAll`, which evaluates the initial assignments with them, as the engine and sbmlsim do), its tolerance vector is computed with that pre-initialization (roadrunner scales the absolute tolerance of a rate rule variable by its initial value: `PODOSE_hctz` 2.5e-11, `PODOSE_mid` 7.5e-12), and before it measures a model it checks that every state of roadrunner's state vector after the reset is the first row of the engine within 1e-12 relative (all six models: "roadrunner and the engine start from the same state"). No initial assignment of a benchmark model reads a target of its pre-initialization (`PODOSE_hctz`, `PODOSE_mid`, `c1`).
