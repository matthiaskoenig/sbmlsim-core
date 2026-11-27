"""The chart builder of the figure contract `figure/1`, drawn by the renderer of the package.

A `Chart` is a figure of plots in a grid, `ChartPlot`s with the axes `ChartAxis`,
whose elements (curves, bands, bars, box plots, heat maps, ...) its methods add;
`Chart.spec` is its `figure/1` spec, `Chart.svg`, `Chart.png` and `Chart.save`
draw it with `sbmlsim_core.render_svg` and `render_png`:

    from sbmlsim_core.chart import Chart, ChartAxis

    chart = Chart(title="decay")
    plot = chart.plot(x=ChartAxis(label="time", unit="hr"), y=ChartAxis(label="A"))
    plot.curve([0, 1, 2], [1.0, 0.5, 0.25], label="A")
    chart.save("decay.svg")

The package needs numpy only: quantities of pint and data arrays of xarray are
read when given, by duck typing (`sbmlsim_core.chart.units`), and neither is
imported by the chart. The colours are those of matplotlib, read without it
(`sbmlsim_core.chart.colors`), and the lines of a chart are thinned to the
points which can be seen (`sbmlsim_core.chart.thinning`).
"""

from sbmlsim_core.chart.builder import (
    BAND_OPACITY,
    DEFAULT_DPI,
    FIGURE_CONTRACT,
    LEGEND_LOCS,
    MARKERS,
    SAVE_SUFFIXES,
    Aspect,
    AxisName,
    Baseline,
    Chart,
    ChartAxis,
    ChartPlot,
    ColorMap,
    Coordinates,
    Extend,
    FillStyle,
    LineType,
    Placement,
    Values,
    axis_label,
    legend_loc_spec,
)
from sbmlsim_core.chart.colors import CYCLE, color_alpha, normalize_color
from sbmlsim_core.chart.thinning import THIN_TOLERANCE, thin_elements
from sbmlsim_core.chart.units import short_unit

__all__ = [
    "BAND_OPACITY",
    "CYCLE",
    "DEFAULT_DPI",
    "FIGURE_CONTRACT",
    "LEGEND_LOCS",
    "MARKERS",
    "SAVE_SUFFIXES",
    "THIN_TOLERANCE",
    "Aspect",
    "AxisName",
    "Baseline",
    "Chart",
    "ChartAxis",
    "ChartPlot",
    "ColorMap",
    "Coordinates",
    "Extend",
    "FillStyle",
    "LineType",
    "Placement",
    "Values",
    "axis_label",
    "color_alpha",
    "legend_loc_spec",
    "normalize_color",
    "short_unit",
    "thin_elements",
]
