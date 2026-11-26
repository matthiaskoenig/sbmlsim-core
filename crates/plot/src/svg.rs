//! The SVG the crate writes: escaping, numbers, colours, and the backend Charton draws a plot
//! through.
//!
//! Charton draws through [`RenderBackend`], which [`PlotWriter`] implements. It writes the
//! primitives as SVG with plain light-theme values as presentation attributes and Charton's
//! semantic classes (`charton-axis`, `charton-tick-label`, `charton-mark`, ...), and wraps the
//! layers of one element of the spec into one group `<g data-element="<id>">`, with
//! `data-legend` when the element is in a legend group. A host page restyles the figure with a
//! stylesheet on these classes and attributes; nothing of the file is a `var()`, which renderers
//! outside of a browser (cairosvg, Inkscape) do not read.
//!
//! Every coordinate is written with at most two decimals, without trailing zeros and never as
//! `-0`; a primitive with a coordinate which is no finite number is not written at all.

use std::fmt::Write as _;

use charton::coordinate::Rect;
use charton::core::layer::{
    CircleConfig, GradientRectConfig, LineConfig, PathConfig, PathTopology, RectConfig,
    RenderBackend, TextConfig,
};
use charton::core::semantic::{AxisKind, Element as ChartonElement, Group, Role};
use charton::error::ChartonError;
use charton::prelude::SingleColor;

use crate::spec::{FillStyle, FontWeight, Marker, MarkerType};
use crate::text::{LINE_HEIGHT, SUP_RISE, SUP_SCALE, line_count, lines, runs};

/// The character which stands for the id prefix of the figure until the drawing is complete; it
/// is no character of a text of the output, see [`escape`].
pub(crate) const ID_PLACEHOLDER: char = '\u{1}';

/// The font family of every text, Inter first: the font the text is measured with.
pub(crate) const FONT_FAMILY: &str =
    "Inter, -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, Helvetica, Arial, sans-serif";

/// The colours of the light theme, which a host page overrides with a stylesheet.
pub(crate) mod theme {
    /// Text of titles and axis titles.
    pub const FG: &str = "#1b1f24";
    /// Axis lines, ticks and tick labels.
    pub const FG_SOFT: &str = "#3b424d";
    /// Grid lines.
    pub const GRID: &str = "#e3e6ea";
    /// The canvas.
    pub const BACKGROUND: &str = "#ffffff";
}

/// Writes a text escaped for a text node or an attribute value.
///
/// `&`, `<`, `>` and `"` become their entities; a character XML 1.0 does not allow (the control
/// characters but tab, newline and carriage return) becomes U+FFFD.
pub(crate) fn escape(out: &mut String, text: &str) {
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\t' | '\n' | '\r' => out.push(c),
            c if (c as u32) < 0x20 || c == '\u{fffe}' || c == '\u{ffff}' => out.push('\u{fffd}'),
            c => out.push(c),
        }
    }
}

/// Writes a text node escaped (see [`escape`]) with every space kept (since 0.30.0): SVG
/// collapses a run of spaces into one and drops the spaces at the ends of a text, so a space
/// after another one and a space at an end are written as no-break spaces (U+00A0), which no
/// renderer collapses and which are as wide as a space in Inter. The text is drawn as wide as
/// the layout measures it, every space counted.
pub(crate) fn text_node(out: &mut String, text: &str) {
    let last = text.chars().count().saturating_sub(1);
    let mut previous = None;
    let mut kept = String::with_capacity(text.len());
    for (k, c) in text.chars().enumerate() {
        let collapsed = c == ' ' && (k == 0 || k == last || previous == Some(' '));
        kept.push(if collapsed { '\u{a0}' } else { c });
        previous = Some(c);
    }
    escape(out, &kept);
}

/// Writes a coordinate: two decimals at most, no trailing zeros, never `-0`.
pub(crate) fn num(out: &mut String, v: f64) {
    let r = (v * 100.0).round();
    if r == 0.0 || !r.is_finite() {
        out.push('0');
        return;
    }
    let r = r as i64;
    if r < 0 {
        out.push('-');
    }
    let a = r.unsigned_abs();
    let (int, frac) = (a / 100, a % 100);
    let _ = write!(out, "{int}");
    if frac != 0 {
        if frac % 10 == 0 {
            let _ = write!(out, ".{}", frac / 10);
        } else {
            let _ = write!(out, ".{frac:02}");
        }
    }
}

/// Writes ` name="<number>"`.
pub(crate) fn attr_num(out: &mut String, name: &str, v: f64) {
    out.push(' ');
    out.push_str(name);
    out.push_str("=\"");
    num(out, v);
    out.push('"');
}

/// Writes a colour as `#rrggbb`, or `none`.
fn color(out: &mut String, c: &SingleColor) {
    if c.is_none() {
        out.push_str("none");
        return;
    }
    let [r, g, b, _] = c.rgba();
    let byte = |v: f32| (v * 255.0).round().clamp(0.0, 255.0) as u8;
    let _ = write!(out, "#{:02x}{:02x}{:02x}", byte(r), byte(g), byte(b));
}

/// Writes ` name="<colour>"` and, for a colour with an alpha below 1, ` name-opacity`.
fn attr_color(out: &mut String, name: &str, c: &SingleColor) {
    out.push(' ');
    out.push_str(name);
    out.push_str("=\"");
    color(out, c);
    out.push('"');
    let alpha = f64::from(c.rgba()[3]);
    if !c.is_none() && alpha < 1.0 {
        attr_opacity(out, &format!("{name}-opacity"), alpha);
    }
}

/// Writes ` name="<opacity>"` with three decimals, when it is below 1.
pub(crate) fn attr_opacity(out: &mut String, name: &str, v: f64) {
    if v < 1.0 {
        let _ = write!(out, " {name}=\"{}\"", trim(&format!("{:.3}", v.max(0.0))));
    }
}

/// A decimal without trailing zeros.
pub(crate) fn trim(s: &str) -> &str {
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.')
    } else {
        s
    }
}

/// Writes ` stroke-dasharray` for a dash pattern which is not empty.
pub(crate) fn attr_dash(out: &mut String, dash: &[f32]) {
    if dash.iter().all(|d| *d <= 0.0) {
        return;
    }
    out.push_str(" stroke-dasharray=\"");
    for (i, d) in dash.iter().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        num(out, f64::from(d.max(0.0)));
    }
    out.push('"');
}

/// Writes ` name="<colour>"` of a CSS colour text such as `#rrggbb`, as Charton reads it.
pub(crate) fn attr_css(out: &mut String, name: &str, css: &str) {
    attr_color(out, name, &SingleColor::new(css));
}

/// Writes ` font-weight="bold"` for the weight bold and nothing for the weight normal, the
/// weight of a text without the attribute.
pub(crate) fn attr_weight(out: &mut String, weight: FontWeight) {
    if weight == FontWeight::Bold {
        out.push_str(" font-weight=\"bold\"");
    }
}

/// How the lines of a text of several lines stand to the position of the text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Stack {
    /// The first line at the position, the others below it.
    Down,
    /// The lines centred on the position.
    Centred,
    /// The last line at the position, the others above it.
    Up,
}

/// Writes the content of a `<text>` at `(x, y)` of a font size: a text of one line escaped, a
/// text of several lines (split at `\n`) as a `<tspan>` per line at its own `x` and `y`,
/// [`LINE_HEIGHT`] font sizes apart and placed by `stack`. The `x` and `y` are in the
/// coordinates of the text, so the lines of a rotated text are rotated with it.
pub(crate) fn text_lines(
    out: &mut String,
    text: &str,
    (x, y): (f64, f64),
    size: f64,
    stack: Stack,
) {
    let n = line_count(text);
    if n == 1 {
        text_node(out, text);
        return;
    }
    let step = LINE_HEIGHT * size;
    let before = match stack {
        Stack::Down => 0.0,
        Stack::Centred => (n - 1) as f64 / 2.0,
        Stack::Up => (n - 1) as f64,
    };
    for (k, line) in lines(text).enumerate() {
        out.push_str("<tspan");
        attr_num(out, "x", x);
        attr_num(out, "y", y + (k as f64 - before) * step);
        out.push('>');
        text_node(out, line);
        out.push_str("</tspan>");
    }
}

/// Writes a line of a tick label: its plain runs as text, its superscripts as `<tspan>`s of
/// [`SUP_SCALE`] of the size raised by [`SUP_RISE`] of it, the run after one lowered back.
fn label_line(out: &mut String, line: &str, size: f64) {
    let mut raised = false;
    for (text, sup) in runs(line) {
        if sup {
            out.push_str("<tspan");
            attr_num(out, "dy", -SUP_RISE * size);
            attr_num(out, "font-size", SUP_SCALE * size);
            out.push('>');
            text_node(out, text);
            out.push_str("</tspan>");
            raised = true;
        } else if raised {
            out.push_str("<tspan");
            attr_num(out, "dy", SUP_RISE * size);
            out.push('>');
            text_node(out, text);
            out.push_str("</tspan>");
            raised = false;
        } else {
            text_node(out, text);
        }
    }
}

/// Writes the content of the `<text>` of a tick label (since 0.31.0): as [`text_lines`] does,
/// each line with its superscripts (see [`label_line`]).
pub(crate) fn label_lines(
    out: &mut String,
    text: &str,
    (x, y): (f64, f64),
    size: f64,
    stack: Stack,
) {
    let n = line_count(text);
    if n == 1 {
        label_line(out, text, size);
        return;
    }
    let step = LINE_HEIGHT * size;
    let before = match stack {
        Stack::Down => 0.0,
        Stack::Centred => (n - 1) as f64 / 2.0,
        Stack::Up => (n - 1) as f64,
    };
    for (k, line) in lines(text).enumerate() {
        out.push_str("<tspan");
        attr_num(out, "x", x);
        attr_num(out, "y", y + (k as f64 - before) * step);
        out.push('>');
        label_line(out, line, size);
        out.push_str("</tspan>");
    }
}

/// The gap between the end of an axis and the multiplier of its tick labels in pixels.
pub(crate) const MULTIPLIER_GAP: f64 = 4.0;

/// The marker an element draws at every point Charton places.
#[derive(Debug, Clone)]
pub(crate) struct MarkerGlyph {
    /// The shape.
    pub kind: MarkerType,
    /// The diameter in pixels.
    pub size: f64,
    /// The fill colour `#rrggbb`.
    pub fill: String,
    /// The edge colour `#rrggbb`.
    pub edge: String,
    /// The width of the edge in pixels.
    pub edge_width: f64,
    /// The opacity of the marker.
    pub opacity: f64,
    /// Which half the fill fills (since 0.31.0).
    pub fill_style: FillStyle,
    /// The fill of the other half of a half filled marker, `#rrggbb` or `none`.
    pub fill_alt: String,
}

impl MarkerGlyph {
    /// The glyph of the marker of a style in the colour of its element: the fill and the edge
    /// the marker gives, else that colour.
    pub(crate) fn of(m: &Marker, color: &str, opacity: f64) -> Self {
        MarkerGlyph {
            kind: m.kind,
            size: m.size,
            fill: m.fill.clone().unwrap_or_else(|| color.to_string()),
            edge: m.edge.clone().unwrap_or_else(|| color.to_string()),
            edge_width: m.edge_width,
            opacity,
            fill_style: m.fill_style,
            fill_alt: m.fill_alt.clone().unwrap_or_else(|| "none".to_string()),
        }
    }
}

/// What the writer knows of one layer of the Charton chart.
#[derive(Debug, Clone)]
pub(crate) struct LayerInfo {
    /// The element of the spec the layer draws, by its position in the plot; `None` for a layer
    /// which is drawn for the scales only and writes nothing.
    pub element: Option<usize>,
    /// The marker of a layer of points: every circle Charton draws becomes this marker.
    pub marker: Option<MarkerGlyph>,
}

/// The tag of an element group.
#[derive(Debug, Clone)]
pub(crate) struct ElementTag {
    /// The id of the element.
    pub id: String,
    /// The kind of the element, its class `sbmlsim-<kind>`.
    pub kind: &'static str,
    /// The legend group of the element.
    pub legend: Option<String>,
    /// Whether this chart writes the group of the element. The elements of the right y axis
    /// are drawn by the overlay chart of that axis, whose writer writes their groups; the
    /// writer of the primary chart leaves them out, so no group is written twice.
    pub here: bool,
}

/// What the writer draws of one axis besides Charton's spine, ticks and tick labels.
///
/// Charton lays the tick labels out unrotated and without a title, and measures them with its
/// own estimate; the crate measures them with the Inter table, reserves the difference in the
/// margins, rotates the labels and draws the title at the Inter depth of the labels. So the
/// axis title and the panel sit where the Inter measure puts them, and no sine or cosine of the
/// standard library enters the layout (Charton's would be glibc's natively and another in
/// wasm).
#[derive(Debug, Clone, Default)]
pub(crate) struct AxisHooks {
    /// Whether the tick labels are hidden.
    pub labels_hidden: bool,
    /// Whether the tick marks are hidden.
    pub marks_hidden: bool,
    /// Whether the line of the axis is hidden.
    pub line_hidden: bool,
    /// The font size of the tick labels.
    pub tick_size: f64,
    /// The weight of the tick labels.
    pub tick_weight: FontWeight,
    /// The rotation of the tick labels in the degrees of SVG (clockwise), 0 for none.
    pub label_angle: f64,
    /// The title, `None` when it is hidden or empty.
    pub title: Option<String>,
    /// The distance from the spine to the far side of the tick labels, measured in Inter.
    pub labels_depth: f64,
    /// The tick labels in the order of the ticks (since 0.31.0), each with the line Charton is
    /// given for it: Charton measures and places the widest line of a label without its
    /// superscripts, the writer writes the label of several lines and superscripts.
    pub labels: Vec<(String, String)>,
    /// The multiplier of the tick labels, `×10^{n}` (since 0.31.0), written at the end of the
    /// axis as matplotlib's offset text: right below the tick labels of the bottom axis, above
    /// the top of the left axis.
    pub multiplier: Option<String>,
}

/// The grid lines of one axis, as shares along the axis from the start of its domain.
#[derive(Debug, Clone, Default)]
pub(crate) struct GridLines {
    /// Shares of the vertical lines along the x axis.
    pub x: Vec<f64>,
    /// Shares of the horizontal lines along the y axis, 0 at the bottom.
    pub y: Vec<f64>,
}

/// What the writer draws around Charton's primitives of one plot.
#[derive(Debug, Clone, Default)]
pub(crate) struct PlotHooks {
    /// The layers of the chart in their order.
    pub layers: Vec<LayerInfo>,
    /// The elements of the plot in their order.
    pub elements: Vec<ElementTag>,
    /// The x axis.
    pub x: AxisHooks,
    /// The y axis.
    pub y: AxisHooks,
    /// The font size of the axis titles.
    pub title_size: f64,
    /// The weight of the axis titles.
    pub title_weight: FontWeight,
    /// The weight of the title of the plot.
    pub plot_title_weight: FontWeight,
    /// The gap between the tick labels and the title of an axis.
    pub title_gap: f64,
    /// The grid lines.
    pub grid: GridLines,
}

/// One open group of the writer.
struct Frame {
    /// Whether a `<g>` was written, which its end closes.
    wrote: bool,
    /// Whether the primitives inside it are dropped.
    suppress: bool,
    /// Whether it is the plot group, whose end closes the element groups.
    plot: bool,
    /// Whether it is an axis group.
    axis: bool,
    /// Whether it is the group of a layer, whose end ends the marker of the layer.
    mark: bool,
    /// Whether it is inside the group of the tick labels, whose text the writer rotates.
    tick_label: bool,
    /// Whether it is inside the group of the title of the plot.
    title: bool,
}

/// The backend Charton draws one plot through; it writes SVG into `body` and `defs`.
pub(crate) struct PlotWriter<'a> {
    /// The drawing.
    pub body: String,
    /// The `<clipPath>` and gradient definitions.
    pub defs: String,
    /// The prefix of every id, unique per plot of the figure.
    prefix: String,
    hooks: &'a PlotHooks,
    stack: Vec<Frame>,
    clips: usize,
    gradients: usize,
    /// The element whose group is open.
    open_element: Option<usize>,
    /// The next element whose group is not written yet.
    next_element: usize,
    /// The axis whose group is open.
    axis: Option<AxisKind>,
    /// The marker of the open layer.
    marker: Option<MarkerGlyph>,
    /// The panel of the plot, the clip of its plot group.
    pub panel: Option<Rect>,
    /// The tick labels of the open axis written so far.
    tick_labels: usize,
}

impl<'a> PlotWriter<'a> {
    /// A writer for a plot whose ids start with `prefix`.
    pub fn new(prefix: String, hooks: &'a PlotHooks) -> Self {
        PlotWriter {
            body: String::with_capacity(16 * 1024),
            defs: String::new(),
            prefix,
            hooks,
            stack: Vec::new(),
            clips: 0,
            gradients: 0,
            open_element: None,
            next_element: 0,
            axis: None,
            marker: None,
            panel: None,
            tick_labels: 0,
        }
    }

    /// Whether the primitives drawn now are dropped.
    fn suppressed(&self) -> bool {
        self.stack.last().is_some_and(|f| f.suppress)
    }

    /// Opens the group of an element.
    fn open_element_group(&mut self, index: usize) {
        let tag = &self.hooks.elements[index];
        self.body.push_str("<g class=\"sbmlsim-element sbmlsim-");
        self.body.push_str(tag.kind);
        self.body.push_str("\" data-element=\"");
        escape(&mut self.body, &tag.id);
        self.body.push('"');
        if let Some(legend) = &tag.legend {
            self.body.push_str(" data-legend=\"");
            escape(&mut self.body, legend);
            self.body.push('"');
        }
        self.body.push('>');
    }

    /// Writes the groups of the elements up to `until` (exclusive) which have none yet, empty.
    fn catch_up(&mut self, until: usize) {
        if self.open_element.is_some() {
            self.body.push_str("</g>");
            self.open_element = None;
        }
        while self.next_element < until.min(self.hooks.elements.len()) {
            if self.hooks.elements[self.next_element].here {
                self.open_element_group(self.next_element);
                self.body.push_str("</g>");
            }
            self.next_element += 1;
        }
    }

    /// The hooks of the axis whose group is open.
    fn axis_hooks(&self) -> Option<&AxisHooks> {
        match self.axis {
            Some(AxisKind::X) => Some(&self.hooks.x),
            Some(AxisKind::Y) => Some(&self.hooks.y),
            _ => None,
        }
    }

    /// Writes the title of the open axis at the Inter depth of its tick labels, where Charton
    /// would write it: below the bottom axis hanging from its band, left of the left axis
    /// rotated and centred in its band.
    fn axis_title(&mut self) {
        self.multiplier();
        let (Some(panel), Some(hooks)) = (self.panel, self.axis_hooks()) else {
            return;
        };
        let Some(title) = hooks.title.clone() else {
            return;
        };
        let (size, gap, depth, weight) = (
            self.hooks.title_size,
            self.hooks.title_gap,
            hooks.labels_depth,
            self.hooks.title_weight,
        );
        let b = &mut self.body;
        b.push_str("<g class=\"charton-axis-title\"><text");
        // a title of several lines stacks them outwards: below the bottom axis down from the
        // line next to it, left of the left axis (up in the rotated text) from the line next
        // to it
        let (at, stack) = if self.axis == Some(AxisKind::X) {
            let (x, y) = (
                panel.x + panel.width / 2.0,
                panel.y + panel.height + depth + gap,
            );
            attr_num(b, "x", x);
            attr_num(b, "y", y);
            attr_num(b, "font-size", size);
            attr_weight(b, weight);
            let _ = write!(
                b,
                " fill=\"{}\" text-anchor=\"middle\" dominant-baseline=\"hanging\">",
                theme::FG
            );
            ((x, y), Stack::Down)
        } else {
            let (x, y) = (
                panel.x - (depth + gap + size / 2.0),
                panel.y + panel.height / 2.0,
            );
            attr_num(b, "x", x);
            attr_num(b, "y", y);
            attr_num(b, "font-size", size);
            attr_weight(b, weight);
            let _ = write!(
                b,
                " fill=\"{}\" text-anchor=\"middle\" dominant-baseline=\"middle\" transform=\"rotate(-90 ",
                theme::FG
            );
            num(b, x);
            b.push(' ');
            num(b, y);
            b.push_str(")\">");
            ((x, y), Stack::Up)
        };
        text_lines(b, &title, at, size, stack);
        b.push_str("</text></g>");
    }

    /// Writes the multiplier of the tick labels of the open axis (since 0.31.0), as matplotlib
    /// writes its offset text: right aligned at the right end of the bottom axis, hanging below
    /// its tick labels; left aligned at the top of the left axis, above the panel.
    fn multiplier(&mut self) {
        let (Some(panel), Some(hooks)) = (self.panel, self.axis_hooks()) else {
            return;
        };
        let Some(text) = hooks.multiplier.clone() else {
            return;
        };
        let (size, weight, depth) = (hooks.tick_size, hooks.tick_weight, hooks.labels_depth);
        let bottom = self.axis == Some(AxisKind::X);
        let b = &mut self.body;
        b.push_str("<g class=\"sbmlsim-axis-multiplier\"><text");
        let at = if bottom {
            (
                panel.x + panel.width,
                panel.y + panel.height + depth + self.hooks.title_gap,
            )
        } else {
            (panel.x, panel.y - MULTIPLIER_GAP)
        };
        attr_num(b, "x", at.0);
        attr_num(b, "y", at.1);
        attr_num(b, "font-size", size);
        attr_weight(b, weight);
        let _ = write!(b, " fill=\"{}\"", theme::FG_SOFT);
        b.push_str(if bottom {
            " text-anchor=\"end\" dominant-baseline=\"hanging\">"
        } else {
            " text-anchor=\"start\">"
        });
        label_lines(b, &text, at, size, Stack::Down);
        b.push_str("</text></g>");
    }

    /// Draws every element group which is not written yet; for a plot Charton drew no plot
    /// group of.
    pub fn finish(&mut self) {
        self.catch_up(self.hooks.elements.len());
    }

    /// Writes the grid lines into the panel.
    fn grid(&mut self, panel: &Rect) {
        let g = &self.hooks.grid;
        if g.x.is_empty() && g.y.is_empty() {
            return;
        }
        let _ = write!(
            self.body,
            "<g class=\"charton-grid\" stroke=\"{}\" stroke-width=\"1\">",
            theme::GRID
        );
        let (x0, y0, w, h) = (panel.x, panel.y, panel.width, panel.height);
        for s in &g.x {
            let x = x0 + s * w;
            self.body.push_str("<line");
            attr_num(&mut self.body, "x1", x);
            attr_num(&mut self.body, "y1", y0);
            attr_num(&mut self.body, "x2", x);
            attr_num(&mut self.body, "y2", y0 + h);
            self.body.push_str("/>");
        }
        for s in &g.y {
            let y = y0 + h - s * h;
            self.body.push_str("<line");
            attr_num(&mut self.body, "x1", x0);
            attr_num(&mut self.body, "y1", y);
            attr_num(&mut self.body, "x2", x0 + w);
            attr_num(&mut self.body, "y2", y);
            self.body.push_str("/>");
        }
        self.body.push_str("</g>");
    }
}

/// The outline of a filled marker as matplotlib draws it, in units of the marker size with y
/// up (the vertices of matplotlib's marker path and transform), and where matplotlib splits it
/// for a half fill: the offset of the line between the halves from the centre, across x (left,
/// right) and across y (top, bottom).
struct Outline {
    points: Vec<(f64, f64)>,
    split: (f64, f64),
}

/// The vertices of matplotlib's regular polygon (`Path.unit_regular_polygon`) or star
/// (`Path.unit_regular_star`) of `n` corners, a corner at the top, of radius 1/2.
fn regular(n: usize, inner: Option<f64>) -> Vec<(f64, f64)> {
    let corners = if inner.is_some() { 2 * n } else { n };
    (0..corners)
        .map(|k| {
            let a = 2.0 * std::f64::consts::PI / corners as f64 * k as f64
                + std::f64::consts::FRAC_PI_2;
            let r = match inner {
                Some(i) if k % 2 == 1 => 0.5 * i,
                _ => 0.5,
            };
            (r * libm::cos(a), r * libm::sin(a))
        })
        .collect()
}

/// The outline of a filled marker other than the circle, `None` for a marker of lines.
fn outline(kind: MarkerType) -> Option<Outline> {
    let plain = |points: Vec<(f64, f64)>| Outline {
        points,
        split: (0.0, 0.0),
    };
    // matplotlib's `D`: a square of side 1 on its corner, and its `d` 0.6 as wide
    let diamond = |w: f64| {
        let h = std::f64::consts::FRAC_1_SQRT_2;
        vec![(0.0, h), (-w * h, 0.0), (0.0, -h), (w * h, 0.0)]
    };
    Some(match kind {
        MarkerType::None
        | MarkerType::Circle
        | MarkerType::Cross
        | MarkerType::Plus
        | MarkerType::X => {
            return None;
        }
        MarkerType::Square => plain(vec![(-0.5, 0.5), (0.5, 0.5), (0.5, -0.5), (-0.5, -0.5)]),
        MarkerType::Triangle => Outline {
            points: vec![(0.0, 0.5), (-0.5, -0.5), (0.5, -0.5)],
            split: (0.0, -0.1),
        },
        MarkerType::TriangleDown => Outline {
            points: vec![(0.0, -0.5), (0.5, 0.5), (-0.5, 0.5)],
            split: (0.0, 0.1),
        },
        MarkerType::TriangleLeft => Outline {
            points: vec![(-0.5, 0.0), (0.5, -0.5), (0.5, 0.5)],
            split: (0.1, 0.0),
        },
        MarkerType::TriangleRight => Outline {
            points: vec![(0.5, 0.0), (-0.5, 0.5), (-0.5, -0.5)],
            split: (-0.1, 0.0),
        },
        MarkerType::Diamond => plain(diamond(1.0)),
        MarkerType::ThinDiamond => plain(diamond(0.6)),
        MarkerType::Star => {
            let points = regular(5, Some(0.381966));
            // matplotlib splits the star at its inner corners beside the lower points
            let split = points[3].1;
            Outline {
                points,
                split: (0.0, split),
            }
        }
        MarkerType::Pentagon => {
            let points = regular(5, None);
            // matplotlib splits the pentagon at its upper corners
            let split = points[1].1;
            Outline {
                points,
                split: (0.0, split),
            }
        }
        MarkerType::Hexagon => plain(regular(6, None)),
        MarkerType::XFilled => plain(
            [
                (-1.0, -2.0),
                (0.0, -1.0),
                (1.0, -2.0),
                (2.0, -1.0),
                (1.0, 0.0),
                (2.0, 1.0),
                (1.0, 2.0),
                (0.0, 1.0),
                (-1.0, 2.0),
                (-2.0, 1.0),
                (-1.0, 0.0),
                (-2.0, -1.0),
            ]
            .map(|(x, y)| (x / 4.0, y / 4.0))
            .to_vec(),
        ),
        MarkerType::PlusFilled => plain(
            [
                (-1.0, -3.0),
                (1.0, -3.0),
                (1.0, -1.0),
                (3.0, -1.0),
                (3.0, 1.0),
                (1.0, 1.0),
                (1.0, 3.0),
                (-1.0, 3.0),
                (-1.0, 1.0),
                (-3.0, 1.0),
                (-3.0, -1.0),
                (-1.0, -1.0),
            ]
            .map(|(x, y)| (x / 6.0, y / 6.0))
            .to_vec(),
        ),
    })
}

/// The part of a polygon on one side of a line (Sutherland and Hodgman): the points `p` with
/// `inside(p) >= 0`, `inside` linear.
fn clip(points: &[(f64, f64)], inside: impl Fn((f64, f64)) -> f64) -> Vec<(f64, f64)> {
    let mut out = Vec::with_capacity(points.len() + 2);
    for k in 0..points.len() {
        let (a, b) = (points[k], points[(k + 1) % points.len()]);
        let (da, db) = (inside(a), inside(b));
        if da >= 0.0 {
            out.push(a);
        }
        if (da >= 0.0) != (db >= 0.0) {
            let t = da / (da - db);
            out.push((a.0 + t * (b.0 - a.0), a.1 + t * (b.1 - a.1)));
        }
    }
    out
}

/// The two halves of an outline for a fill style, the filled one first, in units of the
/// marker with y up.
type Halves = (Vec<(f64, f64)>, Vec<(f64, f64)>);

/// The halves of an outline split for a half fill: the side `fill_style` names and the other.
fn halves(o: &Outline, style: FillStyle) -> Halves {
    let (sx, sy) = o.split;
    // the signed distance to the line between the halves, positive on the filled side
    let side = move |(x, y): (f64, f64)| match style {
        FillStyle::Left => sx - x,
        FillStyle::Right => x - sx,
        FillStyle::Bottom => sy - y,
        FillStyle::Top | FillStyle::Full => y - sy,
    };
    (clip(&o.points, side), clip(&o.points, move |p| -side(p)))
}

/// Writes the `d` of a closed polygon in units of a marker of `size` with y up, centred at a
/// point of the drawing.
fn polygon_d(b: &mut String, points: &[(f64, f64)], size: f64, (x, y): (f64, f64)) {
    for (i, (dx, dy)) in points.iter().enumerate() {
        b.push(if i == 0 { 'M' } else { 'L' });
        num(b, x + dx * size);
        b.push(' ');
        num(b, y - dy * size);
    }
    b.push('Z');
}

/// Writes the `d` of a half of a circle of radius `r` at a point: the half on the side `style`
/// names, or with `other` the opposite one.
fn half_circle_d(b: &mut String, r: f64, (x, y): (f64, f64), style: FillStyle, other: bool) {
    // the ends of the diameter between the halves, and the side the arc bulges to
    let (from, to) = match style {
        FillStyle::Left | FillStyle::Right => ((x, y - r), (x, y + r)),
        FillStyle::Top | FillStyle::Bottom | FillStyle::Full => ((x - r, y), (x + r, y)),
    };
    // from the top (left) end the arc runs counter-clockwise on the screen through the left
    // (bottom) half, clockwise through the right (top) half
    let left_or_bottom = matches!(style, FillStyle::Left | FillStyle::Bottom) != other;
    let sweep = if left_or_bottom { 0 } else { 1 };
    b.push('M');
    num(b, from.0);
    b.push(' ');
    num(b, from.1);
    b.push('A');
    num(b, r);
    b.push(' ');
    num(b, r);
    let _ = write!(b, " 0 0 {sweep} ");
    num(b, to.0);
    b.push(' ');
    num(b, to.1);
    b.push('Z');
}

/// Writes a marker glyph centred at a point.
///
/// The filled markers have matplotlib's shapes for a marker size: the circle as wide as the
/// size, the square of the side of the size, the triangles as wide and high as the size, the
/// diamond a square of the side of the size on its corner (since 0.31.0); a half filled marker
/// draws its filled half and its other half (of `fill_alt`) each with the edge, as matplotlib.
pub(crate) fn glyph(b: &mut String, m: &MarkerGlyph, x: f64, y: f64) {
    let r = m.size / 2.0;
    // a filled glyph: its fill and, with an edge, its stroke
    let paint = |b: &mut String, fill: &str| {
        let _ = write!(b, " fill=\"{fill}\"");
        if m.edge_width > 0.0 {
            let _ = write!(b, " stroke=\"{}\"", m.edge);
            attr_num(b, "stroke-width", m.edge_width);
        }
        attr_opacity(b, "opacity", m.opacity);
        b.push_str("/>");
    };
    let half = m.fill_style != FillStyle::Full;
    match m.kind {
        MarkerType::None => {}
        MarkerType::Circle if half => {
            for (other, fill) in [(false, m.fill.as_str()), (true, m.fill_alt.as_str())] {
                b.push_str("<path d=\"");
                half_circle_d(b, r, (x, y), m.fill_style, other);
                b.push('"');
                paint(b, fill);
            }
        }
        MarkerType::Circle => {
            b.push_str("<circle");
            attr_num(b, "cx", x);
            attr_num(b, "cy", y);
            attr_num(b, "r", r);
            paint(b, &m.fill);
        }
        MarkerType::Square if !half => {
            // matplotlib's square, of the side of the size (of the area of the circle before
            // 0.31.0)
            b.push_str("<rect");
            attr_num(b, "x", x - r);
            attr_num(b, "y", y - r);
            attr_num(b, "width", 2.0 * r);
            attr_num(b, "height", 2.0 * r);
            paint(b, &m.fill);
        }
        MarkerType::Cross | MarkerType::X | MarkerType::Plus => {
            // matplotlib's `+` and `x`: lines across the square of the size
            let d = r;
            let segments: [(f64, f64, f64, f64); 2] = if m.kind == MarkerType::Plus {
                [(-d, 0.0, d, 0.0), (0.0, -d, 0.0, d)]
            } else {
                [(-d, -d, d, d), (-d, d, d, -d)]
            };
            b.push_str("<path d=\"");
            for (x1, y1, x2, y2) in segments {
                b.push('M');
                num(b, x + x1);
                b.push(' ');
                num(b, y + y1);
                b.push('L');
                num(b, x + x2);
                b.push(' ');
                num(b, y + y2);
            }
            b.push('"');
            b.push_str(" fill=\"none\"");
            let _ = write!(b, " stroke=\"{}\"", m.edge);
            attr_num(b, "stroke-width", m.edge_width.max(1.0) * 1.5);
            attr_opacity(b, "opacity", m.opacity);
            b.push_str("/>");
        }
        kind => {
            let Some(o) = outline(kind) else {
                return;
            };
            if half {
                let (filled, other) = halves(&o, m.fill_style);
                for (points, fill) in [(filled, m.fill.as_str()), (other, m.fill_alt.as_str())] {
                    if points.len() < 3 {
                        continue;
                    }
                    b.push_str("<path d=\"");
                    polygon_d(b, &points, m.size, (x, y));
                    b.push('"');
                    paint(b, fill);
                }
            } else {
                b.push_str("<path d=\"");
                polygon_d(b, &o.points, m.size, (x, y));
                b.push('"');
                paint(b, &m.fill);
            }
        }
    }
}

/// The CSS name of a weight.
fn weight_name(weight: FontWeight) -> &'static str {
    match weight {
        FontWeight::Normal => "normal",
        FontWeight::Bold => "bold",
    }
}

/// Whether every value is a finite number.
fn finite(values: &[f32]) -> bool {
    values.iter().all(|v| v.is_finite())
}

impl RenderBackend for PlotWriter<'_> {
    fn draw_circle(&mut self, c: CircleConfig) {
        if self.suppressed() || !finite(&[c.x, c.y, c.radius]) {
            return;
        }
        if let Some(m) = self.marker.clone() {
            glyph(&mut self.body, &m, f64::from(c.x), f64::from(c.y));
            return;
        }
        let b = &mut self.body;
        b.push_str("<circle");
        attr_num(b, "cx", f64::from(c.x));
        attr_num(b, "cy", f64::from(c.y));
        attr_num(b, "r", f64::from(c.radius));
        attr_color(b, "fill", &c.fill);
        if !c.stroke.is_none() && c.stroke_width > 0.0 {
            attr_color(b, "stroke", &c.stroke);
            attr_num(b, "stroke-width", f64::from(c.stroke_width));
        }
        attr_opacity(b, "opacity", f64::from(c.opacity));
        b.push_str("/>");
    }

    fn draw_rect(&mut self, c: RectConfig) {
        if self.suppressed() || !finite(&[c.x, c.y, c.width, c.height]) {
            return;
        }
        let b = &mut self.body;
        b.push_str("<rect");
        attr_num(b, "x", f64::from(c.x));
        attr_num(b, "y", f64::from(c.y));
        attr_num(b, "width", f64::from(c.width.max(0.0)));
        attr_num(b, "height", f64::from(c.height.max(0.0)));
        attr_color(b, "fill", &c.fill);
        if !c.stroke.is_none() && c.stroke_width > 0.0 {
            attr_color(b, "stroke", &c.stroke);
            attr_num(b, "stroke-width", f64::from(c.stroke_width));
        }
        attr_opacity(b, "opacity", f64::from(c.opacity));
        b.push_str("/>");
    }

    fn draw_path(&mut self, c: PathConfig) {
        // a path of neither fill nor stroke, e.g. the fill pass of a polygon without a fill
        let stroked = !c.stroke.is_none() && c.stroke_width > 0.0;
        if self.suppressed() || (c.fill.is_none() && !stroked) {
            return;
        }
        let closed = c.is_closed();
        let subpaths: Vec<&Vec<(f32, f32)>> = c
            .subpaths
            .iter()
            .filter(|s| s.len() >= 2 && s.iter().all(|(x, y)| x.is_finite() && y.is_finite()))
            .collect();
        if subpaths.is_empty() {
            return;
        }
        let b = &mut self.body;
        b.push_str("<path d=\"");
        for s in &subpaths {
            for (i, (x, y)) in s.iter().enumerate() {
                b.push(if i == 0 { 'M' } else { 'L' });
                num(b, f64::from(*x));
                b.push(' ');
                num(b, f64::from(*y));
            }
            if closed {
                b.push('Z');
            }
        }
        b.push('"');
        attr_color(b, "fill", &c.fill);
        if closed && subpaths.len() > 1 && c.topology == PathTopology::Complex {
            b.push_str(" fill-rule=\"evenodd\"");
        }
        if !c.stroke.is_none() && c.stroke_width > 0.0 {
            attr_color(b, "stroke", &c.stroke);
            attr_num(b, "stroke-width", f64::from(c.stroke_width));
            attr_dash(b, &c.dash);
            if !closed {
                b.push_str(" stroke-linejoin=\"round\"");
            }
        }
        attr_opacity(b, "opacity", f64::from(c.opacity));
        b.push_str("/>");
    }

    fn draw_text(&mut self, c: TextConfig) -> Result<(), ChartonError> {
        if self.suppressed() || !finite(&[c.x, c.y]) {
            return Ok(());
        }
        let mut c = c;
        let (tick_label, title) = self
            .stack
            .last()
            .map_or((false, false), |f| (f.tick_label, f.title));
        if tick_label && let Some(a) = self.axis_hooks() {
            // the size and the weight of the tick labels of the axis
            c.font_size = a.tick_size as f32;
            c.font_weight = weight_name(a.tick_weight).into();
            if a.label_angle != 0.0 {
                // a rotated label of the bottom axis ends at its tick, as Charton draws it
                if self.axis == Some(AxisKind::X) {
                    c.text_anchor = "end".into();
                }
                c.angle = a.label_angle as f32;
            }
        }
        if title {
            c.font_weight = weight_name(self.hooks.plot_title_weight).into();
        }
        // the lines of a plot title stack up from the line Charton places, into the room the
        // layout reserves above it; the other texts (a text element, centred on its position)
        // by their baseline
        let stack = if title {
            Stack::Up
        } else if matches!(c.dominant_baseline.as_str(), "middle" | "central") {
            Stack::Centred
        } else {
            Stack::Down
        };
        let b = &mut self.body;
        b.push_str("<text");
        attr_num(b, "x", f64::from(c.x));
        attr_num(b, "y", f64::from(c.y));
        attr_num(b, "font-size", f64::from(c.font_size));
        attr_color(b, "fill", &c.color);
        if c.text_anchor != "start" && !c.text_anchor.is_empty() {
            b.push_str(" text-anchor=\"");
            escape(b, &c.text_anchor);
            b.push('"');
        }
        if c.dominant_baseline != "alphabetic" && !c.dominant_baseline.is_empty() {
            b.push_str(" dominant-baseline=\"");
            escape(b, &c.dominant_baseline);
            b.push('"');
        }
        if c.font_weight != "normal" && !c.font_weight.is_empty() {
            b.push_str(" font-weight=\"");
            escape(b, &c.font_weight);
            b.push('"');
        }
        if c.font_style != "normal" && !c.font_style.is_empty() {
            b.push_str(" font-style=\"");
            escape(b, &c.font_style);
            b.push('"');
        }
        attr_opacity(b, "opacity", f64::from(c.opacity));
        if c.angle != 0.0 && c.angle.is_finite() {
            b.push_str(" transform=\"rotate(");
            num(b, f64::from(c.angle));
            b.push(' ');
            num(b, f64::from(c.x));
            b.push(' ');
            num(b, f64::from(c.y));
            b.push_str(")\"");
        }
        b.push('>');
        if tick_label {
            // the label Charton was given the widest line of (since 0.31.0): its lines stacked
            // down from the bottom axis and centred on a tick of the left axis, with its
            // superscripts
            let k = self.tick_labels;
            self.tick_labels += 1;
            let full = self
                .axis_hooks()
                .and_then(|a| a.labels.get(k))
                .filter(|(given, _)| *given == c.text)
                .map_or_else(|| c.text.clone(), |(_, full)| full.clone());
            let stack = if self.axis == Some(AxisKind::X) {
                Stack::Down
            } else {
                Stack::Centred
            };
            label_lines(
                &mut self.body,
                &full,
                (f64::from(c.x), f64::from(c.y)),
                f64::from(c.font_size),
                stack,
            );
        } else {
            text_lines(
                &mut self.body,
                &c.text,
                (f64::from(c.x), f64::from(c.y)),
                f64::from(c.font_size),
                stack,
            );
        }
        self.body.push_str("</text>");
        Ok(())
    }

    fn draw_line(&mut self, c: LineConfig) {
        if self.suppressed() || !finite(&[c.x1, c.y1, c.x2, c.y2]) {
            return;
        }
        let b = &mut self.body;
        b.push_str("<line");
        attr_num(b, "x1", f64::from(c.x1));
        attr_num(b, "y1", f64::from(c.y1));
        attr_num(b, "x2", f64::from(c.x2));
        attr_num(b, "y2", f64::from(c.y2));
        attr_color(b, "stroke", &c.color);
        attr_num(b, "stroke-width", f64::from(c.width));
        attr_dash(b, &c.dash);
        attr_opacity(b, "opacity", f64::from(c.opacity));
        b.push_str("/>");
    }

    fn draw_gradient_rect(&mut self, c: GradientRectConfig) {
        if self.suppressed() || !finite(&[c.x, c.y, c.width, c.height]) {
            return;
        }
        let id = format!("{}-gradient-{}", self.prefix, self.gradients);
        self.gradients += 1;
        let d = &mut self.defs;
        d.push_str("<linearGradient id=\"");
        d.push_str(&id);
        if c.is_vertical {
            d.push_str("\" x1=\"0\" y1=\"1\" x2=\"0\" y2=\"0\">");
        } else {
            d.push_str("\" x1=\"0\" y1=\"0\" x2=\"1\" y2=\"0\">");
        }
        for (offset, stop) in &c.stops {
            d.push_str("<stop offset=\"");
            let _ = write!(d, "{}", trim(&format!("{:.4}", offset.clamp(0.0, 1.0))));
            d.push('"');
            attr_color(d, "stop-color", stop);
            d.push_str("/>");
        }
        d.push_str("</linearGradient>");
        let b = &mut self.body;
        b.push_str("<rect");
        attr_num(b, "x", f64::from(c.x));
        attr_num(b, "y", f64::from(c.y));
        attr_num(b, "width", f64::from(c.width.max(0.0)));
        attr_num(b, "height", f64::from(c.height.max(0.0)));
        b.push_str(" fill=\"url(#");
        b.push_str(&id);
        b.push_str(")\"/>");
    }

    fn begin_group(&mut self, g: &Group<'_>) {
        let parent_suppress = self.suppressed();
        let parent_tick_label = self.stack.last().is_some_and(|f| f.tick_label);
        let parent_title = self.stack.last().is_some_and(|f| f.title);
        let mut frame = Frame {
            wrote: false,
            suppress: parent_suppress,
            plot: false,
            axis: false,
            mark: false,
            tick_label: parent_tick_label,
            title: parent_title || g.role == Role::Title,
        };
        match g.role {
            Role::Plot => {
                if let Some(clip) = g.clip {
                    if self.panel.is_none() {
                        self.panel = Some(clip);
                    }
                    self.grid(&clip);
                }
                frame.plot = true;
            }
            Role::Mark => {
                frame.mark = true;
                let info = g.layer.and_then(|i| self.hooks.layers.get(i));
                match info.and_then(|l| l.element.map(|e| (e, l))) {
                    Some((e, info)) => {
                        if self.open_element != Some(e) {
                            self.catch_up(e);
                            self.open_element_group(e);
                            self.open_element = Some(e);
                            self.next_element = e + 1;
                        }
                        self.marker = info.marker.clone();
                    }
                    None => frame.suppress = true,
                }
            }
            Role::Axis(kind) => {
                self.axis = Some(kind);
                frame.axis = true;
            }
            Role::Tick => {
                frame.suppress |= self.axis_hooks().is_some_and(|a| a.marks_hidden);
            }
            Role::AxisLine => {
                frame.suppress |= self.axis_hooks().is_some_and(|a| a.line_hidden);
            }
            Role::TickLabel => {
                frame.suppress |= self.axis_hooks().is_some_and(|a| a.labels_hidden);
                frame.tick_label = true;
                self.tick_labels = 0;
            }
            // Charton gets no axis titles; the writer draws them, see `AxisHooks`
            Role::AxisTitle => frame.suppress = true,
            _ => {}
        }
        // a series is a scope of Charton's own; the element group says what a host needs
        let write = !frame.suppress && !matches!(g.role, Role::Series);
        if write {
            let b = &mut self.body;
            b.push_str("<g class=\"");
            b.push_str(g.role.class());
            if let Some(q) = g.role.qualifier() {
                b.push(' ');
                b.push_str(q);
            }
            if let (Role::Mark, Some(mark)) = (g.role, g.mark) {
                b.push_str(" charton-mark-");
                escape(b, mark);
            }
            b.push('"');
            if let Role::Axis(kind) = g.role {
                let _ = write!(b, " data-axis=\"{}\"", kind.data_value());
            }
            if let Some(clip) = g.clip {
                let id = format!("{}-clip-{}", self.prefix, self.clips);
                self.clips += 1;
                let d = &mut self.defs;
                d.push_str("<clipPath id=\"");
                d.push_str(&id);
                d.push_str("\"><rect");
                attr_num(d, "x", clip.x);
                attr_num(d, "y", clip.y);
                attr_num(d, "width", clip.width);
                attr_num(d, "height", clip.height);
                d.push_str("/></clipPath>");
                let b = &mut self.body;
                b.push_str(" clip-path=\"url(#");
                b.push_str(&id);
                b.push_str(")\"");
            }
            self.body.push('>');
            frame.wrote = true;
        }
        self.stack.push(frame);
    }

    fn end_group(&mut self) {
        let Some(frame) = self.stack.pop() else {
            return;
        };
        if frame.plot {
            self.catch_up(self.hooks.elements.len());
        }
        if frame.axis {
            if frame.wrote {
                self.axis_title();
            }
            self.axis = None;
        }
        if frame.mark {
            self.marker = None;
        }
        if frame.wrote {
            self.body.push_str("</g>");
        }
    }

    fn begin_element(&mut self, _element: ChartonElement<'_>) {}
}

/// A backend which draws nothing and records the panel of a chart: the clip of its first plot
/// group. The figure lays out a plot from where Charton puts its panel with margins of 0 (the
/// bands of its axes and title), so it places every panel where its layout wants it.
#[derive(Debug, Default)]
pub(crate) struct PanelProbe {
    /// The panel, `None` until Charton opened a plot group.
    pub panel: Option<Rect>,
}

impl RenderBackend for PanelProbe {
    fn draw_circle(&mut self, _: CircleConfig) {}
    fn draw_rect(&mut self, _: RectConfig) {}
    fn draw_path(&mut self, _: PathConfig) {}
    fn draw_text(&mut self, _: TextConfig) -> Result<(), ChartonError> {
        Ok(())
    }
    fn draw_line(&mut self, _: LineConfig) {}
    fn draw_gradient_rect(&mut self, _: GradientRectConfig) {}
    fn begin_group(&mut self, g: &Group<'_>) {
        if g.role == Role::Plot && self.panel.is_none() {
            self.panel = g.clip;
        }
    }
    fn end_group(&mut self) {}
    fn begin_element(&mut self, _: ChartonElement<'_>) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers() {
        let mut s = String::new();
        for v in [0.0, -0.001, 1.0, 1.5, 1.25, -2.015, 12.3456, 1e6] {
            num(&mut s, v);
            s.push(' ');
        }
        assert_eq!(s, "0 0 1 1.5 1.25 -2.02 12.35 1000000 ");
    }

    #[test]
    fn spaces_are_kept() {
        let mut s = String::new();
        text_node(&mut s, " A  Circuit ");
        assert_eq!(s, "\u{a0}A \u{a0}Circuit\u{a0}");
        let mut s = String::new();
        text_node(&mut s, "a b & c");
        assert_eq!(s, "a b &amp; c");
    }

    #[test]
    fn escaping() {
        let mut s = String::new();
        escape(&mut s, "<a & \"µ\u{1}\">");
        assert_eq!(s, "&lt;a &amp; &quot;µ\u{fffd}&quot;&gt;");
    }
}
