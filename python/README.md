# sbmlsim-core

Compile SBML models to WebAssembly and simulate them. [sbmlsim-core](https://matthiaskoenig.github.io/sbmlsim-core/) compiles an SBML model into a WebAssembly module (`model.wasm` with its description `model.json`) and simulates it with its engine (`engine.wasm`: CVODE of SUNDIALS with the sparse solver KLU), with the semantics of [sbmlsim](https://matthiaskoenig.github.io/sbmlsim/) and bitwise the same results on every host. This package is its python host: the compiler and the engine in one extension, run on wasmtime, so a model compiled here runs unchanged in a browser with the JavaScript host of the npm package [`sbmlsim-core`](https://www.npmjs.com/package/sbmlsim-core).

## Install

```bash
pip install sbmlsim-core
```

One abi3 wheel per platform for python 3.14 or newer: linux (x86_64, aarch64, manylinux_2_28), macOS (arm64, x86_64) and windows (x86_64). There is no source distribution; a build from source is described in the [repository](https://github.com/matthiaskoenig/sbmlsim-core).

## Compile and simulate

The compiler reads SBML Level 3 (version 1 or 2) core, flat and without a required package but `distrib`, whose uncertainties of a fitted parameter annotate it and are ignored.

```python
import sbmlsim_core

compiled = sbmlsim_core.compile_sbml(sbml_text)  # optimize=True, jacobian="auto"
compiled.wasm  # the bytes of model.wasm
compiled.description  # the dict of model.json (model/2)

# engine.wasm and mathlib.wasm, compiled once per runtime: share it
runtime = sbmlsim_core.Runtime(max_memory_bytes=None)
model = runtime.load(compiled.wasm, compiled.description)
instance = model.instance()  # one per thread
out = instance.simulate(request)  # request: a dict of simulation/1
out.columns, out.values  # names, float64 array (rows, columns)
# a batch on rayon threads, the results in the order of the requests
results = model.simulate_many(requests, threads=None)
```

`model.wasm` and `model.json` written from `compiled.wasm` and `compiled.description` are what the JavaScript host loads, so a model compiled in python is simulated in a page without a server. The request and its fields, the errors and the semantics are documented on the [pages of sbmlsim-core](https://matthiaskoenig.github.io/sbmlsim-core/python/).

## License

MIT for the code of sbmlsim-core. The extension embeds `engine.wasm`, which links SUNDIALS (BSD-3-Clause) and KLU and BTF of SuiteSparse (LGPL-2.1-or-later), so the wheel is under `MIT AND BSD-3-Clause AND LGPL-2.1-or-later` and carries `LICENSE` and `THIRD_PARTY.md`.
