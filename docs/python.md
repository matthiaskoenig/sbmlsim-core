# Python

The python package `sbmlsim-core` is the compiler and the engine in one extension, which runs `engine.wasm` on wasmtime. A model compiled from python is the `model.wasm` and `model.json` the command line writes, so it runs unchanged in a browser with the [JavaScript host](javascript.md).

```bash
pip install sbmlsim-core
```

PyPI has one abi3 wheel for python 3.14 or newer per platform: linux x86_64 and aarch64 (manylinux_2_28), macOS arm64 and x86_64, windows x86_64. Every wheel embeds the same `engine.wasm` and `mathlib.wasm`, the ones of the npm package of the release, so a simulation gives bitwise the same results on every platform and in the browser. There is no source distribution; a build from a checkout needs the toolchain of [Installation](installation.md).

## Compile and simulate

```python
import sbmlsim_core

compiled = sbmlsim_core.compile_sbml(sbml_text)  # optimize=True, jacobian="auto"
compiled.wasm  # the bytes of model.wasm
compiled.description  # the dict of model.json (model/2)

# engine.wasm and mathlib.wasm, compiled once per runtime: share it
runtime = sbmlsim_core.Runtime(max_memory_bytes=None)
model = runtime.load(compiled.wasm, compiled.description)  # compiled once, shared
model.initial_values()  # selection id -> value as loaded
instance = model.instance()  # one per thread
out = instance.simulate(request)  # request: a dict of simulation/1
out.columns, out.values  # names, float64 array (rows, columns)
instance.stats()  # the counters of the integrator
# a batch on rayon threads, the results in the order of the requests
results = model.simulate_many(requests, threads=None)
```

The compiler reads SBML Level 3 Version 1 or 2 core, flat and without a required package but `distrib` (the uncertainties of its fitted parameters, which are ignored), as the command line does ([Compiling a model](compiling.md)). The request is the one of [Simulating](simulating.md). Its nested enums are serde's externally tagged ones: an output is `{"uniform": {"steps": 100}}`, `{"times": [0.0, 1.0]}` or, a unit variant and so a bare string, `"integrator_steps"` (the steps of the integrator, whose number of rows differs by point, which is why `simulate_many` gives one table per point), and the absolute tolerance of the settings is `{"scalar": 1e-12}` or `{"per_state": [1e-12, 1e-10]}` with one value per state in the order of the state vector (a vector of another length is a `PointError` of that point in a batch):

```python
request["output"] = "integrator_steps"
request["settings"]["absolute_tolerance"] = {"per_state": [1e-12] * n_states}
```

`description` and `initial_values()` of a model, `description` of a compiled model and `columns` of an output are computed anew by every access, so read them once. The parallelism of `simulate_many` is `min(threads, len(requests))`, and its pool and its per-thread instances live only for the duration of the call (an instance costs about 0.3 s to make for a model of 700 kB of description), so give it as many points as you can rather than many small batches: 320 points in one call on 20 threads took 2.09 s against 22.0 s serially.

## A page of one file

The package carries what a page needs to simulate the models compiled here without a server: `sbmlsim_core.runtime_wasm()` gives the bytes of `engine.wasm` and `mathlib.wasm` the extension embeds (`{"engine": ..., "mathlib": ...}`), `sbmlsim_core.web.script()` the inline build of the JavaScript host, one classic script with the global `sbmlsimCore`, and `sbmlsim_core.THIRD_PARTY` the notices of the libraries of the engine, which a page that distributes it carries. `sbmlsimCore.createInlineEndpoint({ bundle, files, engine, mathlib })` is the panel endpoint ([JavaScript](javascript.md#the-panel-endpoint)) of a bundle held in memory: `bundle` the content of `bundle.json`, `files` its compiled models by their path in the bundle (`models/<key>/model.wasm` and `model.json`), and the workers of its pool start from a `blob:` URL of the worker the script embeds. So a page which embeds the script, the runtime, a bundle and its models as base64 opens from `file://` as well; sbmlsim writes the app of a model so.

## Errors

Errors derive from `SbmlsimCoreError`: `CompileError`, `LoadError` (a runtime which cannot start, a model which does not match its description, an instance or a pool which cannot be made) and `SimulationError` with `RequestError`, `IntegrationError` (`flag`, `time`), `UnsupportedError` and `InternalError`. A point of a batch which failed is a `PointError` (`kind`, `message`, `flag`, `time`) in its position of the result, so one failing request does not fail the batch.

## Release

A tag `vX.Y.Z` publishes the wheels to PyPI together with the npm package (`.github/workflows/publish.yml`, trusted publishing): the runtime is built once, every wheel is built with it (`SBMLSIM_CORE_WASM_DIR`, `.github/workflows/wheels.yml`, the linux wheels linked with zig against glibc 2.28), installed in a fresh environment and tested on python 3.14 and 3.15, and nothing is published unless every wheel and the JavaScript host passed. The same workflow of the wheels runs for every pull request. A wheel of a checkout is built so:

```bash
export SBMLSIM_CORE_WASM_DIR=$PWD/runtime   # from `sbmlsim-core runtime -o runtime`, or set WASI_SDK_PATH
uvx --from "maturin[zig]==1.16.0" maturin build --release --locked --zig --compatibility manylinux_2_28 --out dist
```
