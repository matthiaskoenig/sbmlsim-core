# Statistics

The statistics of the analyses of sbmlsim run here (0.42.0): the special functions, the linear algebra and the kernels of the evaluation of a fit, of its Fisher information and posterior, of the uncertainty and of a variability, in the crate `sbmlsim-stats`, in python as `sbmlsim_core.stats` and (since 0.43.0) in JavaScript as `sbmlsim-core/stats`, see [JavaScript](#javascript). They replace the arithmetic of numpy and the functions of scipy which sbmlsim called in `sbmlsim.fit` and `sbmlsim.uncertainty`, with the same names, the same edges and the same messages: an argument the python refuses raises the exception of the python with its message (a `ValueError`, and an `OverflowError`, a `ZeroDivisionError` or an `IndexError` where python raises one), and `nan` is returned where the python returns `nan`.

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

# 1024 draws of two uniform variables whose normal scores correlate, and their sum
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

`gram` is the Fisher information `J' J` of a jacobian, and `fisher_statistics` the statistics of `sbmlsim.fit.fisher.FisherInformation` of it: the variance of the residuals, the eigenvalues largest first, the rank (the eigenvalues above `DEFAULT_RANK_TOLERANCE` times the largest one), the condition number, the covariance from the pseudo-inverse, the standard errors and the correlation. An information of a lower rank than its parameters has `nan` standard errors and an infinite condition, it does not raise. `fisher_decomposition` (since 0.44.0) gives the decomposition the statistics come from, the eigenvalues largest first and the pseudo-inverse, and `fisher_statistics_of_decomposition` the statistics of a decomposition which is given, as `FisherInformation` derives them from a record which stores them: a record whose decomposition came from LAPACK (the SVD of a release before sbmlsim-core computed it) gives the numbers it was written with, to the bit, and `fisher_statistics(m)` is `fisher_statistics_of_decomposition(*fisher_decomposition(m))` to the bit. The pseudo-inverse is taken as it is (one of LAPACK is not symmetric to the bit), and eigenvalues which are not largest first, values which are not finite and shapes which do not fit raise a `ValueError`. `fisher_intervals` (the t intervals of a least squares fit, the normal ones of a likelihood, on the scale of every parameter and transformed back) and `fisher_relative_errors` are `FisherInformation.intervals` and `.relative_errors`; a scale is a name of `ParameterScaleType` (`"LINEAR"`, `"LOG"`, `"LOG10"`), of PEtab (`"lin"`, `"log"`, `"log10"`) or a member of `ParameterScaleType`.

The posterior of `sbmlsim.fit.posterior`: `posterior_laplace` (the covariance of the Laplace approximation of a precision, with the message of the python for a direction the precision does not constrain), `rank_correlation` and `normal_correlation` of `ParameterPosterior`, and `log_sd` and `lognormal_cv`, the marginal of a parameter on a logarithmic scale. The eigendecomposition is the cyclic Jacobi method, which also holds a matrix of 200 parameters to numpy.

## The distributions and the special functions

Since 0.44.0 the distribution functions and the special functions of scipy which sbmlsim called are functions of their own: `norm_cdf` and `norm_ppf` (`scipy.stats.norm`, the standard normal distribution, without `loc` and `scale`), `t_cdf(x, df)` and `t_ppf(q, df)` (`scipy.stats.t`), `chi2_cdf(x, df)` and `chi2_ppf(q, df)` (`scipy.stats.chi2`), `erf` and `gamma` (`scipy.special`). They take arrays of any shape and broadcast the value and the degrees of freedom against each other as scipy broadcasts them, with its `ValueError` for shapes which do not broadcast (`operands could not be broadcast together with shapes (3,) (2,) `, the degrees of freedom first of a quantile, as scipy names them); a number (or an array of no dimension) gives a float, another array an array of numpy of its shape. Their edges are scipy's: `nan` of `nan` and of an argument outside of a domain (a probability outside of `[0, 1]`, degrees of freedom which are not positive), the ends of a support at the probabilities zero and one, infinitely many degrees of freedom of the t distribution the normal one, `gamma` infinite at a zero of its sign and `nan` at a negative whole number.

```python
from sbmlsim_core import stats

stats.chi2_ppf(0.95, 1)  # 3.841458820694124, the threshold of a profile likelihood
stats.chi2_ppf(0.95, [1, 2, 3])  # array([3.84145882, 5.99146455, 7.81472776])
stats.norm_ppf([[0.025], [0.975]])  # array([[-1.95996398], [ 1.95996398]])
stats.t_cdf(2.0, [1.0, 5.0, float("inf")])  # the last one the normal distribution
```

The chi-squared distribution is the regularized incomplete gamma function `P(df / 2, x / 2)` of the crate (`special::gamma_p`, `gamma_q` and `gamma_p_inv`, `scipy.special.gammainc`, `gammaincc` and `gammaincinv`): its series below the mean and its continued fraction above (the factor `x^a e^-x / Γ(a)` through the formula of Stirling from `a` ten on, so the logarithms of the gamma function do not cancel), the closed forms of one and two degrees of freedom (`erf` and `e^-x`) and from `a` 1e6 on the uniform asymptotic expansion of Temme. Its quantile is the method of Newton on the logarithm of the smaller tail in `ln x`, guarded by bisection between bounds of the quantile, its last steps in `x` itself, so the quantile keeps the digits of `x`.

## The statistics of the draws

The statistics of `sbmlsim.uncertainty.statistics` drop the draws which are `NaN`: `mean_se` and `sd_se` (a statistic, its standard error and the number of draws), `quantile` and `quantiles` (`np.nanquantile`, the linear method), `quantile_ci` (a quantile and its distribution-free interval from the order statistics of the binomial distribution) and `first_order_given_data` (the first-order index of an input from given draws, by binning). `precise` is `_precise` of `sbmlsim.uncertainty.propagation`, whether the draws of an output are precise enough, and `interval_parameters` the parameters of the distribution of a reported interval, `Uncertain.interval`.

## The draws and their correlations

`unit_points(d, n, *, design="sobol", seed, start=0)` gives the points of the unit cube of the designs of `sbmlsim.uncertainty.draws.unit_points`: `"sobol"` (the scrambled Sobol' sequence, which continues from the point `start`), `"lhs"` (a Latin hypercube, every column a random permutation of the strata with a uniform point in its stratum, without a continuation) and `"random"` (which continues from `start`: the stream jumps ahead to it in a time logarithmic in `start * d`, so any start costs about the same). `copula` is the Gaussian copula of `draw`, `Φ(Φ⁻¹(u) Fᵀ)` with the factor `F` of the correlation and the points clipped to `EPS` from the ends, and `normal_score_rho` the coefficient of the normal scores of a covariance of two normal or log-normal marginals. The correlations of `sbmlsim.uncertainty.covariance`: `nearest_correlation` (the nearest correlation matrix in the Frobenius norm, Higham 2002), `is_psd` (at the tolerance `PSD_RTOL`) and `validate_covariance` (`Covariance._validate`, with its messages).

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
| the error function (since 0.44.0) | `1e-15` relative, of mpmath at 40 digits |
| the gamma function and the quantile of the chi-squared distribution (since 0.44.0) | `1e-13` relative, of mpmath at 40 digits |
| the regularized incomplete gamma function and the distribution function of the chi-squared distribution (since 0.44.0) | `1e-13` relative times the condition `max(1, abs(x - a) / 100)` of `P(a, x)`, of mpmath at 40 digits |
| the edges of the distributions and the special functions | the values of scipy, to the bit |
| the Fisher statistics of a decomposition (since 0.44.0) | the ones `FisherInformation` derives from it, to the bit |
| the regularized incomplete beta function, the distribution functions and quantiles of the t and the binomial distribution, and the agreement of a subset, which takes a t quantile | `1e-10` relative |
| the quantiles of the binomial distribution | the same integers |
| the eigenvalues, eigenvectors, factors and pseudo-inverses, of up to 200 parameters | `1e-10` of the largest value of the matrix (LAPACK sums in another order) |

Where the crate and scipy differ beyond the rounding, the crate is the more accurate one. The incomplete gamma function is held to mpmath, not to scipy: `scipy.special.gammainc` is off by 1e-8 relative 8 standard deviations below the mean of `a` 1e6, where the crate agrees with mpmath to 1e-13. A relative change of `x` by one rounding changes `P(a, x)` by about `abs(x - a)` roundings, so far in its tails every method of doubles keeps fewer digits; the quantile of the chi-squared distribution keeps the digits of `x` there (its condition is the inverse). The logarithm of the beta function takes the formula of Stirling with its remainder for a parameter of at least ten (the scheme of `lbeta` of R), so the logarithms of the gamma function do not cancel: at a large and a small parameter, where `scipy.special.betaln` loses digits (1.5e-10 relative at 0.5 and 1e6), it agrees with mpmath at 50 digits to 1e-14. The p-values of the probability statements are differences of such logarithms of about 100, so their largest difference to the python, 8e-13 of a p-value after 500 draws, is the error of scipy's `betaln`, not the crate's.

## Determinism

Every transcendental function comes from the crate libm, and every random draw from xoshiro256++ seeded through SplitMix64, written out in sbmlsim-core, so one input and one seed give the same bits on every target, natively and in WebAssembly. The tests hold the hashes of the bits of every function over its fixtures and of the draws of fixed seeds, and the continuous integration runs them natively and under wasmtime (`cargo test -p sbmlsim-stats --target wasm32-wasip1`). The test of the nearest correlation matrix of 200 targets, which holds its bits as well, takes minutes in wasm and runs on request only (`cargo test -p sbmlsim-stats --release -- --ignored`), natively and not in the continuous integration; the python tests run the same case on every wheel. The crate also builds for `wasm32-unknown-unknown`, the build of its [JavaScript binding](#javascript).

## JavaScript

Since 0.43.0 every function of `sbmlsim_core.stats` runs in a page as well: `compiler.wasm` carries the crate (the feature `stats` of `sbmlsim-wasm`, on by default) and `sbmlsim-core/stats` calls it, the names in camelCase, the optional arguments of python in an object of options, the errors a `StatsError` with the message of python and its `kind`, the class of the exception (`ValueError`, `OverflowError`, `ZeroDivisionError`, `IndexError`):

```js
import { createCompiler } from "sbmlsim-core/compile";
import { createStats } from "sbmlsim-core/stats";

const stats = createStats({ compiler: await createCompiler({ compiler: compilerWasm }) });
const u = stats.unitPoints(2, 1024, { design: "sobol", seed: 1 }); // 1024 rows, Float64Arrays
const [q, low, high, n] = stats.quantileCi(u.map(([a, b]) => a + b), 0.95);
```

The arguments are checked by the functions both bindings share (`sbmlsim_stats::binding`), and every double crosses into the module and back as its bits (a `NaN` as a `NaN`), so a page gives the bits of python: `js/test/stats.test.mjs` holds `compiler.wasm` to about a thousand calls of python, bit for bit, the errors by kind and message. See [The statistics](javascript.md#the-statistics) of the JavaScript host.

## The random streams changed

The draws are deliberately not the streams of scipy and numpy which sbmlsim drew from before. The Sobol' points are the Sobol' sequence of sbmlsim-core (the direction numbers of Joe and Kuo, see [Sensitivity analysis](sensitivity.md)) with its scrambling, a random lower triangular matrix per dimension and a random digital shift (LMS+shift), not the scrambling of Owen; the scrambling does not depend on `start`, so a continuation is exact. The random points, the Latin hypercube and the resampling of the subjects of the group means draw from xoshiro256++, not from numpy's PCG64. Reproducing the streams of scipy bit for bit has no value: the draws are held to their distributions (the balance of the Sobol' points, the strata of the hypercube, the moments and correlations of the copula, the mean and the variance of the group means) and to their determinism.

A seed therefore gives other points than the ones sbmlsim drew for it with scipy and numpy, and a result stored with a seed reproduces only with the release of sbmlsim it was computed with: one from before the port to sbmlsim-core 0.42.0 needs that older release. What a seed does not decide, the copula of given points and the statistics of given draws, is the python's.
