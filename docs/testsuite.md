# SBML Test Suite and benchmark

## SBML Test Suite

`sbmlsim-core testsuite testdata/cases` compiles and simulates the semantic cases of the SBML Test Suite and compares them with the baseline `testdata/testsuite_baseline.json`. The cases are prepared first (uv, libsbml):

```bash
uv run scripts/prepare_testsuite.py --out testdata/cases
sbmlsim-core testsuite testdata/cases                          # the analytic Jacobian
sbmlsim-core testsuite testdata/cases --jacobian difference    # the colored difference quotient
sbmlsim-core testsuite testdata/cases --no-optimize            # without the optimizer
sbmlsim-core testsuite testdata/cases --write-baseline         # the baseline from a run
```

`--cases 00001-00100` runs some cases, `--jobs` sets the threads, `--json` writes the result of every case and `--export DIR` writes every passing case for the JavaScript tests, which compare the CSV of Node with the one of wasmtime byte for byte. In every mode each passing case is simulated again to check that it repeats bitwise; a case which passes or fails otherwise than the baseline says fails the run.

1581 of 1789 cases pass, every case which sbmlsim passes among them (the 208 failures are the cases sbmlsim does not pass either). The 208 expected failures are all refused at compilation (`not_compiled`) with a reason in the baseline:

| reason | cases |
|---|---:|
| algebraic rule is not supported | 123 |
| delay csymbol is not supported | 50 |
| not converted: fast reaction (libsbml conversion refuses a fast reaction) | 35 |

CI runs the whole suite in the three modes (the job `testsuite-parity`) and the parity of Node with wasmtime over every passing case (`same_bits_as_wasmtime`).

## Benchmark

`bench/models.toml` lists the models of the benchmark against roadrunner; `bench/RESULTS.md` has the method and the results. A model with `comp`, `distrib` or of another level is prepared first (`scripts/prepare_model.py`, see [Compiling a model](compiling.md#preparing-a-model)):

```bash
uv run scripts/prepare_model.py --bench bench/models.toml        # into target/bench-models/
taskset -c 2 sbmlsim-core bench bench/models.toml --json target/bench/engine.json --export target/bench/export
taskset -c 2 sbmlsim-core bench bench/models.toml --host native --json target/bench/native.json
taskset -c 2 node js/bench.mjs target/bench/export --json target/bench/node.json
node bench/web/serve.mjs target/bench/chrome.json &                # then open http://127.0.0.1:8765/bench/web/index.html
uv run scripts/roadrunner_reference.py bench/models.toml --json target/bench/roadrunner.json
uv run scripts/agreement.py                                        # the engine at rtol 1e-10 against roadrunner at 1e-12 (dense, KLU)
```

The models: two PBPK models of sbmlsim (hctz, 28 states; midazolam, 38), an event model (repeated_dose, an oral dose every 8 h), two machine perfusion models of the liver (mp_core, 43 states; mp_flat, 919) and Chen 2009 of the PEtab benchmark collection (500 states).

- The threshold of the project is at most 2x the simulation time of roadrunner native (LLJIT `O2`), dense and with KLU. No ratio is above it: the largest is 1.38 (the event model, dense, an output of 201 times, wasmtime); the PBPK models take 0.57 to 0.83 of roadrunner's time in wasmtime, Node and Chrome (0.45 to 0.61 natively), Chen and mp_flat 0.16 to 0.97 with KLU and 1.10 to 1.31 dense.
- The native engine (`--host native`: the engine compiled for the machine, the model in wasmtime) is the bound of the integrator without WebAssembly; Node and Chrome agree within a few percent and take 1.0 to 1.3 of its time, wasmtime 1.2 to 1.5.
- The counters of the integrator are the same in wasmtime, Node and Chrome for every configuration (one `engine.wasm`, the same bits), and every timed simulation repeated the bits of the first.
- `KLU_THRESHOLD` is 12: KLU (in the ordering AMD) with the analytic Jacobian is as fast as the dense solver at 11 to 12 states and faster above (0.6 of the dense time at 64 states).
- Agreement: the engine at rtol 1e-10 against roadrunner at rtol 1e-12, within `1e-12 + 1e-7 |c|`, or its largest excess at most 1 % above the one of roadrunner at rtol 1e-10 with the same linear solver. Every model passes with the dense solver and with KLU.

`js/bench.mjs` reads `engine.wasm` from `SBMLSIM_CORE_RUNTIME`, else `js/runtime`. The KLU baseline of roadrunner is a `roadrunner-sparsedq` build (libroadrunner with KLU and its colored sparse difference quotient) when `SBMLSIM_CORE_SPARSEDQ_BUILD` names its `site-packages` directory (or `--sparse-build`); without it `roadrunner_reference.py` takes the KLU of the libroadrunner 2.12 of its environment.
