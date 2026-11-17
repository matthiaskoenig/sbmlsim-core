# The engine against roadrunner on the models of sbmlsim, and what python costs

[sbmlsim](https://github.com/matthiaskoenig/sbmlsim) drives this engine from python and measures it against roadrunner, COPASI and AMICI on its own page of benchmarks. On its four packaged models it reported the engine core at 0.43 to 2.34 ms for one simulation and roadrunner at 0.46 to 2.00 ms, so the engine looked a little slower than roadrunner there. This file measured the same simulations without python, on every host the engine runs on, and found the gap in two layers above the integrator: the engine the wheel ran (`engine.wasm` in wasmtime, the slowest host) and the python layer of sbmlsim. Both were answered.

The table of the hosts against roadrunner is [docs/benchmarks.md](../docs/benchmarks.md), which is measured from `bench/sbmlsim.toml` and is the one place it lives; what is left here is the python layer. An earlier table in this file was dropped because its roadrunner reference had been measured while the machine was busy, which made `demo` and midazolam about twice as slow as they are and every ratio against them too kind to the engine.

`bench/sbmlsim.toml` is the configuration of sbmlsim's benchmark, not of `bench/models.toml`: the same ends, the same doses, 101 output points (`steps=100`) and the settings sbmlsim sends for a run at the defaults of `Simulator`, rtol 1e-6 with roadrunner's vector of absolute tolerances of the base 1e-10. `bench/models.toml` keeps the tighter 1e-12 of the spike, so the two files are not comparable with each other and each is comparable with itself.

## The two layers between sbmlsim and the integrator

The engine was measured on every host, and the wheel the python bindings ship used the slowest of them: it ran `engine.wasm` in wasmtime. Since sbmlsim-core 0.9.0 it runs the native engine, so the column of the wheel is the `native` one, and the two layers which stood between sbmlsim and the integrator were:

1. **The engine of the wheel.** `engine.wasm` in wasmtime against the native engine: 1.9 to 2.3 times on these models, the whole of it in the integrator.
2. **The python layer.** What sbmlsim adds per simulation, which the table below reads off its own benchmark.

### The python layer

sbmlsim's page reported one simulation at its own defaults, which select the solver by the size of the model (`linear_solver="auto"`: KLU above twelve states, so dense for the repressilator and `demo`, KLU for midazolam and HCTZ), and roadrunner 2.12 with its own defaults, which is the KLU column above. Its numbers against the Node column of this file, which is the host of the same engine build:

| model | engine Node ms | sbmlsim core ms | python adds | roadrunner ms | sbmlsim roadrunner ms | python adds |
|---|---:|---:|---:|---:|---:|---:|
| repressilator | 0.199 | 1.38 | 1.18 | 0.274 | 1.02 | 0.75 |
| demo | 0.021 | 0.426 | 0.41 | 0.116 | 0.459 | 0.34 |
| midazolam | 0.321 | 2.03 | 1.71 | 0.402 | 1.46 | 1.06 |
| hctz | 0.408 | 2.34 | 1.93 | 0.662 | 2.00 | 1.34 |

The columns `engine Node ms` and `roadrunner ms` are the measurement of [docs/benchmarks.md](../docs/benchmarks.md); the two `sbmlsim` columns are what its page published before these layers were answered, so the table reads the python layer off a run of sbmlsim against a run of the same engine and the same roadrunner without it.

Under Node the engine takes 0.18 to 0.80 of roadrunner's time (0.08 to 0.74 natively) and sbmlsim reported it at 0.93 to 1.39 of roadrunner's. The whole difference was in the python layer, which cost the engine 1.18 to 1.93 ms per simulation against roadrunner's 0.34 to 1.34 ms, and which was 82 to 95 % of what sbmlsim measured for the engine (`demo`: 0.021 ms of engine inside 0.426 ms).

Profiled in sbmlsim (300 simulations of HCTZ, `cProfile`), the python side of one simulation is:

| part | ms |
|---|---:|
| `Instance.simulate`, the engine | 0.55 |
| `LoadedModel.instance()`, a new instance of the model per simulation | 0.33 |
| `state_tolerances`, the vector of absolute tolerances, computed per simulation | 0.10 |
| the plan, the request, the targets of the state and the result | 0.15 |

So two thirds of what sbmlsim adds is work which does not depend on the simulation: the engine is stateless by the contract `simulation/1`, so an instance can be kept per model, and the tolerances and the targets of the state depend only on the model. The scans have the same shape: `LoadedModel.simulate_many` builds a rayon pool and an instance per thread for every chunk, where `LoadedModel.batch_executor` keeps both across calls (measured: 5.1x on a batch of one point, 1.65x on a chunk of 250 points of `demo`, 1.16x on HCTZ).

Both layers were answered: sbmlsim keeps the instance of a model, the vector of its absolute tolerances and the targets of its changes instead of building them per simulation, and uses the persistent batch executor of the bindings for a scan; this repository gives the wheel the native engine. What is left of the python layer is the part which builds a request and reads a table back, which is the cost of being a python library at all.
