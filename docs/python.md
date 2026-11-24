# Python

The python package `sbmlsim-core` is the compiler and the engine in one extension. It simulates on the native engine: the integrator compiled for the machine of the wheel, with CVODE and SuiteSparse KLU built from their sources, and the model itself as the `model.wasm` and `mathlib.wasm` wasmtime compiles. A model compiled from python is the `model.wasm` and `model.json` the command line writes, so it runs unchanged in a browser with the [JavaScript host](javascript.md).

```bash
pip install sbmlsim-core
```

## A first simulation

```python
import sbmlsim_core

compiled = sbmlsim_core.compile_sbml(open("model.xml").read())
runtime = sbmlsim_core.Runtime()
model = runtime.load(compiled.wasm, compiled.description)
instance = model.instance()

request = {
    "contract": "simulation/1",
    "start": 0.0,
    "end": 10.0,
    "output": {"uniform": {"steps": 100}},
    "selections": ["time", "[S]"],
}
out = instance.simulate(request)

print(out.columns)
print(out.values)  # a float64 array, (rows, columns)
```

A `Runtime` and a model are shared between threads, an instance is one simulation at a time, and `model.simulate_many(requests)` runs a batch on rayon threads. The same `compiled.wasm` and `compiled.description` run unchanged in a browser, see [JavaScript](javascript.md), and the same request is the one the [Rust](rust.md) host takes.

PyPI has one abi3 wheel for python 3.14 or newer per platform: linux x86_64 and aarch64 (manylinux_2_28), macOS arm64 and x86_64, windows x86_64. Every wheel carries the same `model.wasm` compiler and the same `mathlib.wasm` as the npm package of the release, so the math of a model is the same everywhere. There is no source distribution; a build from a checkout needs the toolchain of [Installation](installation.md).

## The engine of the wheel

A wheel simulates on the native engine and not on `engine.wasm`, which is 1.9 to 2.3 times as fast: on the five models of the benchmark of [sbmlsim](https://github.com/matthiaskoenig/sbmlsim) one simulation takes 0.0097 to 37 ms natively against 0.020 to 69 ms with `engine.wasm` in wasmtime ([Benchmarks](benchmarks.md), one performance core). The model is `model.wasm` in both, so only the integrator changes.

What this costs is the last bits. CVODE calls the `pow` and `sqrt` of the C library of the platform where the step size is chosen (`SUNRpowerR`), `engine.wasm` calls those of wasi-libc, so:

- a result of a wheel is **not** bitwise a result of `engine.wasm`, i.e. not bitwise a result of the same model in the browser or under `node`;
- a result of a wheel is not guaranteed bitwise across platforms either, since the C library is the platform's.

How large those differences are, measured: on the five models of the benchmark of sbmlsim the integrator took the **identical path** in the native engine, in wasmtime and under Node, all eighteen configurations of a model, a solver and a Jacobian agreeing on every counter (steps, right hand side evaluations, Jacobian evaluations, nonlinear iterations, error test failures), so nothing of those results moved. On the golden model of `crates/host/tests/native.rs`, which has events, root functions and a steady state in its request, 3 of 30 values differ and the largest relative difference is 1.0e-14, against an integration at a relative tolerance of 1e-6. So the two engines can differ, and where they do it is near the last bits of a double and far below the error of the integration.

Within one platform and one wheel a result is reproducible as before: the C sources are compiled with `-O3 -DNDEBUG -ffp-contract=off -fno-fast-math`, so no fused multiply-add and no fast math enter a result, every instance of a model integrates the same way, and a batch gives the results of a serial run bitwise whatever its threads. The differences are those of an integration at the given tolerances, which is what [Simulating](simulating.md) compares engines at; `sbmlsim-core bench --host wasmtime` and the [JavaScript host](javascript.md) are the way to a result of `engine.wasm`.

## Compile and simulate

```python
import sbmlsim_core

compiled = sbmlsim_core.compile_sbml(sbml_text)  # optimize=True, jacobian="auto"
compiled.wasm  # the bytes of model.wasm
compiled.description  # the dict of model.json (model/2)

# engine.wasm and mathlib.wasm, loaded from the compilation cache or compiled on a miss
runtime = sbmlsim_core.Runtime(max_memory_bytes=None)
model = runtime.load(compiled.wasm, compiled.description)  # compiled once, shared
model.initial_values()  # selection id -> value as loaded
instance = model.instance()  # one per thread
out = instance.simulate(request)  # request: a dict of simulation/1
out.columns, out.values  # names, float64 array (rows, columns)
out.sensitivities  # None, or the forward sensitivities the request asked for
instance.stats()  # the statistics of the integrator, counts and the state of the solver
# a batch on rayon threads, the results in the order of the requests
results = model.simulate_many(requests, threads=None)
```

The compiler reads SBML core of any level and version, flat and without a required package but `distrib` (the uncertainties of its fitted parameters, which are ignored), as the command line does ([Compiling a model](compiling.md)). The request is the one of [Simulating](simulating.md). Its nested enums are serde's externally tagged ones: an output is `{"uniform": {"steps": 100}}`, `{"times": [0.0, 1.0]}` or, a unit variant and so a bare string, `"integrator_steps"` (the steps of the integrator, whose number of rows differs by point, which is why `simulate_many` gives one table per point), and the absolute tolerance of the settings is `{"scalar": 1e-12}` or `{"per_state": [1e-12, 1e-10]}` with one value per state in the order of the state vector (a vector of another length is a `PointError` of that point in a batch):

```python
request["output"] = "integrator_steps"
request["settings"]["absolute_tolerance"] = {"per_state": [1e-12] * n_states}
```

A request which asks for [forward sensitivities](simulating.md#forward-sensitivities) answers with them on the output, `None` without: `out.sensitivities.parameters` are the ids in the order they were asked for and `out.sensitivities.values` is a float64 array of the shape `(rows, columns, parameters)` in C order, the derivative of the column `c` of the row `r` in the parameter `j` at `[r, c, j]`. They come from the native engine, so a request which asks for them goes through `simulate` or `simulate_many` of a model of this package and not through `engine.wasm`, whose output format has no room for them.

`description` and `initial_values()` of a model, `description` of a compiled model and `columns` of an output are computed anew by every access, so read them once. The parallelism of `simulate_many` is `min(threads, len(requests))`, and its pool and its per-thread instances live only for the duration of the call (an instance costs about 0.3 s to make for a model of 700 kB of description), so give it as many points as you can rather than many small batches: 320 points in one call on 20 threads took 2.09 s against 22.0 s serially.

The allocator of the extension is mimalloc on every target but macOS, where the wheel links none and the system allocator does the work, because mimalloc keeps the heap of a thread in two slots of the pthread TLS block which it claims by compiled in number there, so two extension modules which each link a copy of it corrupt each other's heaps (`sbmlsim_core.__allocator__` is `"mimalloc"` or `"system"`, so which of the two a build is comes from the artifact). Where it is linked it is in the wheel and needs nothing of the machine that installs it. A simulation allocates inside the linear memory of the engine and is unchanged by it; the compiler of a model and Cranelift do allocate, so `compile_sbml` of a model of 919 states takes 21 % less time (124.6 to 98.4 ms), a PBPK model 17 % less, `load` of the large model 7 % less and an uncached `Runtime()` 5 % less, and the peak resident set size of such a process falls by 23 %, measured on linux against glibc malloc; what the macOS build gives up is not measured, since it allocates through libmalloc, which is another design. A cold start of a PBPK model spends about 560 ms in `Runtime()`, 3 ms in `compile_sbml`, 10 ms in `load` and 0.74 ms in a simulation of 201 points, and a scan of 512 of them takes 445 ms on one thread and 67 ms on eight ([bench/RESULTS.md](https://github.com/matthiaskoenig/sbmlsim-core/blob/develop/bench/RESULTS.md) has the numbers).

## Compilation cache

`Runtime()` enables Wasmtime's persistent compilation cache for `engine.wasm`, `mathlib.wasm` and every model passed to `load`. Later runtimes, including those in another process, reuse the compiled machine code. The SBML-to-Wasm compiler (`compile_sbml`) still runs when called. Keep sharing a runtime and loaded models within a process to avoid repeated setup.

```python
from pathlib import Path

runtime = sbmlsim_core.Runtime()  # Wasmtime's platform cache directory
runtime = sbmlsim_core.Runtime(cache_dir=Path(".cache/sbmlsim-core"))
runtime.cache_dir  # resolved pathlib.Path, or None if disabled or unavailable
uncached = sbmlsim_core.Runtime(cache=False)
```

The default directory is Wasmtime's platform cache location, for example `$XDG_CACHE_HOME/wasmtime` or `~/.cache/wasmtime` on Linux. `cache_dir` accepts a string or a path; relative paths are resolved against the current directory. Passing it with `cache=False` raises `ValueError`. The constructor releases the GIL during cache initialization and compilation.

Wasmtime keys entries by the Wasm bytes, its compiler version, CPU features and compilation settings. A changed module or incompatible configuration is compiled again. Missing, unreadable or corrupt entries are treated as misses; an unavailable cache directory also falls back to compilation. Cache files can be deleted to reclaim disk space, and Wasmtime applies its default size and age cleanup policy. The cache contains executable machine code, so choose a local directory writable only by trusted users. It does not contain simulation results or share mutable model state. Memory limits and execution deadlines apply to each instance as before, and the wheels remain portable.

## Polynomial chaos

sbmlsim fits a polynomial chaos expansion to the outputs of a scan (`sbmlsim.sensitivity.chaos`) in the coordinates of the unit cube of its design: the products of the shifted Legendre polynomials normalized to one. The parts of it which are regular and cheap per entry run here; the least squares of the fit stay with sbmlsim, whose QR is the one of LAPACK.

```python
import numpy

# the multi-indices of a truncated basis, a row per term: the degree 4 over 22 inputs,
# the hyperbolic truncation q = 0.75 and at most two inputs per term (782 terms)
basis = sbmlsim_core.chaos_expansion(22, 4, q=0.75, max_interaction=2)

# the points of a sample of the unit cube, and its design matrix: a row per point and a
# column per term
u = numpy.random.default_rng(0).random((2048, 22))
x = sbmlsim_core.chaos_design_matrix(u, basis)

# the least squares of sbmlsim: the coefficients of a row per term and a column per
# element of the outputs `y` at the points
y = numpy.column_stack([u[:, 0] * u[:, 1], numpy.exp(u[:, 2])])
coefficients = numpy.linalg.lstsq(x, y, rcond=None)[0]

# the surrogate: the values of the expansion at other points
values = sbmlsim_core.chaos_predict(numpy.full((5, 22), 0.5), basis, coefficients)
```

The multi-indices are those of `sbmlsim.sensitivity.chaos.expansion` in its order, and the design matrix is bitwise the one of its `design_matrix`. A term skips the inputs it does not read, so a basis of many inputs costs per term what its own inputs cost. `chaos_predict` is the product of the design matrix and the coefficients without the design matrix, so a scan of a surrogate of a million points holds no more than its values; coefficients of one value per term give one value per point. Both release the GIL and spread the points over `threads` threads (every CPU of the process by default); a value does not depend on them.

## A page of one file

The package carries what a page needs to simulate the models compiled here without a server: `sbmlsim_core.runtime_wasm()` gives the bytes of `engine.wasm`, `mathlib.wasm` and `compiler.wasm` the extension embeds (`{"engine": ..., "mathlib": ..., "compiler": ...}`, the last one the compiler of a page, see [the compiler](javascript.md#the-compiler)), `sbmlsim_core.web.script()` the inline build of the JavaScript host, one classic script with the global `sbmlsimCore`, and `sbmlsim_core.THIRD_PARTY` the notices of the libraries of the engine, which a page that distributes it carries. `sbmlsimCore.createInlineEndpoint({ bundle, files, engine, mathlib })` is the panel endpoint ([JavaScript](javascript.md#the-panel-endpoint)) of a bundle held in memory: `bundle` the content of `bundle.json`, `files` its compiled models by their path in the bundle (`models/<key>/model.wasm` and `model.json`), and the workers of its pool start from a `blob:` URL of the worker the script embeds. So a page which embeds the script, the runtime, a bundle and its models as base64 opens from `file://` as well; sbmlsim writes the app of a model so.

## Errors

Errors derive from `SbmlsimCoreError`: `CompileError`, `LoadError` (a runtime which cannot start, a model which does not match its description, an instance or a pool which cannot be made) and `SimulationError` with `RequestError`, `IntegrationError` (`flag`, `time`), `UnsupportedError` and `InternalError`. A point of a batch which failed is a `PointError` (`kind`, `message`, `flag`, `time`) in its position of the result, so one failing request does not fail the batch.

## Release

A tag `vX.Y.Z` publishes the wheels to PyPI together with the npm package (`.github/workflows/publish.yml`, trusted publishing): the runtime is built once, every wheel is built with it (`SBMLSIM_CORE_WASM_DIR`, `.github/workflows/wheels.yml`, the linux wheels linked with zig against glibc 2.28), installed in a fresh environment and tested on python 3.14 and 3.15, and nothing is published unless every wheel and the JavaScript host passed. The same workflow of the wheels runs for every pull request. A wheel of a checkout is built so:

```bash
export SBMLSIM_CORE_WASM_DIR=$PWD/runtime   # from `sbmlsim-core runtime -o runtime`, or set WASI_SDK_PATH
uvx --from "maturin[zig]==1.16.0" maturin build --release --locked --zig --compatibility manylinux_2_28 --out dist
```
