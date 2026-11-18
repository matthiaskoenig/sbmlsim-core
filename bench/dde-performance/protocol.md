# Ordinary ODE performance with DDE support

Compare the merged GMRES baseline, commit `d904f63`, with the DDE feature on
identical model bytes and requests. Freeze the 24 scenarios per host from
`bench/ode-performance-data.json`, including zero-duration runs, ordinary and
prepared calls, Adams and BDF, events, and both direct linear solvers. Freeze
the baseline binaries, runtime, model source hashes, CPU and trial schedule
before timing. Freeze the feature executable and its embedded runtime before
the first comparison. Keep all invocations, including failures.

Use 32 trials per host on one pinned logical CPU with warm Wasmtime caches.
Seed 29 fixes eight groups of three feature comparisons (AB) and one
baseline/baseline sham (AA), randomized within each group. Balance forward and
reverse order independently for each family and use the same random job order
for both members of a native pair. Each WASM trial runs three ABBA blocks
within the same process. Each native trial runs two separate executables, one
after the other. Each native observation uses the median of nine warm thread
CPU samples in each process. Both hosts exclude instantiation and preparation
from the timed calls. Record every raw sample and all input hashes.

Normalize reverse comparisons to feature/baseline before analysis. Use the
orientation-stratified, whole-trial bootstrap from the previous ODE confirmation
(50,000 draws). Treat both hosts as one 48-scenario family, with the two host
measurements in each trial forming one vector. Report simultaneous 95% log-scale
intervals and orientation-specific effects. Apply the same procedure to sham
controls. A sham interval excluding one flags bias or unresolved measurement
noise. Never subtract a sham effect.

The prespecified slowdown threshold is a ratio of **1.000**. A lower bound above
one indicates a measured slowdown. An upper bound at or below one supports
zero overhead for that workload. Intervals crossing one are inconclusive and
must be reported with their upper bounds. A strict zero-overhead conclusion
requires all 48 upper bounds at or below one, variation in all estimates, and
all sham intervals containing one. Report point estimates even when the claim
cannot be established.

The WASM driver compares column names, shape, every output bit and all 18 solver
counters across instances. The native driver exposes the 64-bit FNV fingerprint
of each trajectory's value bits and all 18 counters. Require identical
fingerprints and counters in every process for each native scenario. A matching
fingerprint is evidence of numerical identity, with a finite collision risk.
