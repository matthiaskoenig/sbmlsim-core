# Statistics

The statistics of the analyses of sbmlsim run here (0.42.0): the special functions, the linear algebra and the kernels of the evaluation of a fit, of its Fisher information and posterior, of the uncertainty and of a variability, in the crate `sbmlsim-stats` and in python as `sbmlsim_core.stats`. They replace the arithmetic of numpy and the functions of scipy which sbmlsim called in `sbmlsim.fit` and `sbmlsim.uncertainty`, with the same names, the same edges and the same messages: an argument the python refuses raises the exception of the python with its message (a `ValueError`, and an `OverflowError`, a `ZeroDivisionError` or an `IndexError` where python raises one), and `nan` is returned where the python returns `nan`.

The crate knows no simulation, no problem and no table: a function takes numbers and returns numbers, and the caller keeps the flow. An array of any shape and of any type numpy converts to doubles is read as its values in the order numpy ravels it, strided and integer arrays included. Every function on arrays releases the GIL while it computes, so other python threads run meanwhile.

A size is counted without a wrap, also where a `usize` has 32 bits (WebAssembly): an array a function would make past what an array holds, its bytes past the largest `intp`, raises numpy's `ValueError` (`array is too big`) before any memory is asked for, as numpy raises it for the same shape. That is no check of the memory: a size which can be counted is reserved, and where the system overcommits (Linux by default) it runs out of memory when it is filled, as with numpy, so the memory is the caller's.

```python
import numpy
from sbmlsim_core import stats

# a fit of two parameters to 20 data points: the jacobian of the residuals
t = numpy.linspace(0.0, 10.0, 20)
jacobian = numpy.column_stack([numpy.exp(-0.3 * t), -t * numpy.exp(-0.3 * t)])
fisher = stats.fisher_statistics(stats.gram(jacobian), cost=0.05, n=20)
low, high = stats.fisher_intervals(
    [1.0, 0.3], fisher["standard_errors"], ["LINEAR", "LOG10"], dof=18
)
print(fisher["rank"], low, high)  # 2 [0.89 0.27] [1.11 0.34]

# 1024 correlated draws of two standard normal variables and their sum
u = stats.unit_points(2, 1024, design="sobol", seed=1)
z = stats.copula(u, [[1.0, 0.8], [0.8, 1.0]])
y = z[:, 0] + z[:, 1]
print(stats.quantile_ci(y, 0.95))  # the quantile, its interval and the draws

# is the probability of y > 0.5 above 0.6?
sides = [
    stats.p_values(y > 0.5, 0.6, side) for side in ("greater", "less", "equivalent")
]
print(stats.decision(*sides, alpha=0.05))  # ('accepted', 73)
```

## The metrics of a fit

The metrics of the residuals of `sbmlsim.fit.metrics`: `sse`, `mse`, `rmse_from_mse`, `aic_from_mse`, `bic_from_mse` and `r_squared`. The criteria of the data points of `sbmlsim.fit.evaluation`, of a value `y` and its prediction `f`: `fold_counts` and `fold_accepted` (`Fold(fold)`, the points with a ratio, a positive and finite value and prediction, within the fold factor), `sd_accepted` (`Uncertainty(k)`, the points with a standard deviation), the fold errors `gmfe` and `afe`, `nrmse` (`_rms`, the root mean square of the normalized residuals), the agreement of a subset after Bland and Altman `subset_agreement` (at least `MIN_POINTS_SUBSET_AGREEMENT` points with a ratio, the t quantile of `n - 1` degrees of freedom) and the limits of the predicted ratios of Guest et al. `guest_limits` and `within_guest`. A sum is the pairwise sum of numpy, so a metric is numpy's to the last bits.

## The Fisher information and the posterior

`gram` is the Fisher information `J' J` of a jacobian, and `fisher_statistics` the statistics of `sbmlsim.fit.fisher.FisherInformation` of it: the variance of the residuals, the eigenvalues largest first, the rank (the eigenvalues above `DEFAULT_RANK_TOLERANCE` times the largest one), the condition number, the covariance from the pseudo-inverse, the standard errors and the correlation. An information of a lower rank than its parameters has `nan` standard errors and an infinite condition, it does not raise. `fisher_intervals` (the t intervals of a least squares fit, the normal ones of a likelihood, on the scale of every parameter and transformed back) and `fisher_relative_errors` are `FisherInformation.intervals` and `.relative_errors`; a scale is a name of `ParameterScaleType` (`"LINEAR"`, `"LOG"`, `"LOG10"`), of PEtab (`"lin"`, `"log"`, `"log10"`) or a member of `ParameterScaleType`.

The posterior of `sbmlsim.fit.posterior`: `posterior_laplace` (the covariance of the Laplace approximation of a precision, with the message of the python for a direction the precision does not constrain), `rank_correlation` and `normal_correlation` of `ParameterPosterior`, and `log_sd` and `lognormal_cv`, the marginal of a parameter on a logarithmic scale. The eigendecomposition is the cyclic Jacobi method, which also holds a matrix of 200 parameters to numpy.

## The statistics of the draws

The statistics of `sbmlsim.uncertainty.statistics` drop the draws which are `NaN`: `mean_se` and `sd_se` (a statistic, its standard error and the number of draws), `quantile` and `quantiles` (`np.nanquantile`, the linear method), `quantile_ci` (a quantile and its distribution-free interval from the order statistics of the binomial distribution) and `first_order_given_data` (the first-order index of an input from given draws, by binning). `precise` is `_precise` of `sbmlsim.uncertainty.propagation`, whether the draws of an output are precise enough, and `interval_parameters` the parameters of the distribution of a reported interval, `Uncertain.interval`.

## The draws and their correlations

`unit_points(d, n, design=..., seed=..., start=0)` gives the points of the unit cube of the designs of `sbmlsim.uncertainty.draws.unit_points`: `"sobol"` (the scrambled Sobol' sequence, which continues from the point `start`), `"lhs"` (a Latin hypercube, every column a random permutation of the strata with a uniform point in its stratum, without a continuation) and `"random"` (which continues from `start`: the stream jumps ahead to it in a time logarithmic in `start * d`, so any start costs about the same). `copula` is the Gaussian copula of `draw`, `Φ(Φ⁻¹(u) Fᵀ)` with the factor `F` of the correlation and the points clipped to `EPS` from the ends, and `normal_score_rho` the coefficient of the normal scores of a covariance of two normal or log-normal marginals. The correlations of `sbmlsim.uncertainty.covariance`: `nearest_correlation` (the nearest correlation matrix in the Frobenius norm, Higham 2002), `is_psd` (at the tolerance `PSD_RTOL`) and `validate_covariance` (`Covariance._validate`, with its messages).

## The probability statements

The statements about predictions of `sbmlsim.uncertainty.testing`, robust to the number of draws, from whether every draw satisfies a hypothesis: `p_values` (the anytime-valid p-values of the beta-binomial mixture `MIXTURE` after every number of draws, of the sides `"greater"`, `"less"` and `"equivalent"` within a margin, `MARGIN` by default), `confidence_sequence` (each end by `BISECTIONS` bisections), `decide` and `decision` (`_decide`), `needed` (`_needed`, the draws an undecided statement needs at its estimate, up to `MAX_NEEDED`) and `family_level` (Bonferroni).

## The visual predictive check

The kernels of `predictive_check` of `sbmlsim.fit.variability`: `group_means` (`_group_means`, the mean or the geometric mean of virtual groups of subjects resampled with replacement, the subjects which are not finite at every data point removed first) and `bands` (numpy's linear quantiles of the finite values of every data point). Predictions at no data point give the rows of the groups without a draw.

## Accuracy

The tests hold every function to numpy, scipy and the functions of sbmlsim, whose values `crates/stats/oracles/make_oracles.py` computed once and stored as JSON next to it (`crates/stats/tests/oracle_<module>.rs`), and the bindings to the edges and the messages of the python (`python/tests/test_stats.py`):

| Function | Tolerance |
|---|---|
| the deterministic kernels: the metrics, the Fisher statistics, the posterior, the statistics of the draws, the logarithm of the beta function, the p-values and the confidence sequence | `1e-12` relative |
| the distribution function and the quantile of the normal distribution | `1e-13` relative |
| the regularized incomplete beta function, the distribution functions and quantiles of the t and the binomial distribution, and the agreement of a subset, which takes a t quantile | `1e-10` relative |
| the quantiles of the binomial distribution | the same integers |
| the eigenvalues, eigenvectors, factors and pseudo-inverses, of up to 200 parameters | `1e-10` of the largest value of the matrix (LAPACK sums in another order) |

Where the crate and scipy differ beyond the rounding, the crate is the more accurate one. The logarithm of the beta function takes the formula of Stirling with its remainder for a parameter of at least ten (the scheme of `lbeta` of R), so the logarithms of the gamma function do not cancel: at a large and a small parameter, where `scipy.special.betaln` loses digits (1.5e-10 relative at 0.5 and 1e6), it agrees with mpmath at 50 digits to 1e-14. The p-values of the probability statements are differences of such logarithms of about 100, so their largest difference to the python, 8e-13 of a p-value after 500 draws, is the error of scipy's `betaln`, not the crate's.

## Determinism

Every transcendental function comes from the crate libm, and every random draw from xoshiro256++ seeded through SplitMix64, written out in sbmlsim-core, so one input and one seed give the same bits on every target, natively and in WebAssembly. The tests hold the hashes of the bits of every function over its fixtures and of the draws of fixed seeds, and the continuous integration runs them natively and under wasmtime (`cargo test -p sbmlsim-stats --target wasm32-wasip1`). The crate also builds for `wasm32-unknown-unknown`; no JavaScript binding of it exists yet.

## The random streams changed

The draws are deliberately not the streams of scipy and numpy which sbmlsim drew from before. The Sobol' points are the Sobol' sequence of sbmlsim-core (the direction numbers of Joe and Kuo, see [Sensitivity analysis](sensitivity.md)) with its scrambling, a random lower triangular matrix per dimension and a random digital shift (LMS+shift), not the scrambling of Owen; the scrambling does not depend on `start`, so a continuation is exact. The random points, the Latin hypercube and the resampling of the subjects of the group means draw from xoshiro256++, not from numpy's PCG64. Reproducing the streams of scipy bit for bit has no value: the draws are held to their distributions (the balance of the Sobol' points, the strata of the hypercube, the moments and correlations of the copula, the mean and the variance of the group means) and to their determinism.

A seed therefore gives other points than the ones sbmlsim drew for it with scipy and numpy, and a result stored with a seed reproduces only with the release of sbmlsim it was computed with: one from before the port to sbmlsim-core 0.42.0 needs that older release. What a seed does not decide, the copula of given points and the statistics of given draws, is the python's.
