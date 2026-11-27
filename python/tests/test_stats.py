"""Tests of the statistics of the analyses, `sbmlsim_core.stats`, against the values of
sbmlsim: the fixtures of the crate sbmlsim-stats (`crates/stats/oracles/<module>.json`),
which `crates/stats/oracles/make_oracles.py` computed once with the functions of sbmlsim
the bindings replace. A call of the python which raised a ValueError is its message there,
and a binding raises a ValueError with the same one."""

import json
from pathlib import Path
from typing import Any

import numpy
import pytest
from sbmlsim_core import stats

ORACLES = Path(__file__).resolve().parents[2] / "crates" / "stats" / "oracles"

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
    fixture = json.loads((ORACLES / f"{module}.json").read_text())
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


def test_the_fold_factor_is_twofold_by_default() -> None:
    y, f = [1.0, 1.0, 1.0], [2.0, 2.5, 0.5]
    assert stats.fold_counts(y, f) == (2, 3)
    numpy.testing.assert_array_equal(stats.fold_accepted(y, f), [1.0, 0.0, 1.0])


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
    assert stats.fold_counts(y[::1], f, 2.0) == tuple(case["expected"]["counts"])
    numpy.testing.assert_array_equal(
        stats.fold_accepted(y, f, 2), case["expected"]["accepted"]
    )
    # a strided array of integers
    y_strided = numpy.repeat(y, 3)[::3]
    assert not y_strided.flags.c_contiguous
    assert stats.fold_counts(y_strided, f) == tuple(case["expected"]["counts"])
    case = METRICS["residuals/integers"]
    x = numpy.asarray(case["inputs"]["x"], dtype=numpy.int32)
    assert stats.sse(x) == case["expected"]["sse"]
    assert stats.mse(x[::-1]) == case["expected"]["mse"]
