"""The thinning of the lines of a chart to the points which can be seen.

A curve of a simulation carries every step of the integrator, which the
simulations keep as their output: thousands of points for a line which is a
few hundred pixels long, and a curve over another variable than the time, e.g.
one thin line per draw of an uncertainty run, is drawn from all of them. The
spec of a chart (`Chart.spec`, its JSON and the SVG of the renderer) then
carries points no one can tell apart. `thin_elements` reduces the line of
every curve, band and area of a plot to the points which are visibly apart
with the algorithm of Ramer, Douglas and Peucker: a point is left out only
when it is within `THIN_TOLERANCE` of the span of the axes of the segment
between the points kept around it, in the coordinates of the panel (the
`log10` of a value on a logarithmic axis), so the line drawn through the kept
points is within that distance of the one drawn through all of them.

What is kept as it is:

- an element of fewer than `THIN_MIN_POINTS` points, and every element but
  curves, bands and areas,
- a curve with markers or error bars, or without a line: its points are data,
- the first and the last point, a point which is no point of the panel (not
  finite, or not positive on a logarithmic axis) and its neighbours, so a gap
  stays where it is,
- the smallest and the largest value of every column and the two points
  around an end the spec gives the x axis, so the renderer derives the same
  limits of the axes (`crates/plot/src/render/axis.rs` and `cut.rs`) and the
  ticks do not move,
- the values of the points kept, bitwise.

A band and an area are thinned as one: their columns share the x values, and
a point is left out only when it is within the tolerance on every column
(the low and the high end and the median). A jump, two points at the time of
a dose, is kept as any point beyond the tolerance.

The span of an axis is the one its spec gives (`min` and `max`, a `frame`),
else the extent of the values of the thinned elements on it, on the y axes
those within the ends the spec gives the x axis, and on a logarithmic axis at
most its `max_decades` (`LOG_DECADES` by default): never more than the span
the renderer draws, so the tolerance is never more than `THIN_TOLERANCE` of
the panel. An axis of categories, or one of no span, thins nothing.

The algorithm runs in the extension of the package, the crate `sbmlsim-thin`
(`kept_points`): this module chooses the coordinates and the points which are
kept in any case, the extension the points which keep the others within the
tolerance.
"""

from __future__ import annotations

import math
from collections.abc import Iterable, Mapping, Sequence
from dataclasses import dataclass
from typing import Any, Final

import numpy as np

from sbmlsim_core._core import thin_kept_points

__all__ = [
    "LOG_DECADES",
    "THIN_MIN_POINTS",
    "THIN_TOLERANCE",
    "kept_points",
    "thin_elements",
]

#: The largest distance of a point which is left out from the line drawn
#: without it, as a share of the span of the axes. A panel is at most about a
#: thousand pixels long (a wide plot as a PNG of 144 dpi), so a line moves by
#: at most a tenth of a pixel at the size of its figure, and by a pixel when a
#: viewer zooms into it tenfold.
THIN_TOLERANCE: Final = 1e-4

#: The fewest points of a line which is thinned; a shorter one costs nothing.
THIN_MIN_POINTS: Final = 100

#: The decades a logarithmic axis shows below its largest value when its spec
#: gives none, `LOG_DECADES` of the renderer.
LOG_DECADES: Final = 6.0

#: The columns of the values on the y axis of the elements which are thinned,
#: by their kind.
_Y_COLUMNS: Final[Mapping[str, tuple[str, ...]]] = {
    "curve": ("y",),
    "band": ("low", "high", "median"),
    "area": ("from", "to"),
}


@dataclass(frozen=True)
class _Frame:
    """An axis in the coordinates of its panel.

    Attributes:
        log: whether the axis is logarithmic, its coordinate the `log10` of a
            value.
        low: the coordinate of the lower end.
        span: the span of the coordinates, positive.
    """

    log: bool
    low: float
    span: float

    def coordinates(self, values: np.ndarray) -> np.ndarray:
        """Get values as coordinates in units of the span, `NaN` for no point."""
        with np.errstate(divide="ignore", invalid="ignore"):
            u = np.log10(np.where(values > 0, values, np.nan)) if self.log else values
        return (u - self.low) / self.span


def _array(values: Sequence[float | None] | np.ndarray) -> np.ndarray:
    """Get the values of a column as numbers, a gap `NaN`."""
    return np.asarray(values, dtype=float)


def _thinned(element: Mapping[str, Any]) -> tuple[str, ...] | None:
    """Get the y columns of an element which is thinned, `None` if it is kept.

    A curve with markers, error bars or no line is data and kept, as is an
    element of fewer than `THIN_MIN_POINTS` points.
    """
    columns = _Y_COLUMNS.get(element.get("kind", ""))
    if columns is None or len(element.get("x", ())) < THIN_MIN_POINTS:
        return None
    if element["kind"] == "curve":
        style = element.get("style", {})
        marker = style.get("marker", {}).get("type", "none")
        line = style.get("line", {}).get("type", "solid")
        if (
            marker != "none"
            or line == "none"
            or element.get("xerr") is not None
            or element.get("yerr") is not None
        ):
            return None
    return tuple(c for c in columns if element.get(c) is not None)


def _given(axis: Mapping[str, Any]) -> tuple[float | None, float | None]:
    """Get the ends the spec of an axis gives, its `frame` without `min`/`max`."""
    low, high = axis.get("min"), axis.get("max")
    frame = axis.get("frame")
    if frame is not None and low is None and high is None:
        low, high = frame["low"], frame["high"]
    return low, high


def _frame(axis: Mapping[str, Any], values: Iterable[np.ndarray]) -> _Frame | None:
    """Get an axis in the coordinates of its panel, `None` if it thins nothing.

    Args:
        axis: the spec of the axis.
        values: the values of the thinned elements on the axis.
    """
    scale = axis.get("scale", "linear")
    if scale not in ("linear", "log"):
        return None
    log = scale == "log"
    low, high = _given(axis)

    def coordinate(value: float) -> float:
        """Get the coordinate of a value, `NaN` for no point of the axis."""
        if not log:
            return float(value)
        return math.log10(value) if value > 0 else math.nan

    # the extent of the values shown, without a copy of them
    smallest, largest = math.inf, -math.inf
    for v in values:
        shown = np.isfinite(v) & (v > 0) if log else np.isfinite(v)
        if low is not None:
            shown &= v >= low
        if high is not None:
            shown &= v <= high
        if shown.any():
            smallest = min(smallest, float(np.min(v, where=shown, initial=np.inf)))
            largest = max(largest, float(np.max(v, where=shown, initial=-np.inf)))
    lo = coordinate(low) if low is not None else None
    hi = coordinate(high) if high is not None else None
    if (lo is None or hi is None) and smallest > largest:
        return None
    if lo is None:
        lo = coordinate(smallest)
    if hi is None:
        hi = coordinate(largest)
    if log and (low is None or high is None):
        decades = axis.get("max_decades")
        lo = max(lo, hi - (LOG_DECADES if decades is None else float(decades)))
    span = hi - lo
    if not (math.isfinite(span) and span > 0):
        return None
    return _Frame(log=log, low=lo, span=span)


def _x_window(axis: Mapping[str, Any], x: np.ndarray) -> np.ndarray:
    """Get whether the points are within the ends the spec gives the x axis."""
    low, high = axis.get("min"), axis.get("max")
    inside = np.ones(x.shape, dtype=bool)
    if low is not None:
        inside &= x >= low
    if high is not None:
        inside &= x <= high
    return inside


def kept_points(
    xu: np.ndarray,
    yu: np.ndarray,
    tolerance: float,
    keep: np.ndarray | None = None,
) -> np.ndarray:
    """Get the points of a line which are kept, Ramer-Douglas-Peucker.

    The algorithm splits every segment at its point farthest from it, the
    first one on a tie, while that point is farther than the tolerance. It
    runs in the extension of the package (the crate `sbmlsim-thin`), whose
    kept points are bitwise the ones of the implementation on numpy before
    it, which `python/tests/test_chart_thinning_reference.py` keeps.

    Args:
        xu: the x coordinates of the points, in units of the span of the axis.
        yu: the y coordinates of one or more columns over the same x, a row
            per column, in units of the span of their axis.
        tolerance: the largest distance of a point which is left out from
            the segment between the points kept around it, on every column.
        keep: the points which are kept in any case.

    Returns:
        Whether every point is kept: the first and the last one, the ones of
        `keep`, every point which is not finite and its neighbours, and the
        points which keep every other one within the tolerance.
    """
    return thin_kept_points(xu, np.atleast_2d(yu), tolerance, keep)


def _extremes(columns: Iterable[np.ndarray], where: np.ndarray) -> np.ndarray:
    """Get the points of the smallest and the largest value of every column within a mask.

    The first point of the smallest and of the largest value, a point outside
    the mask or not finite never.
    """
    keep = np.zeros(where.shape, dtype=bool)
    for values in columns:
        ok = where & np.isfinite(values)
        if ok.all():
            keep[np.argmin(values)] = True
            keep[np.argmax(values)] = True
        elif ok.any():
            keep[np.argmin(np.where(ok, values, np.inf))] = True
            keep[np.argmax(np.where(ok, values, -np.inf))] = True
    return keep


def _thin(
    element: Mapping[str, Any],
    columns: tuple[str, ...],
    x_axis: Mapping[str, Any],
    x_frame: _Frame,
    y_frame: _Frame,
    tolerance: float,
) -> dict[str, Any]:
    """Get an element with the points of its line which are kept."""
    x = _array(element["x"])
    ys = [_array(element[c]) for c in columns]
    xu = x_frame.coordinates(x)
    yu = np.vstack([y_frame.coordinates(y) for y in ys])
    inside = _x_window(x_axis, x)
    keep = _extremes([xu, *yu], inside)
    if not inside.all():
        keep |= _extremes([xu, *yu], np.ones_like(inside))
    # the two points around an end of the x axis, where the renderer cuts
    crossing = np.flatnonzero(inside[1:] != inside[:-1])
    keep[crossing] = True
    keep[crossing + 1] = True
    kept = kept_points(xu, yu, tolerance, keep=keep)
    if kept.all():
        return dict(element)
    index = np.flatnonzero(kept)
    thinned = dict(element)
    for c in ("x", *columns):
        values = element[c]
        thinned[c] = (
            values[index]
            if isinstance(values, np.ndarray)
            else [values[i] for i in index.tolist()]
        )
    return thinned


def thin_elements(
    elements: Sequence[Mapping[str, Any]],
    axes: Mapping[str, Mapping[str, Any]],
    tolerance: float = THIN_TOLERANCE,
) -> list[dict[str, Any]]:
    """Get the elements of a plot with their lines thinned, see the module.

    Args:
        elements: the elements of the plot, dictionaries of `figure/1` whose
            columns are lists or numpy arrays.
        axes: the specs of the axes of the plot by their names, `x`, `y` and
            `y2`.
        tolerance: the largest distance of a point which is left out, a
            share of the span of the axes, see `THIN_TOLERANCE`.

    Returns:
        The elements, a thinned one a new dictionary with new lists of its
        columns, the others as they are.
    """
    thinned = {i: _thinned(e) for i, e in enumerate(elements)}
    thinned = {i: c for i, c in thinned.items() if c}
    if not thinned:
        return [dict(e) for e in elements]
    x_axis = axes["x"]
    x_frame = _frame(x_axis, (_array(elements[i]["x"]) for i in thinned))
    if x_frame is None:
        return [dict(e) for e in elements]
    y_frames: dict[str, _Frame | None] = {}
    for name in {elements[i].get("axis", "y") for i in thinned}:
        if name not in axes:
            y_frames[name] = None
            continue
        values: list[np.ndarray] = []
        for i, columns in thinned.items():
            if elements[i].get("axis", "y") != name:
                continue
            inside = _x_window(x_axis, _array(elements[i]["x"]))
            values.extend(_array(elements[i][c])[inside] for c in columns)
        y_frames[name] = _frame(axes[name], values)
    out: list[dict[str, Any]] = []
    for i, element in enumerate(elements):
        columns = thinned.get(i)
        y_frame = y_frames.get(element.get("axis", "y"))
        if columns is None or y_frame is None:
            out.append(dict(element))
        else:
            out.append(_thin(element, columns, x_axis, x_frame, y_frame, tolerance))
    return out
