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
- the criterion of Guest et al. for ratios: `guest_limits` and `within_guest`.

An array of any shape and of any type numpy converts to doubles is read as its values in
the order numpy ravels it. A sum is the pairwise sum of numpy, so a metric is numpy's to
the last bits.
"""

from sbmlsim_core._core import (
    STATS_MIN_POINTS_SUBSET_AGREEMENT as MIN_POINTS_SUBSET_AGREEMENT,
)
from sbmlsim_core._core import stats_afe as afe
from sbmlsim_core._core import stats_aic_from_mse as aic_from_mse
from sbmlsim_core._core import stats_bic_from_mse as bic_from_mse
from sbmlsim_core._core import stats_fold_accepted as fold_accepted
from sbmlsim_core._core import stats_fold_counts as fold_counts
from sbmlsim_core._core import stats_gmfe as gmfe
from sbmlsim_core._core import stats_guest_limits as guest_limits
from sbmlsim_core._core import stats_mse as mse
from sbmlsim_core._core import stats_nrmse as nrmse
from sbmlsim_core._core import stats_r_squared as r_squared
from sbmlsim_core._core import stats_rmse_from_mse as rmse_from_mse
from sbmlsim_core._core import stats_sd_accepted as sd_accepted
from sbmlsim_core._core import stats_sse as sse
from sbmlsim_core._core import stats_subset_agreement as subset_agreement
from sbmlsim_core._core import stats_within_guest as within_guest

__all__ = [
    "MIN_POINTS_SUBSET_AGREEMENT",
    "afe",
    "aic_from_mse",
    "bic_from_mse",
    "fold_accepted",
    "fold_counts",
    "gmfe",
    "guest_limits",
    "mse",
    "nrmse",
    "r_squared",
    "rmse_from_mse",
    "sd_accepted",
    "sse",
    "subset_agreement",
    "within_guest",
]
