"""The global sensitivity analysis against SALib (the group `sensitivity-tests`): the designs
where they are deterministic, and the estimators on the same values and the same resamples.

The random streams of `sbmlsim_core.sensitivity` are not numpy's, so the points of a seed are
not the ones SALib draws for it. What is compared is everything a seed does not decide: the
Sobol' sequence without scrambling and its scrambling for the same random bits (the crate tests
the latter), the search curves of FAST for the same phases, the trajectories of Morris for the
same starts, orders and directions, and every estimate and interval for the resamples SALib
draws, element by element."""

import copy
import math
import warnings

import numpy
import pytest

SALib = pytest.importorskip("SALib")
scipy_qmc = pytest.importorskip("scipy.stats.qmc")

from SALib.analyze import fast as salib_fast  # noqa: E402
from SALib.analyze import morris as salib_morris  # noqa: E402
from SALib.analyze import sobol as salib_sobol  # noqa: E402
from SALib.sample import fast_sampler  # noqa: E402
from SALib.sample import morris as morris_sampler  # noqa: E402
from SALib.sample import sobol as sobol_sampler  # noqa: E402
from sbmlsim_core import sensitivity  # noqa: E402

PI = numpy.pi
D = 3


def problem(d: int) -> dict:
    """The problem of SALib on the unit cube."""
    return {
        "num_vars": d,
        "names": [f"x{k}" for k in range(d)],
        "bounds": [[0.0, 1.0]] * d,
    }


def elements(u: numpy.ndarray) -> numpy.ndarray:
    """A column per element: the Ishigami function with three pairs of coefficients."""
    x = u * 2.0 * PI - PI
    return numpy.stack(
        [
            numpy.sin(x[:, 0])
            + a * numpy.sin(x[:, 1]) ** 2
            + b * x[:, 2] ** 4 * numpy.sin(x[:, 0])
            for a, b in [(7.0, 0.1), (1.0, 0.5), (0.0, 0.0)]
        ],
        axis=1,
    )


def same(mine: numpy.ndarray, theirs: object, atol: float = 1e-14) -> None:
    """An estimate of one element is the one of SALib within the rounding."""
    numpy.testing.assert_allclose(
        mine, numpy.asarray(theirs, dtype=float), rtol=1e-10, atol=atol
    )


# --- the designs ---


@pytest.mark.parametrize(("d", "n", "skip"), [(1, 64, 0), (7, 128, 0), (40, 32, 16)])
def test_the_sequence_without_scrambling_is_scipy_s(d: int, n: int, skip: int) -> None:
    engine = scipy_qmc.Sobol(d, scramble=False)
    if skip:
        engine.fast_forward(skip)
    numpy.testing.assert_array_equal(
        sensitivity.sobol_sequence(d, n, seed=None, skip=skip), engine.random(n)
    )


def test_every_dimension_of_the_direction_numbers_is_scipy_s() -> None:
    with warnings.catch_warnings():
        warnings.simplefilter("ignore", UserWarning)
        expected = scipy_qmc.Sobol(2048, scramble=False).random(1024)
    numpy.testing.assert_array_equal(
        sensitivity.sobol_sequence(2048, 1024, seed=None), expected
    )


@pytest.mark.parametrize("second_order", [False, True])
def test_the_matrix_of_saltelli_without_scrambling_is_salib_s(
    second_order: bool,
) -> None:
    for skip in (0, 8):
        with warnings.catch_warnings():
            # SALib warns that a skip below n does not converge
            warnings.simplefilter("ignore", UserWarning)
            expected = sobol_sampler.sample(
                problem(4),
                16,
                calc_second_order=second_order,
                scramble=False,
                skip_values=skip,
            )
        found = sensitivity.saltelli(
            4, 16, seed=None, second_order=second_order, skip=skip
        )
        numpy.testing.assert_array_equal(found, expected)


def test_the_search_curves_of_fast_are_salib_s(monkeypatch: pytest.MonkeyPatch) -> None:
    uniforms = [0.1, 0.73, 0.5, 0.999]

    class Phases:
        """The generator of SALib, which gives the phases of the blocks."""

        def __init__(self) -> None:
            self.draws = iter(uniforms)

        def random(self) -> float:
            return next(self.draws)

    for d, n, m in [(4, 1025, 4), (2, 65, 4), (3, 257, 2)]:
        monkeypatch.setattr(fast_sampler, "handle_seed", lambda seed: Phases())
        expected = fast_sampler.sample(problem(d), n, M=m, seed=0)
        phases = [2 * math.pi * u for u in uniforms[:d]]
        found = sensitivity.fast_sample(d, n, m=m, phases=phases)
        # the sine and the arcsine of numpy and of libm round alike within an ulp or two
        numpy.testing.assert_allclose(found, expected, rtol=0.0, atol=2e-15)


@pytest.mark.parametrize("levels", [2, 4, 5, 6])
def test_the_trajectories_of_morris_are_salib_s(levels: int) -> None:
    """Every trajectory is the one SALib builds from its start, order and directions."""
    from SALib.sample.morris.morris import _compute_b_star
    from SALib.util import _compute_delta

    d, trajectories = 4, 25
    delta = _compute_delta(levels)
    grid = numpy.linspace(0, 1 - delta, int(levels / 2))
    points = sensitivity.morris_sample(d, trajectories, levels=levels, seed=9)
    for t in range(trajectories):
        trajectory = points[t * (d + 1) : (t + 1) * (d + 1)]
        steps = numpy.diff(trajectory, axis=0)
        start = trajectory.min(axis=0)
        assert numpy.isin(start, grid).all()
        # the step which moves a parameter is its column of P*, its sign the diagonal of D*
        moves = numpy.abs(steps).argmax(axis=0)
        p_star = numpy.zeros((d, d))
        p_star[numpy.arange(d), moves] = 1.0
        d_star = numpy.diag(numpy.sign(steps[moves, numpy.arange(d)]))
        expected = _compute_b_star(
            numpy.ones((d + 1, d)),
            start[None, :],
            delta,
            numpy.tril(numpy.ones([d + 1, d], dtype=int), -1),
            numpy.eye(d),
            p_star,
            d_star,
        )
        numpy.testing.assert_array_equal(trajectory, expected)


def test_morris_draws_every_start_order_and_direction() -> None:
    d, trajectories = 3, 400
    points = sensitivity.morris_sample(d, trajectories, levels=4, seed=1)
    reference = morris_sampler.sample(problem(d), trajectories, num_levels=4, seed=1)
    for sample in (points, reference):
        shaped = sample.reshape(trajectories, d + 1, d)
        firsts = shaped[:, 0, :]
        # both draw the four levels of the first point alike, a quarter each
        for level in numpy.linspace(0, 1, 4):
            share = numpy.isclose(firsts, level).mean()
            assert share == pytest.approx(0.25, abs=0.04)


# --- the estimators, on the same values and the resamples SALib draws ---


@pytest.mark.parametrize("second_order", [False, True])
def test_the_sobol_indices_are_salib_s(second_order: bool) -> None:
    u = sensitivity.saltelli(D, 64, seed=2, second_order=second_order)
    y = elements(u)
    n = 64
    # the resamples of SALib, `default_rng(seed).integers(N, size=(N, num_resamples))`
    resamples = numpy.random.default_rng(2).integers(n, size=(n, 100)).T
    found = sensitivity.sobol_indices(
        y, D, second_order=second_order, resamples=resamples
    )
    for element in range(y.shape[1]):
        expected = salib_sobol.analyze(
            problem(D),
            y[:, element],
            calc_second_order=second_order,
            num_resamples=100,
            conf_level=0.95,
            print_to_console=False,
            seed=2,
        )
        for key in ("S1", "S1_conf", "ST", "ST_conf"):
            same(found[key][:, element], expected[key])
        if second_order:
            upper = numpy.triu_indices(D, k=1)
            for key in ("S2", "S2_conf"):
                same(found[key][..., element][upper], expected[key][upper])


def test_the_fast_indices_are_salib_s() -> None:
    u = sensitivity.fast_sample(D, 257, seed=3)
    y = elements(u)
    # the starts of the windows of SALib, `randint(0, n)` of the legacy state of its seed, a
    # parameter after the other
    state = numpy.random.RandomState(11)
    starts = numpy.array(
        [[state.randint(0, 257) for _ in range(100)] for _ in range(D)]
    )
    found = sensitivity.fast_indices(y, D, resamples=starts)
    for element in range(y.shape[1]):
        with warnings.catch_warnings():
            # SALib warns that its intervals of FAST are unreliable
            warnings.simplefilter("ignore", UserWarning)
            expected = salib_fast.analyze(
                problem(D),
                y[:, element],
                M=4,
                num_resamples=100,
                conf_level=0.95,
                print_to_console=False,
                seed=11,
            )
        for key in ("S1", "S1_conf", "ST", "ST_conf"):
            same(found[key][:, element], expected[key], atol=1e-12)


@pytest.mark.parametrize(("levels", "trajectories"), [(4, 20), (6, 13)])
def test_the_elementary_effects_are_salib_s(levels: int, trajectories: int) -> None:
    grid = sensitivity.morris_sample(D, trajectories, levels=levels, seed=4)
    y = elements(grid)
    # the resamples of SALib, from a child of its generator, a parameter after the other
    child = numpy.random.default_rng(4).spawn(1)[0]
    drawn = copy.deepcopy(child)
    resamples = numpy.stack(
        [drawn.integers(trajectories, size=(100, trajectories)) for _ in range(D)]
    )
    found = sensitivity.morris_effects(grid, y, levels=levels, resamples=resamples)
    for element in range(y.shape[1]):
        expected = salib_morris.analyze(
            problem(D),
            grid,
            y[:, element],
            num_levels=levels,
            scaled=False,
            num_resamples=100,
            conf_level=0.95,
            print_to_console=False,
            seed=numpy.random.default_rng(4),
        )
        for key in ("mu", "mu_star", "sigma"):
            same(found[key][:, element], expected[key])
        # the interval of a parameter whose effects do not vary in magnitude is the standard
        # deviation of equal means, which cancels, so the two summation orders round it
        # differently at the scale of the effects
        scale = float(
            numpy.max(numpy.abs(numpy.asarray(expected["mu_star"], dtype=float)))
        )
        same(found["mu_star_conf"][:, element], expected["mu_star_conf"], 1e-12 * scale)


def test_the_intervals_of_a_seed_are_those_of_salib_in_distribution() -> None:
    """The resamples of a seed are other ones, the intervals estimate the same spread."""
    u = sensitivity.saltelli(D, 512, seed=5)
    y = elements(u)[:, :1]
    ours = numpy.array(
        [
            sensitivity.sobol_indices(y, D, seed=seed, num_resamples=400)["S1_conf"][
                :, 0
            ]
            for seed in range(5)
        ]
    ).mean(axis=0)
    theirs = numpy.array(
        [
            salib_sobol.analyze(
                problem(D),
                y[:, 0],
                calc_second_order=False,
                num_resamples=400,
                print_to_console=False,
                seed=seed + 1,
            )["S1_conf"]
            for seed in range(5)
        ]
    ).mean(axis=0)
    numpy.testing.assert_allclose(ours, theirs, rtol=0.1)
