# GMRES on the machine perfusion model

GMRES completes the 919-state liver perfusion model with both analytic and colored
difference-quotient preconditioners. On this model, KLU remains faster: GMRES takes
1.29-1.32 times the native time and 1.42-1.43 times the Wasmtime time. Automatic solver
selection therefore keeps the existing dense/KLU policy.

## Implementation

Set `settings.linear_solver` to `"gmres"` for CVODE BDF with Newton iteration,
matrix-free directional differences for Jacobian-vector products, and left ILU(0)
preconditioning. Only the preconditioner assembles the sparse model Jacobian.
The `jacobian` setting selects its analytic or colored difference-quotient values;
nonfinite analytic derivatives retain the existing difference-quotient fallback.

The Krylov subspace contains at most 30 vectors, with two restarts per linear solve.
ILU(0) retains the model pattern plus the diagonal, without fill or pivoting. Symbolic
storage persists between simulations; each segment invalidates the saved numeric
Jacobian. A failed pivot is recoverable and lets CVODE refresh the Jacobian or reduce
the step. Algebraic-rule models continue to use IDA with direct solvers. Models
without states retain the existing zero-rate integration path for events.

The engine uses the vendored SUNDIALS 7.9 `SUNLinSol_SPGMR` implementation and public
CVODE preconditioner callbacks. The [SUNDIALS manual](https://sundials.readthedocs.io/en/latest/sunlinsol/SUNLinSol_links.html#the-sunlinsol-spgmr-module)
describes the scaled, preconditioned GMRES solver. Five new counters expose linear
iterations, linear convergence failures, Jacobian-vector evaluations, preconditioner
evaluations and preconditioner solves. The private host/engine output format is 2
because the statistics buffer grew; the `model/2` and `simulation/1` contracts are
unchanged. Hosts and `engine.wasm` must be upgraded together.

## Timing

Measured on 2026-11-18 with a Core i7-12700, pinned to logical CPU 2, Rust 1.100.0,
release builds and the existing deterministic C build flags. The workload is
`perfusion` from `bench/sbmlsim.toml`: 919 states, 4,242 Jacobian entries, 60 minutes,
101 output rows, rtol 1e-6 and the model's per-state absolute tolerances with base
1e-10. Both hosts call the compiled model through Wasmtime; the native host runs
the integrator natively.

Each configuration runs seven timed simulations after two warmups in each of three
rounds. Configurations take turns in a fixed order within each round. The table gives
the median of the three round medians, excluding compilation and instantiation.
Every timed simulation repeats bitwise within its configuration. These are
descriptive measurements from one machine, rather than a general crossover claim.

| Solver | Preconditioner/Jacobian | Native ms | Wasmtime ms |
| --- | --- | ---: | ---: |
| KLU | Analytic | 37.10 | 68.11 |
| KLU | Colored difference | 36.50 | 67.77 |
| GMRES | Analytic ILU(0) | 47.86 | 96.53 |
| GMRES | Colored difference ILU(0) | 48.02 | 96.88 |

| Configuration | Steps | Nonlinear iterations | Total RHS calls | Jacobian evaluations | Krylov iterations | Preconditioner solves |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| KLU analytic | 1,158 | 1,726 | 1,729 | 24 | 0 | 0 |
| KLU difference | 1,161 | 1,650 | 1,917 | 22 | 0 | 0 |
| GMRES analytic | 1,047 | 1,259 | 3,402 | 18 | 2,140 | 3,349 |
| GMRES difference | 1,046 | 1,250 | 3,621 | 19 | 2,140 | 3,337 |

Both GMRES configurations report zero linear convergence failures. They require fewer
steps and nonlinear iterations than KLU, but their 2,140 directional differences add
RHS calls. These counts suggest that the cost of matrix-vector products and Krylov
operations outweighs the savings from incomplete factorization for this sparse model.
The native and Wasmtime counters match for every measured configuration.

## Accuracy

`scripts/gmres_agreement.py` compares all 919 state selections at the 101 output times
against analytic KLU at rtol 1e-13 and absolute tolerances 1e-5 times the benchmark
vector. The fixed pointwise criterion is
`|value - reference| <= 1e-12 + 1e-7 |reference|`.
The ratio below divides the observed error by that allowed error; values at most 1
pass. Absolute tolerances scale with the benchmark vector by 1, 1e-2 and 1e-4 for
the three rows respectively.

| Relative tolerance | KLU analytic maximum ratio | GMRES analytic maximum ratio | GMRES difference maximum ratio |
| --- | ---: | ---: | ---: |
| 1e-6 | 20,560.9 | 11,721.2 | 8,110.6 |
| 1e-10 | 4.879 | 5.318 | 4.607 |
| 1e-12 | 0.04080 | 0.02009 | 0.02193 |

At rtol 1e-12, every sampled value passes for all three configurations. GMRES's
largest absolute differences from the tighter KLU reference are 1.67e-12 and
1.88e-12 for analytic and difference preconditioning respectively. This establishes
sampled agreement with the reference, rather than a certified error bound.

At rtol 1e-10, KLU and the two GMRES configurations have 5, 8 and 6 values outside
the strict criterion. The worst value is `pyr_z1_cell` near 55.2 minutes. At the
benchmark's rtol 1e-6, none meets this much stricter comparison criterion, although
GMRES has smaller maximum errors than KLU. Local integrator tolerances do not bound
accumulated pointwise error; tighter tolerances are needed for this accuracy target.
The raw report retains all failures at the looser tolerances.

For both GMRES preconditioners at rtol 1e-12, Node and Wasmtime produce identical CSV
bytes on the perfusion model. The rebuilt Python native engine also produces the
same floating-point bits in these two runs. Native/WASM bitwise equality is not a
general host guarantee.

## Reproduction

```bash
export WASI_SDK_PATH="$HOME/opt/wasi-sdk"
cargo build --release -p sbmlsim-cli --features native
# Prepare once if the model is absent; needs the sbmlsim checkout from the TOML.
uv run scripts/prepare_model.py --bench bench/sbmlsim.toml

taskset -c 2 target/release/sbmlsim-core bench bench/sbmlsim.toml \
  --only perfusion --solvers klu,gmres --rounds 3 --warmups 2 --reps 7 \
  --compile-reps 1 --export target/bench-gmres \
  --json target/bench-gmres/wasmtime.json
taskset -c 2 target/release/sbmlsim-core bench bench/sbmlsim.toml \
  --only perfusion --solvers klu,gmres --rounds 3 --warmups 2 --reps 7 \
  --compile-reps 1 --host native --json target/bench-gmres/native.json
python3 scripts/gmres_agreement.py target/bench-gmres/perfusion \
  --json target/bench-gmres/agreement.json
```

[Raw measurements](gmres-data.json) include round medians, all counters, accuracy
checks, model and implementation hashes, runtime hash and source checkout revision.
The agreement script exits nonzero if any rtol 1e-12 run fails the criterion;
the lower-accuracy runs remain diagnostic results.

Validation covers Robertson accuracy, analytic fallback, unsorted sparsity patterns,
missing diagonals, recoverable pivots, repeated/prepared simulations, changes,
delayed events, stateless events, explicit DAE/nonstiff refusal, Python bindings and
Node/Wasmtime agreement. Native and WASM engine tests, the full Rust and Python
suites, the 63-test Node suite, clippy, formatting and type checks pass.
