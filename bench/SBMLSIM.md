# The engine against roadrunner on the models of sbmlsim, and what python costs

[sbmlsim](https://github.com/matthiaskoenig/sbmlsim) drives this engine from python and measures it against roadrunner, COPASI and AMICI on its own page of benchmarks. On its four packaged models it reported the engine core at 0.43 to 2.34 ms for one simulation and roadrunner at 0.46 to 2.00 ms, so the engine looked a little slower than roadrunner there. This file measures the same four simulations without python, on every host the engine runs on, and the answer is the other way round: the engine simulates them in 0.04 to 0.70 of roadrunner's time. The gap was in two layers above the integrator, the engine the wheel ran (`engine.wasm` in wasmtime, the slowest host) and the python layer of sbmlsim, and this file is what measured both.

`bench/sbmlsim.toml` is the configuration of sbmlsim's benchmark, not of `bench/models.toml`: the same ends, the same doses, 101 output points (`steps=100`) and the settings sbmlsim sends for a run at the defaults of `Simulator`, rtol 1e-6 with roadrunner's vector of absolute tolerances of the base 1e-10. `bench/models.toml` keeps the tighter 1e-12 of the spike, so the two files are not comparable with each other and each is comparable with itself.

## The run

```bash
uv run scripts/prepare_model.py --bench bench/sbmlsim.toml
taskset -c 2 target/release/sbmlsim-core bench bench/sbmlsim.toml --json target/bench/sb-wasmtime.json --export target/bench/sb-export
taskset -c 2 target/release/sbmlsim-core bench bench/sbmlsim.toml --host native --json target/bench/sb-native.json
target/release/sbmlsim-core runtime -o js/runtime
taskset -c 2 node js/bench.mjs target/bench/sb-export --json target/bench/sb-node.json
node bench/web/serve.mjs target/bench/sb-chrome.json --port 8771 &   # then /bench/web/index.html?export=/target/bench/sb-export
uv run scripts/roadrunner_reference.py bench/sbmlsim.toml --json target/bench/sb-roadrunner.json
uv run scripts/roadrunner_reference.py bench/sbmlsim.toml --compare target/bench/sb-roadrunner.json \
    target/bench/sb-native.json target/bench/sb-wasmtime.json target/bench/sb-node.json target/bench/sb-chrome.json
```

The method is the one of `bench/RESULTS.md`: the median of 20 runs after 3 warm ups, 3 rounds, the configurations taking turns, one performance core (`taskset -c 2`, Chrome on 4-15). roadrunner is libroadrunner 2.12.0 with its own defaults, so its KLU column is the sparse direct solver and the colored sparse difference quotient Jacobian of roadrunner 2.12. Node 24.21.0, headless Chrome 157 in a cross-origin isolated module worker, wasmtime 50.0.1 (Cranelift), the native engine compiled for the machine.

## One simulation, every host

| model | states | solver | output | roadrunner ms | native ms | ratio | wasmtime ms | ratio | node ms | ratio | chrome ms | ratio |
|---|---:|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| repressilator | 6 | dense | times | 0.279 | 0.139 | 0.50 | 0.285 | 1.02 | 0.192 | 0.69 | 0.200 | 0.72 |
| repressilator | 6 | klu | times | 0.274 | 0.138 | 0.50 | 0.299 | 1.09 | 0.196 | 0.71 | 0.195 | 0.71 |
| demo | 6 | dense | times | 0.228 | 0.0094 | 0.04 | 0.0202 | 0.09 | 0.0194 | 0.09 | 0.020 | 0.09 |
| demo | 6 | klu | times | 0.242 | 0.0103 | 0.04 | 0.0229 | 0.09 | 0.0233 | 0.10 | 0.025 | 0.10 |
| midazolam | 38 | dense | times | 1.07 | 0.375 | 0.35 | 0.755 | 0.70 | 0.455 | 0.42 | 0.447 | 0.42 |
| midazolam | 38 | klu | times | 0.736 | 0.232 | 0.32 | 0.539 | 0.73 | 0.324 | 0.44 | 0.320 | 0.43 |
| hctz | 28 | dense | times | 0.877 | 0.446 | 0.51 | 0.859 | 0.98 | 0.506 | 0.58 | 0.500 | 0.57 |
| hctz | 28 | klu | times | 0.677 | 0.339 | 0.50 | 0.710 | 1.05 | 0.406 | 0.60 | 0.400 | 0.59 |

The engine is faster than roadrunner on every model and every host but wasmtime on the two models whose integration is short (repressilator 1.02 to 1.09, hctz 0.98 to 1.05), where Cranelift's code is 25 to 75 % slower than V8's on the same `engine.wasm`: Node and Chrome agree within 2 % everywhere and are the hosts a browser uses. `demo` is the extreme case, five integration steps of a model of six states, where roadrunner's fixed cost per call dominates and the engine is 10 to 25 times faster.

## The two layers between sbmlsim and the integrator

The numbers above were measured with every host, and the wheel the python bindings ship used the slowest of them: it ran `engine.wasm` in wasmtime. Since this change it runs the native engine, so the column of the wheel is the `native` one, and the two layers which stood between sbmlsim and the integrator were:

1. **The engine of the wheel.** `engine.wasm` in wasmtime against the native engine: 2.1 to 2.3 times on these models, the whole of it in the integrator.
2. **The python layer.** What sbmlsim adds per simulation, which the table below reads off its own benchmark.

### The python layer

sbmlsim's page reported one simulation at its own defaults, which select the solver by the size of the model (`linear_solver="auto"`: KLU above twelve states, so dense for the repressilator and `demo`, KLU for midazolam and HCTZ), and roadrunner 2.12 with its own defaults, which is the KLU column above. Its numbers against the Node column of this file, which is the host of the same engine build:

| model | engine Node ms | sbmlsim core ms | python adds | roadrunner ms | sbmlsim roadrunner ms | python adds |
|---|---:|---:|---:|---:|---:|---:|
| repressilator | 0.192 | 1.38 | 1.19 | 0.274 | 1.02 | 0.75 |
| demo | 0.019 | 0.426 | 0.41 | 0.242 | 0.459 | 0.22 |
| midazolam | 0.324 | 2.03 | 1.71 | 0.736 | 1.46 | 0.72 |
| hctz | 0.406 | 2.34 | 1.93 | 0.677 | 2.00 | 1.32 |

The engine takes 0.08 to 0.70 of roadrunner's time and sbmlsim reported it at 0.93 to 1.35 of roadrunner's. The whole difference was in the python layer, which cost the engine 1.19 to 1.93 ms per simulation against roadrunner's 0.22 to 1.32 ms, and which was 83 to 98 % of what sbmlsim measured for the engine (`demo`: 0.019 ms of engine inside 0.426 ms).

Profiled in sbmlsim (300 simulations of HCTZ, `cProfile`), the python side of one simulation is:

| part | ms |
|---|---:|
| `Instance.simulate`, the engine | 0.55 |
| `LoadedModel.instance()`, a new instance of the model per simulation | 0.33 |
| `state_tolerances`, the vector of absolute tolerances, computed per simulation | 0.10 |
| the plan, the request, the targets of the state and the result | 0.15 |

So two thirds of what sbmlsim adds is work which does not depend on the simulation: the engine is stateless by the contract `simulation/1`, so an instance can be kept per model, and the tolerances and the targets of the state depend only on the model. The scans have the same shape: `LoadedModel.simulate_many` builds a rayon pool and an instance per thread for every chunk, where `LoadedModel.batch_executor` keeps both across calls (measured: 5.1x on a batch of one point, 1.65x on a chunk of 250 points of `demo`, 1.16x on HCTZ).

Both layers were answered: sbmlsim keeps the instance of a model, the vector of its absolute tolerances and the targets of its changes instead of building them per simulation, and uses the persistent batch executor of the bindings for a scan; this repository gives the wheel the native engine. What is left of the python layer is the part which builds a request and reads a table back, which is the cost of being a python library at all.
