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

A scale of a parameter is a name of `ParameterScaleType` (`"LINEAR"`, `"LOG"`,
`"LOG10"`), of PEtab (`"lin"`, `"log"`, `"log10"`) or a member of `ParameterScaleType`.

An array of any shape and of any type numpy converts to doubles is read as its values in
the order numpy ravels it. A sum is the pairwise sum of numpy, so a metric is numpy's to
the last bits.
"""

from sbmlsim_core._core import (
    STATS_DEFAULT_RANK_TOLERANCE as DEFAULT_RANK_TOLERANCE,
)
from sbmlsim_core._core import (
    STATS_MIN_POINTS_SUBSET_AGREEMENT as MIN_POINTS_SUBSET_AGREEMENT,
)
from sbmlsim_core._core import stats_afe as afe
from sbmlsim_core._core import stats_aic_from_mse as aic_from_mse
from sbmlsim_core._core import stats_bic_from_mse as bic_from_mse
from sbmlsim_core._core import stats_fisher_intervals as fisher_intervals
from sbmlsim_core._core import stats_fisher_relative_errors as fisher_relative_errors
from sbmlsim_core._core import stats_fisher_statistics as fisher_statistics
from sbmlsim_core._core import stats_fold_accepted as fold_accepted
from sbmlsim_core._core import stats_fold_counts as fold_counts
from sbmlsim_core._core import stats_gmfe as gmfe
from sbmlsim_core._core import stats_gram as gram
from sbmlsim_core._core import stats_guest_limits as guest_limits
from sbmlsim_core._core import stats_log_sd as log_sd
from sbmlsim_core._core import stats_lognormal_cv as lognormal_cv
from sbmlsim_core._core import stats_mse as mse
from sbmlsim_core._core import stats_normal_correlation as normal_correlation
from sbmlsim_core._core import stats_nrmse as nrmse
from sbmlsim_core._core import stats_posterior_laplace as posterior_laplace
from sbmlsim_core._core import stats_r_squared as r_squared
from sbmlsim_core._core import stats_rank_correlation as rank_correlation
from sbmlsim_core._core import stats_rmse_from_mse as rmse_from_mse
from sbmlsim_core._core import stats_sd_accepted as sd_accepted
from sbmlsim_core._core import stats_sse as sse
from sbmlsim_core._core import stats_subset_agreement as subset_agreement
from sbmlsim_core._core import stats_within_guest as within_guest

__all__ = [
    "DEFAULT_RANK_TOLERANCE",
    "MIN_POINTS_SUBSET_AGREEMENT",
    "afe",
    "aic_from_mse",
    "bic_from_mse",
    "fisher_intervals",
    "fisher_relative_errors",
    "fisher_statistics",
    "fold_accepted",
    "fold_counts",
    "gmfe",
    "gram",
    "guest_limits",
    "log_sd",
    "lognormal_cv",
    "mse",
    "normal_correlation",
    "nrmse",
    "posterior_laplace",
    "r_squared",
    "rank_correlation",
    "rmse_from_mse",
    "sd_accepted",
    "sse",
    "subset_agreement",
    "within_guest",
]
