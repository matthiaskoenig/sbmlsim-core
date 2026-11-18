# Ordinary ODE performance after adding delay support

**Native.** Of 24 scenarios, 7 had a simultaneous 95% interval entirely above 1.000, 2 entirely below, and 15 spanning 1.000. The median of the 24 point estimates was 1.0053×. Individual estimates ranged from 0.9751× (decay / prepared / dense / analytic / times) to 1.0390× (robertson / prepared / klu / DQ / steps). One sham interval excluded 1.000.

**WASM.** Of 24 scenarios, 3 had a simultaneous 95% interval entirely above 1.000, 5 entirely below, and 16 spanning 1.000. The median of the 24 point estimates was 0.9984×. Individual estimates ranged from 0.9778× (midazolam / ordinary / klu / DQ / steps) to 1.0230× (adams / prepared). 2 sham intervals excluded 1.000.

The experiment did not meet the prespecified zero-overhead criterion. A ratio above 1 means the feature took longer. Intervals crossing 1 leave the direction unresolved. Sham results flag measurement variation and were retained without adjustment. The estimates apply to these warmed workloads on this machine.

Ten scenarios had measured slowdowns, seven had speedups, and 31 remained inconclusive. The widest upper bound among the inconclusive scenarios allowed a 7.81% slowdown for prepared Robertson with KLU and difference quotients on WASM.

The seven native slowdowns came from all four midazolam requests and three of the four Robertson requests. Each had a ratio above one in both orders and a matching sham interval containing one. The largest estimate was 3.90% for prepared Robertson with KLU and difference quotients (simultaneous interval 2.40% to 5.44%). On WASM, prepared Adams slowed by 2.30%, ordinary dense decay by 2.08%, and prepared dense decay by 2.02%. All three had ratios above one in both orders. The prepared dense decay sham also showed an order-normalized difference, with an interval wholly below one. Two other shams flagged differences for native prepared hydrochlorothiazide with KLU and WASM prepared initial-only. These controls warrant caution in interpreting the magnitudes. Equal output bits or fingerprints and solver counters rule out a change in the recorded solver work as the explanation for the time differences. The timings do not isolate a particular engine instruction or host cost.

Each host ran 24 scenarios in 32 trials on logical CPU 0 of an Intel Core i7-12700. Each trial ran all 24 scenarios. The 24 baseline/feature trials and eight identical-baseline sham trials balanced forward and reverse order. WASM compared instances within a process in three ABBA blocks. Native compared separate pinned executable processes using medians of nine thread CPU measurements per case. Both hosts used identical model bytes and requests. The orientation-stratified bootstrap used whole trial vectors and 50,000 draws for simultaneous intervals over all 48 scenarios. [Frozen protocol](protocol.md)

The WASM driver verified identical columns, row counts, every output value bit, and all 18 solver counters. Every native scenario reproduced the same 64-bit fingerprint of trajectory value bits and all 18 counters across 64 processes. A fingerprint collision is possible. Prepared and ordinary runs each have their own signature.

The baseline and feature compilers produced byte-identical model.wasm files and compact model.json descriptions for all 1,701 previously compiled semantic cases. This comparison covers 3,402 artifacts. [Artifact manifest](model-artifacts.json)

## Native results

AB compares the feature with the baseline. AA compares the baseline with itself. Forward and reverse columns report order-specific ratios normalized to feature/baseline. DQ denotes the colored difference quotient.

| Scenario | AB ratio | AB simultaneous 95% | AB forward / reverse | AA ratio | AA simultaneous 95% | AA forward / reverse |
|---|---:|---:|---:|---:|---:|---:|
| adams / ordinary | 0.9869 | 0.9719 to 1.0022 | 0.9825 / 0.9914 | 1.0031 | 0.9928 to 1.0135 | 1.0045 / 1.0016 |
| adams / prepared | 1.0014 | 0.9668 to 1.0371 | 1.0090 / 0.9938 | 0.9733 | 0.8821 to 1.0738 | 0.9233 / 1.0260 |
| decay / ordinary / dense / analytic / times | 0.9767 | 0.9614 to 0.9922 | 0.9810 / 0.9723 | 0.9972 | 0.9860 to 1.0086 | 0.9940 / 1.0004 |
| decay / ordinary / klu / DQ / steps | 1.0200 | 0.9706 to 1.0719 | 1.0304 / 1.0097 | 0.9976 | 0.9837 to 1.0117 | 0.9974 / 0.9978 |
| decay / prepared / dense / analytic / times | 0.9751 | 0.9455 to 1.0057 | 0.9681 / 0.9822 | 0.9948 | 0.9786 to 1.0112 | 0.9944 / 0.9952 |
| decay / prepared / klu / DQ / steps | 0.9964 | 0.9845 to 1.0084 | 1.0021 / 0.9907 | 1.0056 | 0.9901 to 1.0213 | 0.9954 / 1.0158 |
| events / ordinary / dense / analytic / times | 0.9840 | 0.9656 to 1.0028 | 0.9873 / 0.9808 | 0.9920 | 0.9664 to 1.0183 | 1.0062 / 0.9781 |
| events / ordinary / klu / DQ / steps | 1.0117 | 0.9988 to 1.0247 | 1.0223 / 1.0012 | 1.0040 | 0.9837 to 1.0247 | 0.9996 / 1.0084 |
| events / prepared / dense / analytic / times | 0.9797 | 0.9625 to 0.9972 | 0.9799 / 0.9796 | 0.9876 | 0.9614 to 1.0146 | 0.9752 / 1.0002 |
| events / prepared / klu / DQ / steps | 0.9963 | 0.9760 to 1.0170 | 1.0010 / 0.9916 | 0.9850 | 0.9441 to 1.0276 | 0.9953 / 0.9748 |
| hctz / ordinary / dense / analytic / times | 0.9957 | 0.9821 to 1.0096 | 0.9941 / 0.9974 | 1.0057 | 0.9811 to 1.0310 | 1.0168 / 0.9948 |
| hctz / ordinary / klu / DQ / steps | 1.0085 | 0.9945 to 1.0228 | 1.0151 / 1.0020 | 1.0172 | 0.9868 to 1.0484 | 1.0240 / 1.0104 |
| hctz / prepared / dense / analytic / times | 1.0025 | 0.9867 to 1.0186 | 1.0037 / 1.0014 | 1.0019 | 0.9710 to 1.0338 | 1.0311 / 0.9735 |
| hctz / prepared / klu / DQ / steps | 1.0108 | 0.9956 to 1.0261 | 1.0240 / 0.9977 | 1.0152 | 1.0004 to 1.0302 | 1.0192 / 1.0112 |
| initial-only / ordinary | 0.9971 | 0.9739 to 1.0209 | 0.9972 / 0.9970 | 0.9817 | 0.9577 to 1.0064 | 0.9666 / 0.9971 |
| initial-only / prepared | 0.9959 | 0.9800 to 1.0121 | 1.0000 / 0.9918 | 0.9996 | 0.9807 to 1.0189 | 1.0112 / 0.9881 |
| midazolam / ordinary / dense / analytic / times | 1.0212 | 1.0008 to 1.0420 | 1.0242 / 1.0182 | 1.0023 | 0.9888 to 1.0159 | 1.0099 / 0.9947 |
| midazolam / ordinary / klu / DQ / steps | 1.0275 | 1.0128 to 1.0425 | 1.0251 / 1.0300 | 1.0009 | 0.9863 to 1.0158 | 0.9951 / 1.0068 |
| midazolam / prepared / dense / analytic / times | 1.0145 | 1.0021 to 1.0271 | 1.0073 / 1.0218 | 1.0048 | 0.9783 to 1.0321 | 1.0135 / 0.9962 |
| midazolam / prepared / klu / DQ / steps | 1.0294 | 1.0080 to 1.0513 | 1.0234 / 1.0354 | 1.0122 | 0.9915 to 1.0333 | 1.0098 / 1.0145 |
| robertson / ordinary / dense / analytic / times | 1.0081 | 0.9966 to 1.0198 | 1.0036 / 1.0127 | 1.0067 | 0.9851 to 1.0288 | 1.0021 / 1.0114 |
| robertson / ordinary / klu / DQ / steps | 1.0342 | 1.0187 to 1.0499 | 1.0436 / 1.0249 | 1.0057 | 0.9927 to 1.0189 | 1.0019 / 1.0095 |
| robertson / prepared / dense / analytic / times | 1.0163 | 1.0037 to 1.0291 | 1.0159 / 1.0167 | 0.9889 | 0.9671 to 1.0112 | 0.9825 / 0.9953 |
| robertson / prepared / klu / DQ / steps | 1.0390 | 1.0240 to 1.0544 | 1.0350 / 1.0431 | 0.9902 | 0.9682 to 1.0126 | 0.9984 / 0.9820 |

## WASM results

AB compares the feature with the baseline. AA compares the baseline with itself. Forward and reverse columns report order-specific ratios normalized to feature/baseline. DQ denotes the colored difference quotient.

| Scenario | AB ratio | AB simultaneous 95% | AB forward / reverse | AA ratio | AA simultaneous 95% | AA forward / reverse |
|---|---:|---:|---:|---:|---:|---:|
| adams / ordinary | 1.0018 | 0.9889 to 1.0148 | 1.0068 / 0.9968 | 0.9957 | 0.9762 to 1.0156 | 1.0101 / 0.9815 |
| adams / prepared | 1.0230 | 1.0116 to 1.0344 | 1.0230 / 1.0229 | 0.9931 | 0.9842 to 1.0019 | 0.9940 / 0.9921 |
| decay / ordinary / dense / analytic / times | 1.0208 | 1.0135 to 1.0282 | 1.0205 / 1.0211 | 1.0048 | 0.9919 to 1.0179 | 0.9986 / 1.0110 |
| decay / ordinary / klu / DQ / steps | 0.9991 | 0.9865 to 1.0117 | 1.0014 / 0.9967 | 0.9895 | 0.9769 to 1.0024 | 0.9871 / 0.9920 |
| decay / prepared / dense / analytic / times | 1.0202 | 1.0119 to 1.0287 | 1.0165 / 1.0240 | 0.9925 | 0.9855 to 0.9996 | 0.9988 / 0.9862 |
| decay / prepared / klu / DQ / steps | 0.9952 | 0.9829 to 1.0077 | 0.9924 / 0.9980 | 1.0103 | 0.9791 to 1.0424 | 1.0253 / 0.9954 |
| events / ordinary / dense / analytic / times | 1.0088 | 0.9965 to 1.0212 | 1.0058 / 1.0118 | 1.0096 | 0.9571 to 1.0650 | 1.0020 / 1.0173 |
| events / ordinary / klu / DQ / steps | 0.9924 | 0.9837 to 1.0011 | 0.9913 / 0.9935 | 0.9883 | 0.9673 to 1.0097 | 0.9823 / 0.9943 |
| events / prepared / dense / analytic / times | 1.0141 | 0.9996 to 1.0288 | 1.0211 / 1.0071 | 1.0036 | 0.9871 to 1.0204 | 0.9986 / 1.0086 |
| events / prepared / klu / DQ / steps | 0.9902 | 0.9804 to 1.0001 | 0.9888 / 0.9915 | 1.0042 | 0.9947 to 1.0139 | 1.0130 / 0.9956 |
| hctz / ordinary / dense / analytic / times | 0.9966 | 0.9881 to 1.0051 | 0.9981 / 0.9951 | 1.0056 | 0.9997 to 1.0116 | 1.0051 / 1.0061 |
| hctz / ordinary / klu / DQ / steps | 0.9802 | 0.9722 to 0.9883 | 0.9840 / 0.9765 | 1.0015 | 0.9925 to 1.0106 | 1.0031 / 0.9999 |
| hctz / prepared / dense / analytic / times | 1.0003 | 0.9945 to 1.0060 | 1.0025 / 0.9980 | 1.0003 | 0.9952 to 1.0054 | 0.9945 / 1.0061 |
| hctz / prepared / klu / DQ / steps | 0.9816 | 0.9742 to 0.9892 | 0.9834 / 0.9798 | 0.9961 | 0.9897 to 1.0025 | 0.9928 / 0.9994 |
| initial-only / ordinary | 0.9938 | 0.9860 to 1.0016 | 0.9995 / 0.9881 | 1.0002 | 0.9979 to 1.0025 | 0.9982 / 1.0021 |
| initial-only / prepared | 0.9977 | 0.9764 to 1.0194 | 0.9893 / 1.0061 | 0.9830 | 0.9702 to 0.9961 | 0.9879 / 0.9782 |
| midazolam / ordinary / dense / analytic / times | 0.9932 | 0.9871 to 0.9995 | 0.9944 / 0.9921 | 0.9995 | 0.9881 to 1.0111 | 0.9954 / 1.0037 |
| midazolam / ordinary / klu / DQ / steps | 0.9778 | 0.9714 to 0.9843 | 0.9763 / 0.9793 | 0.9953 | 0.9889 to 1.0017 | 0.9954 / 0.9952 |
| midazolam / prepared / dense / analytic / times | 1.0001 | 0.9939 to 1.0064 | 0.9970 / 1.0032 | 0.9995 | 0.9921 to 1.0070 | 1.0024 / 0.9966 |
| midazolam / prepared / klu / DQ / steps | 0.9850 | 0.9759 to 0.9942 | 0.9832 / 0.9868 | 1.0018 | 0.9951 to 1.0085 | 1.0010 / 1.0025 |
| robertson / ordinary / dense / analytic / times | 1.0066 | 0.9945 to 1.0188 | 1.0007 / 1.0125 | 1.0051 | 0.9878 to 1.0226 | 1.0157 / 0.9945 |
| robertson / ordinary / klu / DQ / steps | 1.0067 | 0.9961 to 1.0173 | 1.0078 / 1.0055 | 0.9865 | 0.9666 to 1.0069 | 0.9879 / 0.9852 |
| robertson / prepared / dense / analytic / times | 0.9993 | 0.9788 to 1.0202 | 0.9936 / 1.0050 | 0.9878 | 0.9701 to 1.0058 | 1.0002 / 0.9755 |
| robertson / prepared / klu / DQ / steps | 0.9815 | 0.8936 to 1.0781 | 0.9543 / 1.0096 | 0.9987 | 0.9933 to 1.0042 | 1.0049 / 0.9925 |

## Compiled ordinary solver paths

Disassembly of the frozen native drivers at the hashes below shows identical opcode sequences for the ordinary RHS (139 instructions), Jacobian callback (142), root callback (126), CVODE integration wrapper (137), event-free segment driver (588), host WasmModel RHS bridge (149), and CVode (3,423). Their operands also matched after removing linked address differences. The top-level ordinary run_model specialization changed from 1,366 to 1,198 instructions and the event segment driver from 1,410 to 1,407.

The WASM engines have identical opcode sequences for the ordinary RHS (74 instructions), Jacobian callback (227), root callback (51), CVODE start (936), integration wrapper (240), and CVode (7,111). The event-free segment driver changed from 1,102 to 1,104 opcodes and the event segment driver from 2,510 to 2,518. In the event-free driver, the differences include loop branch direction, block layout and local variable operations. Op counts describe compiled function bodies. Execution frequency, branch targets, instruction placement and JIT-generated machine code can affect time independently of those counts. This inspection does not establish the cause of the measured slowdowns.

## Reproduction and provenance

The baseline commit was `d904f63c03d1c99d05e75ffdfec90b2c226db8be`. The feature source corresponds to commit `019e226de992661cf453e3087c4ee1918fde46e2`. SHA-256 identifies the exact executable and runtime measured.

| Artifact | Baseline SHA-256 | Feature SHA-256 |
|---|---|---|
| Native driver | `1a33bd9e8d98b5b1eff2fe6ff1007f48e460bc40bb2748c8fad0309fe6cdc0c5` | `35396b8c7ed5a00e262bdd83969d3bc7f91909bf7f7519c2c9daf66bde05c780` |
| WASM engine | `1391125a9dd353e3d0ad10e1015e3156a0ca8c137513ad3beda5770442dd7be6` | `8dfc2bec303e78a81460ba059625c4bbe0a3a44c10f0cfe18fd8dfc071a8604d` |
| Math library | `8f725c0b975e7b2c43a7bd002d4d298c104ef34927e9d669e73e76771bf62a8c` | `8f725c0b975e7b2c43a7bd002d4d298c104ef34927e9d669e73e76771bf62a8c` |

An earlier driver completed 11 trials before timing stopped for correctness fixes. Those trials are archived separately and excluded from all estimates. [Interrupted observations](interrupted-835947f4.json.gz)

The archived observations include all native CPU and wall samples, WASM paired CPU samples, orders, requests, checksums and confidence intervals. The frozen source model hashes and all five baseline-compiled model hashes are in the manifest. [All observations](timings-019e226.json.gz) · [Summary and signatures](summary-019e226.json) · [Baseline manifest](baseline.json) · [Measurement script](../../scripts/dde_performance.py)

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
.venv/bin/python scripts/dde_performance.py measure --feature target/dde-feature-reproduction/ode_performance
```

The prepared midazolam and hydrochlorothiazide source files came from sbmlsim 0.29.0. Compare source hashes and the schedule with the baseline manifest before interpreting a repeat run. A new build can have a different executable hash. The reported hashes identify the binaries used for these results.
