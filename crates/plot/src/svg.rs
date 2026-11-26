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

use crate::spec::{FontWeight, MarkerType};
use crate::text::{LINE_HEIGHT, line_count, lines};

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

/// Writes a marker glyph centred at a point.
pub(crate) fn glyph(b: &mut String, m: &MarkerGlyph, x: f64, y: f64) {
    let r = m.size / 2.0;
    // a filled glyph: its fill and, with an edge, its stroke
    let paint = |b: &mut String| {
        let _ = write!(b, " fill=\"{}\"", m.fill);
        if m.edge_width > 0.0 {
            let _ = write!(b, " stroke=\"{}\"", m.edge);
            attr_num(b, "stroke-width", m.edge_width);
        }
        attr_opacity(b, "opacity", m.opacity);
        b.push_str("/>");
    };
    match m.kind {
        MarkerType::None => {}
        MarkerType::Circle => {
            b.push_str("<circle");
            attr_num(b, "cx", x);
            attr_num(b, "cy", y);
            attr_num(b, "r", r);
            paint(b);
        }
        MarkerType::Square => {
            // a square of the area of the circle
            let s = r * 0.886;
            b.push_str("<rect");
            attr_num(b, "x", x - s);
            attr_num(b, "y", y - s);
            attr_num(b, "width", 2.0 * s);
            attr_num(b, "height", 2.0 * s);
            paint(b);
        }
        MarkerType::Triangle | MarkerType::Diamond => {
            let points: &[(f64, f64)] = if m.kind == MarkerType::Triangle {
                &[(0.0, -1.15), (1.0, 0.58), (-1.0, 0.58)]
            } else {
                &[(0.0, -1.0), (0.75, 0.0), (0.0, 1.0), (-0.75, 0.0)]
            };
            b.push_str("<path d=\"");
            for (i, (dx, dy)) in points.iter().enumerate() {
                b.push(if i == 0 { 'M' } else { 'L' });
                num(b, x + dx * r);
                b.push(' ');
                num(b, y + dy * r);
            }
            b.push_str("Z\"");
            paint(b);
        }
        MarkerType::Cross | MarkerType::X | MarkerType::Plus => {
            let d = if m.kind == MarkerType::Plus {
                r
            } else {
                r * std::f64::consts::FRAC_1_SQRT_2
            };
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
            // a tick label is one line (as the layout measures it and 0.27.0 wrote it), a
            // newline in it written as it is
            text_node(b, &c.text);
        } else {
            text_lines(
                b,
                &c.text,
                (f64::from(c.x), f64::from(c.y)),
                f64::from(c.font_size),
                stack,
            );
        }
        b.push_str("</text>");
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
