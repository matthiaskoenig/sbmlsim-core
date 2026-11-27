"""The statistics of the analyses of sbmlsim: the kernels of numpy and scipy in Rust.

The crate `sbmlsim-stats`, which `sbmlsim.fit.metrics` and `sbmlsim.fit.evaluation` run
on, the functions of the same names and edges: a value of an argument the python refuses
raises a `ValueError` with its message, and `nan` is returned where it returns `nan`.

- the metrics of residuals: `sse`, `mse`, `rmse_from_mse`, `aic_from_mse`,
  `bic_from_mse`, `r_squared` and `nrmse`, the root mean square of the normalized
  residuals;
- the criteria of the data points, a value `y` and its prediction `f`: the fold criterion
  of the points with a ratio, a positive and finite value and prediction (`fold_counts`,
  `fold_accepted`), the uncertainty criterion of the points with a standard deviation
  (`sd_accepted`), the fold errors (`gmfe`, `afe`) and the limits of agreement of a subset
  (`subset_agreement`, at least `MIN_POINTS_SUBSET_AGREEMENT` points with a ratio);
- the criterion of Guest et al. for ratios: `guest_limits` and `within_guest`;
- the Fisher information of a fit, `sbmlsim.fit.fisher.FisherInformation`: `gram`, the
  information `J' J` of a jacobian, `fisher_statistics` (the variance of the residuals,
  the eigenvalues, the rank, the condition number, the covariance, the standard errors and
  the correlation, a direction the data do not constrain below `DEFAULT_RANK_TOLERANCE`),
  `fisher_intervals` and `fisher_relative_errors`;
- the posterior, `sbmlsim.fit.posterior`: `posterior_laplace`, the covariance of the
  Laplace approximation of a precision, `rank_correlation`, `normal_correlation`, `log_sd`
  and `lognormal_cv`, the marginal of a parameter on a logarithmic scale.
- the uncertainty, `sbmlsim.uncertainty`: the statistics of draws with their errors,
  `mean_se`, `sd_se` and `quantile_ci` (a quantile and its distribution-free interval),
  `quantile` and `quantiles` (numpy's linear ones), `first_order_given_data` (the
  first-order index of an input from given draws), `precise` (`_precise` of the
  propagation, the precision of the draws of an output) and `interval_parameters`, the
  parameters of the distribution of a reported interval of `Uncertain.interval`. They drop
  the draws which are `NaN`, and raise the exception of the python where it raises one
  (an `OverflowError`, a `ZeroDivisionError` or an `IndexError` besides a `ValueError`).
- the draws and the correlations of an uncertainty analysis, `sbmlsim.uncertainty.draws`
  and `.covariance`: `unit_points` (the points of the unit cube of the designs `sobol`,
  `lhs` and `random`), `copula` (the Gaussian copula of a correlation of the normal scores,
  the points clipped to `EPS` from the ends), `normal_score_rho` (the coefficient of the
  normal scores of a covariance of two marginals), `nearest_correlation` (Higham 2002),
  `is_psd` (at the tolerance `PSD_RTOL`) and `validate_covariance` (`Covariance._validate`
  with its messages). The points are the streams of sbmlsim-core (its Sobol' sequence and
  xoshiro256++), not those of scipy and numpy: one seed gives the same points on every
  platform, and the draws match the python in their distributions.
- the probability statements, `sbmlsim.uncertainty.testing`, from whether every draw
  satisfies a hypothesis: `p_values` (the anytime-valid p-values of the sides `greater`,
  `less` and `equivalent` within a margin, `MARGIN` by default), `confidence_sequence`
  (of the Jeffreys mixture `MIXTURE`, each end by `BISECTIONS` bisections), `decide` and
  `decision` (`_decide`), `needed` (the draws an undecided statement needs, up to
  `MAX_NEEDED`) and `family_level` (Bonferroni).
- the visual predictive check, `sbmlsim.fit.variability`: `group_means` (the mean or the
  geometric mean of virtual groups of subjects resampled with replacement, on the stream
  xoshiro256++ of sbmlsim-core, not numpy's) and `bands` (numpy's linear quantiles of the
  finite values of every data point).

A scale of a parameter is a name of `ParameterScaleType` (`"LINEAR"`, `"LOG"`,
`"LOG10"`), of PEtab (`"lin"`, `"log"`, `"log10"`) or a member of `ParameterScaleType`.

An array of any shape and of any type numpy converts to doubles is read as its values in
the order numpy ravels it. A sum is the pairwise sum of numpy, so a metric is numpy's to
the last bits.
"""

from sbmlsim_core._core import STATS_BISECTIONS as BISECTIONS
from sbmlsim_core._core import (
    STATS_DEFAULT_RANK_TOLERANCE as DEFAULT_RANK_TOLERANCE,
)
from sbmlsim_core._core import STATS_EPS as EPS
from sbmlsim_core._core import STATS_MARGIN as MARGIN
from sbmlsim_core._core import STATS_MAX_NEEDED as MAX_NEEDED
from sbmlsim_core._core import (
    STATS_MIN_POINTS_SUBSET_AGREEMENT as MIN_POINTS_SUBSET_AGREEMENT,
)
from sbmlsim_core._core import STATS_MIXTURE as MIXTURE
from sbmlsim_core._core import STATS_PSD_RTOL as PSD_RTOL
from sbmlsim_core._core import stats_afe as afe
from sbmlsim_core._core import stats_aic_from_mse as aic_from_mse
from sbmlsim_core._core import stats_bands as bands
from sbmlsim_core._core import stats_bic_from_mse as bic_from_mse
from sbmlsim_core._core import stats_confidence_sequence as confidence_sequence
from sbmlsim_core._core import stats_copula as copula
from sbmlsim_core._core import stats_decide as decide
from sbmlsim_core._core import stats_decision as decision
from sbmlsim_core._core import stats_family_level as family_level
from sbmlsim_core._core import stats_first_order_given_data as first_order_given_data
from sbmlsim_core._core import stats_fisher_intervals as fisher_intervals
from sbmlsim_core._core import stats_fisher_relative_errors as fisher_relative_errors
from sbmlsim_core._core import stats_fisher_statistics as fisher_statistics
from sbmlsim_core._core import stats_fold_accepted as fold_accepted
from sbmlsim_core._core import stats_fold_counts as fold_counts
from sbmlsim_core._core import stats_gmfe as gmfe
from sbmlsim_core._core import stats_gram as gram
from sbmlsim_core._core import stats_group_means as group_means
from sbmlsim_core._core import stats_guest_limits as guest_limits
from sbmlsim_core._core import stats_interval_parameters as interval_parameters
from sbmlsim_core._core import stats_is_psd as is_psd
from sbmlsim_core._core import stats_log_sd as log_sd
from sbmlsim_core._core import stats_lognormal_cv as lognormal_cv
from sbmlsim_core._core import stats_mean_se as mean_se
from sbmlsim_core._core import stats_mse as mse
from sbmlsim_core._core import stats_nearest_correlation as nearest_correlation
from sbmlsim_core._core import stats_needed as needed
from sbmlsim_core._core import stats_normal_correlation as normal_correlation
from sbmlsim_core._core import stats_normal_score_rho as normal_score_rho
from sbmlsim_core._core import stats_nrmse as nrmse
from sbmlsim_core._core import stats_p_values as p_values
from sbmlsim_core._core import stats_posterior_laplace as posterior_laplace
from sbmlsim_core._core import stats_precise as precise
from sbmlsim_core._core import stats_quantile as quantile
from sbmlsim_core._core import stats_quantile_ci as quantile_ci
from sbmlsim_core._core import stats_quantiles as quantiles
from sbmlsim_core._core import stats_r_squared as r_squared
from sbmlsim_core._core import stats_rank_correlation as rank_correlation
from sbmlsim_core._core import stats_rmse_from_mse as rmse_from_mse
from sbmlsim_core._core import stats_sd_accepted as sd_accepted
from sbmlsim_core._core import stats_sd_se as sd_se
from sbmlsim_core._core import stats_sse as sse
from sbmlsim_core._core import stats_subset_agreement as subset_agreement
from sbmlsim_core._core import stats_unit_points as unit_points
from sbmlsim_core._core import stats_validate_covariance as validate_covariance
from sbmlsim_core._core import stats_within_guest as within_guest

__all__ = [
    "BISECTIONS",
    "DEFAULT_RANK_TOLERANCE",
    "EPS",
    "MARGIN",
    "MAX_NEEDED",
    "MIN_POINTS_SUBSET_AGREEMENT",
    "MIXTURE",
    "PSD_RTOL",
    "afe",
    "aic_from_mse",
    "bands",
    "bic_from_mse",
    "confidence_sequence",
    "copula",
    "decide",
    "decision",
    "family_level",
    "first_order_given_data",
    "fisher_intervals",
    "fisher_relative_errors",
    "fisher_statistics",
    "fold_accepted",
    "fold_counts",
    "gmfe",
    "gram",
    "group_means",
    "guest_limits",
    "interval_parameters",
    "is_psd",
    "log_sd",
    "lognormal_cv",
    "mean_se",
    "mse",
    "nearest_correlation",
    "needed",
    "normal_correlation",
    "normal_score_rho",
    "nrmse",
    "p_values",
    "posterior_laplace",
    "precise",
    "quantile",
    "quantile_ci",
    "quantiles",
    "r_squared",
    "rank_correlation",
    "rmse_from_mse",
    "sd_accepted",
    "sd_se",
    "sse",
    "subset_agreement",
    "unit_points",
    "validate_covariance",
    "within_guest",
]
