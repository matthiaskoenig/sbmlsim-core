# Rejected output recorder experiment

The recorder change was reverted after prepared native Adams slowed by 22.35%. The retained implementation corresponds to the first completed comparison at 019e226. It still has measured ordinary ODE slowdowns, up to 3.90% on native and 2.30% on WASM. The results below describe the rejected recorder build. [Retained implementation results](results-019e226.md)

The follow-up build at f9988f3 recorded 15 slowdowns, ten speedups and 23 inconclusive results across the 48 ordinary ODE workloads. The strict 1.000 criterion failed. The first completed build at 019e226 had ten slowdowns, seven speedups and 31 inconclusive results. Each build was independently compared with the same baseline. [First completed report](results-019e226.md) · [Follow-up protocol](followup-protocol.md)

**Native.** Of 24 scenarios, 11 had a simultaneous 95% interval entirely above 1.000, 0 entirely below, and 13 spanning 1.000. The median of the 24 point estimates was 1.0317×. Individual estimates ranged from 0.9801× (initial-only / ordinary) to 1.2235× (adams / prepared). One sham interval excluded 1.000.

**WASM.** Of 24 scenarios, 4 had a simultaneous 95% interval entirely above 1.000, 10 entirely below, and 10 spanning 1.000. The median of the 24 point estimates was 0.9866×. Individual estimates ranged from 0.9408× (midazolam / prepared / klu / DQ / steps) to 1.0300× (events / ordinary / dense / analytic / times). 2 sham intervals excluded 1.000.

The experiment did not meet the prespecified zero-overhead criterion. A ratio above 1 means the feature took longer. Intervals crossing 1 leave the direction unresolved. Sham results flag measurement variation and were retained without adjustment. The estimates apply to these warmed workloads on this machine.

Both native Adams requests were slower in both orientations. Ordinary Adams took 21.46% longer (simultaneous 95% interval 19.57% to 23.38%) and prepared Adams took 22.35% longer (20.41% to 24.31%). Native dense decay and dense event scenarios also slowed in both ordinary and prepared calls. The four WASM slowdowns affected ordinary and prepared Adams and dense decay. Of the inconclusive comparisons, prepared native decay with KLU had the highest upper bound, allowing a 7.40% slowdown.

One native control (prepared dense hydrochlorothiazide) and two WASM controls (ordinary midazolam, both solvers) had intervals entirely below one. All 15 measured slowdowns had matching sham intervals containing one. The two completed experiments do not directly pair the two feature drivers, so they quantify each build against the baseline only.

The follow-up changed row recording to write directly into reserved output space and combined selection evaluation with its error check. Native disassembly confirms one virtual call in the row function compared with two in the first feature build. The follow-up function occupies 726 bytes and contains 165 static instructions, compared with 657 bytes and 160 instructions previously. The combined method still calls WasmModel::selections through a fixed target, saves three registers and reserves 48 bytes of stack. Those counts do not establish which operation caused the slowdown.

Each host ran 24 scenarios in 32 trials on logical CPU 0 of an Intel Core i7-12700. Each trial ran all 24 scenarios. The 24 baseline/feature trials and eight identical-baseline sham trials balanced forward and reverse order. WASM compared instances within a process in three ABBA blocks. Native compared separate pinned executable processes using medians of nine thread CPU measurements per case. Both hosts used identical model bytes and requests. The orientation-stratified bootstrap used whole trial vectors and 50,000 draws for simultaneous intervals over all 48 scenarios. [Frozen protocol](protocol.md) · [Follow-up protocol](followup-protocol.md)

The WASM driver verified identical columns, row counts, every output value bit, and all 18 solver counters. Every native scenario reproduced the same 64-bit fingerprint of trajectory value bits and all 18 counters across 64 processes. A fingerprint collision is possible. Prepared and ordinary runs each have their own signature.

The baseline and feature compilers produced byte-identical model.wasm files and compact model.json descriptions for all 1,701 previously compiled semantic cases. This comparison covers 3,402 artifacts. [Artifact manifest](model-artifacts.json)

## Native results

AB compares the feature with the baseline. AA compares the baseline with itself. Forward and reverse columns report order-specific ratios normalized to feature/baseline. DQ denotes the colored difference quotient.

| Scenario | AB ratio | AB simultaneous 95% | AB forward / reverse | AA ratio | AA simultaneous 95% | AA forward / reverse |
|---|---:|---:|---:|---:|---:|---:|
| adams / ordinary | 1.2146 | 1.1957 to 1.2338 | 1.2008 / 1.2286 | 0.9917 | 0.9735 to 1.0103 | 0.9905 / 0.9930 |
| adams / prepared | 1.2235 | 1.2041 to 1.2431 | 1.2151 / 1.2319 | 1.0405 | 0.9594 to 1.1285 | 1.0047 / 1.0776 |
| decay / ordinary / dense / analytic / times | 1.1422 | 1.1221 to 1.1627 | 1.1304 / 1.1541 | 0.9880 | 0.9685 to 1.0079 | 0.9866 / 0.9894 |
| decay / ordinary / klu / DQ / steps | 1.0402 | 1.0300 to 1.0506 | 1.0410 / 1.0394 | 0.9853 | 0.9667 to 1.0042 | 0.9826 / 0.9880 |
| decay / prepared / dense / analytic / times | 1.1221 | 1.0736 to 1.1727 | 1.0995 / 1.1451 | 1.0002 | 0.9829 to 1.0178 | 0.9982 / 1.0022 |
| decay / prepared / klu / DQ / steps | 1.0356 | 0.9985 to 1.0740 | 1.0311 / 1.0400 | 0.9949 | 0.9784 to 1.0117 | 0.9937 / 0.9962 |
| events / ordinary / dense / analytic / times | 1.0944 | 1.0702 to 1.1192 | 1.0897 / 1.0991 | 1.0022 | 0.9896 to 1.0150 | 0.9876 / 1.0170 |
| events / ordinary / klu / DQ / steps | 1.0189 | 0.9952 to 1.0430 | 1.0224 / 1.0153 | 0.9998 | 0.9819 to 1.0181 | 1.0111 / 0.9887 |
| events / prepared / dense / analytic / times | 1.1051 | 1.0853 to 1.1252 | 1.0994 / 1.1108 | 0.9916 | 0.9823 to 1.0011 | 0.9965 / 0.9868 |
| events / prepared / klu / DQ / steps | 1.0304 | 1.0094 to 1.0519 | 1.0215 / 1.0395 | 0.9988 | 0.9822 to 1.0157 | 1.0091 / 0.9886 |
| hctz / ordinary / dense / analytic / times | 1.0044 | 0.9922 to 1.0168 | 1.0018 / 1.0070 | 0.9987 | 0.9815 to 1.0162 | 1.0121 / 0.9854 |
| hctz / ordinary / klu / DQ / steps | 0.9953 | 0.9762 to 1.0149 | 0.9969 / 0.9938 | 1.0062 | 0.9142 to 1.1073 | 1.0589 / 0.9561 |
| hctz / prepared / dense / analytic / times | 0.9955 | 0.9755 to 1.0160 | 0.9960 / 0.9951 | 0.9820 | 0.9667 to 0.9975 | 0.9856 / 0.9784 |
| hctz / prepared / klu / DQ / steps | 1.0053 | 0.9926 to 1.0181 | 1.0061 / 1.0045 | 0.9692 | 0.8989 to 1.0451 | 0.9447 / 0.9944 |
| initial-only / ordinary | 0.9801 | 0.9555 to 1.0054 | 0.9869 / 0.9734 | 1.0084 | 0.9714 to 1.0469 | 0.9871 / 1.0303 |
| initial-only / prepared | 1.0111 | 0.9709 to 1.0529 | 1.0027 / 1.0195 | 1.0184 | 0.9629 to 1.0770 | 1.0414 / 0.9959 |
| midazolam / ordinary / dense / analytic / times | 0.9805 | 0.9590 to 1.0026 | 0.9885 / 0.9727 | 0.9905 | 0.9637 to 1.0180 | 0.9835 / 0.9975 |
| midazolam / ordinary / klu / DQ / steps | 0.9957 | 0.9807 to 1.0110 | 1.0001 / 0.9914 | 0.9922 | 0.9734 to 1.0114 | 1.0011 / 0.9834 |
| midazolam / prepared / dense / analytic / times | 0.9838 | 0.9612 to 1.0070 | 0.9817 / 0.9860 | 1.0077 | 0.9956 to 1.0199 | 1.0082 / 1.0072 |
| midazolam / prepared / klu / DQ / steps | 0.9919 | 0.9630 to 1.0216 | 0.9878 / 0.9960 | 1.0025 | 0.9824 to 1.0229 | 0.9858 / 1.0194 |
| robertson / ordinary / dense / analytic / times | 1.0329 | 0.9999 to 1.0670 | 1.0245 / 1.0414 | 0.9961 | 0.9809 to 1.0115 | 0.9821 / 1.0102 |
| robertson / ordinary / klu / DQ / steps | 1.0503 | 1.0368 to 1.0640 | 1.0490 / 1.0517 | 1.0087 | 0.9986 to 1.0190 | 1.0153 / 1.0022 |
| robertson / prepared / dense / analytic / times | 1.0463 | 1.0316 to 1.0613 | 1.0452 / 1.0475 | 1.0116 | 0.9965 to 1.0269 | 1.0065 / 1.0166 |
| robertson / prepared / klu / DQ / steps | 1.0531 | 1.0419 to 1.0644 | 1.0545 / 1.0516 | 0.9996 | 0.9904 to 1.0088 | 1.0043 / 0.9949 |

## WASM results

AB compares the feature with the baseline. AA compares the baseline with itself. Forward and reverse columns report order-specific ratios normalized to feature/baseline. DQ denotes the colored difference quotient.

| Scenario | AB ratio | AB simultaneous 95% | AB forward / reverse | AA ratio | AA simultaneous 95% | AA forward / reverse |
|---|---:|---:|---:|---:|---:|---:|
| adams / ordinary | 1.0241 | 1.0151 to 1.0333 | 1.0245 / 1.0237 | 1.0026 | 0.9935 to 1.0118 | 0.9933 / 1.0121 |
| adams / prepared | 1.0189 | 1.0119 to 1.0259 | 1.0201 / 1.0177 | 1.0000 | 0.9819 to 1.0184 | 1.0009 / 0.9991 |
| decay / ordinary / dense / analytic / times | 1.0213 | 1.0119 to 1.0308 | 1.0225 / 1.0201 | 0.9996 | 0.9892 to 1.0102 | 1.0002 / 0.9991 |
| decay / ordinary / klu / DQ / steps | 0.9862 | 0.9806 to 0.9919 | 0.9888 / 0.9837 | 0.9983 | 0.9910 to 1.0056 | 0.9973 / 0.9992 |
| decay / prepared / dense / analytic / times | 1.0182 | 1.0086 to 1.0279 | 1.0172 / 1.0191 | 1.0022 | 0.9914 to 1.0132 | 1.0042 / 1.0003 |
| decay / prepared / klu / DQ / steps | 0.9956 | 0.9783 to 1.0132 | 0.9928 / 0.9984 | 0.9845 | 0.9525 to 1.0176 | 0.9834 / 0.9857 |
| events / ordinary / dense / analytic / times | 1.0300 | 0.9959 to 1.0653 | 1.0341 / 1.0259 | 0.9906 | 0.9720 to 1.0096 | 0.9988 / 0.9825 |
| events / ordinary / klu / DQ / steps | 0.9772 | 0.9675 to 0.9870 | 0.9764 / 0.9781 | 1.0042 | 0.9950 to 1.0135 | 1.0047 / 1.0037 |
| events / prepared / dense / analytic / times | 1.0084 | 0.9967 to 1.0202 | 1.0114 / 1.0054 | 0.9791 | 0.9539 to 1.0050 | 0.9769 / 0.9813 |
| events / prepared / klu / DQ / steps | 0.9760 | 0.9702 to 0.9818 | 0.9770 / 0.9750 | 0.9980 | 0.9919 to 1.0042 | 0.9956 / 1.0005 |
| hctz / ordinary / dense / analytic / times | 0.9860 | 0.9796 to 0.9924 | 0.9917 / 0.9803 | 0.9990 | 0.9882 to 1.0098 | 0.9976 / 1.0004 |
| hctz / ordinary / klu / DQ / steps | 0.9647 | 0.9551 to 0.9744 | 0.9692 / 0.9602 | 0.9989 | 0.9913 to 1.0065 | 1.0043 / 0.9935 |
| hctz / prepared / dense / analytic / times | 0.9870 | 0.9678 to 1.0066 | 0.9800 / 0.9942 | 1.0022 | 0.9978 to 1.0066 | 0.9963 / 1.0082 |
| hctz / prepared / klu / DQ / steps | 0.9572 | 0.9469 to 0.9675 | 0.9614 / 0.9530 | 1.0007 | 0.9910 to 1.0105 | 0.9997 / 1.0017 |
| initial-only / ordinary | 0.9828 | 0.9645 to 1.0015 | 0.9838 / 0.9819 | 1.0067 | 0.9729 to 1.0416 | 1.0227 / 0.9910 |
| initial-only / prepared | 0.9991 | 0.9708 to 1.0283 | 0.9933 / 1.0050 | 1.0242 | 0.9859 to 1.0640 | 1.0523 / 0.9968 |
| midazolam / ordinary / dense / analytic / times | 0.9763 | 0.9680 to 0.9848 | 0.9803 / 0.9724 | 0.9923 | 0.9877 to 0.9970 | 0.9893 / 0.9954 |
| midazolam / ordinary / klu / DQ / steps | 0.9462 | 0.9358 to 0.9566 | 0.9508 / 0.9415 | 0.9948 | 0.9897 to 1.0000 | 0.9933 / 0.9963 |
| midazolam / prepared / dense / analytic / times | 0.9735 | 0.9674 to 0.9796 | 0.9714 / 0.9756 | 0.9978 | 0.9914 to 1.0042 | 0.9997 / 0.9960 |
| midazolam / prepared / klu / DQ / steps | 0.9408 | 0.9307 to 0.9510 | 0.9421 / 0.9395 | 1.0002 | 0.9929 to 1.0075 | 1.0039 / 0.9965 |
| robertson / ordinary / dense / analytic / times | 0.9956 | 0.9794 to 1.0120 | 0.9902 / 1.0010 | 1.0038 | 0.9963 to 1.0114 | 0.9981 / 1.0095 |
| robertson / ordinary / klu / DQ / steps | 0.9914 | 0.9787 to 1.0042 | 0.9941 / 0.9887 | 0.9919 | 0.9692 to 1.0152 | 1.0067 / 0.9773 |
| robertson / prepared / dense / analytic / times | 1.0063 | 0.9903 to 1.0226 | 1.0039 / 1.0087 | 0.9923 | 0.9780 to 1.0069 | 0.9900 / 0.9946 |
| robertson / prepared / klu / DQ / steps | 0.9858 | 0.9628 to 1.0094 | 0.9898 / 0.9819 | 0.9962 | 0.9897 to 1.0027 | 0.9933 / 0.9991 |

## Reproduction and provenance

The baseline commit was `d904f63c03d1c99d05e75ffdfec90b2c226db8be`. The feature source corresponds to commit `f9988f3ff9a3385a862f61d774e33d24bfead32c`. SHA-256 identifies the exact executable and runtime measured.

| Artifact | Baseline SHA-256 | Feature SHA-256 |
|---|---|---|
| Native driver | `1a33bd9e8d98b5b1eff2fe6ff1007f48e460bc40bb2748c8fad0309fe6cdc0c5` | `b5002eba158d14f033b69a7f0f6379d3e72b5905da6696d732c05de9b9cbbf9a` |
| WASM engine | `1391125a9dd353e3d0ad10e1015e3156a0ca8c137513ad3beda5770442dd7be6` | `9485d1654b74452bce74c9574f65cc199bbe43bb60648545592380a7afb14972` |
| Math library | `8f725c0b975e7b2c43a7bd002d4d298c104ef34927e9d669e73e76771bf62a8c` | `8f725c0b975e7b2c43a7bd002d4d298c104ef34927e9d669e73e76771bf62a8c` |

An earlier driver completed 11 trials before timing stopped for correctness fixes. Those trials are archived separately and excluded from all estimates. [Interrupted observations](interrupted-835947f4.json.gz)

The archived observations include all native CPU and wall samples, WASM paired CPU samples, orders, requests, checksums and confidence intervals. The frozen source model hashes and all five baseline-compiled model hashes are in the manifest. [All observations](timings.json.gz) · [Summary and signatures](summary.json) · [Baseline manifest](baseline.json) · [Measurement script](../../scripts/dde_performance.py)

Reproduction requires the Rust and WASI build tools used by the project, Python with NumPy, and a Linux host with CPU 0 available. In a clean feature checkout, use the following commands. Set `WASI_SDK_PATH` to the installed SDK. The first worktree supplies the baseline compiler and driver. The feature checkout supplies the benchmark protocol and the comparison driver.

```sh
git worktree add --detach ../sbmlsim-dde-baseline d904f63
mkdir -p target/dde-baseline
(cd ../sbmlsim-dde-baseline && cargo build --release -p sbmlsim-cli --features native --bin sbmlsim-core)
(cd ../sbmlsim-dde-baseline && cargo build --release -p sbmlsim-host --features native --example ode_performance)
cp ../sbmlsim-dde-baseline/target/release/sbmlsim-core target/dde-baseline/
cp ../sbmlsim-dde-baseline/target/release/examples/ode_performance target/dde-baseline/
target/dde-baseline/ode_performance runtime target/dde-baseline/runtime
uv run scripts/prepare_model.py --bench bench/models.toml
python3 scripts/dde_performance.py freeze --cpu 0
cargo build --release -p sbmlsim-host --features native --example ode_performance
mkdir -p target/dde-feature-reproduction
cp target/release/examples/ode_performance target/dde-feature-reproduction/
target/dde-feature-reproduction/ode_performance runtime target/dde-feature-reproduction/runtime
.venv/bin/python scripts/dde_performance.py measure --feature target/dde-feature-reproduction/ode_performance --results target/dde-perf-results-final
```

The prepared midazolam and hydrochlorothiazide source files came from sbmlsim 0.29.0. Compare source hashes and the schedule with the baseline manifest before interpreting a repeat run. A new build can have a different executable hash. The reported hashes identify the binaries used for these results.
