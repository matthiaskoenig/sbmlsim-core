"""The kept points of the thinning against its implementation on numpy.

`kept_points` of `sbmlsim_core.chart.thinning` runs in the extension of the
package (the crate `sbmlsim-thin`) and keeps bitwise the points of the
implementation on numpy it replaced, which this module keeps as the reference:
on random lines and on the lines which test the edges of the algorithm, gaps,
jumps, logarithmic axes, flat segments, repeated points, ties and overflows.
"""

from collections.abc import Callable

import numpy as np
import pytest
from sbmlsim_core.chart.thinning import THIN_TOLERANCE, kept_points


def _distances(
    xu: np.ndarray, yu: np.ndarray, points: np.ndarray, a: np.ndarray, b: np.ndarray
) -> np.ndarray:
    """Get the distance of points to the segments between two points, the most over the columns.

    The overflows of the lines of huge coordinates are not numbers, quietly.
    """
    with np.errstate(all="ignore"):
        dx = xu[b] - xu[a]
        px = xu[points] - xu[a]
        dy = yu[:, b] - yu[:, a]
        py = yu[:, points] - yu[:, a]
        length = dx * dx + dy * dy
        t = np.where(length > 0, (px * dx + py * dy) / length, 0.0)
        t = np.clip(t, 0.0, 1.0)
        return np.hypot(px - t * dx, py - t * dy).max(axis=0)


def _reference(
    xu: np.ndarray, yu: np.ndarray, tolerance: float, keep: np.ndarray | None = None
) -> np.ndarray:
    """Get the kept points of a line by the implementation on numpy, every segment of a level at once."""
    yu = np.atleast_2d(yu)
    n = xu.size
    kept = np.zeros(n, dtype=bool) if keep is None else keep.copy()
    if n <= 2:
        kept[:] = True
        return kept
    kept[[0, -1]] = True
    gap = ~(np.isfinite(xu) & np.isfinite(yu).all(axis=0))
    kept |= gap
    kept[1:] |= gap[:-1]
    kept[:-1] |= gap[1:]
    todo = np.flatnonzero(~kept)
    while todo.size:
        ends = np.flatnonzero(kept)
        segment = np.searchsorted(ends, todo) - 1
        d = _distances(xu, yu, todo, ends[segment], ends[segment + 1])
        first = np.r_[True, segment[1:] != segment[:-1]]
        starts = np.flatnonzero(first)
        group = np.cumsum(first) - 1
        peak = np.maximum.reduceat(d, starts)
        split = peak > tolerance
        if not split.any():
            break
        farthest = np.flatnonzero((d == peak[group]) & split[group])
        farthest = farthest[np.r_[True, group[farthest][1:] != group[farthest][:-1]]]
        kept[todo[farthest]] = True
        stay = split[group]
        stay[farthest] = False
        todo = todo[stay]
    return kept


def _span(values: np.ndarray) -> np.ndarray:
    """Get values in units of the span of their finite ones."""
    finite = values[np.isfinite(values)]
    if finite.size == 0 or finite.max() == finite.min():
        return values
    return (values - finite.min()) / (finite.max() - finite.min())


def _log(values: np.ndarray) -> np.ndarray:
    """Get values as the coordinates of a logarithmic axis, `NaN` for no point."""
    with np.errstate(divide="ignore", invalid="ignore"):
        return _span(np.log10(np.where(values > 0, values, np.nan)))


def _steps(rng: np.random.Generator, n: int) -> np.ndarray:
    """Get the times of the steps of an integrator over two doses, dense after each."""
    half = n // 2
    first = np.geomspace(1e-4, 24, half) * rng.uniform(0.9, 1.1)
    return np.sort(np.concatenate([first, 24 + np.geomspace(1e-4, 24, n - half)]))


def _pk(rng: np.random.Generator, t: np.ndarray) -> np.ndarray:
    """Get a concentration of two doses, with a jump at the second dose."""
    ka, ke = rng.uniform(0.5, 3.0), rng.uniform(0.05, 0.3)
    c = np.exp(-ke * (t % 24)) - np.exp(-ka * (t % 24))
    return np.where(t >= 24, c + np.exp(-ke * t) + 0.5, c)


def _lines(seed: int) -> list[tuple[str, np.ndarray, np.ndarray]]:
    """Get lines of every kind as their coordinates, a row per column."""
    rng = np.random.default_rng(seed)
    lines: list[tuple[str, np.ndarray, np.ndarray]] = []
    n = int(rng.integers(100, 4000))
    t = _steps(rng, n)
    pk = _pk(rng, t)
    lines.append(("integrator", _span(t), _span(pk)[None]))
    lines.append(("log axis", _span(t), _log(pk)[None]))
    lines.append(("log-log axes", _log(t), _log(pk)[None]))
    lines.append(
        ("band", _span(t), np.vstack([_span(pk * f) for f in (0.8, 1.0, 1.25)]))
    )
    lines.append(
        ("log band", _span(t), np.vstack([_log(pk * f) for f in (0.8, 1.0, 1.25)]))
    )
    walk = np.cumsum(rng.normal(size=n))
    lines.append(("random walk", np.linspace(0, 1, n), _span(walk)[None]))
    lines.append(("noise", np.sort(rng.uniform(size=n)), rng.uniform(size=(2, n))))
    gaps = _span(pk.copy())
    gaps[rng.integers(0, n, size=int(rng.integers(1, 20)))] = np.nan
    gaps[: int(rng.integers(0, 5))] = np.nan
    lines.append(("gaps", _span(t), gaps[None]))
    run = int(rng.integers(0, n - 10))
    holes = _span(pk.copy())
    holes[run : run + 10] = np.nan
    holes[-1] = np.inf
    lines.append(("holes", _span(t), holes[None]))
    x = np.repeat(np.linspace(0, 1, n // 2), 2)
    lines.append(("jumps", x, np.cumsum(rng.uniform(-1, 1, size=x.size))[None] / n))
    flat = np.where(t < 24, 0.0, _span(pk))
    lines.append(("flat", _span(t), flat[None]))
    lines.append(("constant", _span(t), np.zeros((1, n))))
    staircase = np.floor(np.linspace(0, 8, n)) / 8
    lines.append(("staircase", np.linspace(0, 1, n), staircase[None]))
    repeated = np.repeat(rng.uniform(size=n // 4), 4)
    lines.append(("repeated", np.repeat(np.linspace(0, 1, n // 4), 4), repeated[None]))
    zigzag = np.tile([0.0, 1.0], n // 2)
    lines.append(("ties", np.linspace(0, 1, zigzag.size), zigzag[None]))
    lines.append(("back and forth", rng.uniform(size=n), rng.uniform(size=(1, n))))
    lines.append(("tiny", _span(t) * 1e-300, _span(pk)[None] * 1e-300))
    lines.append(("subnormal", _span(t) * 1e-310, _span(pk)[None] * 1e-310))
    lines.append(("huge", _span(t) * 1e300, _span(pk)[None] * 1e300))
    lines.append(("overflow", _span(t) * 1e200, _span(pk)[None]))
    near = _span(pk) + rng.normal(scale=THIN_TOLERANCE, size=n)
    lines.append(("near the tolerance", _span(t), near[None]))
    return lines


_KEEPS: dict[str, Callable[[np.random.Generator, int], np.ndarray | None]] = {
    "none": lambda rng, n: None,
    "some": lambda rng, n: rng.uniform(size=n) < 0.01,
}


@pytest.mark.parametrize("keep", list(_KEEPS))
@pytest.mark.parametrize("tolerance", [0.0, 1e-6, THIN_TOLERANCE, 1e-2, 0.5])
@pytest.mark.parametrize("seed", range(4))
def test_the_kept_points_are_those_of_numpy(
    seed: int, tolerance: float, keep: str
) -> None:
    rng = np.random.default_rng(1000 + seed)
    # no tolerance keeps nearly every point, which the reference splits a level
    # at a time, a pass over the line per point: the first points of the lines
    end = 400 if tolerance == 0.0 else None
    for name, x, y in _lines(seed):
        xu, yu = x[:end], y[:, :end]
        given = _KEEPS[keep](rng, xu.size)
        expected = _reference(xu, yu, tolerance, given)
        kept = kept_points(xu, yu, tolerance, given)
        assert kept.dtype == np.bool_
        assert np.array_equal(kept, expected), (
            f"{name}: {np.flatnonzero(kept != expected)[:10]} differ"
        )


@pytest.mark.parametrize("n", [0, 1, 2, 3, 4])
def test_a_short_line_is_that_of_numpy(n: int) -> None:
    xu = np.linspace(0, 1, n)
    yu = np.sin(7 * xu)
    assert np.array_equal(kept_points(xu, yu, 1e-4), _reference(xu, yu, 1e-4))


def test_a_line_of_one_dimension_is_one_column() -> None:
    xu = np.linspace(0, 1, 500)
    yu = np.sin(9 * xu) ** 3
    assert np.array_equal(kept_points(xu, yu, 1e-4), _reference(xu, yu, 1e-4))


def test_the_columns_need_not_be_contiguous() -> None:
    xu = np.linspace(0, 1, 1001)
    yu = np.vstack([np.sin(5 * xu), np.cos(3 * xu)])
    expected = _reference(xu[::2], yu[:, ::2], 1e-4)
    assert np.array_equal(kept_points(xu[::2], yu[:, ::2], 1e-4), expected)
    assert np.array_equal(kept_points(xu[::2], yu.T.copy().T[:, ::2], 1e-4), expected)


def test_the_given_points_are_not_changed() -> None:
    xu = np.linspace(0, 1, 300)
    keep = np.zeros(300, dtype=bool)
    keep[150] = True
    kept = kept_points(xu, xu**2, 1e-4, keep)
    assert kept[150] and keep.sum() == 1 and kept.sum() > 3


def test_columns_of_another_length_are_an_error() -> None:
    with pytest.raises(ValueError, match="rows of"):
        kept_points(np.zeros(5), np.zeros((1, 4)), 1e-4)
    with pytest.raises(ValueError, match="kept in any case"):
        kept_points(np.zeros(5), np.zeros(5), 1e-4, np.zeros(4, dtype=bool))
