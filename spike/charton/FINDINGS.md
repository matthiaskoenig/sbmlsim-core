# Findings of the spike: Charton 0.9.1 for the element kinds of `figure/1`

The spike (`src/main.rs`, `cargo run --release` from this directory) builds one figure per element kind of `figure/1` with Charton 0.9.1 (crates.io, `=0.9.1`), writes them into `out/`, times each with 200 repetitions and probes the questions below. `samples/` keeps four of the SVGs the decisions rest on; every other SVG named here is in `out/` after a run. Throwaway code, not a member of the workspace.

## Decisions

**(a) One Charton composite chart (`LayeredChart`) per plot, with Charton's legend off.** Charton draws the marks, the scales, the axes (ticks, tick labels, rotated labels, axis titles), the grid lines, the plot title and the layout of the panel of one plot; the crate draws the legends, the composition of the figure (grid of plots, figure title, row legends), the right y axis and the colour bars of its own colour scales, and feeds Charton data it has prepared (log axes, bands, boxes, violins, histograms, see (e)). Reason: Charton's axes and marks were right in every probe at 0.04 to 0.4 ms per plot, while its legend draws a point symbol per category only (no line, dash, band or line-with-marker swatch, no declared legends, no row legend), so the legend is the crate's either way and the rest of the plot is not. Based on `out/curve.svg`, `out/bar.svg`, `out/categorical.svg`, `samples/grid.svg`. The crate places and sizes the panel itself where it needs to (aligned panels of a grid, the stable frame of the web app): a first pass with margins 0 through a recording backend (`RenderBackend` is public, `LayeredChart::render` takes any backend, 0.03 ms) gives the depth of the axes, and the margins of the second pass put the panel where the crate wants it; in `samples/grid.svg` the panels of a column start at the same x to the bit (55.05 and 66.1). Exact axis ends are `with_x_domain` plus a zero `Expansion` (`samples/right_axis.svg` ends at 24, not at the nice 25).

**(b) The right y axis is an overlay chart plus an axis drawn by the crate.** The elements of the right axis are a second `LayeredChart` of the same size with axes, grid, legend and background off and its margins set to the panel of the primary chart, which the recording backend measures; its panel came out equal to the primary's (57.65, 37.6, 367.35, 299.4 both). The spine, ticks, tick labels and title of the right axis are drawn by the crate into the right margin the primary chart reserves. Reason: Charton has no secondary axis and always draws the y axis on the left. Based on `samples/right_axis.svg`. A secondary axis upstream would remove the overlay, but nothing waits on it.

**(c) A layer's group carries `data-element` by rewriting `data-layer`.** Charton writes the marks of layer N into one `<g class="charton-mark charton-mark-N ..." data-layer="N" data-mark="...">`; the crate adds `data-element="<id>"` to it by one string replacement per plot, before the plot is nested into the figure (layer indices restart per plot). `SvgOptions` offers no per-layer attributes; its `with_id_prefix` is still needed, one prefix per plot, so the clip paths of the plots of a grid do not collide. `data-legend` sits on the crate's own legend groups. Theme tokens go into a scoped `<style>` (`#<figure id> .charton-tick-label text { fill: var(--token, default) }`), since Charton writes colours as presentation attributes (`rgba(...)`) and refuses `var()` in them by design; CSS rules override presentation attributes, shown by restyling the tick labels and the `data-element="model"` curve of `samples/curve_tagged.svg` (cairosvg reads no `var()`, so that check used plain colours; the browser check belongs to the web app task). Fallback if the rewriting gets in the way: an SVG backend of the crate on the public `RenderBackend` trait, which sees the layer index in `begin_group` and writes what it wants. Based on `samples/curve_tagged.svg` (4 mark groups tagged).

**(d) The text measure is a width table of the bundled font (Inter) in the crate; Charton's estimate stays inside Charton's axes until an upstream PR makes it pluggable.** Charton's `estimate_text_width` is `pub(crate)` and a class count (0.55 em per lowercase letter and digit, 0.65 per capital, 0.3 per narrow glyph). Against the advances of Charton's own `Inter-Regular.ttf` at 13 px it is off by 11 % on average and up to 33 %; it underestimates every number by 13 to 18 % (an Inter digit is 0.63 em), so the depth of a y axis is a few pixels short and the tick labels sit close to the axis title. The crate lays out its legends, right axis, colour bar and figure bands with an advance table of Inter (ASCII, Latin-1, Greek, a few kB), the same in native and wasm, so the bytes are the same everywhere. Based on `out/categorical.svg` and the table below.

**(e) What Charton lacks**, each "draw in crate" (the plan of step 1) or "upstream PR" (contributed, not waited for):

| Part | Charton 0.9.1 | Decision |
|---|---|---|
| Legends: line, dash, marker, line+marker, band and bar swatches, declared legends, row legends, legend outside a grid | point symbols per category only | draw in crate |
| Grid of independent plots, figure title, aligned panels | facets of one dataset only | draw in crate (nested `<svg>`, two-pass margins) |
| Right y axis | none | draw in crate (overlay chart + axis); upstream PR optional |
| Log x on line and area, log x and y on path and polygon, log on text | refused by `validate_scale_compatibility` (`line chart expects [Linear, Temporal, Discrete] scale for channel X`) | draw in crate: every log axis is log10 of the data on a linear Charton axis with explicit ticks (`with_x_ticks`/`with_y_ticks`), one rule for every mark; upstream PR to allow `Log` where the renderers normalize through the scale anyway |
| Log tick labels | `1.00, 0.10, 0.01`, decades only, the top decade unlabelled (`out/log_axes.svg`) | draw in crate (the explicit ticks above) |
| Area between two curves (band, shaded area): `y2` on `mark_area` | ignored, the baseline is 0 or a stack (the first run drew no area and a band of 0..low) | draw in crate: a closed polygon per run of finite values, a NaN splits the run (`out/band.svg`, `out/area.svg`); upstream PR for `y2` on area |
| Horizontal error bars | `mark_errorbar` is vertical (`coord_flip` is per chart) | draw in crate: a path with one path group per point (`out/curve.svg`) |
| Horizontal, diagonal and dashed rules | `mark_rule` is vertical (x, y..y2), no dash | draw in crate: two-point lines, which take a dash (`out/rule.svg`) |
| Box from given statistics | `mark_boxplot` computes its statistics from raw data | draw in crate: floating bar q1..q3, whisker rules, median path at the category positions (`with_category_labels`), outlier points (`out/boxes.svg`; `mark_tick` drew the median as a dot) |
| Violin from a given density | `mark_violin` estimates its density | draw in crate: a polygon per category at `index +- density` with `with_category_labels` (`out/violin.svg`) |
| Histogram from given bins | `mark_hist` bins raw data; a bar needs a discrete x | draw in crate: a polygon per bin (`out/histogram.svg`) |
| Diverging colour scale centred on a value, custom ramps (RdBu, the web app's `RAMP`) | sequential maps only, no custom map | draw in crate: the crate resolves the colours and passes them as a custom palette indexed by the cell's category (`samples/heatmap.svg`, correct but tied to Charton's order of categories), and draws the colour bar of such a scale itself; upstream PR for `ColorMap::Custom` with a midpoint |
| Text colour per heatmap cell (contrast) | one colour per text layer | draw in crate: two text layers, dark and light text (`samples/heatmap.svg` has black on dark cells) |
| Text offset from its anchor, text outside the panel | no dx/dy, text is clipped to the panel (`out/text.svg`, "steady state" cut at the right edge) | draw in crate: offsets in data space, annotations outside the panel as crate text |
| Theme tokens | presentation attributes, no `var()` | draw in crate: scoped `<style>` (c) |
| Colour bar of a continuous colour on lines (scan over 11 doses) | yes, gradient with ticks (`out/colorbar.svg`) | Charton, for its own maps (viridis); the crate's bar for crate scales |
| PNG | `to_png`, tiny-skia with Inter bundled, 9 ms at 500x400 (`out/curve_charton.png`) | Charton |

## Timings

Native: `cargo run --release` (opt-level 3), Intel Core i7-12700, pinned to a performance core with `taskset -c 2`; wasm: the same binary for `wasm32-wasip1` without the `png` feature, opt-level `s`, LTO, stripped, run with wasmtime, also pinned. ms per figure is building the charts from the arrays plus `to_svg`, over 200 repetitions; the figures with geometry passes (`right_axis`, `grid`) include them.

| figure | what | native ms | wasm ms | SVG bytes |
|---|---|---:|---:|---:|
| curve | line with NaN gap and dash, 12 points with x and y error bars | 0.168 | 0.206 | 23845 |
| band | band polygon (300 points) and median | 0.279 | 0.359 | 27000 |
| area | area polygon with NaN gap and line | 0.114 | 0.161 | 14934 |
| bar | 4 bars with error bars | 0.062 | 0.089 | 11605 |
| boxes | 3 boxes from stats with outliers | 0.071 | 0.098 | 11963 |
| violin | 3 violins from densities | 0.226 | 0.313 | 22372 |
| histogram | 20 bins | 0.103 | 0.146 | 19399 |
| heatmap | 4x5 cells, diverging, cell text | 0.148 | 0.197 | 25238 |
| text | 4 annotations | 0.062 | 0.091 | 13480 |
| rule | identity, 3 levels, vertical rule | 0.075 | 0.103 | 13570 |
| polygon | hexagon | 0.047 | 0.068 | 9260 |
| colorbar | 11 lines of 120 points, colour bar | 0.407 | 0.547 | 41871 |
| log_axes | log x (crate), log y (Charton) | 0.088 | 0.126 | 12012 |
| categorical | 5 categories, rotated labels | 0.042 | 0.061 | 8423 |
| right_axis | two charts, two geometry passes, crate axis | 0.137 | 0.201 | 18140 |
| grid | 4 plots, 8 geometry passes, row legends | 0.804 | 1.047 | 68742 |

Also: a geometry pass through the recording backend 0.028 ms, `to_svg` of a built chart 0.143 ms (curve), `to_png` 8.96 ms (curve, 500x400). The earlier benchmark of the design (8 curves of 500 points, points, error bars, log y) was 1.08 ms. The wasm binary of the spike (Charton, std, the figures) is 972 kB, 323 kB gzipped (at opt-level 3: 1.62 MB); the renderer's own module will be of that order.

## Text measure

Charton's estimate against the advances of `Inter-Regular.ttf` at 13 px (fontTools):

| text | Inter | estimate | error |
|---|---:|---:|---:|
| `0` | 8.2 | 7.2 | -13 % |
| `10` | 13.5 | 11.1 | -18 % |
| `1000` | 29.9 | 25.4 | -15 % |
| `1e-05` | 34.8 | 29.2 | -16 % |
| `Smith2001` | 64.4 | 55.9 | -13 % |
| `time [hr]` | 52.1 | 54.6 | +5 % |
| `concentration [mg/l]` | 125.2 | 126.8 | +1 % |
| `Cmax` | 35.3 | 29.9 | -15 % |
| `5-95 %` | 45.9 | 39.6 | -14 % |
| `WWW MMM` | 77.3 | 57.9 | -25 % |
| `iiii llll` | 28.8 | 38.3 | +33 % |

Over 22 labels of the figures: mean absolute error 11 %, largest 33 %.

## Visual problems seen (PNGs of `out/` by cairosvg)

- `mark_area` with `y2` (first run of `area` and `band`): no area at all, and a band of 0..low instead of low..high. Fixed by the polygons of (e).
- `mark_tick` (first run of `boxes`): the median was a dot. Fixed by a path.
- `out/text.svg`: the text starts on its point (no offset) and is clipped at the panel edge.
- `out/log_axes.svg`: Charton's log y labels are `1.00, 0.10, 0.01`, decades only, the top decade unlabelled.
- `samples/heatmap.svg`: black cell text on the darkest cells; the rows run bottom up (`with_reverse` on y exists).
- Every chart: the tick labels of the y axis sit close to its title, since numbers are measured 15 % short (d).
- Charton extends an axis to its nice ticks (data to 24 gives an axis to 25) unless the domain and a zero expansion are given.
- `samples/grid.svg` before the two-pass margins: the panels of a column started at different x; aligned after.
- The plot title of a chart centres over its panel, not over the figure (`TitleFrame::Figure` exists).

## Notes for Tasks 3 to 6

- `Precision` is `f32`: every coordinate of a primitive is a 32 bit float, written with 3 decimals.
- Colours go out as `rgba(r,g,b,a)`; the semantic classes (`charton-axis-x`, `charton-tick-label`, `charton-mark-N`, ...) and `data-layer`, `data-mark`, `data-series`, `data-value` are what a stylesheet and the browser address.
- A NaN in x or y splits `mark_line` into segments; polygons take their runs from the crate.
- A layer with a path group per point writes one `<g class="charton-series">` per group, so x error bars cost a group per point.
- With the log rule of (e) the crate never uses Charton's `Scale::Log`, so the rule that every layer of a log axis must name it does not arise.
