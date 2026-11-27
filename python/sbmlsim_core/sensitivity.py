"""The global sensitivity analysis: designs on the unit cube and the estimators of their indices.

The designs and the estimators of SALib in Rust (the crate `sbmlsim-sensitivity`), which
`sbmlsim.sensitivity` runs on:

- `saltelli` (on `sobol_sequence`) and `sobol_indices`: the first order, total and second
  order Sobol indices;
- `fast_sample` and `fast_indices`: the first order and total indices of the extended FAST;
- `morris_sample` and `morris_effects`: the elementary effects of Morris.

A design is a matrix of a row per point and a column per parameter on the unit cube, which a
caller maps through the distributions of its parameters. An estimator takes the values of the
points as a matrix of a row per point and a column per element (the time points of a
timecourse, the conditions of a scan) and estimates every element in the same pass, with
bootstrap intervals over resamples drawn from a seed. A seed gives the same points and the
same intervals on every platform, but not the ones SALib draws for it, since the random
streams are not numpy's; see the page Sensitivity analysis of the documentation.
"""

from sbmlsim_core._core import sensitivity_fast_indices as fast_indices
from sbmlsim_core._core import sensitivity_fast_sample as fast_sample
from sbmlsim_core._core import sensitivity_morris_effects as morris_effects
from sbmlsim_core._core import sensitivity_morris_sample as morris_sample
from sbmlsim_core._core import sensitivity_saltelli as saltelli
from sbmlsim_core._core import sensitivity_sobol_indices as sobol_indices
from sbmlsim_core._core import sensitivity_sobol_sequence as sobol_sequence

__all__ = [
    "fast_indices",
    "fast_sample",
    "morris_effects",
    "morris_sample",
    "saltelli",
    "sobol_indices",
    "sobol_sequence",
]
