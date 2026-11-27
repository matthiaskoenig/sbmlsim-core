# Figures

`sbmlsim-plot` is the renderer of the figures of sbmlsim. One description of a figure, the JSON spec `figure/1`, is drawn to a static SVG or PNG file by the python package and to the same SVG inside a page by `plot.wasm`, so a figure of a report and the figure of an interactive panel are one drawing. The renderer computes the same bits on every target (the transcendental functions are the ones of `libm`), which is why the SVG of the wasm module is byte for byte the SVG of the native renderer; the tests compare them, and `crates/plot-wasm/check_golden.mjs` runs the module in Node against the golden files of the crate.

The same crate answers two questions a page needs beside the drawing: the geometry of the figure (the size it is drawn at and where the panels, their scales and the legend entries are, in pixels) and the data point under a pointer (`pick`). A figure a pointer moves over is prepared once, so a pick costs about 0.1 ms.

A minimal spec:

```json
{
  "contract": "figure/1",
  "width": 480,
  "height": 320,
  "grid": {"rows": 1, "columns": 1},
  "plots": [{
    "id": "main",
    "row": 0,
    "column": 0,
    "x": {"label": "time", "unit": "h"},
    "y": {"label": "concentration", "unit": "mM"},
    "elements": [{
      "id": "c",
      "kind": "curve",
      "label": "C",
      "x": [0, 1, 2, 3],
      "y": [0.0, 1.0, null, 0.5]
    }]
  }]
}
```

## The contract figure/1

The contract id is `figure/1` (`FIGURE_CONTRACT` in python, `sbmlsim_plot::FIGURE_CONTRACT` in Rust). Every object refuses unknown fields, every name is snake case, and a field with a default may be absent. A field added to the contract is optional, and its default is what the renderer drew before it, so a spec of an earlier version draws as it did; the version which added a field is named with it ("since 0.30.0"). In every numeric data array JSON `null` is a gap, a value which is not there: it breaks a line and leaves out a marker or a bar, and a value the renderer cannot draw is written back as `null`. `NaN` and `Infinity` are no JSON: the `json.dumps` of python writes them for a `float("nan")` unless it is given `allow_nan=False`, and the renderer refuses them with the path of the value and a message saying that a gap is written as `null` (in python `None`). A color is the text `#rrggbb`.

### Figure

| field | type | default | meaning |
|---|---|---|---|
| `contract` | text | required | must be `figure/1` |
| `width`, `height` | number | required | size of the figure in pixels, positive and at most 16384; legends outside the plots make the figure drawn larger, see below |
| `title` | text | none | title above all plots |
| `fonts` | Fonts | see below | font sizes and weights |
| `grid` | Grid | required | the grid the plots are placed in |
| `legend` | LegendSettings | see below | legend settings of the figure |
| `plots` | list of Plot | required | the plots |

`Fonts` has `title` (16), `label` (13), `tick` (11) and `legend` (11), in points, each positive. Since 0.28.0 it also has `tick_y` (the size of the tick labels of the y axes and the color bars, positive; absent, `tick`, which then is the size of the x tick labels only), since 0.31.0 `plot_title` (the size of the plot titles, positive; absent, `label`, which the plot titles took before, so a figure title of `title` and plot titles of their own size) and the weights `title_weight` (`bold`, the figure title and the plot titles), `label_weight` (`normal`, the axis titles and the color bar labels), `tick_weight` (`normal`, every tick label) and `legend_weight` (`normal`, the legend labels), each `normal` or `bold`; a text of the weight `bold` is measured with the advances of Inter Bold and drawn with `font-weight="bold"`, so the room of bold tick labels and legend labels is their bold width. `Grid` has `rows` and `columns` (at least 1 and at most 64) and `wspace` and `hspace` (0.3), the space between plots as a fraction of a plot, not negative. The panel of a plot is at least 20 pixels wide and high over its span (since 0.30.0; before, every column and row of the grid was at least 20 pixels, so a grid of many thin columns or rows reached beyond the figure and cut its last plots off): a grid of 32 columns with `wspace` 1.2 fills its figure as long as its plots span a few columns each, and only a grid whose plots cannot have that least panel in the figure grows it, see below. `LegendSettings` has `scope` (`plot`, a legend per plot, or `row`, one legend per row of the grid), `position` (`inside`, `outside`, right of the plot, or, since 0.31.0, `bottom`, see below) and, since 0.30.0, `loc`, where a legend inside goes: `best` (the default) or a location of matplotlib's `loc`, `upper_right`, `upper_left`, `lower_left`, `lower_right`, `right`, `center_left`, `center_right`, `lower_center`, `upper_center` or `center`. Since 0.31.0 it also has `columns` and `title`, of the legend below the grid.

A legend `inside` goes to the location of its plot (of a row: of its rightmost plot with a legend), the `legend_loc` of the plot or else the `loc` of the legend settings (since 0.30.0). A fixed location puts it there, `LEGEND_INSET` (6 pixels) from the edges of the panel, whatever data it covers. `best` takes, as matplotlib's `best` does, the location of the nine of matplotlib (the upper right, upper left, lower left and lower right corner, the middle of the left and of the right side, of the bottom and of the top, and the center, in this order on a tie) whose frame has the least badness of matplotlib's `Legend._find_best_position` (since 0.33.0): the vertices of the lines strictly inside the frame (every point of a curve, with markers alone too, the caps of its error bars, the ends of a rule, the outline of a band, an area, a violin, a polygon, the lines of a box, an arrow), the boxes overlapping it (every bar and histogram bin, every text) and the lines with a segment which meets it; a heatmap and contours count nothing, as matplotlib's collections without offsets. From 0.28.0 to 0.31.1 the location covering the least ink of the data won, its lines sampled with their widths and the interiors of bands and polygons filled, which put a legend beside a band where matplotlib puts it over the band. A legend which does not fit into its panel goes outside, right of its plot (of its row); one which fits stays inside however much data it covers, as in matplotlib. A location whose frame overlaps an inset color bar of the plot (since 0.31.0) loses to every one which does not, the least overlap first, so a legend at `best` goes around the bar. Before 0.30.0 only the four corners were candidates (the lower right before the lower left on a tie), and from 0.28.0 a legend whose best corner covered more than 5 % of its frame with ink went outside; before 0.28.0 the corners were scored by the lines and points of the data alone.

Every text keeps its spaces (since 0.30.0): SVG collapses a run of spaces into one and drops the spaces at the ends of a text, so the renderer writes a space after another one and a space at an end as a no-break space (U+00A0), which is as wide as a space and which no renderer collapses; `"A  Circuit"` keeps its two spaces, as wide as the layout measures them. Since 0.28.0 every text of the spec, a figure or plot title, an axis label with its unit, a legend label, a text element and a color bar label, may hold `\n`, which splits it into lines 1.2 font sizes apart, each a `<tspan>` with its own `x` and `y`; a text is as wide as its widest line, but for the text of a heatmap cell, which stays one line: SVG draws it as one text, so a `\n` in it is measured as the space it renders as and reserves the room of that one line (a tick label too before 0.31.0, see Axis). The figure title band grows by the lines after the first, its lines centred in it; the lines of a plot title stack up from the line of a one-line title, which keeps its place above the panel; the lines of an axis title or a color bar label stack outwards from the line next to the axis or the bar (below the x axis, left of the y axis, right of the right axis and of a bar), so the room of the axis grows by them; a legend entry grows by them, its lines centred on the entry and its swatch at its middle, and the legend, its placement and the room of an outside legend in its row take the grown entries; the lines of a text element are centred on its position.

A legend `bottom` (since 0.31.0) is one legend of the figure below the grid, matplotlib's `fig.legend(loc="lower center", ncols=n)`: its entries are those of all plots merged as the entries of a row legend (`legend_order` of every plot, `declared_legends` of every plot), in `columns` columns (at least 1; absent, as many as fit into the width of the figure in one row), filled top down and then left to right as matplotlib fills them, each column as wide as its widest label and the columns two font sizes apart; it is centred on the figure, `LEGEND_GAP` (10 pixels) below the room of the last row, the figure grows down by it, and the `title` of the legend settings is drawn centred above the entries. The plots then have no legends of their own, whatever `scope` says. A legend of a plot takes the `legend_title` of the plot, a row legend the first `legend_title` of the plots of its row (since 0.31.0): drawn centred above the entries in the legend font, the frame of the legend grown by it.

The `width` and `height` of the spec are the size of the figure before its legends grow it: a legend right of a plot or of a row (`outside`, or an `inside` legend which does not fit into its panel, see above, and is placed right of it) widens the figure by its room, and a legend taller than its row heightens it, while the panels keep their size. Since 0.30.0 a grid whose plots cannot have their least panel (20 pixels over their span) in the width or the height of the spec grows the figure by what it needs as well, so no plot is ever drawn beyond the edge of the figure. The SVG has the grown size as its `width`, `height` and `viewBox`, the PNG is the grown size times `dpi / 72`, and the geometry reports it as `width` and `height`, so a page sizes its container from the geometry and not from the spec.

### Plot

| field | type | default | meaning |
|---|---|---|---|
| `id` | text | required | id of the plot, unique |
| `row`, `column` | integer | required | top left cell of the grid, from 0 |
| `row_span`, `column_span` | integer | 1 | cells it spans, within the grid |
| `title` | text | none | title of the plot |
| `x`, `y` | Axis | required | the x axis and the left y axis |
| `y2` | Axis | none | the right y axis, which elements select with `axis: "y2"` |
| `legend` | boolean | true | whether the plot has a legend |
| `legend_loc` | text | none | where the legend of the plot goes inside its panel, a location of `LegendSettings.loc`; absent, the `loc` of the figure; a row legend inside takes the one of its host plot (since 0.30.0) |
| `legend_title` | text | none | the title of the legend of the plot, drawn above its entries (matplotlib's `ax.legend(title=...)`); a row legend takes the first of its row (since 0.31.0) |
| `legend_columns` | integer | 1 | the columns of the legend of the plot, at least 1, filled top down and then left to right (matplotlib's `ncol`), each as wide as its widest label and two font sizes apart; inside, right of the plot or of its row (a row legend takes the first of its row); `best` and the room of the legend take the size of its columns (since 0.31.1) |
| `colorbar` | ColorBar | none | a color bar |
| `colorbars` | list of ColorBar | none | several color bars, stacked right of the panel in their order, each with its own label; refused together with `colorbar` (since 0.28.0) |
| `declared_legends` | list of list of text | none | the legends the plot may show in some state of a page, each a list of entry labels: the legend takes the room of the largest of them, so the plot and the figure keep their size when entries come and go, and a label without an element is not drawn; absent, the legend takes the room of the entries drawn |
| `legend_swatches` | map of text to Style | none | the swatch of a legend group by group (without one, the swatch of a group of a band and a curve draws the band behind the line and the marker of the curve since 0.31.0, matplotlib's legend of a tuple of handles), drawn with this style instead of the style of the element of its entry (or of the neutral color of a group of elements of different colors), as the kind of that element; a style without `color` takes the color of the entry; every key must be a legend group of the plot; e.g. the lines of a scan in one group shown with the middle color of their map (since 0.28.0) |
| `aspect` | `equal` or number | none | the aspect of the panel (since 0.30.0): `equal`, one unit of y as long as one unit of x, or a positive number, one unit of y that many times as long as one unit of x (matplotlib's `set_aspect` with `adjustable="datalim"`), on the coordinates of the axes (a decade on a log axis, a category on a category axis); the panel keeps its size and the limits of the axis whose units would be the longer ones are widened about their middle, so the data keeps its room on both axes, given `min` and `max` included; absent, the axes take their limits independently. A regular hexagon of a lobule is drawn with `aspect: "equal"` and no measure of the panel |
| `legend_order` | list of text | none | the order of the legend entries as legend groups: the listed groups first in this order, the others after them in the order their elements first appear; a listed group without elements is ignored, a group listed twice is refused; it lets a writer draw elements under others and still list them last; in a row legend (`legend.scope` `row`) the entries merged from the plots of the row are ordered by the `legend_order` of those plots: the groups any of them lists first, in the order of the first plot which lists them, the others after them in the order they first appear |
| `elements` | list of Element | required | what the plot draws, in the order of drawing |

### Axis

| field | type | default | meaning |
|---|---|---|---|
| `label` | text | none | label of the axis |
| `unit` | text | none | unit shown with the label |
| `scale` | `linear`, `log`, `category` | `linear` | the scale; a category axis puts its categories at the positions 0, 1, ... |
| `categories` | list of text | none | names of the categories, required for a category axis and refused for another |
| `min`, `max` | number | none | exact ends of the axis, `max` above `min`, positive on a log axis |
| `frame` | Range (`low`, `high`) | none | values the axis must hold, with the margins applied, in contrast to `min` and `max`, which are the ends themselves; `low` positive on a log axis |
| `max_decades` | number | 6 | the decades shown below the largest value of a log axis, positive, so the tiny first values of an integration do not stretch it; data within the decades keep their limits |
| `reverse` | boolean | false | reversed direction |
| `grid` | boolean | false | grid lines |
| `label_visible` | boolean | true | whether the label is shown |
| `ticks_visible` | boolean | true | whether the tick labels are shown, and the tick marks without `tick_marks_visible` |
| `tick_marks_visible` | boolean | none | whether the tick marks are shown, independent of the labels; absent, the marks follow `ticks_visible` as before 0.28.0 (since 0.28.0) |
| `line_visible` | boolean | true | whether the line of the axis is shown; a plot without axes, e.g. a map of a lobule, hides the lines of both (since 0.30.0) |
| `tick_rotation` | number | 0 | rotation of the tick labels in degrees |
| `ticks` | list of number | none | the values of the ticks instead of the ones the axis computes, matplotlib's `set_xticks`, e.g. every 24 hours of a time axis; finite, positive on a log axis, refused on a category axis; a tick outside of the limits is not drawn and the ticks do not change the limits (since 0.31.0) |
| `tick_labels` | list of text | none | the labels of `ticks`, one per tick, which needs `ticks`; absent, the numbers of the ticks (since 0.31.0) |

A tick label (of an axis and of a color bar) may hold `\n` (since 0.31.0), which splits it into lines as any text: the lines of a label of the bottom axis stack down from the tick, those of a left or right axis or a vertical bar are centred on the tick, and the axis takes the room of the widest line and of the lines (`"0\nfasted"`). In a tick label `^{...}` is a superscript, drawn in 0.7 of the size of the label and raised by 0.4 of it with `dy`, and measured so by the layout (since 0.31.0). The labels of the ticks the axis computes follow matplotlib (since 0.31.0): a log axis whose ticks are decades writes them `10^{n}` (`LogFormatterSciNotation`, the minus of an exponent U+2212), one whose ticks are 1, 2 and 5 or 1 and 3 of a decade writes them `m×10^{n}` and its decades `10^{n}` when a tick reaches 1e5 or lies below 1e-3, and else plainly; a linear axis whose largest tick is at least 1e6 or below 1e-4 (matplotlib's `axes.formatter.limits` of -5 and 6) writes the ticks divided by the power of ten of that tick and the multiplier `×10^{n}` at the end of the axis as matplotlib's offset text, right aligned below the tick labels of the bottom axis and left aligned above the panel for the left axis (right aligned above it for the right one; the room above the panel grows by a line of the tick size), in a group of the class `sbmlsim-axis-multiplier`. A color bar writes every label with its power (`2×10^{6}`). Since 0.33.0 the ticks of a log axis are those of matplotlib's `LogLocator` (3.11): major ticks at decades, every `stride` decades when more than the tick space of the axis asks for (three tick label sizes per tick along x, two along y, at most 9), minor ticks at 2 to 9 of every decade when the major ones are every decade and fewer than ten, drawn shorter (2/3.5 of a tick) and thinner (0.75 pixels) in a group `sbmlsim-ticks`, matplotlib's `AutoLocator` as the minor ticks when at most one tick falls into the limits; the labels those of `LogFormatterSciNotation`: the decades alone (`10^{n}`) when the limits span more than one decade tick, also the minor ticks of 2, 3, 4 and 6 (`2×10^{−1}`) when they span more than 0.4 decades, all of them below; the grid lines are at the major ticks. Given `ticks` are the major ones, the minor ticks drawn beside them. Before 0.33.0 a log axis took the ladders 1-2-5 and 1-3 for a span of few decades (`0.1, 0.2, 0.5`) and had no minor ticks. Before 0.31.0 a tick label was one line, `\n` written as it is, and a large or small value was written `1.2e+5`, from 1e5 and below 1e-3.

### Element

An element is one flat object with the common fields and the fields of its `kind`; a field of another kind is refused.

| common field | type | default | meaning |
|---|---|---|---|
| `id` | text | required | unique within the figure |
| `kind` | text | required | one of the kinds below |
| `label` | text | none | label in the legend |
| `legend` | text | none | the legend group; absent, the id when a label is set, else the element is in no legend. Elements of one group have one legend entry, which a page toggles them by |
| `axis` | `y` or `y2` | `y` | the y axis it is drawn against; `y2` needs the `y2` axis of the plot |
| `style` | Style | see below | the style |

| kind | fields | meaning |
|---|---|---|
| `curve` | `x`, `y`, optional `xerr`, `yerr`, `error_cap`, `error_width` | a line and/or markers with error bars, all of the length of `x`; `error_cap` (0, not negative, since 0.30.0) is the length in pixels of a cap across each end of an error bar (matplotlib's `capsize`), 0 for none, so an error shorter than the radius of its marker still shows its caps; `error_width` (1, not negative, since 0.31.0) the width of the lines of the error bars and their caps in pixels (matplotlib's `elinewidth`) |
| `band` | `x`, `low`, `high`, optional `median` | a band between two curves with a median line |
| `area` | `x`, `from`, `to` | a filled area between two curves |
| `bar` | `x`, `height`, optional `err`, `horizontal` (false), `width`, `error_cap` | bars at positions or category indices; `width` in axis units, positive; a bar is outlined by the `line` of its style when the style gives one and its type is not `none` (since 0.28.0), and has no outline without it; `error_cap` as of a curve (since 0.30.0), `error_width` as of a curve (since 0.31.0) |
| `box` | `positions`, `stats`, optional `width` | box plots, `stats` one per position: `low_whisker <= q1 <= median <= q3 <= high_whisker` and `outliers` (default empty); `width` (since 0.31.0, positive) the width of a box in units of the x axis, matplotlib's `widths`, on a category axis a share of the slot of a category (two boxes side by side at ±0.2 with `width` 0.35); absent, half of the spacing of the positions |
| `violin` | `positions`, `densities` | violins, one `Density` (`values` and `density` of one length) per position |
| `histogram` | `edges`, `counts` | `edges` has one more value than `counts` and is not empty |
| `heatmap` | `rows`, `columns`, `values`, `scale`, optional `text`, `x_edges`, `y_edges` | `values` has a row per name of `rows` of the length of `columns`; `scale` is the ColorScale of the cells; `text` is the text of the cells in the same shape. Without edges the cells lie on the categories of the axes (a linear axis without categories becomes the category axis of the names of the first heatmap of its plot). `x_edges` (since 0.30.0) are the edges of the columns on a numeric x axis instead, one more than columns, at least 2, finite and strictly increasing or decreasing: column `j` spans `x_edges[j]..x_edges[j + 1]`, the axis stays linear or log and sticks to the outer edges as matplotlib's `pcolormesh`, so the heatmap shares its axis with another plot, e.g. the time of a space-time heatmap above a curve of the same time; `columns` may then be absent (empty), and names given must be as many as the columns of the edges. `y_edges` likewise for the rows on y. A picked cell along an axis of edges reads out its span (`x_low`, `x_high`, `y_low`, `y_high`) |
| `text` | `x`, `y`, `text`, `anchor` (`start`, `middle`, `end`; `start`), `size`, `weight`, `family` | a text at a position; `size` its font size, positive, the `tick` size of the fonts when absent, and `weight` (`normal` or `bold`; `normal`) its weight (both since 0.30.0), so a schematic draws bold headings of a larger size beside plain text; `family` (`sans` or `monospace`; `sans`, since 0.31.1) its font: a monospace text is measured 0.6 of its size per character and drawn with `font-family="ui-monospace, SFMono-Regular, Menlo, Consolas, Noto Sans Mono, monospace"` in the SVG and with the embedded Noto Sans Mono in the PNG, so the columns of a table of lines of text padded with spaces align (a dot operator U+22C5 in it is written as the middle dot U+00B7, which every monospace font has); the python function `text_width(text, size, bold, family="monospace")` gives the measure |
| `rule` | `orientation` (`horizontal`, `vertical`), `value` | a line across the plot at `value` of the other axis |
| `polygon` | `paths` | filled polygons, each ring of `[x, y]` pairs with at least 3 points |

### Style

`color` (`#rrggbb`; absent, the cycle by element position), `opacity` (1, between 0 and 1), `line` (`type` of `solid`, `dashed`, `dotted`, `dashdot`, `none`, default `solid`, and `width` in pixels, 2, not negative; absent, this default line, which every kind draws but a bar, whose outline is a line the style gives, since 0.28.0), `marker` (`type` of `none`, `circle`, `square`, `triangle`, `diamond`, `cross`, `plus`, `x`, and since 0.31.0 `triangle_down`, `triangle_left`, `triangle_right`, `star`, `x_filled`, `plus_filled`, `pentagon`, `hexagon` and `thin_diamond`, matplotlib's `^`, `D`, `+`, `x`, `v`, `<`, `>`, `*`, `X`, `P`, `p`, `h` and `d`, default `none`; `size` 6; `fill` and `edge` colors, absent the color of the element; `fill` may be `none` for a hollow marker, drawn as its edge alone, since 0.28.0; `edge_width` 1; since 0.31.0 `fill_style`, `full`, `left`, `right`, `bottom` or `top`, matplotlib's `fillstyle`, which half `fill` fills, and `fill_alt`, the fill of the other half, a color or `none`, the default, matplotlib's `markerfacecoloralt`) and `fill` (`color` and `opacity` 1).

The markers have the shapes of matplotlib for a `size` (since 0.31.0): the circle as wide as the size, the square of the side of the size, the triangles as wide and as high as the size, the diamond a square of the side of the size on its corner and the thin diamond 0.6 as wide, the star, the pentagon and the hexagon in the circle of the size, the filled x and plus in the square of the size, and the plus and the x lines across that square. Before 0.31.0 the square had the area of the circle, the triangle was equilateral, the diamond was the thin one (0.75 as wide as high) and the x reached 0.7 of the half size. A half filled marker draws its two halves, each with the edge, so the edge runs around the marker and between the halves, as matplotlib draws it; the halves of a triangle, a pentagon and a star are split where matplotlib splits them (a tenth of the size off the centre for a triangle). A marker of lines has no fill and ignores the fill style.

### Color scales

A `ColorScale` has `name` (see below) or, since 0.31.0, `stops`, `low` and `high` (`high` above `low`), `center` (the value at the middle of a diverging map, between `low` and `high`), `log` (false; `low` must be positive) and `start` and `end` (0 and 1, since 0.28.0; `0 <= start < end <= 1`): the values from `low` to `high` take the part `start..end` of the color map, and a value beyond an end the color of that end. The lines of a scan which leave out the lightest colors of viridis take `end: 0.85`, and a bar of that scale shows exactly the colors of the lines over exactly the range of the data, with no value no line has.

`stops` (since 0.31.0, refused together with `name`) is a color map of colors at positions, `[[0.0, "#rrggbb"], [0.3, "#rrggbb"], ..., [1.0, "#rrggbb"]]`, matplotlib's `LinearSegmentedColormap.from_list(name, [(0.0, c0), (0.3, c1), ...])`: at least two stops, the first at 0 and the last at 1, the positions not decreasing (two stops at one position are a step), the colors `#rrggbb`. Its 256 colors are interpolated linearly in sRGB between the stops as matplotlib's lookup table and read as a named map, so a value has the color matplotlib gives it: the scans of the renal and hepatic classes use maps of the colors of their classes, and a map without the light centre of `rdbu_r` is its colors at the positions which remain. A scale has a `name` or `stops`; before 0.31.0 `name` was required.

A `ColorBar` has a `scale`, an optional `label` and, since 0.28.0, `ticks` (the values of the ticks, between `low` and `high`; computed from the scale without them), `tick_labels` (one label per tick, which needs `ticks`; the numbers of the ticks without them) and `categorical` (false): a categorical bar (whose `tick_labels` may stand without `ticks`, one label per band, so as many as `high - low + 1`, at the integers from `low` to `high`) draws one band of one color per integer from `low` to `high` (both integers, a linear scale, at most 256 bands), the band of `k` in the color of the value `k`, with its ticks, the integers among the ticks of the scale unless `ticks` gives them, at the middle of their bands; it labels the lines of a scan over named values. Since 0.31.0 a bar also has `marks`, `placement`, `loc`, `length` and `thickness`. `marks` are lines across the bar at values of its scale (matplotlib's `cbar.ax.axhline`, the default of a scan), each `{"value": v, "color": "#rrggbb", "width": w}`, the value between the ends of the scale, the color black and the width 2 pixels by default, drawn over the colors and under the outline. `placement` is `right` (the default, vertical right of the panel, the low end at the bottom), `top` (horizontal above the panels of the row of its plot, centred over them, its ticks, tick labels and label above it, matplotlib's `fig.colorbar(..., location="top")` over the axes of a row; the room above the row grows by it, above the titles of its plots, and one bar of a row goes above it) or `inset` (horizontal inside the panel of its plot at `loc`, its ticks, tick labels and label below it, matplotlib's horizontal colour bar in `ax.inset_axes`; the bar with its labels sits `LEGEND_INSET` from the edges of the panel as a legend there, and a legend at `best` goes around it). `loc` is the location of an inset, a location of a legend but `best`, `upper_right` by default, refused for another placement. `length` is the length of the bar as a share of what it runs along, above 0 and at most 1, matplotlib's `shrink`: of the height of the panel right of it (1), of the span of the panels of the row above them (1; 0.4 for a bar of 40 % of the row) and of the width of the panel inset (0.5), the bar centred on it; `thickness` its thickness in pixels (12). A horizontal bar runs from its low end at the left, its tick labels centred on their ticks, the lines of a tick label of several lines stacked away from the bar.

The first bar of a plot writes its gradient under the id `<plot>-colorbar` as before, a further bar under `<plot>-colorbar-<index>`, and a categorical bar draws its bands as rectangles and needs none.

The names of the colour maps are the sequential and the diverging maps of matplotlib in lower case, each also reversed as `<name>_r` (since 0.30.0; `viridis`, `magma`, `rdbu`, `coolwarm` and `greys` before, which keep their colours), the values of `COLOR_SCALES` in Rust and in python:

| group of matplotlib | names |
|---|---|
| perceptually uniform sequential | `viridis`, `plasma`, `inferno`, `magma`, `cividis` |
| sequential | `greys`, `purples`, `blues`, `greens`, `oranges`, `reds`, `ylorbr`, `ylorrd`, `orrd`, `purd`, `rdpu`, `bupu`, `gnbu`, `pubu`, `ylgnbu`, `pubugn`, `bugn`, `ylgn` |
| sequential (2) | `binary`, `gist_yarg`, `gist_gray`, `gray`, `bone`, `pink`, `spring`, `summer`, `autumn`, `winter`, `cool`, `wistia`, `hot`, `afmhot`, `gist_heat`, `copper` |
| diverging | `piyg`, `prgn`, `brbg`, `puor`, `rdgy`, `rdbu`, `rdylbu`, `rdylgn`, `spectral`, `coolwarm`, `bwr`, `seismic`, `berlin`, `managua`, `vanimo` |

A map is the 256 colours of matplotlib's lookup table of that map (`matplotlib.colormaps[name](i)` for `i` in `0..256`, written by `crates/plot/src/colors_table.py` with the version of matplotlib it names), and `<name>_r` the colours of matplotlib's `<name>_r`, so a value has the colour matplotlib gives it. A name in the case of matplotlib (`Blues`) is refused with the name in lower case as a hint.

The python function `color_scale(name, low, high, values, center=None, log=False, start=0.0, end=1.0, gamma=None)` gives the `#rrggbb` of values on a scale, so a page and a file color a legend swatch alike; `name` is a name of `COLOR_SCALES` or (since 0.31.0) the stops of a map, a list of `(position, "#rrggbb")`; it raises `ValueError` for an unknown name, for stops which are no color map and for a part which is not `0 <= start < end <= 1` (`start` and `end` since 0.28.0) and for a `gamma` which is not positive (since 0.33.0).

The python function `text_width(text, size, bold=False)` gives the width in pixels of a text at the font size `size` px in Inter Regular or Bold (since 0.28.0), exactly the measure the layout of the figures uses, so a caller measures a text as the renderer does and keeps no width table of its own; the width of a text of several lines (`\n`) is the width of its widest line.

### The fields of 0.33.0

Every field is optional and absent the drawing of 0.31.1, but where noted.

- **Error bars of two sides**: `xerr` and `yerr` of a curve and `err` of bars are an array of errors or `{"low": [...], "high": [...]}`, matplotlib's error of shape `(2, n)`: the error below and the one above every value, not negative, each of the length of the values; a gap (`null`) is a side which is not drawn, its half of the bar and its cap left out (a median with quantiles, an SD above the mean alone); a side of 0 has its cap at the value, as matplotlib. A pick answers `yerr_low` and `yerr_high` of such errors.
- **Contours**: an element `contour` of `x` (the coordinates of the columns of a grid, strictly monotonic), `y` (of its rows), `values` (per row, a gap no value), `levels` (strictly increasing), `filled` (false), `scale` (a color scale), `labels` (one per level, an empty one for none) and `label_size` (the `tick` size): lines at the levels in the color of their level (else of the element) in the line of the style, matplotlib's `contour`, with their labels inline as matplotlib's `clabel(inline=True)` places them (a line long enough for a label, its straightest block of a label width of vertices, not nearer than 1.2 label widths to a label before, rotated along the line and never upside down, the line broken 5 pixels beside it); or with `filled` the bands between neighbouring levels in the color of their middle, matplotlib's `contourf` (each band above its lower level up to its upper one, the first including its lower level), drawn without antialiasing so the bands and their cells meet without seams. The crossings are interpolated linearly on the edges of the cells, the lines oriented and started as contourpy's; the axes stick to the ends of the grid.
- **Power scales**: `gamma` of a color scale (positive, not with `log` or `center`), matplotlib's `PowerNorm`: a value at the share `t` takes the color at `t^gamma`; its bar has its colors evenly along it and its ticks at their shares. matplotlib's `CenteredNorm` is a `center` between ends equally far from it. The python function `color_scale` takes `gamma`.
- **Inset plots**: `inset` of a plot, `{"plot": host, "x", "y", "width", "height"}` in fractions of the panel of the host from its lower left corner, matplotlib's `ax.inset_axes`: the plot is drawn there with its own axes, ticks, labels and legend (always inside its panel) over the host, its panel white and its canvas without background; it takes no room of the grid, has the row and column of its host and spans 1, its host comes before it in `plots` and is no inset. A legend of the host at `best` goes around it.
- **Arrows**: an element `arrow` from (`x_from`, `y_from`) to (`x`, `y`), matplotlib's `annotate` and `FancyArrowPatch`: a line in the line of the style with a `head` of `open` (`->`), `filled` (`-|>`, the default) or `none` (`-`), `head_length` and `head_width` (0.4 of the `tick` size each), ending 2 pixels before its ends; or with `body` (a width in pixels) a filled arrow of that body (`arrowprops` of `width`, `headwidth`, `headlength`, three body widths by default) shortened by `shrink` of its length at both ends. An optional `text` at the tail (`size`, `weight`, `anchor`, `baseline`, its baseline by default) starts the arrow at its edge. Its color is the text color without one of the style.
- **Texts**: a text has `rotation` (degrees counterclockwise), `baseline` (`baseline`, `center`, `top`, `bottom`; absent, centred as before), `box` (`fill` white or `none`, `opacity`, `edge` none, `edge_width` 1, `padding` 0.3 font sizes, `style` `square` or `round`, matplotlib's `bbox`) and `loc` (a location of a legend but `best`, without `x` and `y`: the text in a white box with a black edge in that corner, 0.4 font sizes of padding and 0.5 off the edges, matplotlib's `AnchoredText`). A rotated text is anchored by the box of its rotated block, as matplotlib's default `rotation_mode`. Every text of a figure draws `_{...}` as a subscript (0.7 of its size, lowered by 0.15 of it, mathtext's `sub1`) and `^{...}` as a superscript, measured so; a script of a text of another dominant baseline is corrected for the baseline of its smaller size, so it sits where it does in a text of the alphabetic baseline.
- **Marks in fractions and unclipped**: `coordinates` (`data` or `axes`) and `clip` (true) of a curve, a polygon, a text and an arrow: `axes` positions are fractions of the panel (matplotlib's `transform=ax.transAxes`) and put nothing on the axes; an element with `clip: false` is drawn after the panel over the whole figure in a group of the class `sbmlsim-unclipped` (the slashes of a broken axis, a text at the edge of a panel).
- **Frames and colored tick labels**: `frame` of a plot (`color`, the color of the axis lines, and `width`, 1) draws all four sides of the panel instead of the axis lines, matplotlib's spines; `tick_label_color` of an axis is one color for its tick labels or one per tick of `ticks` or per category.
- **Stacks**: `base` of bars and of a histogram, one per bar or bin, matplotlib's `bottom`: the bar runs from its base to its base and height, its error bar at its end, the axis sticking to the bases. A histogram draws every run of neighbouring bins as one outline, without seams between its bars, and the edges of its bars only with a line of its style (before, a white edge of 0.8 pixels between neighbouring bins).
- **Boxes**: `median_color` and `whisker_opacity` of a box.
- **Markers**: `octagon`, `tri_down`, `tri_up`, `tri_left` and `tri_right`, matplotlib's `8`, `1`, `2`, `3`, `4`.
- **Thin rows**: Charton keeps the panel the layout gives it however small (its least panel of 100 pixels overlapped the plots of a grid of rows under about 100 pixels and cut off the x label of its last row), and an axis whose tick labels and tick marks are hidden takes no room for them.
- **Fonts**: `plot_title_weight` (the weight of the plot titles, `title_weight` without), `colorbar_label` (the size of the labels of the color bars, `label` without) and `legend_family` (`sans` or `monospace`, of every legend).
- **Legends**: `legends` of a plot, further legends in its panel, each `{"groups": [...], "title", "loc", "anchor", "columns", "family"}`, matplotlib's second legend (`ax.add_artist`): its groups leave the legend of the plot, at its `loc` (`best` by default, avoiding the legends of the panel placed before it) or `anchor`. `legend_anchor` of a plot, a point in fractions of the panel, anchors its legend there by its `legend_loc` (`upper_right` without), half a legend font size off the point, matplotlib's `bbox_to_anchor` with `loc`; it may reach out of the panel and takes no room. `legend_tuples` of a plot names the groups whose swatch draws the markers of all their elements side by side, matplotlib's `HandlerTuple`. The swatch of a curve with error bars draws its caps (`error_cap`) and their width. `legends` of the figure, each `{"groups", "position", "title", "columns", "family"}` with `position` `right` (right of the grid at its top, right of the row legends, matplotlib's `"outside right upper"`) or `bottom` (below the grid, centred, matplotlib's `"outside lower center"`), take their groups out of the legends of all plots; two of a side are stacked and the figure grows by them.
- **Color bars**: `extend` (`neither`, `min`, `max`, `both`, matplotlib's `extend`): triangles in the colors of the ends, 0.05 of the bar long, the bar shorter by them; `rows` (a bar right of its panel spans that many rows of the grid from the row of its plot, matplotlib's `fig.colorbar(..., ax=axes[:, -1])`); the placements `strip` (the colors along the top of the panel, as wide as it and `thickness` high, 0.03 of the panel by default, without ticks, labels or outline; a legend at `best` avoids it) and `panel` (vertical in place of the panel of its plot, which draws nothing else, as wide as the panel without a `thickness`, its ticks and label right of it: a figure of a color bar alone).
- **Top axis**: `x2` of a plot, a second x axis on top of the panel with ticks and labels of its own (matplotlib's `twiny`), linear or log, its limits its `min` and `max` or else those of the x axis; no element is drawn against it, the title of the plot goes above it.
- **Row titles**: `row_titles` of the figure, one per row (an empty one for none), centred above the panels of each row in the size and weight of the plot titles, the room above the row grown by 1.5 plot title sizes.
- **Heatmaps** are drawn without antialiasing (`shape-rendering="crispEdges"` on their group), as matplotlib's `pcolormesh`, so their cells meet without seams.

### The fields of 0.34.0

Every field is optional and absent the drawing of 0.33.0, but where noted.

- **Legends of the figure above the grid and under a column**: `position` `top` of a legend of the figure (`legends`), matplotlib's `fig.legend(loc="outside upper center")`: below the title of the figure and centred on it, the grid moved down by it and the figure grown by it, its columns as many as fit into the width of the figure without `columns`; `column` of a legend `top` or `bottom` (a column of the grid) centres it on the panels of that column, of the plots which span it alone, else of every plot over it (matplotlib's `bbox_to_anchor` of a figure legend at the middle of a column). The legends of different columns sit side by side, each column stacking its own, the legends of the whole figure above them (`top`) or below them (`bottom`).
- **Room for a legend anchored beyond its panel**: a `legend_anchor` of a plot or an `anchor` of a further legend below 0 or above 1 reserves the room of the part of the legend beyond the panel on that side (the plots right of it move over, as in matplotlib's constrained layout), and the figure grows when the legend would still reach out of it; an anchor inside takes no room as before.
- **Fonts and frames of every legend**: `size` (the font size of its labels and title, positive), `weight` (of its labels; the title keeps `legend_weight`, as matplotlib's title of a legend) and `frame` (true) of a legend of the figure and of a further legend of a plot (`family` since 0.33.0), and `legend_size`, `legend_weight`, `legend_family` and `legend_frame` of a plot for its own legend (a row legend takes the first of its row, no frame when a plot of its row has none). A legend without a frame has no background either, matplotlib's `frameon=False`. The swatches keep their size.
- **Boxes**: `median_opacity` of a box (between 0 and 1, the opacity of the style without), seaborn's opaque median in a translucent box.
- **Arrows**: the opacity of an arrow with a `body` is the opacity of the whole arrow (`opacity` of its path), as matplotlib draws the alpha of its patch; its fill and its edge had it apart before and the edge over the fill was darker. The goldens change.
- **Contours at the ends of the data**: a corner of a cell is above a level when its value is greater than the level (contourpy's `z > level`, at or above before), so a level at the least value of the data draws the edge of the region of that value (the zero lines of a map of values not below 0) and a level at the largest value draws nothing, as matplotlib. Checked against the lines of matplotlib 3.11. The goldens change where a value lies exactly at a level.
- **Edges of bars and bins**: `edge_color` (`#rrggbb`) and `edge_width` (pixels, not negative) of bars and of a histogram, matplotlib's `edgecolor` and `linewidth`: with either every bar or bin is outlined, in the color of the element without `edge_color` and the width of the line of the style (1 without one) without `edge_width`, e.g. white edges between stacked bars or neighbouring bins. The swatch of bars and of a histogram draws their edge, so a white bin with a black edge shows in its legend.
- **Fonts of the color bars**: `fonts.colorbar_label_weight` (the weight of the labels of the color bars, `label_weight` without) and `fonts.colorbar_tick` (the size of their tick labels, positive, `tick_y` and else `tick` without).
- **Drawing order**: `z` of an element (a finite number, matplotlib's `zorder`): the elements of a plot are drawn by increasing `z`, those of the left and the right y axis together (all elements in the plot group of the panel), on a tie the elements of the left axis before those of the right one, each in their order; absent 0, and -1 for an image, which goes under the other elements as matplotlib's image (`zorder` 0) under its patches and lines. A plot whose elements need no other order than before draws the same bytes.
- **Color bars**: `tick_side` (`top` or `bottom`) of a horizontal bar (`top`, `bottom`, `inset`), the side of its ticks, tick labels and label (matplotlib's `ticklocation`; above a bar above a row and below the others without), and the placement `bottom`: a horizontal bar below the panels of the row of its plot and their axes, centred under them (matplotlib's `fig.colorbar(..., location="bottom")` under the axes of a row, below the grid for the last row), the row grown by it; one bar below a row.
- **Fonts of an axis**: `tick_weight` of an axis (the weight of its tick labels, the `tick_weight` of the fonts without) and `label_size` (the size of its title, positive, the `label` size of the fonts without), e.g. an inset with titles of 11.
- **Images**: an element `image` of `png` (the base64 of a PNG, the standard alphabet with its padding, at most 64 Mpx), `extent` (`[x0, x1, y0, y1]`, finite, each side of two different values), `origin` (`upper`, the first row at `y1`, or `lower`, at `y0`) and `interpolation` (`auto`, `smooth` or `nearest`), matplotlib's `imshow(image, extent=..., origin=...)`: the image stretched over its extent in the values of the axes (an extent running the other way mirrors it), the axes sticking to its edges, at the `opacity` of its style, under the other elements of its plot (see `z`), e.g. the scan of a figure of a study under the data digitized from it. `auto` draws an image enlarged three times or more with square pixels and smooths it else, as matplotlib's default; `nearest` always with square pixels (`image-rendering: pixelated`, with `optimizeSpeed` as its attribute). The SVG embeds the PNG as a `data:image/png;base64` URI and the PNG of the figure draws it; the `aspect` of the plot keeps the proportions of the image (matplotlib's default `aspect="equal"`). An image takes no color of the cycle, has no swatch and no pick, and the spec is as large as its PNG. The python function `image_png(pixels)` gives the `png` of an RGB or RGBA array of shape `(rows, columns, 3 or 4)`, its first row the top, of integers from 0 to 255 or of numbers from 0 to 1, as matplotlib reads an image.

### Errors

A spec the renderer refuses is reported as `path: what is wrong`, the path in the notation of the spec, e.g. `plots[0].elements[1].y: 3 values, x has 4`, `plots[0].x.categories: a category axis needs categories`, `plots[0].elements[0].scale.name: unknown color scale ...` or `contract: expected figure/1, got ...`. Besides the shapes above the checks are: the plot ids and the element ids are unique within the figure (the element ids of all plots together), a plot lies in the grid, an element on `y2` needs the axis, colors are `#rrggbb` (a marker fill may also be `none`), a key of `legend_swatches` is a legend group of its plot (`plots[0].legend_swatches.<group>: ... is no legend group of the plot`), opacities lie between 0 and 1, widths and sizes are not negative, a plot has `colorbar` or `colorbars` and not both and a color bar has ticks between the ends of its scale and as many tick labels as ticks. Since 0.31.0 also: a scale has a `name` or `stops` and not both, stops are a color map as above (`plots[0].colorbar.scale.stops[1]: the last stop must be at 1`), the marks of a bar lie between the ends of its scale, the `loc` of a bar is that of an inset and not `best`, its `length` is above 0 and at most 1 and its `thickness` positive, one bar of a row goes above it (`plots[1].colorbar.placement: row 0 has a bar above it already, plots[0].colorbar.placement`), an axis with `ticks` is no category axis, its ticks are finite (positive on a log axis) and as many as its `tick_labels`, which need them, a `fill_alt` is a color or `none`, an `error_width` is not negative, the `width` of a box positive, `fonts.plot_title` positive and `legend.columns` at least 1. Since 0.30.0 also: an `aspect` is `equal` or a positive number, an `error_cap` is not negative, the `size` of a text is positive and its `weight` `normal` or `bold`, and the edges of a heatmap are as described above, the counts of `values` and `text` taken from the edges along an axis with edges (`plots[0].elements[0].values[1]: 2 values, x_edges gives 3`).

## Semantic classes of the SVG

The SVG carries plain light-theme values as presentation attributes and nothing of it is a `var()`, which renderers outside of a browser (cairosvg, Inkscape) do not read. A page restyles it with a stylesheet on the classes and attributes below, which are its styling contract: the list is exhaustive, `figure/1` writes no other class or `data-` attribute, and one of them is not renamed or removed within the contract.

| class or attribute | on | what it is |
|---|---|---|
| `sbmlsim-figure` | root `<svg>` | the figure |
| `sbmlsim-figure-title` | `<g>` | the title of the figure |
| `sbmlsim-plot`, `data-plot="<id>"` | nested `<svg>` | a plot, its canvas |
| `charton-background` | `<g>` | the background of a panel |
| `charton-title` | `<g>` | the title of a plot |
| `charton-plot` | `<g>` | the panel the data is drawn in, clipped to it |
| `charton-grid` | `<g>` | the grid lines of a panel |
| `charton-axis` with `charton-axis-x`, `charton-axis-y` or `charton-axis-y2`, `data-axis="x"`, `"y"` or `"y2"` | `<g>` | an axis |
| `charton-axis-line`, `charton-tick`, `charton-tick-label`, `charton-axis-title` | `<g>` | the line, the ticks, the tick labels and the title of an axis; an axis with `line_visible` false has no `charton-axis-line` |
| `sbmlsim-element sbmlsim-<kind>`, `data-element="<id>"`, `data-legend="<group>"` | `<g>` | an element of the kind `<kind>` (`curve`, `band`, `area`, `bar`, `box`, `violin`, `histogram`, `heatmap`, `text`, `rule`, `polygon`); `data-legend` when it is in a legend group |
| `charton-mark` with `charton-mark-path`, `charton-mark-polygon`, `charton-mark-point` or `charton-mark-text` | `<g>` | the marks of an element: lines, filled shapes, markers and texts |
| `sbmlsim-legend`, `data-plot="<id>"` or `data-row="<n>"` | `<g>` | a legend, of a plot or of a row of the grid |
| `sbmlsim-legend-frame` | `<rect>` | the frame of a legend |
| `sbmlsim-legend-entry`, `data-legend="<group>"` | `<g>` | an entry of a legend, of a `sbmlsim-legend-swatch` and a `sbmlsim-legend-label` |
| `sbmlsim-colorbar` | `<g>` | the color bar of a plot, with `sbmlsim-colorbar-ticks`, `sbmlsim-colorbar-tick-labels` and `sbmlsim-colorbar-label`, and since 0.31.0 `sbmlsim-colorbar-marks`; a bar above a row has also the class `sbmlsim-colorbar-top` and is drawn by the figure after the plots, a bar below a row `sbmlsim-colorbar-bottom` (since 0.34.0), an inset `sbmlsim-colorbar-inset` |
| `sbmlsim-contour-line`, `sbmlsim-contour-band`, `sbmlsim-contour-label` | `<path>`, `<g>` | the lines, the filled bands and the labels of a contour element (since 0.33.0) |
| `sbmlsim-arrow`, `sbmlsim-arrow-head` | `<path>` | the line or the body and the head of an arrow (since 0.33.0) |
| `sbmlsim-image-raster` | `<image>` | the raster of an image element in its group `sbmlsim-image` (since 0.34.0) |
| `sbmlsim-text-box` | `<rect>` | the box of a text (since 0.33.0) |
| `sbmlsim-unclipped` | `<g>` | the part of an element drawn after the panel without its clip, with the `data-element` of its element (since 0.33.0) |
| `sbmlsim-frame`, `sbmlsim-inset-panel` | `<rect>` | the frame of a panel and the white panel of an inset (since 0.33.0) |
| `sbmlsim-row-title` | `<g>` | the title of a row of the grid (since 0.33.0) |
| `sbmlsim-axis-multiplier` | `<g>` | the multiplier `×10^n` of the tick labels of an axis, inside the group of its axis (since 0.31.0) |
| `sbmlsim-legend-title` | `<text>` | the title of a legend (since 0.31.0); the legend below the grid is a `sbmlsim-legend` with `data-figure="legend"` |
| `sbmlsim-error` | `<text>` | the message of a plot the renderer could not draw, in place of its marks |

The gradient of a color bar is a `<linearGradient>` with the id `<plot>-colorbar` for the first bar of a plot and `<plot>-colorbar-<k>` for the bar `k` (counting from 0, so `-colorbar-1` is the second) of `colorbars`; a categorical bar has no gradient: its bands are one `<rect>` per integer, from the bottom up, inside its `sbmlsim-colorbar` group.

A page toggles the elements of a legend group (the groups with the same `data-legend`) by clicking the entry of that `data-legend`; `attachFigure` and `hydrateFigures` (see [JavaScript](javascript.md#interactive-figures)) do so, and add the following to the SVG of the figure, which is not part of the SVG `figure/1` writes (since 0.28.0):

| class or attribute | on | what it is |
|---|---|---|
| `sbmlsim-hidden` | `[data-element]` | the element is in a hidden legend group (`display: none`) |
| `sbmlsim-selected` | `[data-element]` | the element is in the selected legend group |
| `sbmlsim-dimmed` | `[data-element]` | another group is selected, so the element is drawn at a quarter of its opacity |
| `role="button"`, `tabindex="0"`, `aria-pressed` | `.sbmlsim-legend-entry[data-legend]` | the entry is a toggle button; `aria-pressed="true"` means its group is shown, `"false"` that it is hidden |
| `sbmlsim-tooltip` | `<div>` in the container | the read-out of the point under the pointer, `role="status"` |
| `sbmlsim-figure-style` | `<style>` (an id) | the stylesheet of these classes, injected once per document |
| `data-figure="<key>"`, `data-figure-spec="<key>"`, `data-hydrated` | `<figure>`, `<script type="application/json">`, `<figure>` | the markup of a figure of a static report: the container of the inline SVG, its `figure/1` spec as a child, and the mark of a figure which was attached |

The keyboard on a legend entry: Enter or Space toggles its group, Shift+Enter selects it, and Escape clears the selection (with the focus or the pointer last in the figure). A click on a data point or on the line of a curve selects its group, a click on it again clears it. The script of a spec must be a direct child of its `figure[data-figure]`, and its text is the JSON of the spec with `</` written as `<\/`.

Coordinates have at most two decimals, and a primitive with a coordinate which is no finite number is not written.

## Python

```python
import json
import sbmlsim_core

spec = json.dumps(figure, allow_nan=False)  # a figure/1 dict, a gap is None
svg = sbmlsim_core.render_svg(spec)  # str
png = sbmlsim_core.render_png(spec, dpi=144.0)  # bytes, 72 dpi is the drawn size
geometry = sbmlsim_core.figure_geometry(spec)  # width, height, plots, legend_entries
hit = sbmlsim_core.pick(spec, 120.0, 80.0, 8.0)  # dict or None
colors = sbmlsim_core.color_scale("viridis", 0.0, 1.0, [0.0, 0.5, 1.0])
names = sbmlsim_core.COLOR_SCALES  # viridis, viridis_r, ..., blues, blues_r
width = sbmlsim_core.text_width("Smith2001", 13.0, bold=True)  # 68.08 px
png_text = sbmlsim_core.image_png(pixels)  # the `png` of an image element, since 0.34.0
```

`render_svg`, `render_png`, `figure_geometry` and `pick` take the JSON text of the spec and raise a `ValueError` with the path and message of an invalid spec; `render_png` raises a `RuntimeError` for a figure which cannot be rasterized, e.g. an image of more than 64 Mpx. `geometry["width"]` and `geometry["height"]` are the size the figure is drawn at (the size of the SVG, see above), `geometry["plots"]` has per plot its `id`, its panel `rect` (`x`, `y`, `width`, `height` in pixels) and the `x`, `y` and `y2` scales (`kind`, `low`, `high`, `reverse`); `legend_entries` has per entry its `legend` group, its `owner` (a plot id, `row:<n>`, or since 0.31.0 `figure` for the legend below the grid) and its `rect`. A hit has the `plot`, the `element`, the `legend` group, the `index` of the point in the data of the element (the cell of a heatmap in row order), the `values` at the point in data units (`x`, `y`, `xerr`, `yerr` of a curve, `low`, `high` and `median` of a band, the five numbers of a box, `value` of a heatmap cell, with `x_low`, `x_high`, `y_low` and `y_high` along the edges of a heatmap over edges, and so on) and the `label` (of a heatmap cell its `row / column`, the names it has). A pointer is measured against the points of the elements; since 0.28.0 a curve which draws its line is also picked along the segments of the line, between two consecutive points it draws, and a segment answers with its nearer end, so a pointer on the line between sparse points picks the curve. `sbmlsim_core.plot_wasm()` gives the bytes of `plot.wasm`, so a page which inlines everything needs no file.

A spec is built in python with the chart builder `sbmlsim_core.chart` (`Chart`, `ChartPlot`, `ChartAxis`), which takes numbers, quantities and data arrays and draws with these functions, see [Charts](chart.md).

## JavaScript

`js/plot.mjs` (the export `sbmlsim-core/plot`) loads `plot.wasm`; its documentation is in the [JavaScript page](javascript.md#draw-a-figure) and in `js/README.md`:

```js
import { loadPlot } from "sbmlsim-core/plot";

const plot = await loadPlot(new URL("./plot.wasm", import.meta.url)); // or its bytes or a WebAssembly.Module
const svg = plot.render(spec);
const figure = plot.prepare(spec);          // parsed and laid out once
canvas.onpointermove = (e) => show(figure.pick(e.offsetX, e.offsetY));
figure.dispose();
```

`loadPlot` takes a URL, a path (Node), the bytes of `plot.wasm` or the `WebAssembly.Module` compiled already, which a page that loads several renderers compiles once. A prepared figure answers `svg()`, `geometry()` and `pick(x, y, radius)` without parsing or planning again. A refused spec raises a `PlotError` with `path: what is wrong` and the `Plot` stays usable; a panic or a trap breaks the instance, every later call throws, and `loadPlot` gives a new one.

## The module and its ABI

`plot.wasm` is built for `wasm32-wasip1` and exports a C ABI like `compiler.wasm` (`allocate`, `render`, `geometry`, `pick`, `deallocate`, and `prepare`, `render_prepared`, `geometry_prepared`, `pick_prepared`, `release` for prepared figures). The precise contract, the five WASI functions it imports and why the target is `wasm32-wasip1` are in the documentation of the crate `crates/plot-wasm` (`src/lib.rs`).

## PNG

PNG is drawn natively only, behind the feature `png` of the crate (the python package enables it, the wasm module does not): the SVG is rasterized with resvg at `dpi / 72` times the drawn size of the figure (the size of the spec grown by its legends, the `width` and `height` of the geometry), with the font Inter (OFL-1.1) embedded in the binary, so a PNG does not depend on the fonts of the machine. Since 0.31.1 the binary also embeds subsets of Noto Sans Mono (Regular and Bold, the family `monospace`) and of Noto Sans Math (OFL-1.1), which draws the arrows and the mathematical operators Inter has not, such as the dot operator U+22C5 of a unit (`mg⋅l⁻¹`), as the fallback of a text of Inter; the layout measures them with the advances of that font, which the tables of `text.rs` carry next to the signs of units Inter has (`·`, `×`, `−`, `µ`, `°`, `±`, `≤`, `≥` and the superscript and subscript digits). Before 0.31.1 a character outside of the tables was measured as the digit zero and the PNG drew no glyph for the operators Inter has not. An image is at most 64 Mpx (`64 * 1024 * 1024` pixels, 256 MiB of pixels in memory): a larger one, of a large figure at a high dpi, is refused with a `RuntimeError` naming its size. The notices of the fonts and of the crates are in `THIRD_PARTY.md`.

## Performance

Measured on the benchmark figure of the crate:

| operation | time |
|---|---|
| native SVG render | about 1 to 1.5 ms |
| `plot.wasm` render in Node, JSON included | about 5 to 10 ms |
| pick on a prepared figure | about 0.1 ms |
| PNG at 144 dpi | about 36 ms |

`plot.wasm` is 1.48 MB raw and 462 kB gzipped (`gzip -9`), of which the 60 tables of the colour maps are 61 kB raw.
