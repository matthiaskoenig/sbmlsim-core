"""Tests of the global sensitivity analysis, `sbmlsim_core.sensitivity`: the properties of the
designs and the estimators on functions whose indices are known. The comparison with SALib,
on the deterministic parts of the designs and on the same values and resamples for the
estimators, is `test_sensitivity_salib.py`."""

import numpy
import pytest
from sbmlsim_core import sensitivity

PI = numpy.pi

#: the analytic indices of the Ishigami function with a = 7, b = 0.1
ISHIGAMI_S1 = [0.3139, 0.4424, 0.0]
ISHIGAMI_ST = [0.5576, 0.4424, 0.2437]


def ishigami(u: numpy.ndarray, a: float = 7.0, b: float = 0.1) -> numpy.ndarray:
    """The Ishigami function of points of the unit cube."""
    x = u * 2.0 * PI - PI
    return (
        numpy.sin(x[:, 0])
        + a * numpy.sin(x[:, 1]) ** 2
        + b * x[:, 2] ** 4 * numpy.sin(x[:, 0])
    )


def elements(u: numpy.ndarray) -> numpy.ndarray:
    """A column per element: the Ishigami function with other coefficients and a constant."""
    return numpy.stack(
        [ishigami(u), ishigami(u, 1.0, 0.5), ishigami(u, 0.0, 0.0), numpy.ones(len(u))],
        axis=1,
    )


# --- the designs ---


def test_a_seed_gives_its_points() -> None:
    for draw in (
        lambda seed: sensitivity.saltelli(3, 16, seed=seed),
        lambda seed: sensitivity.fast_sample(3, 65, seed=seed),
        lambda seed: sensitivity.morris_sample(3, 10, seed=seed),
    ):
        numpy.testing.assert_array_equal(draw(5), draw(5))
        assert not numpy.array_equal(draw(5), draw(6))


def test_the_sobol_sequence_is_balanced() -> None:
    for seed in (None, 0, 2**64 - 1):
        points = sensitivity.sobol_sequence(10, 256, seed=seed)
        assert points.shape == (256, 10)
        cells = numpy.floor(points * 256).astype(int)
        for column in cells.T:
            assert sorted(column) == list(range(256))
        # the first two dimensions are a (0, 8, 2)-net: one point in every square of 1/16
        squares = numpy.floor(points[:, :2] * 16).astype(int)
        assert len({(a, b) for a, b in squares}) == 256


def test_the_matrix_of_saltelli_crosses_the_sequence() -> None:
    d, n = 4, 8
    base = sensitivity.sobol_sequence(2 * d, n, seed=3)
    for second_order in (False, True):
        points = sensitivity.saltelli(d, n, seed=3, second_order=second_order)
        step = 2 * d + 2 if second_order else d + 2
        assert points.shape == (n * step, d)
        a, b = base[:, :d], base[:, d:]
        numpy.testing.assert_array_equal(points[0::step], a)
        numpy.testing.assert_array_equal(points[step - 1 :: step], b)
        for j in range(d):
            ab = a.copy()
            ab[:, j] = b[:, j]
            numpy.testing.assert_array_equal(points[1 + j :: step], ab)
            if second_order:
                ba = b.copy()
                ba[:, j] = a[:, j]
                numpy.testing.assert_array_equal(points[1 + d + j :: step], ba)


def test_a_skip_continues_the_sequence() -> None:
    whole = sensitivity.sobol_sequence(3, 32, seed=1)
    numpy.testing.assert_array_equal(
        sensitivity.sobol_sequence(3, 16, seed=1, skip=16), whole[16:]
    )


def test_a_block_of_fast_runs_its_parameter_at_the_highest_frequency() -> None:
    d, n, m = 4, 1025, 4
    points = sensitivity.fast_sample(d, n, m=m, seed=1)
    assert points.shape == (n * d, d)
    assert ((points >= 0.0) & (points <= 1.0)).all()
    omega = (n - 1) // (2 * m)
    for i in range(d):
        block = points[i * n : (i + 1) * n]
        spectrum = numpy.abs(numpy.fft.rfft(block - block.mean(axis=0), axis=0))
        peaks = spectrum[1:].argmax(axis=0) + 1
        assert peaks[i] == omega
        assert all(peaks[j] < omega // (2 * m) + 1 for j in range(d) if j != i)


def test_the_phases_of_fast_are_given_or_drawn() -> None:
    points = sensitivity.fast_sample(2, 65, phases=[0.0, 0.0])
    # without a phase the first point of a block is the middle of every curve
    numpy.testing.assert_allclose(points[[0, 65]], 0.5, atol=1e-15)
    with pytest.raises(ValueError, match="one of them"):
        sensitivity.fast_sample(2, 65)
    with pytest.raises(ValueError, match="one of them"):
        sensitivity.fast_sample(2, 65, seed=1, phases=[0.0, 0.0])
    with pytest.raises(ValueError, match="4 m"):
        sensitivity.fast_sample(2, 64, seed=1)


@pytest.mark.parametrize("levels", [2, 4, 5, 6])
def test_a_trajectory_of_morris_moves_every_parameter_once(levels: int) -> None:
    d, trajectories = 5, 40
    delta = levels / (2 * (levels - 1))
    points = sensitivity.morris_sample(d, trajectories, levels=levels, seed=2)
    assert points.shape == (trajectories * (d + 1), d)
    steps = numpy.diff(points.reshape(trajectories, d + 1, d), axis=1)
    moved = numpy.abs(steps) > 0
    assert (moved.sum(axis=2) == 1).all()
    assert (moved.sum(axis=1) == 1).all()
    numpy.testing.assert_allclose(numpy.abs(steps[moved]), delta, rtol=1e-15)
    starts = points.reshape(trajectories, d + 1, d).min(axis=1)
    grid = numpy.linspace(0, 1 - delta, levels // 2)
    assert numpy.isin(starts, grid).all()
    assert (points <= 1.0).all()


def test_the_arguments_of_a_design_are_checked() -> None:
    with pytest.raises(ValueError, match="2048"):
        sensitivity.sobol_sequence(2049, 4, seed=1)
    with pytest.raises(ValueError, match="1024"):
        sensitivity.saltelli(1025, 4, seed=1)
    with pytest.raises(ValueError, match="positive"):
        sensitivity.saltelli(0, 4, seed=1)
    with pytest.raises(ValueError, match="seed"):
        sensitivity.morris_sample(2, 4, seed=-1)
    with pytest.raises(ValueError, match="levels"):
        sensitivity.morris_sample(2, 4, levels=1, seed=1)


# --- the estimators ---


def test_the_sobol_indices_of_the_ishigami_function() -> None:
    u = sensitivity.saltelli(3, 4096, seed=1, second_order=True)
    found = sensitivity.sobol_indices(elements(u), 3, second_order=True, seed=1)
    assert found["S1"].shape == (3, 4) and found["S2"].shape == (3, 3, 4)
    numpy.testing.assert_allclose(found["S1"][:, 0], ISHIGAMI_S1, atol=0.03)
    numpy.testing.assert_allclose(found["ST"][:, 0], ISHIGAMI_ST, atol=0.03)
    assert found["S2"][0, 2, 0] == pytest.approx(0.2437, abs=0.05)
    assert numpy.isnan(found["S2"][2, 0, 0])
    assert ((found["S1_conf"][:, 0] > 0.0) & (found["S1_conf"][:, 0] < 0.1)).all()
    # the constant element has zero indices and intervals
    for key in ("S1", "S1_conf", "ST", "ST_conf"):
        assert (found[key][:, 3] == 0.0).all()


def test_the_fast_indices_of_the_ishigami_function() -> None:
    u = sensitivity.fast_sample(3, 1025, seed=3)
    found = sensitivity.fast_indices(elements(u), 3, seed=3)
    numpy.testing.assert_allclose(found["S1"][:, 0], ISHIGAMI_S1, atol=0.06)
    assert found["ST"][0, 0] > found["ST"][2, 0] > 0.0
    assert (found["S1"][:, 3] == 0.0).all() and (found["ST"][:, 3] == 0.0).all()


def test_the_elementary_effects_of_a_linear_function_are_its_slopes() -> None:
    u = sensitivity.morris_sample(3, 30, seed=4)
    slopes = numpy.array([2.0, -1.0, 0.0])
    found = sensitivity.morris_effects(u, (u @ slopes)[:, None], seed=4)
    numpy.testing.assert_allclose(found["mu"][:, 0], slopes, atol=1e-12)
    numpy.testing.assert_allclose(found["mu_star"][:, 0], numpy.abs(slopes), atol=1e-12)
    numpy.testing.assert_allclose(found["sigma"][:, 0], 0.0, atol=1e-12)


def test_an_element_is_estimated_as_if_it_were_alone() -> None:
    u = sensitivity.saltelli(3, 64, seed=2)
    y = elements(u)
    both = sensitivity.sobol_indices(y, 3, seed=7, num_resamples=50)
    alone = sensitivity.sobol_indices(y[:, 1:2], 3, seed=7, num_resamples=50)
    for key in ("S1", "S1_conf", "ST", "ST_conf"):
        numpy.testing.assert_array_equal(both[key][:, 1:2], alone[key])
    # a view which is not contiguous is read in its order
    numpy.testing.assert_array_equal(
        sensitivity.sobol_indices(numpy.asfortranarray(y), 3, seed=7)["S1"],
        sensitivity.sobol_indices(y, 3, seed=7)["S1"],
    )


def test_the_resamples_are_drawn_or_given() -> None:
    u = sensitivity.saltelli(3, 16, seed=2)
    y = elements(u)
    given = numpy.random.default_rng(0).integers(16, size=(30, 16))
    found = sensitivity.sobol_indices(y, 3, resamples=given)
    assert numpy.isfinite(found["S1_conf"][:, 0]).all()
    with pytest.raises(ValueError, match="not both"):
        sensitivity.sobol_indices(y, 3, seed=1, resamples=given)
    with pytest.raises(ValueError, match="needs a seed"):
        sensitivity.sobol_indices(y, 2)
    with pytest.raises(ValueError, match="with a seed only"):
        sensitivity.sobol_indices(y, 3, resamples=given, num_resamples=30)
    with pytest.raises(ValueError, match="shape"):
        sensitivity.sobol_indices(y, 3, resamples=given[:, :8])
    with pytest.raises(ValueError, match="below"):
        sensitivity.sobol_indices(y, 3, resamples=given + 16)
    with pytest.raises(ValueError, match="confidence level"):
        sensitivity.sobol_indices(y, 3, seed=1, conf_level=1.0)


def test_the_points_of_a_design_are_checked() -> None:
    with pytest.raises(ValueError, match="points per sample"):
        sensitivity.sobol_indices(numpy.zeros((9, 1)), 3, seed=1)
    with pytest.raises(ValueError, match="points per sample"):
        sensitivity.sobol_indices(numpy.zeros((10, 1)), 3, second_order=True, seed=1)
    with pytest.raises(ValueError, match="block of points per parameter"):
        sensitivity.fast_indices(numpy.zeros((10, 1)), 3, seed=1)
    with pytest.raises(ValueError, match="trajectories of 4 points"):
        sensitivity.morris_effects(numpy.zeros((10, 3)), numpy.zeros((10, 1)), seed=1)
    with pytest.raises(ValueError, match="moves 0 parameters"):
        sensitivity.morris_effects(numpy.zeros((8, 3)), numpy.zeros((8, 1)), seed=1)


def test_no_element_gives_empty_indices() -> None:
    u = sensitivity.saltelli(2, 8, seed=1)
    found = sensitivity.sobol_indices(numpy.zeros((len(u), 0)), 2, seed=1)
    assert found["S1"].shape == (2, 0)
