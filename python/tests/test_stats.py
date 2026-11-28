"""Tests of the statistics of the analyses, `sbmlsim_core.stats`, against the values of
sbmlsim: the fixtures of the crate sbmlsim-stats (`crates/stats/oracles/<module>.json`),
which `crates/stats/oracles/make_oracles.py` computed once with the functions of sbmlsim
the bindings replace. A call of the python which raised a ValueError is its message there,
and a binding raises a ValueError with the same one."""

import builtins
import enum
import json
import math
import time
from pathlib import Path
from typing import Any, Literal

import numpy
import pytest
from sbmlsim_core import stats

ORACLES = Path(__file__).resolve().parents[2] / "crates" / "stats" / "oracles"

#: the designs of the unit points
Design = Literal["sobol", "lhs", "random"]
#: the tolerance of a deterministic kernel, relative
KERNEL = 1e-12
#: the tolerance of a value of the t distribution, relative
DISTRIBUTION = 1e-10


def decode(value: Any) -> Any:
    """A value of a fixture with the strings "nan", "inf" and "-inf" as their doubles."""
    if isinstance(value, dict):
        return {key: decode(item) for key, item in value.items()}
    if isinstance(value, list):
        return [decode(item) for item in value]
    if value in ("nan", "inf", "-inf"):
        return float(value)
    return value


def cases(module: str, prefix: str) -> dict[str, dict[str, Any]]:
    """The cases of the fixture of `module` whose names start with `prefix`."""
    fixture = json.loads((ORACLES / f"{module}.json").read_text(encoding="utf-8"))
    found = {
        name: decode(case) for name, case in fixture.items() if name.startswith(prefix)
    }
    assert found, f"{module}.json has no case {prefix}*"
    return found


METRICS = cases("metrics", "")


def of(prefix: str) -> list[tuple[str, dict[str, Any]]]:
    """The cases of `metrics.json` whose names start with `prefix`, as test parameters."""
    return [(name, case) for name, case in METRICS.items() if name.startswith(prefix)]


def assert_outcome(call: Any, expected: Any, rtol: float = KERNEL) -> Any:
    """Hold a call of a binding to the outcome of the python: the same message of the
    ValueError, or values within `rtol`; returns the value."""
    if isinstance(expected, dict) and "error" in expected:
        with pytest.raises(ValueError) as raised:
            call()
        assert str(raised.value) == expected["error"]
        return None
    found = call()
    numpy.testing.assert_allclose(found, expected, rtol=rtol, atol=0.0)
    return found


@pytest.mark.parametrize(("name", "case"), of("residuals/"))
def test_the_metrics_of_residuals_are_the_ones_of_sbmlsim(
    name: str, case: dict[str, Any]
) -> None:
    x = numpy.asarray(case["inputs"]["x"])
    k = case["inputs"]["k"]
    expected = case["expected"]
    assert_outcome(lambda: stats.sse(x), expected["sse"])
    mse = assert_outcome(lambda: stats.mse(x), expected["mse"])
    if mse is not None:
        assert_outcome(lambda: stats.rmse_from_mse(mse), expected["rmse"])
        assert_outcome(lambda: stats.aic_from_mse(mse, x.size, k), expected["aic"])
        assert_outcome(lambda: stats.bic_from_mse(mse, x.size, k), expected["bic"])
    assert_outcome(lambda: stats.nrmse(x), expected["nrmse"])


def test_the_metrics_of_an_mse_are_the_ones_of_sbmlsim_and_refuse_as_it_does() -> None:
    case = METRICS["from_mse"]
    inputs, expected = case["inputs"], case["expected"]
    for index, (mse, n, k) in enumerate(
        zip(inputs["mse"], inputs["n"], inputs["k"], strict=True)
    ):
        assert_outcome(
            lambda mse=mse: stats.rmse_from_mse(mse), expected["rmse"][index]
        )
        assert_outcome(
            lambda mse=mse, n=n, k=k: stats.aic_from_mse(mse, n, k),
            expected["aic"][index],
        )
        assert_outcome(
            lambda mse=mse, n=n, k=k: stats.bic_from_mse(mse, n, k),
            expected["bic"][index],
        )


@pytest.mark.parametrize(("name", "case"), of("r_squared/"))
def test_r_squared_is_the_one_of_sbmlsim(name: str, case: dict[str, Any]) -> None:
    y, f = case["inputs"]["y"], case["inputs"]["f"]
    assert_outcome(lambda: stats.r_squared(y, f), case["expected"])


@pytest.mark.parametrize(("name", "case"), of("fold/"))
def test_the_fold_criteria_are_the_ones_of_sbmlsim(
    name: str, case: dict[str, Any]
) -> None:
    inputs, expected = case["inputs"], case["expected"]
    y, f, fold = (
        numpy.asarray(inputs["y"]),
        numpy.asarray(inputs["f"]),
        inputs["fold"],
    )
    counts = assert_outcome(lambda: stats.fold_counts(y, f, fold), expected["counts"])
    if counts is None:
        # the criterion of a point refuses what the counts refuse
        assert_outcome(lambda: stats.fold_accepted(y, f, fold), expected["counts"])
        return
    assert isinstance(counts, tuple)
    assert all(isinstance(count, int) for count in counts)
    accepted = stats.fold_accepted(y, f, fold)
    assert accepted.dtype == numpy.float64
    numpy.testing.assert_array_equal(accepted, expected["accepted"])


def test_the_fold_factor_is_twofold_by_default_and_none_is_refused() -> None:
    y, f = [1.0, 1.0, 1.0, 4.0], [2.0, 2.5, 0.5, 1.9]
    assert stats.fold_counts(y, f) == stats.fold_counts(y, f, 2) == (2, 4)
    numpy.testing.assert_array_equal(
        stats.fold_accepted(y, f), stats.fold_accepted(y, f, fold=2)
    )
    numpy.testing.assert_array_equal(stats.fold_accepted(y, f), [1.0, 0.0, 1.0, 0.0])
    # as the python, whose fold factor is a float: None is no number
    with pytest.raises(TypeError):
        stats.fold_counts(y, f, None)  # ty: ignore[invalid-argument-type]
    with pytest.raises(TypeError):
        stats.fold_accepted(y, f, fold=None)  # ty: ignore[invalid-argument-type]
    with pytest.raises(TypeError):
        stats.rmse_from_mse(None)  # ty: ignore[invalid-argument-type]


@pytest.mark.parametrize(("name", "case"), of("sd/"))
def test_the_uncertainty_criterion_is_the_one_of_sbmlsim(
    name: str, case: dict[str, Any]
) -> None:
    inputs = case["inputs"]
    accepted = stats.sd_accepted(inputs["y"], inputs["f"], inputs["sd"], inputs["k"])
    numpy.testing.assert_array_equal(accepted, case["expected"])


def test_the_uncertainty_criterion_is_two_sd_by_default_and_refuses_other_lengths() -> (
    None
):
    numpy.testing.assert_array_equal(
        stats.sd_accepted([1.0, 1.0], [3.0, 3.1], [1.0, 1.0]), [1.0, 0.0]
    )
    with pytest.raises(ValueError, match="standard deviations as data points"):
        stats.sd_accepted([1.0, 2.0], [1.0, 2.0], [1.0])


@pytest.mark.parametrize(("name", "case"), of("fold_errors/"))
def test_the_fold_errors_and_the_agreement_are_the_ones_of_sbmlsim(
    name: str, case: dict[str, Any]
) -> None:
    y, f = case["inputs"]["y"], case["inputs"]["f"]
    expected = case["expected"]
    assert_outcome(lambda: stats.gmfe(y, f), expected["gmfe"])
    assert_outcome(lambda: stats.afe(y, f), expected["afe"])
    agreement = stats.subset_agreement(y, f)
    if expected["subset_agreement"] is None:
        assert agreement is None
        return
    assert isinstance(agreement, tuple)
    assert len(agreement) == 4
    bias, limit, bias_ci, limit_ci = expected["subset_agreement"]
    numpy.testing.assert_allclose(agreement[:2], [bias, limit], rtol=KERNEL)
    # with the quantile of the t distribution
    numpy.testing.assert_allclose(agreement[2:], [bias_ci, limit_ci], rtol=DISTRIBUTION)


def test_a_subset_needs_ten_points_with_a_ratio() -> None:
    assert stats.MIN_POINTS_SUBSET_AGREEMENT == 10
    assert stats.subset_agreement([1.0] * 9, [2.0] * 9) is None
    assert stats.subset_agreement([1.0] * 10, [2.0] * 10) is not None


def test_the_limits_of_guest_are_the_ones_of_sbmlsim() -> None:
    case = METRICS["guest_limits"]
    low, high = stats.guest_limits(case["inputs"]["ratio"])
    numpy.testing.assert_allclose(low, case["expected"]["low"], rtol=KERNEL)
    numpy.testing.assert_allclose(high, case["expected"]["high"], rtol=KERNEL)
    # the shape of the ratios, as numpy
    ratios = numpy.array([[0.5, 1.0, 2.0], [3.0, 4.0, 5.0]])
    low, high = stats.guest_limits(ratios)
    assert low.shape == high.shape == (2, 3)
    numpy.testing.assert_array_equal(low.ravel(), stats.guest_limits(ratios.ravel())[0])
    low, high = stats.guest_limits(2.0)
    assert low.shape == high.shape == ()


@pytest.mark.parametrize(("name", "case"), of("within_guest/"))
def test_within_guest_is_the_one_of_sbmlsim_and_refuses_as_it_does(
    name: str, case: dict[str, Any]
) -> None:
    observed, predicted = case["inputs"]["observed"], case["inputs"]["predicted"]
    assert_outcome(lambda: stats.within_guest(observed, predicted), case["expected"])


def test_within_guest_writes_the_observed_ratios_of_any_shape_as_python() -> None:
    observed = numpy.array([[1.0, -2.0], [3.0, 4.0]])
    with pytest.raises(ValueError) as raised:
        stats.within_guest(observed, numpy.ones((2, 2)))
    assert str(raised.value) == (
        "The observed ratios must be positive and finite: [[1.0, -2.0], [3.0, 4.0]]."
    )
    # equal sizes of other shapes, which the python refuses as well
    with pytest.raises(ValueError, match="different lengths, 4 and 4"):
        stats.within_guest(numpy.ones((2, 2)), numpy.ones(4))


def test_a_strided_an_integer_and_a_fortran_array_are_read_as_their_values() -> None:
    case = METRICS["residuals/random_100"]
    x = numpy.asarray(case["inputs"]["x"])
    # every second value of an array twice as long, and a column of a matrix
    strided = numpy.repeat(x, 2)[::2]
    column = numpy.stack([x, -x], axis=1)[:, 0]
    for array in (strided, column):
        assert not array.flags.c_contiguous
        assert stats.sse(array) == case["expected"]["sse"]
        assert stats.mse(array) == case["expected"]["mse"]
        assert stats.nrmse(array) == case["expected"]["nrmse"]
    # a matrix in Fortran order is read in the order of its values, as numpy ravels it
    matrix = numpy.asfortranarray(x.reshape(10, 10))
    assert not matrix.flags.c_contiguous
    assert stats.sse(matrix) == stats.sse(matrix.ravel()) == case["expected"]["sse"]
    # integers, converted to doubles as the python converts them
    case = METRICS["fold/integers"]
    y = numpy.asarray(case["inputs"]["y"])
    f = numpy.asarray(case["inputs"]["f"])
    assert y.dtype.kind == f.dtype.kind == "i"
    assert stats.fold_counts(y, f, 2.0) == tuple(case["expected"]["counts"])
    numpy.testing.assert_array_equal(
        stats.fold_accepted(y, f, 2), case["expected"]["accepted"]
    )
    # views of doubles with a stride of two values and a negative one, which numpy
    # does not copy and the binding reads in the order of their values
    y_float = numpy.repeat(y.astype(numpy.float64), 2)[::2]
    f_float = numpy.repeat(f.astype(numpy.float64), 2)[::2]
    assert y_float.dtype == numpy.float64
    assert not y_float.flags.c_contiguous
    assert y_float.base is not None
    assert stats.fold_counts(y_float, f_float) == tuple(case["expected"]["counts"])
    numpy.testing.assert_array_equal(
        stats.fold_accepted(y_float, f_float), case["expected"]["accepted"]
    )
    y_reversed = y.astype(numpy.float64)[::-1]
    f_reversed = f.astype(numpy.float64)[::-1]
    assert y_reversed.strides[0] < 0
    assert stats.fold_counts(y_reversed, f_reversed) == tuple(
        case["expected"]["counts"]
    )
    numpy.testing.assert_array_equal(
        stats.fold_accepted(y_reversed, f_reversed),
        case["expected"]["accepted"][::-1],
    )
    # a strided array of integers
    y_strided = numpy.repeat(y, 3)[::3]
    assert not y_strided.flags.c_contiguous
    assert stats.fold_counts(y_strided, f) == tuple(case["expected"]["counts"])
    case = METRICS["residuals/integers"]
    x = numpy.asarray(case["inputs"]["x"], dtype=numpy.int32)
    assert stats.sse(x) == case["expected"]["sse"]
    assert stats.mse(x[::-1]) == case["expected"]["mse"]
    # the residuals of a negative stride are summed in their order, as numpy sums the copy
    case = METRICS["residuals/random_1000"]
    x = numpy.asarray(case["inputs"]["x"])
    reversed_view = x[::-1]
    assert reversed_view.strides[0] < 0
    assert stats.sse(reversed_view) == stats.sse(reversed_view.copy())
    assert stats.sse(reversed_view) == pytest.approx(
        case["expected"]["sse"], rel=KERNEL
    )


def large_residuals(n: int) -> numpy.ndarray:
    """The residuals of `large_residuals` of `crates/stats/oracles/make_oracles.py`."""
    i = numpy.arange(n, dtype=numpy.int64)
    a = (i * 2654435761) % 1000003
    return (a.astype(float) / 1000003.0 - 0.5) * numpy.ldexp(
        1.0, (i % 31 - 15).astype(int)
    )


@pytest.mark.parametrize(("name", "case"), of("large_residuals/"))
def test_the_metrics_of_many_residuals_are_the_pairwise_sums_of_numpy(
    name: str, case: dict[str, Any]
) -> None:
    # more than a block of the pairwise sum and than a buffer of numpy, bit for bit
    x = large_residuals(case["inputs"]["n"])
    expected = case["expected"]
    assert x[:4].tolist() == expected["first"]
    assert stats.sse(x) == expected["sse"]
    assert stats.mse(x) == expected["mse"]
    assert stats.nrmse(x) == expected["nrmse"]


@pytest.mark.parametrize(
    ("call", "message"),
    [
        (
            lambda: stats.fold_counts([1.0], [1.0], 1),
            "The fold factor must be larger than 1, but '1' given.",
        ),
        (
            lambda: stats.fold_accepted([1.0], [1.0], numpy.int64(0)),
            "The fold factor must be larger than 1, but '0' given.",
        ),
        (
            lambda: stats.fold_counts([1.0], [1.0], numpy.float64(0.5)),
            "The fold factor must be larger than 1, but '0.5' given.",
        ),
        (
            lambda: stats.rmse_from_mse(-1),
            "RMSE requires a non-negative MSE, but '-1' given.",
        ),
        (
            lambda: stats.aic_from_mse(-3, 10, 2),
            "AIC requires a non-negative MSE, but '-3' given.",
        ),
        (
            lambda: stats.bic_from_mse(numpy.int32(-2), 10, 2),
            "BIC requires a non-negative MSE, but '-2' given.",
        ),
        (
            lambda: stats.aic_from_mse(0, 0, 2),
            "AIC requires a positive number of points, but '0' given.",
        ),
    ],
)
def test_a_number_given_as_an_integer_is_written_as_python_writes_it(
    call: Any, message: str
) -> None:
    # the python writes the object it was given, `'1'` of the int 1 and not `'1.0'`
    with pytest.raises(ValueError) as raised:
        call()
    assert str(raised.value) == message


# --- fisher and posterior ---------------------------------------------------------------------

FISHER = cases("fisher", "")


def of_fisher(prefix: str) -> list[tuple[str, dict[str, Any]]]:
    """The cases of `fisher.json` whose names start with `prefix`, as test parameters."""
    return [(name, case) for name, case in FISHER.items() if name.startswith(prefix)]


def square(values: list[float], k: int) -> numpy.ndarray:
    """A row-major list of `k * k` values as the matrix."""
    return numpy.asarray(values, dtype=float).reshape(k, k)


def assert_matrix_close(found: numpy.ndarray, expected: numpy.ndarray) -> None:
    """Hold a matrix to the expected one at `KERNEL` relative to `sqrt(|e_ii e_jj|)`, not a
    number to not a number."""
    assert found.shape == expected.shape
    numpy.testing.assert_array_equal(numpy.isnan(found), numpy.isnan(expected))
    root = numpy.sqrt(numpy.abs(numpy.diag(expected)))
    scale = numpy.outer(root, root)
    both = ~numpy.isnan(expected) & (found != expected)
    assert numpy.all(numpy.abs(found - expected)[both] <= KERNEL * scale[both])


@pytest.mark.parametrize(("name", "case"), of_fisher("fisher/"))
def test_the_fisher_statistics_are_the_ones_of_sbmlsim(
    name: str, case: dict[str, Any]
) -> None:
    inputs, expected = case["inputs"], case["expected"]
    k = inputs["k"]
    jacobian = numpy.asarray(inputs["j"]).reshape(inputs["rows"], k)
    assert_matrix_close(stats.gram(jacobian), square(expected["gram"], k))
    found = stats.fisher_statistics(
        square(inputs["information"], k),
        inputs["cost"],
        inputs["n"],
        likelihood=inputs["likelihood"],
    )
    assert set(found) == {
        "sigma2",
        "spectrum",
        "rank",
        "condition",
        "covariance",
        "standard_errors",
        "correlation",
    }
    numpy.testing.assert_allclose(found["sigma2"], expected["sigma2"], rtol=KERNEL)
    spectrum = numpy.asarray(expected["spectrum"])
    numpy.testing.assert_allclose(
        found["spectrum"], spectrum, rtol=0.0, atol=KERNEL * numpy.abs(spectrum).max()
    )
    assert found["rank"] == expected["rank"]
    se = numpy.asarray(expected["standard_errors"])
    numpy.testing.assert_array_equal(
        numpy.isnan(found["standard_errors"]), numpy.isnan(se)
    )
    if found["rank"] < k:
        # the smallest eigenvalue is the noise of the rounding, both say rank deficient
        assert found["condition"] >= 1e8
        assert expected["condition"] >= 1e8
    else:
        numpy.testing.assert_allclose(
            found["condition"], expected["condition"], rtol=KERNEL
        )
    assert_matrix_close(found["covariance"], square(expected["covariance"], k))
    numpy.testing.assert_allclose(found["standard_errors"], se, rtol=KERNEL)
    numpy.testing.assert_allclose(
        found["correlation"],
        square(expected["correlation"], k),
        rtol=0.0,
        atol=KERNEL,
    )


@pytest.mark.parametrize(("name", "case"), of_fisher("fisher/"))
def test_the_intervals_and_the_relative_errors_are_the_ones_of_sbmlsim(
    name: str, case: dict[str, Any]
) -> None:
    inputs, expected = case["inputs"], case["expected"]
    lower, upper = stats.fisher_intervals(
        inputs["values"],
        expected["standard_errors"],
        inputs["scales"],
        level=inputs["alpha"],
        dof=inputs["dof"],
    )
    numpy.testing.assert_allclose(lower, expected["lower"], rtol=KERNEL, atol=0.0)
    numpy.testing.assert_allclose(upper, expected["upper"], rtol=KERNEL, atol=0.0)
    numpy.testing.assert_allclose(
        stats.fisher_relative_errors(
            inputs["values"], expected["standard_errors"], inputs["scales"]
        ),
        expected["relative_errors"],
        rtol=KERNEL,
        atol=0.0,
    )


class ParameterScaleType(enum.Enum):
    """The scales of `sbmlsim.fit.options.ParameterScaleType`, by their names."""

    LINEAR = 1
    LOG = 2
    LOG10 = 3


def test_a_scale_is_its_name_its_petab_name_or_a_member_of_an_enum() -> None:
    values, se = [2.0, 3.0, 4.0], [0.1, 0.2, 0.3]
    expected = stats.fisher_intervals(values, se, ["LINEAR", "LOG", "LOG10"])
    for scales in (
        ["lin", "log", "log10"],
        [ParameterScaleType.LINEAR, ParameterScaleType.LOG, ParameterScaleType.LOG10],
        ("LINEAR", ParameterScaleType.LOG, "log10"),
    ):
        found = stats.fisher_intervals(values, se, scales)
        numpy.testing.assert_array_equal(found, expected)
    with pytest.raises(ValueError, match="'ln' is none of"):
        stats.fisher_relative_errors(values, se, ["LINEAR", "ln", "LOG"])
    with pytest.raises(ValueError, match="3 values, 2 standard errors and 3 scales"):
        stats.fisher_intervals(values, se[:2], ["LOG"] * 3)
    assert stats.log_sd(ParameterScaleType.LOG10, 1.0) == math.log(10.0)
    assert stats.log_sd("log", 0.5) == 0.5


@pytest.mark.parametrize(("name", "case"), of_fisher("posterior/"))
def test_the_laplace_approximation_is_the_one_of_sbmlsim(
    name: str, case: dict[str, Any]
) -> None:
    inputs, expected = case["inputs"], case["expected"]
    k = inputs["k"]
    precision = square(inputs["precision"], k)
    call = lambda: stats.posterior_laplace(  # noqa: E731
        precision, pids=inputs["pids"], opid=inputs["opid"]
    )
    if "error" in expected:
        assert_outcome(call, expected)
        with pytest.raises(ValueError, match=r"^the data and the priors do not"):
            stats.posterior_laplace(precision)
        return
    covariance = call()
    assert_matrix_close(covariance, square(expected["covariance"], k))
    numpy.testing.assert_array_equal(covariance, stats.posterior_laplace(precision))
    r = square(expected["correlation"], k)
    numpy.testing.assert_allclose(
        stats.rank_correlation(r),
        square(expected["rank_correlation"], k),
        rtol=0.0,
        atol=KERNEL,
    )
    numpy.testing.assert_allclose(
        stats.normal_correlation(r),
        square(expected["normal_correlation"], k),
        rtol=0.0,
        atol=KERNEL,
    )
    sd = expected["standard_deviations"]
    log_sd = [
        stats.log_sd(scale, s) for scale, s in zip(inputs["scales"], sd, strict=True)
    ]
    numpy.testing.assert_allclose(log_sd, expected["log_sd"], rtol=KERNEL)
    cv = [
        math.nan if scale == "LINEAR" else stats.lognormal_cv(sigma)
        for scale, sigma in zip(inputs["scales"], log_sd, strict=True)
    ]
    numpy.testing.assert_allclose(cv, expected["cv"], rtol=KERNEL)
    for level, label in ((0.95, "95"), (0.5, "50")):
        lower, upper = stats.fisher_intervals(
            inputs["values"], sd, inputs["scales"], level=level
        )
        numpy.testing.assert_allclose(lower, expected[f"lower{label}"], rtol=KERNEL)
        numpy.testing.assert_allclose(upper, expected[f"upper{label}"], rtol=KERNEL)


@pytest.mark.parametrize(("name", "case"), of_fisher("rank_correlation/"))
def test_the_rank_correlation_clips_as_sbmlsim(name: str, case: dict[str, Any]) -> None:
    k = case["inputs"]["k"]
    numpy.testing.assert_allclose(
        stats.rank_correlation(square(case["inputs"]["r"], k)),
        square(case["expected"], k),
        rtol=KERNEL,
    )


def test_a_singular_information_has_no_errors_of_its_null_space() -> None:
    found = stats.fisher_statistics(numpy.array([[4.0, 0.0], [0.0, 0.0]]), 3.0, 12)
    assert found["rank"] == 1
    assert found["condition"] == math.inf
    assert math.isnan(found["standard_errors"][1])
    assert found["standard_errors"][0] == math.sqrt(0.15)
    assert stats.DEFAULT_RANK_TOLERANCE == 1e-8


def test_a_number_of_points_without_degrees_of_freedom_has_no_variance() -> None:
    information = numpy.array([[2.0, 1.0], [1.0, 3.0]])
    for n in (-3, 0, 2):
        found = stats.fisher_statistics(information, 1.0, n)
        assert math.isnan(found["sigma2"])
        assert numpy.isnan(found["standard_errors"]).all()
        assert (
            stats.fisher_statistics(information, 1.0, n, likelihood=True)["sigma2"]
            == 1.0
        )
    assert stats.fisher_statistics(information, 1.0, 3)["sigma2"] == 2.0


def large_jacobian(rows: int, k: int) -> numpy.ndarray:
    """The jacobian of `large_jacobian` of `crates/stats/oracles/make_oracles.py`."""
    x = numpy.arange(rows * k, dtype=numpy.int64).reshape(rows, k)
    return ((((x * x) % 1000003 * 2654435761 + x) % 1000003) % 19 - 9).astype(float)


def test_the_statistics_of_an_information_of_200_parameters_are_the_ones_of_sbmlsim() -> (
    None
):
    case = FISHER["fisher_large/200"]
    inputs, expected = case["inputs"], case["expected"]
    k = inputs["k"]
    jacobian = large_jacobian(inputs["rows"], k)
    information = stats.gram(jacobian)
    numpy.testing.assert_array_equal(information, jacobian.T @ jacobian)
    found = stats.fisher_statistics(information, inputs["cost"], inputs["n"])
    assert found["rank"] == expected["rank"] == k
    numpy.testing.assert_allclose(found["sigma2"], expected["sigma2"], rtol=KERNEL)
    numpy.testing.assert_allclose(
        found["condition"], expected["condition"], rtol=KERNEL
    )
    spectrum = numpy.asarray(expected["spectrum"])
    numpy.testing.assert_allclose(
        found["spectrum"], spectrum, rtol=0.0, atol=KERNEL * spectrum.max()
    )
    se = numpy.asarray(expected["standard_errors"])
    numpy.testing.assert_allclose(found["standard_errors"], se, rtol=KERNEL)
    stored = inputs["rows_stored"]
    covariance = numpy.asarray(expected["covariance_rows"]).reshape(len(stored), k)
    assert numpy.all(
        numpy.abs(found["covariance"][stored] - covariance)
        <= KERNEL * numpy.outer(se[stored], se)
    )
    numpy.testing.assert_allclose(
        found["correlation"][stored],
        numpy.asarray(expected["correlation_rows"]).reshape(len(stored), k),
        rtol=0.0,
        atol=KERNEL,
    )


def test_the_shapes_of_the_matrices_are_checked() -> None:
    with pytest.raises(ValueError, match="matrix of the residuals by the parameters"):
        stats.gram(numpy.ones(3))
    with pytest.raises(ValueError, match="square matrix"):
        stats.fisher_statistics(numpy.ones((2, 3)), 1.0, 5)
    with pytest.raises(ValueError, match="square matrix"):
        stats.posterior_laplace(numpy.ones(4))
    with pytest.raises(ValueError, match="square matrix"):
        stats.rank_correlation(numpy.ones((3, 2)))
    with pytest.raises(ValueError, match="square matrix"):
        stats.normal_correlation(numpy.ones((1, 2, 2)))
    with pytest.raises(ValueError, match="not 1 names"):
        stats.posterior_laplace(numpy.eye(2), pids=["a"])
    with pytest.raises(ValueError, match="not finite"):
        stats.fisher_statistics(numpy.array([[1.0, math.nan], [math.nan, 1.0]]), 1.0, 5)
    with pytest.raises(ValueError, match="no parameters"):
        stats.fisher_statistics(numpy.ones((0, 0)), 1.0, 5)


def test_a_strided_an_integer_and_a_fortran_matrix_are_read_as_their_values() -> None:
    case = FISHER["fisher/least_squares"]
    inputs, expected = case["inputs"], case["expected"]
    jacobian = numpy.asarray(inputs["j"]).reshape(inputs["rows"], inputs["k"])
    # every second row of a jacobian twice as long, and the jacobian in Fortran order
    strided = numpy.repeat(jacobian, 2, axis=0)[::2]
    fortran = numpy.asfortranarray(jacobian)
    for array in (strided, fortran):
        assert not array.flags.c_contiguous
        numpy.testing.assert_array_equal(stats.gram(array), stats.gram(jacobian))
    information = square(inputs["information"], inputs["k"])
    reference = stats.fisher_statistics(information, inputs["cost"], inputs["n"])
    found = stats.fisher_statistics(
        numpy.asfortranarray(information), inputs["cost"], inputs["n"]
    )
    for key in ("covariance", "standard_errors", "correlation", "spectrum"):
        numpy.testing.assert_array_equal(found[key], reference[key])
    # integers, converted to doubles
    integers = numpy.array([[4, 2], [2, 3]])
    assert integers.dtype.kind == "i"
    numpy.testing.assert_array_equal(
        stats.posterior_laplace(integers), stats.posterior_laplace(integers * 1.0)
    )
    numpy.testing.assert_array_equal(
        stats.gram(numpy.array([[1, 2], [3, 4]])), [[10.0, 14.0], [14.0, 20.0]]
    )
    numpy.testing.assert_allclose(
        stats.fisher_intervals(
            numpy.array([2, 3]), numpy.array([1, 1])[::-1], ["LOG", "LINEAR"]
        ),
        stats.fisher_intervals([2.0, 3.0], [1.0, 1.0], ["LOG", "LINEAR"]),
    )
    assert expected["rank"] == 4


# --- uncertainty ------------------------------------------------------------------------------

UNCERTAINTY = cases("uncertainty", "")

#: the tolerance of a value of the quantile of the normal distribution, relative
NORMAL = 1e-13
#: the probabilities of the quantiles and of their intervals, `QUANTILE_QS` of the oracles
QUANTILE_QS = [0.05, 0.5, 0.95]
#: the probabilities of the quantiles, `QUANTILES_QS` of the oracles
QUANTILES_QS = [0.0, 0.05, 0.25, 0.5, 0.95, 1.0]
#: the exceptions the python raised, by the names the fixture holds
EXCEPTIONS = {
    "ValueError": ValueError,
    "OverflowError": OverflowError,
    "ZeroDivisionError": ZeroDivisionError,
    "IndexError": IndexError,
}


def of_uncertainty(prefix: str) -> list[tuple[str, dict[str, Any]]]:
    """The cases of `uncertainty.json` whose names start with `prefix`, as parameters."""
    return [
        (name, case) for name, case in UNCERTAINTY.items() if name.startswith(prefix)
    ]


def assert_raised(call: Any, expected: Any, rtol: float = KERNEL) -> Any:
    """Hold a call of a binding to the outcome of the python: the same class and message
    of the exception it raised, or values within `rtol`; returns the value."""
    if isinstance(expected, dict) and "error" in expected:
        with pytest.raises(EXCEPTIONS[expected["exception"]]) as raised:
            call()
        assert type(raised.value) is EXCEPTIONS[expected["exception"]]
        assert str(raised.value) == expected["error"]
        return None
    found = call()
    numpy.testing.assert_allclose(
        numpy.asarray(found, dtype=float), expected, rtol=rtol, atol=0.0
    )
    return found


def draws_of(case: dict[str, Any]) -> numpy.ndarray:
    """The draws of a case of the statistics: its `x`, or the formula of its `n`."""
    if "x" in case["inputs"]:
        return numpy.asarray(case["inputs"]["x"], dtype=float)
    x = large_residuals(case["inputs"]["n"])
    assert x[:4].tolist() == case["inputs"]["first"]
    return x


@pytest.mark.parametrize(("name", "case"), of_uncertainty("statistics"))
def test_the_statistics_of_draws_are_the_ones_of_sbmlsim(
    name: str, case: dict[str, Any]
) -> None:
    x = draws_of(case)
    expected = case["expected"]
    mean_se = assert_raised(lambda: stats.mean_se(x), expected["mean_se"])
    assert isinstance(mean_se[2], int)
    assert_raised(lambda: stats.sd_se(x), expected["sd_se"])
    for q, outcome in zip(QUANTILE_QS, expected["quantile_ci"], strict=True):
        found = assert_raised(lambda q=q: stats.quantile_ci(x, q), outcome)
        numpy.testing.assert_array_equal(found, stats.quantile_ci(x, q, 0.95))
    assert_raised(lambda: stats.quantiles(x, QUANTILES_QS), expected["quantiles"])
    for q, outcome in zip(QUANTILES_QS, expected["quantiles"], strict=True):
        assert_raised(lambda q=q: stats.quantile(x, q), outcome)


@pytest.mark.parametrize(("name", "case"), of_uncertainty("statistics_formula/"))
def test_the_mean_of_many_draws_is_the_one_of_numpy_bit_for_bit(
    name: str, case: dict[str, Any]
) -> None:
    x = draws_of(case)
    assert list(stats.mean_se(x)) == case["expected"]["mean_se"]
    assert stats.sd_se(x)[0] == case["expected"]["sd_se"][0]


@pytest.mark.parametrize(
    ("name", "case"), of_uncertainty("quantile_ci/") + of_uncertainty("quantiles/")
)
def test_quantile_ci_refuses_and_returns_nan_as_sbmlsim(
    name: str, case: dict[str, Any]
) -> None:
    inputs = case["inputs"]
    if name.startswith("quantiles/"):
        assert_raised(
            lambda: stats.quantiles(inputs["x"], inputs["qs"]), case["expected"]
        )
        assert_raised(
            lambda: stats.quantile(inputs["x"], max(inputs["qs"])), case["expected"]
        )
        return
    assert_raised(
        lambda: stats.quantile_ci(inputs["x"], inputs["q"], inputs["level"]),
        case["expected"],
    )


def paired_draws(n: int) -> tuple[numpy.ndarray, numpy.ndarray]:
    """The draws of `paired_draws` of `crates/stats/oracles/make_oracles.py`."""
    x = large_residuals(n)
    return x, x * x + large_residuals(n + 13)[13:]


@pytest.mark.parametrize(
    ("name", "case"),
    of_uncertainty("first_order/") + of_uncertainty("first_order_formula/"),
)
def test_the_first_order_index_from_given_data_is_the_one_of_sbmlsim(
    name: str, case: dict[str, Any]
) -> None:
    inputs = case["inputs"]
    if "n" in inputs:
        x, y = paired_draws(inputs["n"])
        assert x[:4].tolist() == inputs["first_x"]
        assert y[:4].tolist() == inputs["first_y"]
        assert stats.first_order_given_data(x, y) == case["expected"]
        return
    x, y = (
        numpy.asarray(inputs["x"], dtype=float),
        numpy.asarray(inputs["y"], dtype=float),
    )
    if inputs["n_bins"] is None:
        assert_raised(lambda: stats.first_order_given_data(x, y), case["expected"])
    assert_raised(
        lambda: stats.first_order_given_data(x, y, inputs["n_bins"]), case["expected"]
    )


@pytest.mark.parametrize(
    ("name", "case"), of_uncertainty("precise/") + of_uncertainty("precise_formula/")
)
def test_the_precision_of_an_output_is_the_one_of_sbmlsim(
    name: str, case: dict[str, Any]
) -> None:
    inputs = case["inputs"]
    values = (
        large_residuals(inputs["n"])
        if "n" in inputs
        else numpy.asarray(inputs["values"], dtype=float)
    )
    found = assert_raised(
        lambda: stats.precise(values, inputs["rel_tol"]), case["expected"]
    )
    if found is not None:
        assert found[0] is case["expected"][0]


@pytest.mark.parametrize(("name", "case"), of_uncertainty("interval/"))
def test_the_parameters_of_an_interval_are_the_ones_of_uncertain_interval(
    name: str, case: dict[str, Any]
) -> None:
    inputs = case["inputs"]
    assert_raised(
        lambda: stats.interval_parameters(
            inputs["low"],
            inputs["high"],
            inputs["level"],
            lognormal=inputs["lognormal"],
        ),
        case["expected"],
        rtol=NORMAL,
    )


def test_an_interval_writes_its_numbers_as_python_writes_them() -> None:
    # the python writes the objects it was given, `2` of the int 2 and not `2.0`
    for call, message in [
        (
            lambda: stats.interval_parameters(2, 1, lognormal=False),
            "The interval [2, 1] is empty.",
        ),
        (
            lambda: stats.interval_parameters(1, numpy.int64(1), lognormal=True),
            "The interval [1, 1] is empty.",
        ),
        (
            lambda: stats.interval_parameters(-1, 2.5, lognormal=True),
            "The log-normal interval [-1, 2.5] must be positive.",
        ),
        (
            lambda: stats.interval_parameters(1.0, 2.0, 1, lognormal=True),
            "The level 1 of an interval must be in (0, 1).",
        ),
        (
            lambda: stats.interval_parameters(
                1.0, 2.0, numpy.float64(0.0), lognormal=False
            ),
            "The level 0.0 of an interval must be in (0, 1).",
        ),
    ]:
        with pytest.raises(ValueError) as raised:
            call()
        assert str(raised.value) == message
    # the level of 95 % by default, and the parameters of integers those of their doubles
    assert stats.interval_parameters(1, 2, lognormal=True) == stats.interval_parameters(
        1.0, 2.0, 0.95, lognormal=True
    )
    with pytest.raises(TypeError):
        stats.interval_parameters(1.0, 2.0)  # ty: ignore[missing-argument]


def test_no_bins_and_negative_bins_are_refused_as_numpy_refuses_them() -> None:
    case = UNCERTAINTY["first_order/bins_0"]
    x, y = case["inputs"]["x"], case["inputs"]["y"]
    for bins in (0, -3):
        with pytest.raises(
            ValueError, match=r"^number sections must be larger than 0\.$"
        ):
            stats.first_order_given_data(x, y, bins)


def test_strided_integer_and_fortran_draws_are_read_as_their_values() -> None:
    x = large_residuals(1000)
    case = UNCERTAINTY["statistics_formula/1000"]
    strided = numpy.repeat(x, 2)[::2]
    fortran = numpy.asfortranarray(x.reshape(40, 25))
    assert not strided.flags.c_contiguous
    assert not fortran.flags.c_contiguous
    # a matrix in Fortran order is read in the order numpy ravels it, the one of `x`
    for array in (strided, fortran):
        assert list(stats.mean_se(array)) == case["expected"]["mean_se"]
    assert stats.quantile_ci(fortran, 0.5) == stats.quantile_ci(fortran.ravel(), 0.5)
    xp, yp = paired_draws(1000)
    expected = UNCERTAINTY["first_order_formula/1000"]["expected"]
    assert stats.first_order_given_data(numpy.repeat(xp, 3)[::3], yp) == expected
    # integers, converted to doubles
    integers = numpy.array([3, 1, 2, 5, 4, 8, 7, 6, 9], dtype=numpy.int32)
    doubles = integers.astype(float)
    assert stats.mean_se(integers) == stats.mean_se(doubles)
    assert stats.sd_se(integers[::-1]) == stats.sd_se(doubles[::-1])
    assert stats.quantile(integers, 0.3) == stats.quantile(doubles, 0.3)
    numpy.testing.assert_array_equal(
        stats.quantiles(integers, numpy.array([0, 1])), [1.0, 9.0]
    )
    assert stats.precise(integers, 1.0) == stats.precise(doubles, 1.0)
    assert stats.first_order_given_data(
        integers, integers[::-1]
    ) == stats.first_order_given_data(doubles, doubles[::-1])


# --- draws and correlation --------------------------------------------------------------------

DRAWS = cases("draws", "")


def of_draws(prefix: str) -> list[tuple[str, dict[str, Any]]]:
    """The cases of `draws.json` whose names start with `prefix`, as test parameters."""
    return [(name, case) for name, case in DRAWS.items() if name.startswith(prefix)]


def large_correlation(k: int) -> numpy.ndarray:
    """The matrix of `large_correlation` of `make_oracles.py`, of a formula of integers."""
    i, j = numpy.indices((k, k))
    r = (((i + j) * 7 + i * j) % 19 - 9) / 10.0
    numpy.fill_diagonal(r, 1.0)
    return r


@pytest.mark.parametrize(("name", "case"), of_draws("nearest/"))
def test_the_nearest_correlation_is_the_one_of_higham_of_sbmlsim(
    name: str, case: dict[str, Any]
) -> None:
    inputs = case["inputs"]
    k = inputs["k"]
    m = square(inputs["m"], k) if "m" in inputs else large_correlation(k)
    found = stats.nearest_correlation(m, inputs["max_iterations"], inputs["tol"])
    assert found.shape == (k, k)
    # the zeros of numpy are rounding of the order of the largest value, 1
    numpy.testing.assert_allclose(
        found, square(case["expected"], k), rtol=KERNEL, atol=KERNEL
    )


def test_the_nearest_correlation_of_200_targets_is_the_one_of_sbmlsim() -> None:
    # every 97th value and the sums of the rows against numpy, at 1e-12 of the largest
    # value of the matrix, 1: Jacobi and LAPACK round differently
    case = DRAWS["nearest_large/200"]
    inputs, expected = case["inputs"], case["expected"]
    found = stats.nearest_correlation(
        large_correlation(inputs["k"]), inputs["max_iterations"], inputs["tol"]
    )
    numpy.testing.assert_allclose(
        found.ravel()[:: inputs["stride"]], expected["strided"], rtol=0.0, atol=KERNEL
    )
    numpy.testing.assert_allclose(
        found.sum(axis=1), expected["row_sums"], rtol=0.0, atol=KERNEL
    )


def test_the_nearest_correlation_refuses_the_shapes_numpy_refuses() -> None:
    with pytest.raises(ValueError) as raised:
        stats.nearest_correlation(numpy.ones((2, 3)))
    assert str(raised.value) == (
        "operands could not be broadcast together with shapes (2,3) (3,2) "
    )
    with pytest.raises(ValueError) as raised:
        stats.nearest_correlation([1.0, 2.0])
    assert str(raised.value) == (
        "1-dimensional array given. Array must be at least two-dimensional"
    )
    assert stats.nearest_correlation(numpy.zeros((0, 0))).shape == (0, 0)


@pytest.mark.parametrize(("name", "case"), of_draws("is_psd/"))
def test_is_psd_is_the_one_of_sbmlsim(name: str, case: dict[str, Any]) -> None:
    inputs = case["inputs"]
    m = square(inputs["m"], inputs["k"])
    assert stats.is_psd(m, inputs["rtol"]) is case["expected"]
    assert stats.is_psd(m) is case["expected"]


def test_is_psd_refuses_an_empty_matrix_as_numpy() -> None:
    assert DRAWS["psd_rtol"]["expected"] == stats.PSD_RTOL
    with pytest.raises(ValueError) as raised:
        stats.is_psd(numpy.zeros((0, 0)))
    assert str(raised.value) == (
        "zero-size array to reduction operation minimum which has no identity"
    )
    with pytest.raises(ValueError) as raised:
        stats.is_psd(numpy.ones((2, 3)))
    assert str(raised.value) == "Last 2 dimensions of the array must be square"


@pytest.mark.parametrize(("name", "case"), of_draws("validate/"))
def test_the_validation_is_the_one_of_covariance_with_its_messages(
    name: str, case: dict[str, Any]
) -> None:
    inputs = case["inputs"]
    matrix = numpy.asarray(inputs["matrix"], dtype=float).reshape(inputs["shape"])
    units = None if inputs["units"] is None else ["mM"] * inputs["units"]

    def call() -> None:
        stats.validate_covariance(
            inputs["targets"], matrix, kind=inputs["kind"], units=units
        )

    if case["expected"] is None:
        call()
    else:
        assert_outcome(call, case["expected"])


@pytest.mark.parametrize(("name", "case"), of_draws("rho/"))
def test_the_coefficient_of_the_normal_scores_is_the_one_of_sbmlsim(
    name: str, case: dict[str, Any]
) -> None:
    inputs = case["inputs"]
    a, b = tuple(inputs["a"]), tuple(inputs["b"])
    assert_raised(
        lambda: stats.normal_score_rho(inputs["m"], a, b, inputs["scale"]),
        case["expected"],
    )


def test_a_coefficient_of_two_families_or_another_family_is_refused() -> None:
    with pytest.raises(ValueError, match="one family"):
        stats.normal_score_rho(
            0.1, ("normal", 1.0, 1.0), ("log-normal", 1.0, 1.0), "log"
        )
    with pytest.raises(ValueError) as raised:
        stats.normal_score_rho(0.1, ("gamma", 1.0, 1.0), ("normal", 1.0, 1.0))
    assert str(raised.value) == (
        "The family \"gamma\" of a marginal is none of 'normal' and 'log-normal'."
    )


@pytest.mark.parametrize(("name", "case"), of_draws("copula/"))
def test_the_copula_of_given_points_is_the_one_of_draw(
    name: str, case: dict[str, Any]
) -> None:
    inputs = case["inputs"]
    n, d = inputs["n"], inputs["d"]
    u = numpy.asarray(inputs["u"], dtype=float).reshape(n, d)
    found = stats.copula(u, square(inputs["r"], d))
    assert found.shape == (n, d)
    expected = numpy.asarray(case["expected"], dtype=float).reshape(n, d)
    numpy.testing.assert_allclose(found, expected, rtol=KERNEL, atol=0.0)
    if "identity" in name and "outside" not in name:
        assert numpy.array_equal(found, u, equal_nan=True)


def test_the_copula_checks_its_shapes() -> None:
    with pytest.raises(ValueError, match="not of the shape \\(3,\\)"):
        stats.copula([0.5, 0.5, 0.5], numpy.eye(3))
    with pytest.raises(ValueError, match="does not fit points of 2 variables"):
        stats.copula(numpy.full((4, 2), 0.5), numpy.eye(3))


def test_the_points_of_a_seed_are_the_same_and_of_another_seed_others() -> None:
    for design in ("sobol", "lhs", "random"):
        a = stats.unit_points(3, 64, design=design, seed=42)
        assert a.shape == (64, 3)
        assert numpy.array_equal(a, stats.unit_points(3, 64, design=design, seed=42))
        assert not numpy.array_equal(
            a, stats.unit_points(3, 64, design=design, seed=43)
        )
        assert ((a >= 0.0) & (a < 1.0)).all()
    assert numpy.array_equal(
        stats.unit_points(2, 8, seed=1), stats.unit_points(2, 8, design="sobol", seed=1)
    )


def test_sobol_and_random_points_continue_from_start() -> None:
    for design in ("sobol", "random"):
        long = stats.unit_points(4, 48, design=design, seed=7)
        tail = stats.unit_points(4, 16, design=design, seed=7, start=32)
        assert numpy.array_equal(tail, long[32:])


def test_the_points_refuse_as_the_python() -> None:
    with pytest.raises(ValueError) as raised:
        stats.unit_points(2, 4, design="lhs", seed=1, start=2)
    assert str(raised.value) == (
        "A Latin hypercube has no continuation, its points of more draws are another "
        "hypercube: give the number of draws, or use 'sobol' or 'random' for more draws."
    )
    # a design the stub does not name, which the binding refuses
    with pytest.raises(ValueError) as raised:
        stats.unit_points(2, 4, design="x", seed=1)  # ty: ignore[invalid-argument-type]
    assert str(raised.value) == "The design 'x' is none of 'sobol', 'lhs' and 'random'."
    with pytest.raises(ValueError) as raised:
        stats.unit_points(2, 4, design=None, seed=1)  # ty: ignore[invalid-argument-type]
    assert (
        str(raised.value) == "The design None is none of 'sobol', 'lhs' and 'random'."
    )
    with pytest.raises(ValueError) as raised:
        stats.unit_points(2, 4, design="random", seed=-1)
    assert str(raised.value) == "expected non-negative integer"
    assert stats.unit_points(2, 0, seed=1).shape == (0, 2)
    assert stats.unit_points(0, 4, design="lhs", seed=1).shape == (4, 0)


def test_negative_arguments_of_the_points_are_refused_as_the_python() -> None:
    dimensions = "d must be a non-negative integer value"
    shape = "negative dimensions are not allowed"
    lhs = "A Latin hypercube has no continuation"
    cases: list[tuple[Design, int, int, int, type[Exception], str]] = [
        ("sobol", -1, 4, 0, ValueError, dimensions),
        ("sobol", 2, -4, 0, ValueError, shape),
        (
            "sobol",
            2,
            4,
            -1,
            OverflowError,
            "can't convert negative value to npy_uint32",
        ),
        # the start of a sobol sequence before its points
        ("sobol", 2, -4, -1, OverflowError, "can't convert negative value"),
        ("lhs", -1, 4, 0, ValueError, dimensions),
        ("lhs", 2, -4, 0, ValueError, shape),
        ("lhs", -1, -4, -1, ValueError, lhs),
        ("random", -1, 4, 0, ValueError, shape),
        ("random", 2, -4, 0, ValueError, shape),
        (
            "random",
            2,
            4,
            -1,
            ValueError,
            "The start of random points must not be negative",
        ),
    ]
    for design, d, n, start, error, message in cases:
        with pytest.raises(error) as raised:
            stats.unit_points(d, n, design=design, seed=1, start=start)
        assert type(raised.value) is error
        assert str(raised.value).startswith(message), (design, d, n, start)


def test_the_points_are_uniform_and_a_latin_hypercube_stratified() -> None:
    n = 1000
    for design in ("sobol", "lhs", "random"):
        u = stats.unit_points(3, n, design=design, seed=2026)
        for column in u.T:
            # the statistic of Kolmogorov and Smirnov, below its 1 % critical value
            s = numpy.sort(column)
            i = numpy.arange(n)
            ks = max((s - i / n).max(), ((i + 1) / n - s).max())
            assert ks < 1.63 / math.sqrt(n), (design, ks)
    lhs = stats.unit_points(3, n, design="lhs", seed=3)
    for column in lhs.T:
        assert numpy.array_equal(numpy.sort(numpy.floor(column * n)), numpy.arange(n))


def test_the_copula_correlates_the_normal_scores_of_the_draws() -> None:
    from statistics import NormalDist

    r = numpy.array([[1.0, 0.7], [0.7, 1.0]])
    ppf = numpy.vectorize(NormalDist().inv_cdf)
    for design in ("sobol", "lhs", "random"):
        u = stats.copula(stats.unit_points(2, 4096, design=design, seed=5), r)
        z = ppf(u)
        assert abs(numpy.corrcoef(z.T)[0, 1] - 0.7) < 0.02, design


def test_the_units_are_any_iterable_and_counted_only_for_a_covariance() -> None:
    # `tuple(units)` of the python; a correlation ignores their number
    stats.validate_covariance(
        ["a", "b"], numpy.eye(2), kind="correlation", units=["mM"]
    )
    stats.validate_covariance(["a", "b"], numpy.eye(2), units=(u for u in ["mM", None]))
    with pytest.raises(ValueError, match="needs one unit per target"):
        stats.validate_covariance(["a", "b"], numpy.eye(2), units=iter(["mM"]))
    with pytest.raises(TypeError, match="'int' object is not iterable"):
        stats.validate_covariance(["a", "b"], numpy.eye(2), units=1)  # ty: ignore[invalid-argument-type]


def test_strided_integer_and_fortran_matrices_of_the_correlations() -> None:
    m = numpy.asfortranarray(large_correlation(6))
    expected = stats.nearest_correlation(numpy.ascontiguousarray(m))
    assert numpy.array_equal(stats.nearest_correlation(m), expected)
    wide = numpy.zeros((6, 12))
    wide[:, ::2] = m
    assert numpy.array_equal(stats.nearest_correlation(wide[:, ::2]), expected)
    assert stats.is_psd(numpy.eye(3, dtype=int))
    stats.validate_covariance(["a", "b"], numpy.array([[2, 1], [1, 2]]))
    u = stats.unit_points(3, 8, seed=1)
    r = numpy.array([[1.0, 0.5, 0.0], [0.5, 1.0, 0.0], [0.0, 0.0, 1.0]])
    strided = numpy.zeros((8, 6))
    strided[:, ::2] = u
    assert numpy.array_equal(stats.copula(strided[:, ::2], r), stats.copula(u, r))


# --- tests and variability ---------------------------------------------------------------------

TESTING = cases("testing", "")


def of_testing(prefix: str) -> list[tuple[str, dict[str, Any]]]:
    """The cases of `testing.json` whose names start with `prefix`, as test parameters."""
    return [(name, case) for name, case in TESTING.items() if name.startswith(prefix)]


def satisfied_of(inputs: dict[str, Any]) -> numpy.ndarray:
    """The draws of a case of `sequence/`: its booleans or the formula of `pattern`."""
    if "satisfied" in inputs:
        return numpy.asarray(inputs["satisfied"], dtype=bool)
    i = numpy.arange(inputs["n"])
    return ((i * inputs["step"] + inputs["shift"]) % inputs["period"]) < inputs["ones"]


def strided(values: numpy.ndarray, stride: int) -> numpy.ndarray:
    """Every `stride`-th value from the first and the last value, as the fixture."""
    indices = list(range(0, values.size, stride))
    if values.size and indices[-1] != values.size - 1:
        indices.append(values.size - 1)
    return values[indices]


def test_the_constants_of_the_tests_are_the_ones_of_sbmlsim() -> None:
    expected = TESTING["constants"]["expected"]
    assert expected["margin"] == stats.MARGIN
    assert expected["max_needed"] == stats.MAX_NEEDED
    assert expected["bisections"] == stats.BISECTIONS
    assert stats.MIXTURE == (0.5, 0.5)


@pytest.mark.parametrize(("name", "case"), of_testing("sequence/"))
def test_the_statements_of_draws_are_the_ones_of_sbmlsim(
    name: str, case: dict[str, Any]
) -> None:
    inputs, expected = case["inputs"], case["expected"]
    x = satisfied_of(inputs)
    p0, margin, alpha = inputs["p0"], inputs["margin"], inputs["alpha"]
    found = {}
    for side in ("greater", "less", "equivalent"):
        p = stats.p_values(x, p0, side, margin)
        assert p.shape == x.shape
        wanted = numpy.asarray(expected[f"p_{side}"])
        numpy.testing.assert_allclose(
            strided(p, inputs["stride"]), wanted, rtol=KERNEL, atol=0.0
        )
        found[side] = p
    decision, n_decided = stats.decision(
        found["greater"], found["less"], found["equivalent"], alpha
    )
    assert (decision, n_decided) == (expected["decision"], expected["n_decided"])
    if decision != "undecided":
        side = {"accepted": "greater", "rejected": "less"}.get(decision, "equivalent")
        assert stats.decide(found[side], alpha) == n_decided
    low, high = stats.confidence_sequence(x, 1.0 - alpha)
    numpy.testing.assert_allclose(
        [low, high], [expected["low"], expected["high"]], rtol=KERNEL, atol=0.0
    )
    if x.size:
        needed = stats.needed(x.mean(), x.size, p0, margin, alpha)
        assert needed == expected["needed"]


@pytest.mark.parametrize(("name", "case"), of_testing("sequence_alpha/"))
def test_the_confidence_sequence_refuses_a_level_as_sbmlsim(
    name: str, case: dict[str, Any]
) -> None:
    inputs, expected = case["inputs"], case["expected"]
    level = 1.0 - inputs["alpha"]
    if isinstance(expected, dict):
        with pytest.raises(getattr(builtins, expected["exception"])) as raised:
            stats.confidence_sequence(inputs["satisfied"], level)
        assert str(raised.value) == expected["error"]
    else:
        found = stats.confidence_sequence(inputs["satisfied"], level)
        numpy.testing.assert_allclose(found, expected, rtol=KERNEL, atol=0.0)


def test_the_draws_needed_are_the_ones_of_sbmlsim() -> None:
    case = TESTING["needed"]
    inputs = case["inputs"]
    found = [
        stats.needed(*arguments)
        for arguments in zip(
            inputs["estimate"],
            inputs["n"],
            inputs["p0"],
            inputs["margin"],
            inputs["alpha"],
            strict=True,
        )
    ]
    assert found == case["expected"]
    assert stats.needed(0.5, 0, 0.5, 0.05, 0.05) is None
    # the python loops without an end on a negative number of draws
    with pytest.raises(ValueError) as raised:
        stats.needed(0.5, -1, 0.5, 0.05, 0.05)
    assert str(raised.value) == "The number of draws must not be negative, not -1."


@pytest.mark.parametrize(("name", "case"), of_testing("needed_error/"))
def test_the_draws_needed_refuse_an_estimate_as_sbmlsim(
    name: str, case: dict[str, Any]
) -> None:
    inputs, expected = case["inputs"], case["expected"]
    arguments = [inputs[key] for key in ("estimate", "n", "p0", "margin", "alpha")]
    if isinstance(expected, dict):
        with pytest.raises(getattr(builtins, expected["exception"])) as raised:
            stats.needed(*arguments)
        assert str(raised.value) == expected["error"]
    else:
        assert stats.needed(*arguments) == expected


def test_the_family_level_is_the_one_of_sbmlsim() -> None:
    case = TESTING["family_level"]
    found = [
        stats.family_level(level, m)
        for level, m in zip(case["inputs"]["level"], case["inputs"]["m"], strict=True)
    ]
    assert found == case["expected"]


def test_the_draws_are_read_as_numpy_reads_them_as_booleans() -> None:
    x = numpy.array([1, 0, 2, 0, 1, 1, 0, 1])
    expected = stats.p_values(x.astype(bool), 0.3, "greater")
    # an integer, a float (a not a number is true), a list and a strided array
    assert numpy.array_equal(stats.p_values(x, 0.3, "greater"), expected)
    floats = numpy.where(x > 0, numpy.nan, 0.0)
    assert numpy.array_equal(stats.p_values(floats, 0.3, "greater"), expected)
    assert numpy.array_equal(
        stats.p_values(x.astype(bool).tolist(), 0.3, "greater"), expected
    )
    wide = numpy.zeros(16, dtype=bool)
    wide[::2] = x > 0
    assert numpy.array_equal(stats.p_values(wide[::2], 0.3, "greater"), expected)
    assert stats.confidence_sequence(wide[::2]) == stats.confidence_sequence(x)
    assert stats.confidence_sequence([]) == (0.0, 1.0)
    assert stats.p_values([], 0.3, "less").shape == (0,)
    # the margin by default
    assert numpy.array_equal(
        stats.p_values(x, 0.3, "equivalent"),
        stats.p_values(x, 0.3, "equivalent", stats.MARGIN),
    )
    with pytest.raises(ValueError, match="'both' is none of 'greater', 'less'"):
        stats.p_values(x, 0.3, "both")  # ty: ignore[invalid-argument-type]
    assert stats.decide([1.0, 0.5, 0.01, 0.0], 0.05) == 3
    assert stats.decide([], 0.05) is None
    assert stats.decision([1.0], [1.0], [1.0], 0.05) == ("undecided", None)


@pytest.mark.parametrize(("name", "case"), of_testing("bands/"))
def test_the_bands_are_the_quantiles_of_numpy_of_the_finite_values(
    name: str, case: dict[str, Any]
) -> None:
    inputs, expected = case["inputs"], case["expected"]
    rows, cols = inputs["rows"], inputs["cols"]
    distribution = numpy.asarray(inputs["distribution"], dtype=float).reshape(
        rows, cols
    )
    if isinstance(expected, dict):
        with pytest.raises(ValueError) as raised:
            stats.bands(distribution, inputs["qs"])
        assert str(raised.value) == expected["error"]
        return
    found = stats.bands(distribution, inputs["qs"])
    assert found.shape == (cols, len(inputs["qs"]))
    numpy.testing.assert_allclose(
        found.ravel(), numpy.asarray(expected, dtype=float), rtol=KERNEL, atol=0.0
    )


def test_the_bands_read_strided_and_integer_distributions() -> None:
    distribution = numpy.arange(24).reshape(8, 3)
    expected = stats.bands(distribution.astype(float), [0.1, 0.9])
    assert numpy.array_equal(
        stats.bands(distribution, numpy.array([0.1, 0.9])), expected
    )
    wide = numpy.zeros((8, 6))
    wide[:, ::2] = distribution
    assert numpy.array_equal(stats.bands(wide[:, ::2], [0.1, 0.9]), expected)
    with pytest.raises(ValueError, match="matrix of the groups by the data points"):
        stats.bands([1.0, 2.0], [0.5])


def predictions(subjects: int, points: int) -> numpy.ndarray:
    """Positive predictions of subjects at data points, of a formula of integers."""
    s, j = numpy.indices((subjects, points))
    return 1.0 + ((s * 7 + j * 3) % 11) / 4.0 + j


def test_a_group_of_one_subject_is_the_subject_itself() -> None:
    values = predictions(13, 4)
    values[2, 1] = numpy.nan
    values[5, 3] = numpy.inf
    found = stats.group_means(values, 1, 500, seed=7)
    finite = numpy.all(numpy.isfinite(values), axis=1)
    assert numpy.array_equal(found, values[finite])
    assert numpy.array_equal(
        stats.group_means(values, 1, 500, geometric=True, seed=7), found
    )


def test_the_group_means_have_the_mean_of_the_subjects_and_the_variance_of_a_mean() -> (
    None
):
    subjects, size, groups = 200, 5, 20000
    values = predictions(subjects, 3)
    means = stats.group_means(values, size, groups, seed=11)
    assert means.shape == (groups, 3)
    mean, variance = values.mean(axis=0), values.var(axis=0)
    se = numpy.sqrt(variance / (size * groups))
    assert numpy.all(numpy.abs(means.mean(axis=0) - mean) < 4.0 * se)
    ratio = means.var(axis=0) / (variance / size)
    assert numpy.all(numpy.abs(ratio - 1.0) < 0.05)


def test_the_geometric_mean_of_a_group_is_at_most_its_mean() -> None:
    values = predictions(200, 3)
    geometric = stats.group_means(values, 4, 20000, geometric=True, seed=5)
    arithmetic = stats.group_means(values, 4, 20000, geometric=False, seed=5)
    assert numpy.all(geometric <= arithmetic * (1.0 + 1e-15))
    logs = numpy.log(values)
    se = numpy.sqrt(logs.var(axis=0) / (4 * 20000))
    found = numpy.log(geometric).mean(axis=0)
    assert numpy.all(numpy.abs(found - logs.mean(axis=0)) < 4.0 * se)


def test_a_seed_gives_the_same_groups_and_the_groups_refuse_as_numpy() -> None:
    values = predictions(30, 2)
    a = stats.group_means(values, 3, 100, seed=42)
    assert numpy.array_equal(a, stats.group_means(values, 3, 100, seed=42))
    assert not numpy.array_equal(a, stats.group_means(values, 3, 100, seed=43))
    strided = numpy.zeros((30, 4))
    strided[:, ::2] = values
    assert numpy.array_equal(stats.group_means(strided[:, ::2], 3, 100, seed=42), a)
    integers = numpy.arange(12).reshape(6, 2)
    assert numpy.array_equal(
        stats.group_means(integers, 2, 10, seed=1),
        stats.group_means(integers.astype(float), 2, 10, seed=1),
    )
    for size, groups in [(-1, 10), (2, -1)]:
        with pytest.raises(ValueError) as raised:
            stats.group_means(values, size, groups, seed=1)
        assert str(raised.value) == "negative dimensions are not allowed"
    with pytest.raises(ValueError) as raised:
        stats.group_means(values, 2, 10, seed=-1)
    assert str(raised.value) == "expected non-negative integer"
    with pytest.raises(ValueError, match="matrix of the subjects by the data points"):
        stats.group_means([1.0, 2.0], 2, 10, seed=1)
    assert stats.group_means(numpy.full((3, 2), numpy.nan), 2, 10, seed=1).shape == (
        0,
        2,
    )


#: the ValueError of numpy of an array whose bytes exceed the largest intp
ARRAY_TOO_BIG = (
    "array is too big; `arr.size * arr.dtype.itemsize` is larger than the maximum "
    "possible size."
)


def test_groups_too_many_to_hold_raise_a_value_error() -> None:
    """The ValueError of numpy of the members or the statistics, raised before any memory is
    asked for."""
    values = predictions(4, 2)
    for size, groups in [(2, 2**62), (0, 2**62), (2, 2**63 - 1)]:
        with pytest.raises(ValueError) as raised:
            stats.group_means(values, size, groups, seed=1)
        assert str(raised.value) == ARRAY_TOO_BIG


# The sizes below are arrays no memory holds or loops no time ends: a binding which
# allocated or looped would fail these tests by running out of memory or time, so they run
# under a cap of the memory (the continuous integration's runner, `systemd-run -p MemoryMax`
# locally). numpy makes the empty inputs without memory.


def test_points_too_many_to_hold_raise_a_value_error() -> None:
    cases: list[tuple[Design, int, int]] = [
        ("random", 2**33, 2**31),
        ("lhs", 2**33, 2**33),
        ("sobol", 2**33, 2**31),
        ("random", 0, 2**62),
    ]
    for design, d, n in cases:
        with pytest.raises(ValueError) as raised:
            stats.unit_points(d, n, design=design, seed=0)
        assert str(raised.value) == ARRAY_TOO_BIG, design


def test_random_points_jump_to_a_far_start() -> None:
    """The stream jumps ahead in a time logarithmic in the start, so the points of a far
    start come at once, and are the tail of the points of an earlier start."""
    begin = time.perf_counter()
    far = stats.unit_points(1, 1, design="random", seed=0, start=2**62)
    earlier = stats.unit_points(1, 3, design="random", seed=0, start=2**62 - 2)
    assert time.perf_counter() - begin < 5.0
    assert far.shape == (1, 1)
    numpy.testing.assert_array_equal(far, earlier[2:])
    long = stats.unit_points(3, 10**6 + 5, design="random", seed=4)
    tail = stats.unit_points(3, 5, design="random", seed=4, start=10**6)
    numpy.testing.assert_array_equal(tail, long[10**6 :])


def test_a_gram_matrix_too_big_to_hold_raises_a_value_error() -> None:
    """A jacobian of no residuals of 2^32 + 1 parameters asks for a Gram matrix whose number
    of values wraps a 64 bit integer."""
    with pytest.raises(ValueError) as raised:
        stats.gram(numpy.empty((0, 2**32 + 1)))
    assert str(raised.value) == ARRAY_TOO_BIG


def test_groups_at_no_data_point_draw_nothing() -> None:
    """No statistic and no draw: the rows at once for any number of groups and subjects."""
    begin = time.perf_counter()
    assert stats.group_means(numpy.empty((3, 0)), 2, 2**40, seed=0).shape == (2**40, 0)
    assert stats.group_means(numpy.empty((2**40, 0)), 2, 3, seed=0).shape == (3, 0)
    assert stats.group_means(numpy.empty((2**40, 0)), 1, 3, seed=0).shape == (2**40, 0)
    assert time.perf_counter() - begin < 5.0
    # members past what an array holds, as numpy refuses `rng.integers(size=(groups, size))`
    with pytest.raises(ValueError) as raised:
        stats.group_means(numpy.empty((3, 0)), 2, 2**62, seed=0)
    assert str(raised.value) == ARRAY_TOO_BIG


def test_bands_too_many_to_hold_raise_a_value_error() -> None:
    with pytest.raises(ValueError) as raised:
        stats.bands(numpy.empty((0, 2**59)), [0.05, 0.95])
    assert str(raised.value) == ARRAY_TOO_BIG


# --- the decomposition of a Fisher information ------------------------------------------


def same_bits(found: Any, expected: Any) -> bool:
    """Whether two arrays have the same doubles to the bit, `nan` as `nan`."""
    found, expected = numpy.asarray(found, float), numpy.asarray(expected, float)
    both_nan = numpy.isnan(found) & numpy.isnan(expected)
    return found.shape == expected.shape and bool(
        numpy.all((found.view(numpy.int64) == expected.view(numpy.int64)) | both_nan)
    )


@pytest.mark.parametrize(("name", "case"), of_fisher("fisher/"))
def test_the_statistics_are_the_ones_of_their_decomposition_to_the_bit(
    name: str, case: dict[str, Any]
) -> None:
    inputs = case["inputs"]
    information = square(inputs["information"], inputs["k"])
    arguments = (inputs["cost"], inputs["n"])
    spectrum, inverse = stats.fisher_decomposition(information)
    for likelihood in (False, True):
        direct = stats.fisher_statistics(information, *arguments, likelihood=likelihood)
        derived = stats.fisher_statistics_of_decomposition(
            spectrum, inverse, *arguments, likelihood=likelihood
        )
        assert set(direct) == set(derived)
        for key, value in direct.items():
            assert same_bits(derived[key], value), (name, key)
        assert same_bits(direct["spectrum"], spectrum)


@pytest.mark.parametrize(
    ("name", "case"), list(cases("fisher_decomposition", "decomposition/").items())
)
def test_the_statistics_of_a_decomposition_of_lapack_are_the_ones_of_sbmlsim(
    name: str, case: dict[str, Any]
) -> None:
    inputs, expected = case["inputs"], case["expected"]
    k = inputs["k"]
    found = stats.fisher_statistics_of_decomposition(
        inputs["spectrum"],
        square(inputs["inverse"], k),
        inputs["cost"],
        inputs["n"],
        likelihood=inputs["likelihood"],
    )
    assert found["rank"] == expected["rank"]
    for key in ("sigma2", "condition", "standard_errors"):
        assert same_bits(found[key], expected[key]), key
    for key in ("covariance", "correlation"):
        assert same_bits(found[key], square(expected[key], k)), key


def test_a_decomposition_which_does_not_fit_is_refused() -> None:
    inverse = numpy.eye(2)
    with pytest.raises(
        ValueError, match="must be a vector of 2 values, not of the shape"
    ):
        stats.fisher_statistics_of_decomposition([2.0, 1.0, 0.5], inverse, 1.0, 10)
    with pytest.raises(ValueError, match="must be a square matrix"):
        stats.fisher_statistics_of_decomposition([2.0], [1.0, 2.0], 1.0, 10)
    with pytest.raises(ValueError, match="largest first"):
        stats.fisher_statistics_of_decomposition([1.0, 2.0], inverse, 1.0, 10)
    with pytest.raises(ValueError, match="no parameters"):
        stats.fisher_decomposition(numpy.zeros((0, 0)))


# --- the distributions and the special functions ----------------------------------------

SPECIAL = cases("special", "")

#: the tolerances of the functions against their cases, relative
TOLERANCES = {
    "normal_cdf": 1e-13,
    "normal_ppf": 1e-13,
    "t_cdf": DISTRIBUTION,
    "t_ppf": DISTRIBUTION,
    "erf": 1e-15,
    "gamma": 1e-13,
    "chi2_ppf": 1e-13,
}


def relative_error(found: Any, expected: Any) -> numpy.ndarray:
    """The relative errors of `found` against `expected` as the oracle tests of the crate
    count them: zero of the same values (`nan` of `nan`) and of two values below the smallest
    normal double, which scipy flushes to zero."""
    found, expected = numpy.asarray(found, float), numpy.asarray(expected, float)
    tiny = numpy.finfo(float).tiny
    with numpy.errstate(invalid="ignore"):
        error = numpy.abs(found - expected) / numpy.maximum(numpy.abs(expected), tiny)
        same = (found == expected) | (numpy.isnan(found) & numpy.isnan(expected))
        same |= (numpy.abs(found) < tiny) & (numpy.abs(expected) < tiny)
    return numpy.where(same, 0.0, numpy.where(numpy.isnan(error), numpy.inf, error))


@pytest.mark.parametrize(
    ("case", "function", "inputs"),
    [
        ("normal_cdf", "norm_cdf", ["x"]),
        ("normal_ppf", "norm_ppf", ["p"]),
        ("t_cdf", "t_cdf", ["x", "df"]),
        ("t_cdf_large_df", "t_cdf", ["x", "df"]),
        ("t_ppf", "t_ppf", ["p", "df"]),
        ("erf", "erf", ["x"]),
        ("gamma", "gamma", ["x"]),
        ("chi2_ppf", "chi2_ppf", ["p", "df"]),
    ],
)
def test_a_distribution_is_the_one_of_scipy_or_mpmath(
    case: str, function: str, inputs: list[str]
) -> None:
    values = [numpy.asarray(SPECIAL[case]["inputs"][name]) for name in inputs]
    expected = numpy.asarray(SPECIAL[case]["expected"])
    found = getattr(stats, function)(*values)
    tolerance = TOLERANCES[case.removesuffix("_large_df")]
    assert relative_error(found, expected).max() <= tolerance


def test_the_chi_squared_distribution_function_is_the_one_of_mpmath() -> None:
    # the tolerance times the condition, max(1, min(|x - a|, (x - a)^2 / a)) of a = df / 2
    # at x / 2, the rounding of the exponent of the incomplete gamma function
    case = SPECIAL["chi2_cdf"]
    x, df = numpy.asarray(case["inputs"]["x"]), numpy.asarray(case["inputs"]["df"])
    d, a = numpy.abs(x - df) / 2.0, df / 2.0
    condition = numpy.maximum(numpy.minimum(d, d * d / a), 1.0)
    error = relative_error(stats.chi2_cdf(x, df), case["expected"])
    assert (error / condition).max() <= 1e-13


@pytest.mark.parametrize(
    ("case", "function", "inputs"),
    [
        ("erf_edges", "erf", ["x"]),
        ("gamma_edges", "gamma", ["x"]),
        ("chi2_cdf_edges", "chi2_cdf", ["x", "df"]),
        ("chi2_ppf_edges", "chi2_ppf", ["p", "df"]),
        ("t_cdf_edges", "t_cdf", ["x", "df"]),
        ("t_ppf_edges", "t_ppf", ["p", "df"]),
    ],
)
def test_the_edges_of_a_distribution_are_the_ones_of_scipy(
    case: str, function: str, inputs: list[str]
) -> None:
    values = [SPECIAL[case]["inputs"][name] for name in inputs]
    expected = numpy.asarray(SPECIAL[case]["expected"], float)
    found = numpy.asarray(getattr(stats, function)(*values), float)
    # the normal distribution of infinitely many degrees of freedom differs in its last digit
    tolerance = 1e-13 if case.startswith("t_") else 0.0
    assert relative_error(found, expected).max() <= tolerance
    exact = (relative_error(found, expected) == 0.0) & (found == 0.0)
    # the sign of a zero is scipy's
    assert numpy.array_equal(
        numpy.signbit(found[exact & (expected == 0.0)]),
        numpy.signbit(expected[exact & (expected == 0.0)]),
    )


@pytest.mark.parametrize(
    ("name", "case"),
    [(name, case) for name, case in SPECIAL.items() if name.startswith("broadcast/")],
)
def test_a_distribution_broadcasts_its_arguments_as_scipy(
    name: str, case: dict[str, Any]
) -> None:
    function = getattr(stats, case["inputs"]["function"])
    args = case["inputs"]["args"]
    expected = case["expected"]
    if isinstance(expected, dict) and "error" in expected:
        with pytest.raises(ValueError) as raised:
            function(*args)
        assert str(raised.value) == expected["error"]
        return
    found = function(*args)
    expected = numpy.asarray(expected, float)
    if expected.ndim == 0:
        assert type(found) is float
    else:
        assert isinstance(found, numpy.ndarray)
        assert found.dtype == numpy.float64
    assert numpy.shape(found) == expected.shape
    numpy.testing.assert_allclose(found, expected, rtol=1e-13, atol=0.0)


def test_a_number_gives_a_float_and_an_array_an_array_of_its_shape() -> None:
    assert type(stats.norm_ppf(0.975)) is float
    assert type(stats.chi2_ppf(numpy.float64(0.95), 1)) is float
    assert type(stats.gamma(numpy.array(4.0))) is float
    assert stats.gamma(4) == 6.0
    assert stats.erf(-0.0) == 0.0 and math.copysign(1.0, stats.erf(-0.0)) == -1.0
    found = stats.t_cdf(numpy.arange(6, dtype=numpy.int64).reshape(2, 3), [1, 2, 3])
    assert isinstance(found, numpy.ndarray)
    assert found.shape == (2, 3)
    assert found[1, 2] == stats.t_cdf(5.0, 3.0)
    strided = numpy.linspace(0.05, 0.95, 20)[::3]
    numpy.testing.assert_array_equal(
        stats.norm_ppf(strided), stats.norm_ppf(strided.copy())
    )
    assert stats.chi2_ppf(0.95, 1) == stats.chi2_ppf(0.95, 1.0)
    # the quantile of the profile likelihood of one parameter, `chi2.ppf(0.95, 1)`
    assert abs(stats.chi2_ppf(0.95, 1) - 3.841458820694124) <= 1e-15 * 3.85
