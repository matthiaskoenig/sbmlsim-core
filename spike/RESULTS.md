# Results of the spike: CVODE and KLU in wasm

The spike asks whether "one wasm everywhere" (the same `engine.wasm` with CVODE and KLU and a `model.wasm` per model, run natively in wasmtime and in the browser) is fast enough against roadrunner native before the compiler is built. This file reports the numbers; the gate (at most 2x roadrunner native, dense and KLU, in wasmtime and Chrome) is decided by the owner. Everything else of the spike was throwaway and is not committed.

## Ratios against roadrunner native

Wall time of `simulate(0, end, 201)` divided by roadrunner's: dense against the released libroadrunner (LLJIT `O2`, dense, CVODE's difference quotient), KLU against the `roadrunner-sparsedq` build (KLU with its sparse difference quotient). The engine uses the analytic Jacobian. "scalar" is the wasm build without SIMD, "simd" the same build with `-msimd128` (same results, same CVODE counters).

| model | solver | wasmtime scalar | wasmtime simd | Chrome scalar | Chrome simd | Node scalar | Node simd | native C |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| hctz | dense | 0.74 | 0.61 | 0.65 | 0.55 | 0.84 | 0.65 | 0.44 |
| hctz | KLU | 0.72 | 0.63 | 0.64 | 0.56 | 0.76 | 0.76 | 0.42 |
| mp_core | dense | 0.03 | 0.02 | 0.02 | 0.02 | 0.02 | 0.02 | 0.01 |
| mp_core | KLU | 0.29 | 0.25 | 0.26 | 0.24 | 0.30 | 0.27 | 0.16 |
| mp_flat | dense | 1.88 | 1.24 | 1.63 | 1.07 | 1.66 | 1.17 | 0.98 |
| mp_flat | KLU | 1.11 | 0.94 | 1.06 | 0.91 | 1.06 | 0.92 | 0.79 |

The work is the same on hctz (479 steps, 9 Jacobians everywhere) and close on mp_flat (2020 to 2415 steps, see below). It is not the same on mp_core: roadrunner finds the roots of the condition of `piecewise(1, o2_cell <= 0, ...)` and restarts CVODE at each of them (4648 restarts and 14345 steps dense, 161 restarts and 2274 steps KLU), the engine of the spike has no root finding of piecewise conditions (the expression is continuous there) and takes 998 steps. Per step, mp_core is 0.38 (wasmtime scalar), 0.33 (Chrome scalar), 0.30 / 0.26 (simd) of roadrunner dense and 0.68, 0.60, 0.59 / 0.55 of roadrunner KLU; mp_flat per step is 1.78 / 1.53 / 1.17 / 1.01 dense and 1.11 / 1.06 / 0.95 / 0.91 KLU (wasmtime scalar, Chrome scalar, wasmtime simd, Chrome simd).

The only ratio above 1.2 is mp_flat dense without SIMD (1.6x to 1.9x): 919 states, the time is the dense LU of SUNDIALS (`SUNLinSol_Dense`, no BLAS) on a 919 x 919 matrix, the same code roadrunner runs; SIMD brings it to 1.07x (Chrome) and 1.24x (wasmtime). With KLU, which is what a model of that size uses, the wasm engine is at 0.9x to 1.1x of roadrunner's KLU.

## Setup

- Machine: Intel Core i7-12700 (8 performance cores with 2 threads, 4 efficiency cores; 20 logical CPUs), Linux 7.0, governor `powersave`. Single threaded measurements are pinned with `taskset` to performance cores (core 2 for the command line hosts, cores 4 to 15 for Chrome); without pinning a run lands on an efficiency core now and then and the numbers vary by 2x.
- roadrunner: libroadrunner 2.11.1 (develop build of the sbmlsim environment, LLJIT, optimization level `DEFAULT` = `O2` as sbmlsim sets it through `LoadSBMLOptions`); `roadrunner-sparsedq` build at `~/rr-enzyme/build-sparsedq` (same version, `jacobian = "sparse_difference_quotient"`, `linear_solver = "klu"`, also LLJIT `O2`).
- Engine hosts: native C (gcc 13.3, `-O3`, glibc libm), wasmtime 50.0.1 (Cranelift, defaults), Node 24.21.0, Chrome 157.0.8103.78 headless through Playwright (`channel: "chrome"`), page served with cross-origin isolation so `performance.now` in the workers is fine grained.
- Models (time unit min, no events): `hctz_body_flat.xml` (28 states: 21 species amounts and 7 rate rules; 49 parameters; 88 Jacobian non-zeros) with an oral dose `PODOSE_hctz = 25` mg as the initial state (the model as loaded has no dose and nothing happens), end 1440 min (24 h); `machine_perfusion_liver_core.xml` (43 states, 190 non-zeros) and `machine_perfusion_liver_flat.xml` (919 states: 839 species and 80 rate rules; 1007 reactions; 4262 non-zeros) as loaded, end 360 min: the experiments of the perfusion package (`donors.py` `END = 6.0`, `flow_rate.py` `end=6.0` h) simulate a 6 h perfusion.
- Settings everywhere: BDF (maximum order 5), relative tolerance 1e-6, absolute tolerance 1e-12 as roadrunner applies it (roadrunner scales the absolute tolerance per state; the engine takes the vector of `integrator.getAbsoluteToleranceVector()` after a simulation, checked equal in every roadrunner run), `maximum_num_steps` 20000, 201 output points with `CV_NORMAL` to each output time (as roadrunner does), the same initial state, no fast math anywhere. A measurement is the median of 20 runs after 3 warm ups; the configurations take turns, 3 rounds, the table has the median of the round medians. A run of the engine creates and frees the CVODE memory, the matrix and the linear solver; roadrunner's time is `simulate` after `resetAll` and the setup.

## Step 1: build recipe

Sources, cloned at the latest release tags into `spike/third_party/` (Task 4 vendors them as submodules):

- SUNDIALS `v7.9.0` (commit `312fc0f3684f27209ca9dc9249d194436eb41a7a`)
- SuiteSparse `v7.14.1` (commit `e202a5bcef55f5a780c26830ebd6ff13af193baf`, KLU 2.3.6)

wasi-sdk 34 (clang 23.1.0, wasi-libc 2e6fb9d8ee0c), toolchain file `$WASI_SDK_PATH/share/cmake/wasi-sdk-p1.cmake` with `-DWASI_SDK_PREFIX=$WASI_SDK_PATH` (the brief's `wasi-sdk.cmake` is named `wasi-sdk-p1.cmake` in wasi-sdk 34). Native with gcc. No source patches were needed for either target.

SuiteSparse (first; only the KLU part):

```
cmake -S SuiteSparse -B build/suitesparse [-DCMAKE_TOOLCHAIN_FILE=... -DWASI_SDK_PREFIX=...] \
  -DCMAKE_BUILD_TYPE=Release -DCMAKE_C_FLAGS_RELEASE="-O3 -DNDEBUG" -DCMAKE_INSTALL_PREFIX=$PREFIX \
  -DSUITESPARSE_ENABLE_PROJECTS="suitesparse_config;amd;colamd;btf;klu" -DKLU_USE_CHOLMOD=OFF \
  -DBUILD_SHARED_LIBS=OFF -DBUILD_STATIC_LIBS=ON -DSUITESPARSE_USE_OPENMP=OFF -DSUITESPARSE_USE_FORTRAN=OFF \
  -DSUITESPARSE_USE_CUDA=OFF -DSUITESPARSE_REQUIRE_BLAS=OFF -DSUITESPARSE_USE_64BIT_BLAS=OFF \
  -DBLA_VENDOR=Generic -DSUITESPARSE_DEMOS=OFF -DBUILD_TESTING=OFF
```

`-DBLA_VENDOR=Generic` is required: with `SUITESPARSE_REQUIRE_BLAS=OFF` the configuration of SuiteSparse_config still includes `SuiteSparse__blas_threading.cmake`, which fails on an empty `BLA_VENDOR` (`string sub-command REGEX, mode MATCH needs at least 5 arguments`). Nothing links BLAS. Gives `libklu.a libbtf.a libamd.a libcolamd.a libsuitesparseconfig.a`.

SUNDIALS (CVODE only):

```
cmake -S sundials -B build/sundials [-DCMAKE_TOOLCHAIN_FILE=... -DWASI_SDK_PREFIX=...] \
  -DCMAKE_BUILD_TYPE=Release -DCMAKE_C_FLAGS_RELEASE="-O3 -DNDEBUG" -DCMAKE_INSTALL_PREFIX=$PREFIX \
  -DBUILD_SHARED_LIBS=OFF -DBUILD_STATIC_LIBS=ON -DSUNDIALS_INDEX_SIZE=64 \
  -DBUILD_ARKODE=OFF -DBUILD_CVODES=OFF -DBUILD_IDA=OFF -DBUILD_IDAS=OFF -DBUILD_KINSOL=OFF -DBUILD_CVODE=ON \
  -DENABLE_MPI=OFF -DENABLE_OPENMP=OFF -DBUILD_FORTRAN_MODULE_INTERFACE=OFF -DEXAMPLES_ENABLE_C=OFF -DEXAMPLES_INSTALL=OFF \
  -DENABLE_KLU=ON -DKLU_INCLUDE_DIR=$PREFIX/include/suitesparse -DKLU_LIBRARY_DIR=$PREFIX/lib
```

`sundials_config.h` then has `SUNDIALS_INT64_T`, `SUNDIALS_KLU_ENABLED` (SUNDIALS calls the `klu_l_*` functions). The engine links `sundials_cvode sundials_sunlinsolklu sundials_sunlinsoldense sundials_sunmatrixsparse sundials_sunmatrixdense sundials_nvecserial sundials_core klu btf amd colamd suitesparseconfig m`. The only warnings are deprecations of the work space functions (removed in SUNDIALS 8). The SIMD variant adds `-msimd128` to `CMAKE_C_FLAGS_RELEASE` of both libraries and of the engine; it gives the same CVODE counters as the scalar build on every model.

The engine as a command (`main.c`, wasmtime) and as a reactor (`-mexec-model=reactor`, exports with `__attribute__((export_name(...)))`, Node and the browser) are 0.5 to 0.8 MB; the reactor imports 13 functions of WASI preview 1, which a 50 line shim satisfies in JavaScript (`fd_write`, `clock_time_get`, `proc_exit`, the environment and argument sizes; everything else `ENOSYS`).

## Step 2: generated C and the right hand side check

A throwaway Python script (libsbml, sympy, sbmlmath of the sbmlsim environment) writes per model `model_rhs(t, y, dy, p)` and `model_jac(t, y, jx, p)` (CSC values, columns the states, diagonal always in the pattern) plus the tables (pattern, parameters, initial state, absolute tolerances). The state vector is roadrunner's (`getStateVectorId`: the rate rule variables, then the amounts of the floating species which are states); constant symbols are parameters with roadrunner's values after the load (initial assignments evaluated by roadrunner); assignment rules, species concentrations (amount over compartment) and reaction rates (a reaction id is a symbol of the math) are intermediate variables in dependency order. The values are printed from the libsbml AST with the operations as written (left folds, numbers as doubles), the Jacobian is forward mode through the intermediates with sympy's partial derivatives, `powsimp(force=True)` (sympy writes `d/dx x^n` as `n x^n / x`, which is `NaN` at a concentration 0 and failed hctz at its initial state) and `cse` per block. Generation takes 0.1 s (hctz), 0.5 s (mp_core) and 10.5 s (mp_flat); `model_rhs.c` of mp_flat is 1.0 MB of C, its `model.wasm` 226 kB.

Check against roadrunner at 20 states (a random row of roadrunner's time course, each state times a uniform factor in [0.9, 1.1], set into roadrunner and read back): `getStateVectorRate` is not bound in the python API of 2.11.1; `RoadRunner.getRatesOfChange()` returns the state vector rate in `getStateVectorId` order (checked equal to the selections `x'`), so it is the reference.

| model | max relative difference of the right hand side | max absolute | Jacobian against central differences (of the row scale) | result |
|---|---:|---:|---:|---|
| hctz | 9.0e-14 | 7.1e-15 | 1.2e-6 | PASS |
| mp_core | 3.4e-15 | 6.8e-21 | 7.9e-7 | PASS |
| mp_flat | 4.9e-13 | 3.5e-18 | 7.8e-9 | PASS |

A first version printed the values from sympy, which reorders products; it passed hctz and mp_core but missed 1e-12 on mp_flat (3.5e-12), where a rate of change is a sum of reaction rates which nearly cancel. Printing from the AST in the order of the SBML fixed it, which the compiler of Task 3 should keep in mind (emit the operations as written).

The solutions agree in the accuracy class of the tolerances: against a reference of roadrunner at relative tolerance 1e-12 and absolute 1e-16, in units of `atol_i + rtol max_t |x_i|`, hctz is 2.6 (roadrunner dense and KLU) and 2.5 (engine dense and KLU), mp_core 139 / 76 (roadrunner dense / KLU) and 6.4 / 9.7 (engine), mp_flat 70 / 362 (roadrunner) and 286 / 57 (engine). The column "error (tol)" below is against the released roadrunner, pointwise (`atol_i + rtol |x_ref|`) and scaled (`atol_i + rtol max_t |x_ref|`); roadrunner's own KLU differs from its dense by as much.

## Step 3: timings

`ratio to roadrunner`: dense against `roadrunner release dense`, KLU against `roadrunner sparsedq klu`. `rhs` counts the right hand sides of CVODE including those of the difference quotient of a dense Jacobian (`num_lin_rhs_evals`); roadrunner's KLU counters do not include the model evaluations of its colored sparse difference quotient. `reinits` are roadrunner's restarts at roots. All wasm hosts give the same counters (one wasm, same bits); native C differs from wasm on mp_flat (2020 / 2415 against 2184 / 2304 steps) because glibc's and wasi-libc's `pow` differ in the last bits.


#### hctz

| host | solver | median ms | ratio to roadrunner | µs per step | steps | rhs | jac | lin setups | err test fails | reinits | error (tol) |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| roadrunner release | dense | 1.15 | 1.00 | 2.40 | 479 | 790 | 9 | 63 | 7 | 0 |  |
| roadrunner sparsedq | dense | 1.14 | 0.99 | 2.37 | 479 | 790 | 9 | 63 | 7 | 0 | 0 / 0 |
| native C | dense | 0.5 | 0.44 | 1.05 | 479 | 536 | 9 | 63 | 7 | 0 | 2.2 / 0.13 |
| wasmtime | dense | 0.85 | 0.74 | 1.78 | 479 | 536 | 9 | 63 | 7 | 0 | 2.2 / 0.13 |
| node | dense | 0.96 | 0.84 | 2.01 | 479 | 536 | 9 | 63 | 7 | 0 | 2.2 / 0.13 |
| chrome | dense | 0.75 | 0.65 | 1.57 | 479 | 536 | 9 | 63 | 7 | 0 |  |
| wasmtime simd | dense | 0.7 | 0.61 | 1.45 | 479 | 536 | 9 | 63 | 7 | 0 | 2.2 / 0.13 |
| node simd | dense | 0.75 | 0.65 | 1.56 | 479 | 536 | 9 | 63 | 7 | 0 | 2.2 / 0.13 |
| chrome simd | dense | 0.64 | 0.55 | 1.33 | 479 | 536 | 9 | 63 | 7 | 0 |  |
| roadrunner sparsedq | klu | 0.93 | 1.00 | 1.94 | 479 | 538 | 9 | 63 | 7 | 0 | 0.0059 / 0.00032 |
| native C | klu | 0.39 | 0.42 | 0.82 | 479 | 536 | 9 | 63 | 7 | 0 | 2.2 / 0.13 |
| wasmtime | klu | 0.67 | 0.72 | 1.39 | 479 | 536 | 9 | 63 | 7 | 0 | 2.2 / 0.13 |
| node | klu | 0.71 | 0.76 | 1.48 | 479 | 536 | 9 | 63 | 7 | 0 | 2.2 / 0.13 |
| chrome | klu | 0.6 | 0.64 | 1.25 | 479 | 536 | 9 | 63 | 7 | 0 |  |
| wasmtime simd | klu | 0.59 | 0.63 | 1.23 | 479 | 536 | 9 | 63 | 7 | 0 | 2.2 / 0.13 |
| node simd | klu | 0.71 | 0.76 | 1.48 | 479 | 536 | 9 | 63 | 7 | 0 | 2.2 / 0.13 |
| chrome simd | klu | 0.52 | 0.56 | 1.10 | 479 | 536 | 9 | 63 | 7 | 0 |  |

#### mp_core

| host | solver | median ms | ratio to roadrunner | µs per step | steps | rhs | jac | lin setups | err test fails | reinits | error (tol) |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| roadrunner release | dense | 133 | 1.00 | 9.25 | 14345 | 237984 | 4658 | 11978 | 1871 | 4648 |  |
| roadrunner sparsedq | dense | 132 | 0.99 | 9.18 | 14345 | 237984 | 4658 | 11978 | 1871 | 4648 | 0 / 0 |
| native C | dense | 1.9 | 0.01 | 1.90 | 998 | 1331 | 18 | 162 | 62 | 0 | 2.4e+03 / 1.1e+02 |
| wasmtime | dense | 3.55 | 0.03 | 3.56 | 998 | 1331 | 18 | 162 | 62 | 0 | 2.4e+03 / 1.1e+02 |
| node | dense | 3.1 | 0.02 | 3.11 | 998 | 1331 | 18 | 162 | 62 | 0 | 2.4e+03 / 1.1e+02 |
| chrome | dense | 3.03 | 0.02 | 3.03 | 998 | 1331 | 18 | 162 | 62 | 0 |  |
| wasmtime simd | dense | 2.74 | 0.02 | 2.75 | 998 | 1331 | 18 | 162 | 62 | 0 | 2.4e+03 / 1.1e+02 |
| node simd | dense | 2.56 | 0.02 | 2.56 | 998 | 1331 | 18 | 162 | 62 | 0 | 2.4e+03 / 1.1e+02 |
| chrome simd | dense | 2.42 | 0.02 | 2.42 | 998 | 1331 | 18 | 162 | 62 | 0 |  |
| roadrunner sparsedq | klu | 7.25 | 1.00 | 3.19 | 2274 | 3755 | 175 | 1094 | 192 | 161 | 2.3e+03 / 65 |
| native C | klu | 1.16 | 0.16 | 1.19 | 973 | 1310 | 19 | 164 | 61 | 0 | 2.4e+03 / 1.1e+02 |
| wasmtime | klu | 2.1 | 0.29 | 2.16 | 973 | 1310 | 19 | 164 | 61 | 0 | 2.4e+03 / 1.1e+02 |
| node | klu | 2.15 | 0.30 | 2.21 | 973 | 1310 | 19 | 164 | 61 | 0 | 2.4e+03 / 1.1e+02 |
| chrome | klu | 1.88 | 0.26 | 1.93 | 973 | 1310 | 19 | 164 | 61 | 0 |  |
| wasmtime simd | klu | 1.84 | 0.25 | 1.89 | 973 | 1310 | 19 | 164 | 61 | 0 | 2.4e+03 / 1.1e+02 |
| node simd | klu | 1.94 | 0.27 | 1.99 | 973 | 1310 | 19 | 164 | 61 | 0 | 2.4e+03 / 1.1e+02 |
| chrome simd | klu | 1.72 | 0.24 | 1.76 | 973 | 1310 | 19 | 164 | 61 | 0 |  |

#### mp_flat

| host | solver | median ms | ratio to roadrunner | µs per step | steps | rhs | jac | lin setups | err test fails | reinits | error (tol) |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| roadrunner release | dense | 3.08e+03 | 1.00 | 1496.22 | 2060 | 39814 | 40 | 342 | 146 | 0 |  |
| roadrunner sparsedq | dense | 2.9e+03 | 0.94 | 1407.75 | 2060 | 39814 | 40 | 342 | 146 | 0 | 0 / 0 |
| native C | dense | 3.01e+03 | 0.98 | 1490.69 | 2020 | 2964 | 42 | 325 | 144 | 0 | 1.8e+03 / 2.7e+02 |
| wasmtime | dense | 5.81e+03 | 1.88 | 2659.32 | 2184 | 3010 | 46 | 313 | 128 | 0 | 6.7e+02 / 34 |
| node | dense | 5.13e+03 | 1.66 | 2348.28 | 2184 | 3010 | 46 | 313 | 128 | 0 | 6.7e+02 / 34 |
| chrome | dense | 5.01e+03 | 1.63 | 2293.85 | 2184 | 3010 | 46 | 313 | 128 | 0 |  |
| wasmtime simd | dense | 3.82e+03 | 1.24 | 1746.98 | 2184 | 3010 | 46 | 313 | 128 | 0 | 6.7e+02 / 34 |
| node simd | dense | 3.6e+03 | 1.17 | 1646.92 | 2184 | 3010 | 46 | 313 | 128 | 0 | 6.7e+02 / 34 |
| chrome simd | dense | 3.3e+03 | 1.07 | 1510.47 | 2184 | 3010 | 46 | 313 | 128 | 0 |  |
| roadrunner sparsedq | klu | 83.9 | 1.00 | 36.39 | 2306 | 3177 | 46 | 334 | 135 | 0 | 1.3e+03 / 1.9e+02 |
| native C | klu | 66.4 | 0.79 | 27.49 | 2415 | 3308 | 47 | 335 | 134 | 0 | 8e+02 / 59 |
| wasmtime | klu | 93.3 | 1.11 | 40.48 | 2304 | 3223 | 45 | 344 | 141 | 0 | 1e+03 / 59 |
| node | klu | 88.7 | 1.06 | 38.52 | 2304 | 3223 | 45 | 344 | 141 | 0 | 1e+03 / 59 |
| chrome | klu | 89 | 1.06 | 38.62 | 2304 | 3223 | 45 | 344 | 141 | 0 |  |
| wasmtime simd | klu | 79.2 | 0.94 | 34.39 | 2304 | 3223 | 45 | 344 | 141 | 0 | 1e+03 / 59 |
| node simd | klu | 77.4 | 0.92 | 33.57 | 2304 | 3223 | 45 | 344 | 141 | 0 | 1e+03 / 59 |
| chrome simd | klu | 76.3 | 0.91 | 33.10 | 2304 | 3223 | 45 | 344 | 141 | 0 |  |

## Step 4: the cross-module call

The variant of the design: the host creates the memory, `math.wasm` (the math functions `pow`, `exp`, `log`, `log10`, `fmax`, `fmin` of wasi-libc, read-only tables at 1 kB to 12 kB of the memory, no stack) imports `env.memory`, `model.wasm` (`model_rhs`, `model_jac`; `--import-memory`, `-nostdlib`, no data, no globals, no stack, the math as imports of the module `"math"`) imports `env.memory` and `math.*`, and the engine imports `env.memory` (and re-exports it for WASI), `model.model_rhs` and `model.model_jac`, its data and stack from 64 kB up (`--global-base=65536 --no-stack-first`). The spec has the math in `engine.wasm`, which would make the imports circular (engine imports model, model imports engine); the spike instantiates in the order memory, math, model, engine, so every call is a direct wasm to wasm call. wasmtime links them with `wasmtime run --preload env=env.wasm --preload math=math.wasm --preload model=model_X.wasm bench_X.wasm`, `env.wasm` being a 25 byte module which only exports a memory; Node and Chrome instantiate the three with a `WebAssembly.Memory`. "opaque math" is the single module build with the math of the model as `noinline` calls (no builtin `pow`), which splits the overhead into the loss of the builtin math (clang turns `pow(x, 2)` into `x * x` and merges equal calls) and the cost of the calls across instances. mp_flat dense is left out (its time is the dense LU).

| model | solver | host | single module ms | opaque math ms | cross-module ms | cross / single |
|---|---|---|---:|---:|---:|---:|
| model | solver | host | single module ms | opaque math ms | cross-module ms | cross / single |
|---|---|---|---:|---:|---:|---:|
| hctz | dense | wasmtime | 0.84 | 0.89 | 0.94 | 1.12 |
| hctz | dense | wasmtime simd | 0.71 | 0.75 | 0.77 | 1.09 |
| hctz | dense | node | 0.97 | - | 1.01 | 1.04 |
| hctz | dense | node simd | 0.82 | - | 0.93 | 1.13 |
| hctz | dense | chrome | 0.75 | - | 0.81 | 1.08 |
| hctz | dense | chrome simd | 0.64 | - | 0.7 | 1.10 |
| hctz | klu | wasmtime | 0.67 | 0.72 | 0.76 | 1.15 |
| hctz | klu | wasmtime simd | 0.59 | 0.64 | 0.66 | 1.12 |
| hctz | klu | node | 0.79 | - | 0.9 | 1.14 |
| hctz | klu | node simd | 0.64 | - | 0.87 | 1.37 |
| hctz | klu | chrome | 0.6 | - | 0.66 | 1.09 |
| hctz | klu | chrome simd | 0.52 | - | 0.58 | 1.10 |
| mp_core | dense | wasmtime | 3.5 | 3.56 | 3.59 | 1.03 |
| mp_core | dense | wasmtime simd | 2.73 | 2.74 | 2.94 | 1.07 |
| mp_core | dense | node | 3.23 | - | 3.28 | 1.02 |
| mp_core | dense | node simd | 2.61 | - | 2.69 | 1.03 |
| mp_core | dense | chrome | 3.03 | - | 3.06 | 1.01 |
| mp_core | dense | chrome simd | 2.42 | - | 2.48 | 1.02 |
| mp_core | klu | wasmtime | 2.11 | 2.15 | 2.27 | 1.08 |
| mp_core | klu | wasmtime simd | 1.8 | 1.88 | 1.98 | 1.10 |
| mp_core | klu | node | 2.17 | - | 2.23 | 1.03 |
| mp_core | klu | node simd | 1.89 | - | 2 | 1.06 |
| mp_core | klu | chrome | 1.88 | - | 1.92 | 1.02 |
| mp_core | klu | chrome simd | 1.72 | - | 1.7 | 0.99 |
| mp_flat | klu | wasmtime | 92.7 | 93.5 | 95.2 | 1.03 |
| mp_flat | klu | wasmtime simd | 79.4 | 81.8 | 83.7 | 1.05 |
| mp_flat | klu | node | 88.6 | - | 90.4 | 1.02 |
| mp_flat | klu | node simd | 77.9 | - | 79.4 | 1.02 |
| mp_flat | klu | chrome | 89 | - | 89.8 | 1.01 |
| mp_flat | klu | chrome simd | 76.3 | - | 77.9 | 1.02 |

The cross-module build costs up to 15 % (the larger values on hctz, whose right hand side is a few hundred nanoseconds; one outlier of 37 %, Node simd hctz KLU, 0.64 against 0.87 ms, which the other hosts do not show); a quarter to a half of it is the opaque math, the rest the calls across instances. On the large model it is 1 % to 5 %.

## Worker pool in Chrome

`spike/web/index.html`: the compiled `WebAssembly.Module` is posted to each Web Worker, each instantiates it and runs 3 warm up simulations (V8 tiers the code up from Liftoff to TurboFan), then the workers take the 64 simulations (the parameters matching `Vmax`, for hctz `Ka_dis_hctz`, `f_renal_function`, `BW`, scaled by a factor in [0.8, 1.25] per simulation) one at a time from the page. `navigator.hardwareConcurrency` is 20, N = 19. Wall time from the first task to the last result; "startup" is the instantiation and the warm ups of all workers. Chrome was not pinned here.

| model | solver | workers | wall s | speedup | simulations/s | median per simulation ms | startup ms |
|---|---|---:|---:|---:|---:|---:|---:|
| model | solver | workers | wall s | speedup | simulations/s | median per simulation ms | startup ms |
|---|---|---:|---:|---:|---:|---:|---:|
| hctz | dense | 1 | 0.0461 | 1.00 | 1.39e+03 | 0.65 | 12.8 |
| hctz | dense | 2 | 0.0213 | 2.16 | 3e+03 | 0.64 | 7.92 |
| hctz | dense | 4 | 0.0121 | 3.81 | 5.28e+03 | 0.62 | 11.8 |
| hctz | dense | 19 | 0.00548 | 8.42 | 1.17e+04 | 1.11 | 30.4 |
| hctz | klu | 1 | 0.0371 | 1.00 | 1.72e+03 | 0.51 | 13.2 |
| hctz | klu | 2 | 0.0184 | 2.02 | 3.48e+03 | 0.53 | 7.28 |
| hctz | klu | 4 | 0.00944 | 3.94 | 6.78e+03 | 0.52 | 12.8 |
| hctz | klu | 19 | 0.00832 | 4.46 | 7.69e+03 | 1.91 | 44.5 |
| mp_core | dense | 1 | 0.162 | 1.00 | 396 | 2.44 | 21.6 |
| mp_core | dense | 2 | 0.0805 | 2.01 | 795 | 2.41 | 17.2 |
| mp_core | dense | 4 | 0.0406 | 3.98 | 1.57e+03 | 2.47 | 16.1 |
| mp_core | dense | 19 | 0.0198 | 8.18 | 3.24e+03 | 5.16 | 54.2 |
| mp_core | klu | 1 | 0.11 | 1.00 | 582 | 1.68 | 14 |
| mp_core | klu | 2 | 0.0557 | 1.98 | 1.15e+03 | 1.69 | 10.9 |
| mp_core | klu | 4 | 0.0291 | 3.78 | 2.2e+03 | 1.68 | 17.6 |
| mp_core | klu | 19 | 0.0221 | 4.97 | 2.89e+03 | 5.95 | 59 |
| mp_flat | dense | 1 | 194 | 1.00 | 0.33 | 3.02e+03 | 1e+04 |
| mp_flat | dense | 2 | 98.6 | 1.97 | 0.649 | 3.07e+03 | 1.01e+04 |
| mp_flat | dense | 4 | 56.9 | 3.41 | 1.13 | 3.53e+03 | 1.14e+04 |
| mp_flat | dense | 19 | 52.5 | 3.69 | 1.22 | 1.52e+04 | 5.42e+04 |
| mp_flat | klu | 1 | 4.08 | 1.00 | 15.7 | 65 | 255 |
| mp_flat | klu | 2 | 2.05 | 1.99 | 31.3 | 64.7 | 243 |
| mp_flat | klu | 4 | 1.04 | 3.93 | 61.6 | 65.3 | 247 |
| mp_flat | klu | 19 | 0.428 | 9.54 | 150 | 110 | 481 |

mp_flat with KLU scales to 9.5x on 19 workers (the performance cores with their second threads and the efficiency cores: a simulation takes 110 ms instead of 65 ms). hctz and mp_core finish 64 simulations in 5 to 20 ms with 19 workers, where the messages of the page dominate, so their speedup at 19 says little. mp_flat dense stops at 3.7x: 19 dense 919 x 919 matrices (6.8 MB each) exceed the last level cache and a simulation takes 15 s instead of 3 s.

## Problems and notes for the next tasks

- The work on mp_core differs because roadrunner root-finds piecewise conditions (sys-bio/roadrunner#1383) and restarts CVODE at each root; an engine with the same root finding would take roadrunner's steps. Its per step numbers are in the summary.
- The dense ratio of mp_flat is the dense LU of SUNDIALS in wasm without SIMD; SIMD (`-msimd128`, supported by wasmtime and every current browser) is the single largest lever on it (6.0 s to 3.8 s in wasmtime, 5.0 s to 3.3 s in Chrome) and makes the KLU numbers 10 % to 15 % faster as well. The Rust build of Task 4 should compile SUNDIALS and KLU with it.
- glibc and wasi-libc `pow` differ in the last bits, so native and wasm solutions of the stiff mp_flat take different steps (Ruling R2 of the ledger expects that); all wasm hosts agree.
- Analytic Jacobians need care at concentrations 0 (`x^n / x`), and the right hand side must be emitted in the order of the SBML to agree with roadrunner to 1e-12 on cancelling sums.
- roadrunner's python API has no `getStateVectorRate`; `getRatesOfChange()` is the state vector rate in state order (its docstring says floating species, which is outdated).
- Timing on the hybrid CPU needs pinning to performance cores; the first native measurements without it were off by up to 5x.

## Reproduce

All under `spike/` (throwaway, not committed): `build_libs.sh` (Step 1, targets `native wasm wasm-simd`), `generate_c.py` (Step 2, run with the python of the sbmlsim environment), `CMakeLists.txt` with `main.c`, `engine.c`, `engine.h` (the benchmark commands and the reactors), `build_cross.sh`, `math.c`, `math_imports.h`, `math_opaque.h`, `math_opaque.c` (Step 4), `bench.py` (roadrunner, native C, wasmtime, Node; `--cross` for Step 4; `--rounds 0` for the errors), `node_bench.mjs`, `web/` (`index.html`, `worker.js`, `engine.js`, `serve.py`, `assets.sh`, `run_chrome.mjs single|cross|pool`) and `table.py` (the tables above).
