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

The contract id is `figure/1` (`FIGURE_CONTRACT` in python, `sbmlsim_plot::FIGURE_CONTRACT` in Rust). Every object refuses unknown fields, every name is snake case, and a field with a default may be absent. In every numeric data array JSON `null` is a gap, a value which is not there: it breaks a line and leaves out a marker or a bar, and a value the renderer cannot draw is written back as `null`. `NaN` and `Infinity` are no JSON: the `json.dumps` of python writes them for a `float("nan")` unless it is given `allow_nan=False`, and the renderer refuses them with the path of the value and a message saying that a gap is written as `null` (in python `None`). A color is the text `#rrggbb`.

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

`Fonts` has `title` (16), `label` (13), `tick` (11) and `legend` (11), in points, each positive. Since 0.28.0 it also has `tick_y` (the size of the tick labels of the y axes and the color bars, positive; absent, `tick`, which then is the size of the x tick labels only) and the weights `title_weight` (`bold`, the figure title and the plot titles), `label_weight` (`normal`, the axis titles and the color bar labels), `tick_weight` (`normal`, every tick label) and `legend_weight` (`normal`, the legend labels), each `normal` or `bold`; a text of the weight `bold` is measured with the advances of Inter Bold and drawn with `font-weight="bold"`, so the room of bold tick labels and legend labels is their bold width. `Grid` has `rows` and `columns` (at least 1 and at most 64) and `wspace` and `hspace` (0.3), the space between plots as a fraction of a plot, not negative. `LegendSettings` has `scope` (`plot`, a legend per plot, or `row`, one legend per row of the grid) and `position` (`inside` or `outside`, right of the plot).

A legend `inside` goes into the corner of the panel of its plot (of a row: of its rightmost plot with a legend) whose frame covers the least ink of the data, the upper right, upper left, lower right and lower left corner in this order on a tie. The ink is everything the data draws: every line (sampled at the pixel resolution of the panel, with its width), every marker and error bar, every bar, box, histogram bin and heatmap, and the interiors of bands, areas, violins and polygons, rasterized into a grid of 4 pixel cells over the panel. Since 0.28.0 a legend whose best corner still covers more than `INSIDE_LIMIT` (0.05) of its frame with ink goes outside, right of its plot (of its row), as a legend which does not fit into its panel does, so a legend is never drawn over a crowded panel; before 0.28.0 the corners were scored by the lines and points of the data alone, and bars, error bars, bands, areas and heatmaps did not count.

Since 0.28.0 every text of the spec, a figure or plot title, an axis label with its unit, a legend label, a text element and a color bar label, may hold `\n`, which splits it into lines 1.2 font sizes apart, each a `<tspan>` with its own `x` and `y`; a text is as wide as its widest line, but for a tick label and the text of a heatmap cell, which stay one line: SVG draws them as one text, so a `\n` in them is measured as the space it renders as and reserves the room of that one line. The figure title band grows by the lines after the first, its lines centred in it; the lines of a plot title stack up from the line of a one-line title, which keeps its place above the panel; the lines of an axis title or a color bar label stack outwards from the line next to the axis or the bar (below the x axis, left of the y axis, right of the right axis and of a bar), so the room of the axis grows by them; a legend entry grows by them, its lines centred on the entry and its swatch at its middle, and the legend, its placement and the room of an outside legend in its row take the grown entries; the lines of a text element are centred on its position.

The `width` and `height` of the spec are the size of the figure before its legends grow it: a legend right of a plot or of a row (`outside`, or an `inside` legend which does not fit into its panel or would cover too much of its data, see above, and is placed right of it) widens the figure by its room, and a legend taller than its row heightens it, while the panels keep their size. The SVG has the grown size as its `width`, `height` and `viewBox`, the PNG is the grown size times `dpi / 72`, and the geometry reports it as `width` and `height`, so a page sizes its container from the geometry and not from the spec.

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
| `colorbar` | ColorBar | none | a color bar |
| `colorbars` | list of ColorBar | none | several color bars, stacked right of the panel in their order, each with its own label; refused together with `colorbar` (since 0.28.0) |
| `declared_legends` | list of list of text | none | the legends the plot may show in some state of a page, each a list of entry labels: the legend takes the room of the largest of them, so the plot and the figure keep their size when entries come and go, and a label without an element is not drawn; absent, the legend takes the room of the entries drawn |
| `legend_swatches` | map of text to Style | none | the swatch of a legend group by group, drawn with this style instead of the style of the element of its entry (or of the neutral color of a group of elements of different colors), as the kind of that element; a style without `color` takes the color of the entry; every key must be a legend group of the plot; e.g. the lines of a scan in one group shown with the middle color of their map (since 0.28.0) |
| `legend_order` | list of text | none | the order of the legend entries as legend groups: the listed groups first in this order, the others after them in the order their elements first appear; a listed group without elements is ignored, a group listed twice is refused; it lets a writer draw elements under others and still list them last |
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
| `tick_rotation` | number | 0 | rotation of the tick labels in degrees |

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
| `curve` | `x`, `y`, optional `xerr`, `yerr` | a line and/or markers with error bars, all of the length of `x` |
| `band` | `x`, `low`, `high`, optional `median` | a band between two curves with a median line |
| `area` | `x`, `from`, `to` | a filled area between two curves |
| `bar` | `x`, `height`, optional `err`, `horizontal` (false), `width` | bars at positions or category indices; `width` in axis units, positive; a bar is outlined by the `line` of its style when the style gives one and its type is not `none` (since 0.28.0), and has no outline without it |
| `box` | `positions`, `stats` | box plots, `stats` one per position: `low_whisker <= q1 <= median <= q3 <= high_whisker` and `outliers` (default empty) |
| `violin` | `positions`, `densities` | violins, one `Density` (`values` and `density` of one length) per position |
| `histogram` | `edges`, `counts` | `edges` has one more value than `counts` and is not empty |
| `heatmap` | `rows`, `columns`, `values`, `scale`, optional `text` | `values` has a row per name of `rows` of the length of `columns`; `scale` is the ColorScale of the cells; `text` is the text of the cells in the same shape |
| `text` | `x`, `y`, `text`, `anchor` (`start`, `middle`, `end`; `start`) | a text at a position |
| `rule` | `orientation` (`horizontal`, `vertical`), `value` | a line across the plot at `value` of the other axis |
| `polygon` | `paths` | filled polygons, each ring of `[x, y]` pairs with at least 3 points |

### Style

`color` (`#rrggbb`; absent, the cycle by element position), `opacity` (1, between 0 and 1), `line` (`type` of `solid`, `dashed`, `dotted`, `dashdot`, `none`, default `solid`, and `width` in pixels, 2, not negative; absent, this default line, which every kind draws but a bar, whose outline is a line the style gives, since 0.28.0), `marker` (`type` of `none`, `circle`, `square`, `triangle`, `diamond`, `cross`, `plus`, `x`, default `none`; `size` 6; `fill` and `edge` colors, absent the color of the element; `fill` may be `none` for a hollow marker, drawn as its edge alone, since 0.28.0; `edge_width` 1) and `fill` (`color` and `opacity` 1).

### Color scales

A `ColorScale` has `name` (`viridis`, `magma`, `rdbu`, `coolwarm`, `greys`), `low` and `high` (`high` above `low`), `center` (the value at the middle of a diverging map, between `low` and `high`), `log` (false; `low` must be positive) and `start` and `end` (0 and 1, since 0.28.0; `0 <= start < end <= 1`): the values from `low` to `high` take the part `start..end` of the color map, and a value beyond an end the color of that end. The lines of a scan which leave out the lightest colors of viridis take `end: 0.85`, and a bar of that scale shows exactly the colors of the lines over exactly the range of the data, with no value no line has.

A `ColorBar` has a `scale`, an optional `label` and, since 0.28.0, `ticks` (the values of the ticks, between `low` and `high`; computed from the scale without them), `tick_labels` (one label per tick, which needs `ticks`; the numbers of the ticks without them) and `categorical` (false): a categorical bar (whose `tick_labels` may stand without `ticks`, one label per band, so as many as `high - low + 1`, at the integers from `low` to `high`) draws one band of one color per integer from `low` to `high` (both integers, a linear scale, at most 256 bands), the band of `k` in the color of the value `k`, with its ticks, the integers among the ticks of the scale unless `ticks` gives them, at the middle of their bands; it labels the lines of a scan over named values. The first bar of a plot writes its gradient under the id `<plot>-colorbar` as before, a further bar under `<plot>-colorbar-<index>`, and a categorical bar draws its bands as rectangles and needs none.

The python function `color_scale(name, low, high, values, center=None, log=False, start=0.0, end=1.0)` gives the `#rrggbb` of values on a scale, so a page and a file color a legend swatch alike; it raises `ValueError` for a part which is not `0 <= start < end <= 1` (`start` and `end` since 0.28.0).

### Errors

A spec the renderer refuses is reported as `path: what is wrong`, the path in the notation of the spec, e.g. `plots[0].elements[1].y: 3 values, x has 4`, `plots[0].x.categories: a category axis needs categories`, `plots[0].elements[0].scale.name: unknown color scale ...` or `contract: expected figure/1, got ...`. Besides the shapes above the checks are: the plot ids and the element ids are unique within the figure (the element ids of all plots together), a plot lies in the grid, an element on `y2` needs the axis, colors are `#rrggbb` (a marker fill may also be `none`), a key of `legend_swatches` is a legend group of its plot (`plots[0].legend_swatches.<group>: ... is no legend group of the plot`), opacities lie between 0 and 1, widths and sizes are not negative, a plot has `colorbar` or `colorbars` and not both and a color bar has ticks between the ends of its scale and as many tick labels as ticks.

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
| `charton-axis-line`, `charton-tick`, `charton-tick-label`, `charton-axis-title` | `<g>` | the line, the ticks, the tick labels and the title of an axis |
| `sbmlsim-element sbmlsim-<kind>`, `data-element="<id>"`, `data-legend="<group>"` | `<g>` | an element of the kind `<kind>` (`curve`, `band`, `area`, `bar`, `box`, `violin`, `histogram`, `heatmap`, `text`, `rule`, `polygon`); `data-legend` when it is in a legend group |
| `charton-mark` with `charton-mark-path`, `charton-mark-polygon`, `charton-mark-point` or `charton-mark-text` | `<g>` | the marks of an element: lines, filled shapes, markers and texts |
| `sbmlsim-legend`, `data-plot="<id>"` or `data-row="<n>"` | `<g>` | a legend, of a plot or of a row of the grid |
| `sbmlsim-legend-frame` | `<rect>` | the frame of a legend |
| `sbmlsim-legend-entry`, `data-legend="<group>"` | `<g>` | an entry of a legend, of a `sbmlsim-legend-swatch` and a `sbmlsim-legend-label` |
| `sbmlsim-colorbar` | `<g>` | the color bar of a plot, with `sbmlsim-colorbar-ticks`, `sbmlsim-colorbar-tick-labels` and `sbmlsim-colorbar-label` |
| `sbmlsim-error` | `<text>` | the message of a plot the renderer could not draw, in place of its marks |

The gradient of a color bar is a `<linearGradient>` with the id `<plot>-colorbar` for the first bar of a plot and `<plot>-colorbar-<k>` for the bar `k` (counting from 0, so `-colorbar-1` is the second) of `colorbars`; a categorical bar has no gradient: its bands are one `<rect>` per integer, from the bottom up, inside its `sbmlsim-colorbar` group.

A page toggles the elements of a legend group (the groups with the same `data-legend`) by clicking the entry of that `data-legend`.

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
```

`render_svg`, `render_png`, `figure_geometry` and `pick` take the JSON text of the spec and raise a `ValueError` with the path and message of an invalid spec; `render_png` raises a `RuntimeError` for a figure which cannot be rasterized, e.g. an image of more than 64 Mpx. `geometry["width"]` and `geometry["height"]` are the size the figure is drawn at (the size of the SVG, see above), `geometry["plots"]` has per plot its `id`, its panel `rect` (`x`, `y`, `width`, `height` in pixels) and the `x`, `y` and `y2` scales (`kind`, `low`, `high`, `reverse`); `legend_entries` has per entry its `legend` group, its `owner` (a plot id or `row:<n>`) and its `rect`. A hit has the `plot`, the `element`, the `legend` group, the `index` of the point in the data of the element (the cell of a heatmap in row order), the `values` at the point in data units (`x`, `y`, `xerr`, `yerr` of a curve, `low`, `high` and `median` of a band, the five numbers of a box, `value` of a heatmap cell and so on) and the `label`. A pointer is measured against the points of the elements; since 0.28.0 a curve which draws its line is also picked along the segments of the line, between two consecutive points it draws, and a segment answers with its nearer end, so a pointer on the line between sparse points picks the curve. `sbmlsim_core.plot_wasm()` gives the bytes of `plot.wasm`, so a page which inlines everything needs no file.

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

PNG is drawn natively only, behind the feature `png` of the crate (the python package enables it, the wasm module does not): the SVG is rasterized with resvg at `dpi / 72` times the drawn size of the figure (the size of the spec grown by its legends, the `width` and `height` of the geometry), with the font Inter (OFL-1.1) embedded in the binary, so a PNG does not depend on the fonts of the machine. An image is at most 64 Mpx (`64 * 1024 * 1024` pixels, 256 MiB of pixels in memory): a larger one, of a large figure at a high dpi, is refused with a `RuntimeError` naming its size. The notices of the fonts and of the crates are in `THIRD_PARTY.md`.

## Performance

Measured on the benchmark figure of the crate:

| operation | time |
|---|---|
| native SVG render | about 1 to 1.5 ms |
| `plot.wasm` render in Node, JSON included | about 5 to 10 ms |
| pick on a prepared figure | about 0.1 ms |
| PNG at 144 dpi | about 36 ms |

`plot.wasm` is 1.32 MB raw and 394 kB gzipped.
