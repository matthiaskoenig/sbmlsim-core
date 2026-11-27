# Sensitivity analysis

The global sensitivity analysis of sbmlsim runs here (0.37.0): the designs of its three global methods on the unit cube and the estimators of their indices, in the crate `sbmlsim-sensitivity` and in python as `sbmlsim_core.sensitivity`. They are the designs and the estimators of [SALib](https://salib.readthedocs.io), which sbmlsim called before:

| Method | Design | Estimator | Indices |
|---|---|---|---|
| Sobol | `saltelli`, the matrix of Saltelli (2002) on a scrambled Sobol' sequence (`sobol_sequence`) | `sobol_indices`, Saltelli et al. (2010) and, for the second order, Saltelli (2002) | `S1`, `ST`, `S2` with their intervals |
| extended FAST | `fast_sample`, the search curves of Saltelli et al. (1999) | `fast_indices`, the spectra of the curves | `S1`, `ST` with their intervals |
| Morris | `morris_sample`, the trajectories of Morris (1991) on a grid of levels | `morris_effects`, with the `mu_star` of Campolongo et al. (2007) | `mu`, `mu_star`, `sigma`, `mu_star_conf` |

A design is a matrix of a row per point and a column per parameter on the unit cube, which the caller maps through the distributions of its parameters (sbmlsim through their inverse distribution functions) and simulates. An estimator takes the values of the points as a matrix of a row per point and a column per element, the time points of a timecourse or the conditions of a scan, and estimates every element in the same pass: a timecourse of a thousand time points costs little more than one value.

```python
import numpy
from sbmlsim_core import sensitivity

# the Ishigami function of three parameters on [-pi, pi]
u = sensitivity.saltelli(3, 1024, seed=1)
x = u * 2.0 * numpy.pi - numpy.pi
y = numpy.sin(x[:, 0]) + 7.0 * numpy.sin(x[:, 1]) ** 2
y = y + 0.1 * x[:, 2] ** 4 * numpy.sin(x[:, 0])

found = sensitivity.sobol_indices(y[:, None], 3, seed=1)
print(found["S1"][:, 0], found["ST"][:, 0])  # about [0.31 0.44 0.00] [0.56 0.44 0.24]
```

## The designs

`sobol_sequence(d, n, seed=..., skip=0)` is the Sobol' sequence of `scipy.stats.qmc.Sobol(d)`: the direction numbers of Joe and Kuo (2008, `new-joe-kuo-6.21201`, of which the crate carries the first 2048 dimensions), 30 bits per coordinate and the order of the Gray code. Without scrambling (`seed=None`) it is scipy's bit for bit, its first point the origin; with a seed it is scrambled as scipy scrambles it, by a random lower triangular matrix per dimension and a random digital shift (LMS+shift), which keeps the balance of the sequence: of `n = 2^m` points every interval of `1/n` of a dimension holds one. `saltelli(d, n, seed=..., second_order=False, skip=0)` crosses the `2 d` dimensions of that sequence into the matrix of `SALib.sample.sobol.sample`: per point of the sequence the row `A`, the rows `AB_j`, with the second order the rows `BA_j`, and the row `B`, so `n (d + 2)` or `n (2 d + 2)` points. Its first point is kept, as SALib keeps it, and at most 1024 parameters fit the direction numbers.

`fast_sample(d, n, m=4, seed=...)` is the design of `SALib.sample.fast_sampler.sample`: a block of `n > 4 m^2` points per parameter on the search curve `0.5 + asin(sin(omega s + phi)) / pi`, the parameter of the block at the frequency `floor((n - 1) / (2 m))` and the others at the lower frequencies of SALib, with a random phase per block; `phases=` gives the phases instead of a seed.

`morris_sample(d, trajectories, levels=4, seed=...)` is the design of `SALib.sample.morris.sample` without the selection of optimal trajectories: every trajectory of `d + 1` points starts from the lower levels of the grid and moves every parameter once, in a random order and direction, by `levels / (2 (levels - 1))`.

## The estimators

`sobol_indices(y, d, second_order=False, conf_level=0.95, num_resamples=100, seed=...)`, `fast_indices(y, d, m=4, ...)` and `morris_effects(cube, y, levels=4, ...)` return a dictionary of arrays of the shape `(d, elements)`, `S2` and `S2_conf` of the shape `(d, d, elements)` with the upper triangle set. The values of an element of Sobol are centred and scaled by their standard deviation before the estimation, as SALib does, and an element whose pooled sample of `A` and `B` has a range of at most the machine epsilon has zero indices, as one whose spectrum has no power has with FAST. The elementary effects of Morris are the change of the output over a step divided by the step on the grid of `[0, 1]`, so they have the unit of the output; the direction of a step is read from the points, so the points of the grid moved to the centres of their strata give the same effects.

An interval is `z(conf_level)` times the standard deviation of the estimator over bootstrap resamples, as in SALib: resamples of the samples of the matrix of Saltelli (each with its own variance), windows of half of the points of a block of FAST from random starts, and resamples of the trajectories of Morris per parameter. SALib documents the intervals of FAST as unreliable, so they are indicative only. The resamples are drawn from `seed`, or given as `resamples=` (an array of the drawn indices, whose shape the stub of `_core` names), which is how the tests compare the intervals with SALib's.

## Seeds and reproducibility

Every random draw comes from one generator, xoshiro256++ seeded through SplitMix64, written out in the crate so that no update of a dependency changes a stream, and every sine, arcsine, cosine and logarithm from the crate libm. A seed therefore gives the same points and the same intervals natively and in WebAssembly, bit for bit: `crates/sensitivity/tests/determinism.rs` holds the hashes of the bits of the designs and the estimators of fixed seeds, and the continuous integration runs it natively and under wasmtime (`wasm32-wasip1`). The crate also builds for `wasm32-unknown-unknown`; no JavaScript binding of it exists yet.

The streams are not those of numpy, so the points a seed gives are not the ones SALib draws for it, and the intervals of a seed are other resamples of the same bootstrap. What a seed does not decide is SALib's: the tests (`python/tests/test_sensitivity_salib.py`, with the group `sensitivity-tests`) compare the Sobol' sequence without scrambling with scipy's for all 2048 dimensions, the scrambling with scipy's for the same random bits, the matrix of Saltelli without scrambling, the search curves of FAST for the same phases and the trajectories of Morris for the same starts, orders and directions with SALib's, and every index and interval of every element with SALib's for the same values and the resamples SALib draws, within the rounding (`1e-10` relative).

## Cost

The estimators of Sobol and Morris are sums over the points weighted by how often a resample draws each of them. The transform of a window of FAST is needed at the frequencies of the indices only, `floor(omega / 2) + m` of them, and is the difference of two prefix sums of the block times the twiddles of the frequency, so the hundred windows of a bootstrap cost about one transform of the block; the power of the whole spectrum is the variance of the window less its Nyquist frequency (Parseval). The elements are spread over `threads` threads (every CPU of the process by default, `threads=` of an estimator), each estimated alone, so an index does not depend on the threads; a thread which cannot be started, as in a wasm module without threads, runs its elements on the calling one. With five parameters and 1000 elements a Sobol design of `n = 1024` takes 0.12 s, FAST of `n = 1025` 0.15 s and Morris of 100 trajectories 0.01 s on 20 threads (the estimators of numpy of sbmlsim 0.50 took 0.1 s, 1.1 s and 0.01 s).
