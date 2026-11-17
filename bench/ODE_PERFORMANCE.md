# ODE performance with algebraic-rule support

DAE support is accepted with the measured ODE performance limits below.
The measurements do not establish zero overhead. The final native comparison measured
9.34% higher median thread CPU time across
24 scenarios (1.0934× the baseline).
23 scenario ratios exceeded 1.0, and `adams/native/ordinary` was
22.17% slower (1.2217×).

The tested WASM scenarios do not establish the zero-margin criterion. No simultaneous interval lies entirely above 1.0. 10 intervals lie below 1.0. 14 intervals include 1.0 and allow a slowdown.
The highest point estimate is 1.0206× for `hctz/wasm/ordinary/dense/analytic/times`.
The largest simultaneous upper bound is 1.0609× for `hctz/wasm/ordinary/dense/analytic/times`, allowing up to 6.09% higher thread CPU time in that case.
This measures warmed ODE requests on the frozen workload matrix and benchmark host.
[Raw samples and frozen inputs](ode-performance-data.json)

## Baseline and feature

| Build | Source commit | Engine WASM bytes | Engine SHA-256 prefix |
| --- | --- | ---: | --- |
| Baseline | `8def5c2` | 726,094 | `d0cacb4ab12c1f98` |
| Feature | `ff3ba9a` | 845,608 | `62cfe6a78c1dcf41` |

The feature keeps the direct CVODE path for ODEs. Its ODE callbacks have no DAE
dispatch, serial-vector access uses the direct SUNDIALS entry point, and event
simulation snapshots state only when output requires it. The engine test run passed
81 tests. The documentation merge at `bbe3d96`
kept the measured production source.

The driver checked output columns, row counts, value bits and all 13 solver counters
for every case in all 32 processes. Both engines used the same model and request
bytes. The experiment used Rust 1.100.0 on Linux
(x86_64), pinned to logical CPU 0
on a 12th Gen Intel Core i7-12700. Hashes identify the release runner, engine and
math library binaries, compiled model bytes and descriptions, job matrix, and
protocol scripts. [Raw samples and frozen inputs](ode-performance-data.json)

## WASM baseline versus feature

Each orientation estimate uses 12 independent processes. Ratios above 1.0 indicate
higher thread CPU time for the feature. The simultaneous bounds cover the full
24-scenario family under the prespecified bootstrap. An interval spanning 1.0
leaves the direction unresolved. Its upper bound gives the size of the slowdown
still compatible with the interval. A zero-width interval records a lack of
variation in the collected sample.

| Model | Request | Solver and output | Forward | Reverse | Ratio | Simultaneous 95% CI |
| --- | --- | --- | ---: | ---: | ---: | ---: |
| Decay (Adams) | Ordinary | Adams, sampled | 0.9722× | 0.9641× | 0.9681× | [0.9548, 0.9816] |
| Decay (Adams) | Prepared | Adams, sampled | 0.9707× | 0.9729× | 0.9718× | [0.9500, 0.9940] |
| Decay (BDF) | Ordinary | Dense analytic, sampled | 0.9626× | 0.9677× | 0.9651× | [0.9550, 0.9753] |
| Decay (BDF) | Ordinary | KLU difference, steps | 0.9736× | 0.9727× | 0.9732× | [0.9500, 0.9969] |
| Decay (BDF) | Prepared | Dense analytic, sampled | 0.9749× | 0.9829× | 0.9789× | [0.9671, 0.9907] |
| Decay (BDF) | Prepared | KLU difference, steps | 0.9814× | 0.9681× | 0.9747× | [0.9619, 0.9878] |
| Repeated dose with events | Ordinary | Dense analytic, sampled | 0.9970× | 1.0003× | 0.9987× | [0.9839, 1.0137] |
| Repeated dose with events | Ordinary | KLU difference, steps | 0.9901× | 0.9813× | 0.9857× | [0.9775, 0.9939] |
| Repeated dose with events | Prepared | Dense analytic, sampled | 0.9889× | 0.9865× | 0.9877× | [0.9674, 1.0085] |
| Repeated dose with events | Prepared | KLU difference, steps | 0.9730× | 0.9994× | 0.9861× | [0.9598, 1.0131] |
| Hydrochlorothiazide PBPK | Ordinary | Dense analytic, sampled | 1.0367× | 1.0047× | 1.0206× | [0.9818, 1.0609] |
| Hydrochlorothiazide PBPK | Ordinary | KLU difference, steps | 1.0024× | 1.0045× | 1.0034× | [0.9801, 1.0273] |
| Hydrochlorothiazide PBPK | Prepared | Dense analytic, sampled | 1.0068× | 1.0233× | 1.0150× | [0.9942, 1.0363] |
| Hydrochlorothiazide PBPK | Prepared | KLU difference, steps | 1.0053× | 1.0070× | 1.0061× | [0.9897, 1.0229] |
| Decay (zero duration) | Ordinary | Single output | 1.0007× | 1.0190× | 1.0098× | [0.9909, 1.0291] |
| Decay (zero duration) | Prepared | Single output | 0.9930× | 0.9917× | 0.9924× | [0.9591, 1.0268] |
| Midazolam PBPK | Ordinary | Dense analytic, sampled | 0.9949× | 1.0209× | 1.0078× | [0.9805, 1.0358] |
| Midazolam PBPK | Ordinary | KLU difference, steps | 1.0102× | 1.0254× | 1.0178× | [0.9977, 1.0382] |
| Midazolam PBPK | Prepared | Dense analytic, sampled | 0.9985× | 1.0208× | 1.0096× | [0.9806, 1.0395] |
| Midazolam PBPK | Prepared | KLU difference, steps | 1.0040× | 1.0035× | 1.0038× | [0.9940, 1.0136] |
| Robertson | Ordinary | Dense analytic, sampled | 0.9816× | 0.9742× | 0.9779× | [0.9649, 0.9911] |
| Robertson | Ordinary | KLU difference, steps | 0.9735× | 0.9730× | 0.9732× | [0.9539, 0.9929] |
| Robertson | Prepared | Dense analytic, sampled | 0.9986× | 0.9828× | 0.9907× | [0.9611, 1.0212] |
| Robertson | Prepared | KLU difference, steps | 0.9643× | 0.9552× | 0.9598× | [0.9390, 0.9810] |

## Native baseline versus feature

Four fresh processes ran baseline, feature, feature, and baseline in order.
Both executables used identical release build flags and benchmark code for batch
measurement. The rebuilt baseline example's embedded WASM engine matched the
frozen baseline engine SHA-256. For each scenario, the ratio divides the feature
median of 18 samples by the baseline median of 18 samples, in thread CPU time.

In 20 native scenarios, both feature process medians exceeded both
baseline process medians. Result bit hashes and all 13 solver counters matched
across all four rounds. The native comparisons are descriptive. [Raw native
measurements and build manifest](ode-performance-data.json)

## Identical-engine controls

The eight baseline-versus-baseline processes were analyzed as a separate family.
The identical-engine interval for `events/wasm/ordinary/klu/difference_quotient/steps` excludes 1.0 and flags that case for investigation. These controls diagnose order or measurement effects.

| Model | Request | Solver and output | Forward | Reverse | Ratio | Simultaneous 95% CI |
| --- | --- | --- | ---: | ---: | ---: | ---: |
| Decay (Adams) | Ordinary | Adams, sampled | 1.0034× | 1.0080× | 1.0057× | [0.9893, 1.0224] |
| Decay (Adams) | Prepared | Adams, sampled | 0.9960× | 1.0061× | 1.0011× | [0.9915, 1.0107] |
| Decay (BDF) | Ordinary | Dense analytic, sampled | 0.9995× | 0.9953× | 0.9974× | [0.9903, 1.0044] |
| Decay (BDF) | Ordinary | KLU difference, steps | 1.0025× | 0.9939× | 0.9982× | [0.9879, 1.0086] |
| Decay (BDF) | Prepared | Dense analytic, sampled | 0.9955× | 1.0021× | 0.9988× | [0.9915, 1.0061] |
| Decay (BDF) | Prepared | KLU difference, steps | 1.0251× | 0.9908× | 1.0078× | [0.9748, 1.0420] |
| Repeated dose with events | Ordinary | Dense analytic, sampled | 1.0097× | 0.9983× | 1.0039× | [0.9847, 1.0236] |
| Repeated dose with events | Ordinary | KLU difference, steps | 1.0049× | 1.0128× | 1.0089× | [1.0006, 1.0173] |
| Repeated dose with events | Prepared | Dense analytic, sampled | 0.9740× | 0.9958× | 0.9848× | [0.9612, 1.0090] |
| Repeated dose with events | Prepared | KLU difference, steps | 1.0034× | 1.0028× | 1.0031× | [0.9909, 1.0155] |
| Hydrochlorothiazide PBPK | Ordinary | Dense analytic, sampled | 0.9984× | 0.9953× | 0.9969× | [0.9894, 1.0044] |
| Hydrochlorothiazide PBPK | Ordinary | KLU difference, steps | 0.9692× | 1.0015× | 0.9852× | [0.9395, 1.0331] |
| Hydrochlorothiazide PBPK | Prepared | Dense analytic, sampled | 0.9689× | 1.0061× | 0.9873× | [0.9537, 1.0222] |
| Hydrochlorothiazide PBPK | Prepared | KLU difference, steps | 0.9983× | 0.9964× | 0.9973× | [0.9877, 1.0071] |
| Decay (zero duration) | Ordinary | Single output | 1.0020× | 0.9959× | 0.9989× | [0.9875, 1.0104] |
| Decay (zero duration) | Prepared | Single output | 0.9946× | 1.0012× | 0.9979× | [0.9870, 1.0088] |
| Midazolam PBPK | Ordinary | Dense analytic, sampled | 0.9666× | 1.0334× | 0.9994× | [0.9494, 1.0522] |
| Midazolam PBPK | Ordinary | KLU difference, steps | 0.9878× | 1.0026× | 0.9952× | [0.9869, 1.0035] |
| Midazolam PBPK | Prepared | Dense analytic, sampled | 1.0072× | 1.0169× | 1.0120× | [0.9915, 1.0330] |
| Midazolam PBPK | Prepared | KLU difference, steps | 1.0011× | 0.9974× | 0.9993× | [0.9913, 1.0072] |
| Robertson | Ordinary | Dense analytic, sampled | 1.0072× | 1.0079× | 1.0076× | [0.9755, 1.0407] |
| Robertson | Ordinary | KLU difference, steps | 0.9990× | 1.0085× | 1.0037× | [0.9869, 1.0209] |
| Robertson | Prepared | Dense analytic, sampled | 1.0009× | 1.0561× | 1.0281× | [0.9718, 1.0877] |
| Robertson | Prepared | KLU difference, steps | 0.9886× | 1.0011× | 0.9948× | [0.9880, 1.0017] |

## Method and scope

The frozen schedule interleaves three comparison processes and one control process
in each of eight groups. Seed 29 sets the schedule and the case order in each
process. Every process runs all 24 cases after warm-up, with three paired
baseline, feature, feature, baseline blocks and the engine order reversed in
half the processes. In a control process, both positions use the baseline engine.

For each case and process, the estimate is the median of the three log ratios
of the summed feature and baseline times. The analysis averages process effects
within each orientation, weights both orientations equally, and exponentiates
the estimates and confidence bounds. It resamples complete process vectors
50,000 times within orientations and uses the 95th percentile of the maximum
absolute standardized effect across cases for the simultaneous interval.
[Confirmation protocol](ode-confirmation/confirmation-protocol.md)

The data archives earlier native and WASM batch and paired runs
without pooling them with confirmation. Uncompressed WASM size and uncached
compilation time are separate from warmed simulation timings, with the earlier
cold samples archived. [Raw samples and frozen inputs](ode-performance-data.json)

The archived input hashes refer to the benchmark scripts at commit `140bd0c`.
The published scripts have since received lint and formatting fixes; they reproduce
the frozen trial schedule and analysis results exactly.

## Reproducing the comparison

Copy the [benchmark example](../crates/host/examples/ode_performance.rs) and its
`[[example]]` entry in `crates/host/Cargo.toml` to the baseline checkout. Build
it in both checkouts with the same release flags and native feature. Set the
same `WASI_SDK_PATH` in both builds, or supply each checkout's verified engine
artifacts through `SBMLSIM_CORE_WASM_DIR`.

```sh
cargo build --release -p sbmlsim-host --features native --example ode_performance
```

Prepare one `model.wasm` and `model.json` per model under `target/ode-models`.
The data includes the exact requests, model checksums, SBML source locations, and
the Robertson model. The Linux confirmation uses `taskset`, the thread CPU
clock, and NumPy for analysis. [Model preparation](RESULTS.md)

```sh
jq '.scenarios' bench/ode-performance-data.json > target/ode-models/jobs.json
"$BASELINE_EXAMPLE" runtime target/ode-baseline-runtime
"$FEATURE_EXAMPLE" runtime target/ode-feature-runtime

python3 bench/ode-confirmation/confirm_ode.py run \
    --jobs target/ode-models/jobs.json \
    --baseline-runtime target/ode-baseline-runtime \
    --feature-runtime target/ode-feature-runtime \
    --binary "$FEATURE_EXAMPLE" --cpu 0 --out target/ode-confirmation
python3 bench/ode-confirmation/confirm_ode_stats.py target/ode-confirmation \
    > target/ode-confirmation/result.json
```

For the native comparison, point `BASELINE_EXAMPLE` and `FEATURE_EXAMPLE` at
the two release binaries and use the same job matrix and CPU. The batch output
includes both backends. The native summary uses the cases labeled `native`.

```sh
export SBMLSIM_ODE_BENCH_CACHE="$PWD/target/ode-bench-cache"
taskset -c 0 "$BASELINE_EXAMPLE" target/ode-models/jobs.json > native-0-baseline.json
taskset -c 0 "$FEATURE_EXAMPLE" target/ode-models/jobs.json > native-1-feature.json
taskset -c 0 "$FEATURE_EXAMPLE" target/ode-models/jobs.json > native-2-feature.json
taskset -c 0 "$BASELINE_EXAMPLE" target/ode-models/jobs.json > native-3-baseline.json
```
