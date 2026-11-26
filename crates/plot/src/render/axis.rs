//! The axes of a plot: their limits, their ticks and the labels of the ticks.
//!
//! The rules follow `plot.ts` of sbmlsim-web (and with it `sbmlsim.plot`), so a figure of the
//! renderer and a panel of the web app have the same axes:
//!
//! - a linear axis spans its values with the margin [`AXIS_MARGIN`] of their span on both
//!   sides, a span of 0 is opened to 5 % of the value (or to 1 at 0), and no value gives
//!   `[0, 1]`,
//! - a logarithmic axis spans at most `max_decades` decades (6 by default) below its largest
//!   positive value with the margin of its decades, one value takes the decade below and the
//!   decade above it, and no positive value gives `[0.1, 1]`; a value which is not positive is
//!   no point of it,
//! - a category axis spans its categories from `-0.5` to `n - 0.5`,
//! - `min` and `max` are the ends exactly, and with one of them the other end is derived from
//!   the values cut at it,
//! - a `frame` is the values the axis is derived from instead of the drawn ones.
//!
//! Charton draws every axis as a linear axis of the coordinate `u`, which is the value, its
//! `log10` on a logarithmic axis, and its negative on a reversed axis, with the ticks this module
//! computes. Every transcendental function goes through `libm`, so the native and the wasm
//! build compute the same bits.

use crate::spec::{Axis, AxisScale};

/// The margin of the limits of an axis, a share of its span on both sides (matplotlib's).
pub(crate) const AXIS_MARGIN: f64 = 0.05;
/// The decades of data a logarithmic axis shows when its spec gives none.
pub(crate) const LOG_DECADES: f64 = 6.0;
/// The fewest ticks of an axis.
const LEAST_TICKS: f64 = 3.0;
/// The most ticks of an axis.
const MOST_TICKS: f64 = 8.0;
/// The pixels one tick of the x axis is given.
pub(crate) const X_TICK_ROOM: f64 = 70.0;
/// The pixels one tick of the y axis is given.
pub(crate) const Y_TICK_ROOM: f64 = 50.0;
/// The most decimals of a plain tick label; an axis which needs more is exponential.
const PLAIN_DECIMALS: i32 = 6;
/// The digits after the point of the mantissa of an exponential tick label at least.
const EXPONENTIAL_DIGITS: i32 = 2;
/// The most digits a tick label grows to.
const MOST_DIGITS: i32 = 16;

/// `log10` of the crate, the same bits in every build.
pub(crate) fn log10(v: f64) -> f64 {
    libm::log10(v)
}

/// `10^e` of the crate, the same bits in every build.
pub(crate) fn pow10(e: f64) -> f64 {
    libm::pow(10.0, e)
}

/// How the values of an axis become its coordinate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Transform {
    /// The value.
    Linear,
    /// `log10` of the value; a value which is not positive has no coordinate.
    Log,
    /// The position of a category, 0, 1, ...
    Category,
}

/// An axis with its limits and ticks resolved.
#[derive(Debug, Clone)]
pub(crate) struct ResolvedAxis {
    /// How a value becomes its coordinate.
    pub transform: Transform,
    /// Whether the axis runs from its upper to its lower limit.
    pub reverse: bool,
    /// The lower limit in data units.
    pub low: f64,
    /// The upper limit in data units, above `low`.
    pub high: f64,
    /// The ticks in data units with their labels.
    pub ticks: Vec<(f64, String)>,
    /// The multiplier of the labels, `×10^{n}`, written at the end of the axis (since 0.31.0).
    pub multiplier: Option<String>,
}

impl ResolvedAxis {
    /// The coordinate of a value, NaN for a value the axis cannot show.
    pub fn u(&self, v: f64) -> f64 {
        let t = match self.transform {
            Transform::Log if v > 0.0 && v.is_finite() => log10(v),
            Transform::Log => f64::NAN,
            Transform::Linear | Transform::Category if v.is_finite() => v,
            Transform::Linear | Transform::Category => f64::NAN,
        };
        if self.reverse { -t } else { t }
    }

    /// The coordinates of the ends, the lower one first.
    pub fn domain(&self) -> (f64, f64) {
        let (a, b) = (self.u(self.low), self.u(self.high));
        if a <= b { (a, b) } else { (b, a) }
    }

    /// Where a coordinate lies along the axis, 0 at the start of its domain and 1 at its end.
    pub fn share(&self, u: f64) -> f64 {
        let (lo, hi) = self.domain();
        if hi > lo { (u - lo) / (hi - lo) } else { 0.5 }
    }

    /// The ticks as coordinates with their labels, inside the domain.
    ///
    /// A tick outside of the domain is dropped. The tick rules keep a tick within a relative
    /// 1e-9 of an end (a tick at an exact end computes as `24.000000000000004` or as the `log10`
    /// of a decade a bit past it), which is set onto the end, so the end keeps its tick and
    /// Charton is never given a tick outside of its domain.
    pub fn tick_coordinates(&self) -> Vec<(f64, String)> {
        let (lo, hi) = self.domain();
        // a relative 1e-9 of a decade is an absolute 4.3e-10 of its log10
        let slack = match self.transform {
            Transform::Log => ((hi - lo) * 1e-9).max(1e-9),
            Transform::Linear | Transform::Category => (hi - lo) * 1e-9,
        };
        self.ticks
            .iter()
            .filter_map(|(v, label)| {
                let u = self.u(*v);
                (u >= lo - slack && u <= hi + slack).then(|| (u.clamp(lo, hi), label.clone()))
            })
            .collect()
    }
}

/// The text of an axis title: `<label> [<unit>]`.
pub(crate) fn axis_title(axis: &Axis) -> String {
    let label = axis.label.as_deref().unwrap_or("");
    match axis.unit.as_deref() {
        Some(u) if !u.is_empty() && !label.is_empty() => format!("{label} [{u}]"),
        Some(u) if !u.is_empty() => format!("[{u}]"),
        _ => label.to_string(),
    }
}

/// Resolves an axis over the values drawn against it.
///
/// `length` is the pixels the axis is expected to be long, `room` the pixels one tick is given.
/// `sticky` are the values an end the axis derives sticks to instead of its margin when no
/// value lies beyond them (the base of bars, matplotlib's sticky edges).
pub(crate) fn resolve(
    axis: &Axis,
    values: &[f64],
    sticky: &[f64],
    length: f64,
    room: f64,
) -> ResolvedAxis {
    let count = tick_count(length, room);
    match axis.scale {
        AxisScale::Category => {
            let names = axis.categories.as_deref().unwrap_or(&[]);
            let n = names.len().max(1) as f64;
            let (low, high) = ends_of(&[-0.5, n - 0.5], axis.min, axis.max);
            let ticks = names
                .iter()
                .enumerate()
                .map(|(i, name)| (i as f64, name.clone()))
                .filter(|(i, _)| *i >= low && *i <= high)
                .collect();
            ResolvedAxis {
                transform: Transform::Category,
                reverse: axis.reverse,
                low,
                high,
                ticks,
                multiplier: None,
            }
        }
        AxisScale::Linear | AxisScale::Log => {
            let log = axis.scale == AxisScale::Log;
            let decades = axis.max_decades.unwrap_or(LOG_DECADES);
            let framed;
            let values = match axis.frame {
                Some(r) => {
                    framed = [r.low, r.high];
                    &framed[..]
                }
                None => values,
            };
            let derive = |v: &[f64]| {
                if log {
                    log_limits(v, decades)
                } else {
                    linear_limits(v)
                }
            };
            let (mut low, mut high) = limits_of(values, log, axis.min, axis.max, derive);
            if axis.frame.is_none() {
                (low, high) = stick(values, sticky, log, (axis.min, axis.max), (low, high));
            }
            let ticks = if log {
                log_ticks(low, high, count)
            } else {
                linear_ticks(low, high, count)
            };
            // the given ticks with their labels (since 0.31.0), else the computed ones
            let ticks = axis.ticks.clone().unwrap_or(ticks);
            let (labels, multiplier) = match &axis.tick_labels {
                Some(l) => (l.clone(), None),
                None => axis_labels(&ticks, log, true),
            };
            let ticks = if axis.ticks.is_some() {
                ticks.into_iter().zip(labels).collect()
            } else {
                distinct_ticks(ticks, labels)
            };
            ResolvedAxis {
                transform: if log {
                    Transform::Log
                } else {
                    Transform::Linear
                },
                reverse: axis.reverse,
                low,
                high,
                ticks,
                multiplier,
            }
        }
    }
}

/// The axes of a panel of `(width, height)` pixels with an aspect (since 0.30.0): the limits of
/// one of them widened about their middle until one unit of the coordinate of y is `ratio`
/// times as long as one unit of x, and its ticks resolved again for the new limits.
///
/// The axis whose units are the longer ones is widened, so the data keeps its room on both
/// (matplotlib's `adjustable="datalim"`); on a log axis a unit of the coordinate is a decade.
/// `x_axis` and `y_axis` are the axes of the spec the resolved ones were resolved from.
pub(crate) fn with_aspect(
    (x_axis, y_axis): (&Axis, &Axis),
    (xa, ya): (ResolvedAxis, ResolvedAxis),
    ratio: f64,
    (width, height): (f64, f64),
) -> (ResolvedAxis, ResolvedAxis) {
    let (x0, x1) = xa.domain();
    let (y0, y1) = ya.domain();
    let (dx, dy) = (x1 - x0, y1 - y0);
    if !(dx > 0.0 && dy > 0.0 && width > 0.0 && height > 0.0 && ratio > 0.0) {
        return (xa, ya);
    }
    // the span of y which gives the aspect with the span of x, and the other way round
    let dy_wanted = height * dx / (ratio * width);
    if dy_wanted > dy {
        let ya = widened(y_axis, &ya, dy_wanted, height, Y_TICK_ROOM);
        (xa, ya)
    } else {
        let dx_wanted = ratio * width * dy / height;
        let xa = widened(x_axis, &xa, dx_wanted, width, X_TICK_ROOM);
        (xa, ya)
    }
}

/// An axis resolved again with its limits widened about their middle to a span `wanted` of
/// its coordinate (without a reversal: the value, or its `log10` on a log axis).
fn widened(
    axis: &Axis,
    resolved: &ResolvedAxis,
    wanted: f64,
    length: f64,
    room: f64,
) -> ResolvedAxis {
    let log = resolved.transform == Transform::Log;
    let to = |v: f64| if log { log10(v) } else { v };
    let from = |t: f64| if log { pow10(t) } else { t };
    let middle = (to(resolved.low) + to(resolved.high)) / 2.0;
    let (low, high) = (from(middle - wanted / 2.0), from(middle + wanted / 2.0));
    if !(low.is_finite() && high.is_finite() && low < high) {
        return resolved.clone();
    }
    let ends = Axis {
        min: Some(low),
        max: Some(high),
        frame: None,
        ..axis.clone()
    };
    resolve(&ends, &[], &[], length, room)
}

/// The ticks with their labels, a tick whose label is the one of the tick before dropped: at a
/// resolution near the precision of a double neighbouring ticks round onto one value or one
/// label, which would be written twice.
fn distinct_ticks(ticks: Vec<f64>, labels: Vec<String>) -> Vec<(f64, String)> {
    let mut out: Vec<(f64, String)> = Vec::with_capacity(ticks.len());
    for (t, l) in ticks.into_iter().zip(labels) {
        if out.last().is_none_or(|(_, last)| *last != l) {
            out.push((t, l));
        }
    }
    out
}

/// The limits of an axis with the ends it derives moved onto its sticky values.
///
/// An end which is not given and whose margin passes a sticky value with no value beyond it
/// is set onto that value, as long as the limits stay a span; a sticky value a log axis cannot
/// show is not taken.
fn stick(
    values: &[f64],
    sticky: &[f64],
    log: bool,
    (min, max): (Option<f64>, Option<f64>),
    (mut low, mut high): (f64, f64),
) -> (f64, f64) {
    let shown = |v: &f64| v.is_finite() && (!log || *v > 0.0);
    let (mut smallest, mut largest) = (f64::INFINITY, f64::NEG_INFINITY);
    for v in values.iter().filter(|v| shown(v)) {
        smallest = smallest.min(*v);
        largest = largest.max(*v);
    }
    for s in sticky.iter().filter(|v| shown(v)) {
        if min.is_none() && low < *s && *s <= smallest && *s < high {
            low = *s;
        }
        if max.is_none() && high > *s && *s >= largest && *s > low {
            high = *s;
        }
    }
    (low, high)
}

/// The ends of a category axis: the given ones, else the span of the categories.
fn ends_of(span: &[f64; 2], min: Option<f64>, max: Option<f64>) -> (f64, f64) {
    let low = min.unwrap_or(span[0]);
    let high = max.unwrap_or(span[1]);
    if low < high {
        (low, high)
    } else {
        (span[0], span[1])
    }
}

/// How many ticks an axis of a length is given, between 3 and 8.
fn tick_count(length: f64, room: f64) -> usize {
    (length / room).round().clamp(LEAST_TICKS, MOST_TICKS) as usize
}

/// The limits of a linear axis over its values.
fn linear_limits(values: &[f64]) -> (f64, f64) {
    let (mut low, mut high) = (f64::INFINITY, f64::NEG_INFINITY);
    for &v in values {
        if v.is_finite() {
            low = low.min(v);
            high = high.max(v);
        }
    }
    if !low.is_finite() || !high.is_finite() {
        return (0.0, 1.0);
    }
    let margin = if high > low {
        (high - low) * AXIS_MARGIN
    } else if low != 0.0 {
        low.abs() * AXIS_MARGIN
    } else {
        1.0
    };
    (low - margin, high + margin)
}

/// The decade below and the decade above a positive value.
fn decade_around(value: f64) -> (f64, f64) {
    let e = log10(value);
    let below = pow10(e.floor());
    let above = pow10(e.ceil());
    (
        if below < value { below } else { below / 10.0 },
        if above > value { above } else { above * 10.0 },
    )
}

/// The limits of a logarithmic axis over its values, at most `decades` below the largest.
fn log_limits(values: &[f64], decades: f64) -> (f64, f64) {
    let (mut smallest, mut largest) = (f64::INFINITY, f64::NEG_INFINITY);
    for &v in values {
        if v.is_finite() && v > 0.0 {
            smallest = smallest.min(v);
            largest = largest.max(v);
        }
    }
    if !smallest.is_finite() || !largest.is_finite() {
        return (0.1, 1.0);
    }
    let bottom = smallest.max(largest / pow10(decades));
    let (low, high) = if largest > bottom {
        (bottom, largest)
    } else {
        decade_around(largest)
    };
    let margin = pow10(AXIS_MARGIN * log10(high / low));
    (low / margin, high * margin)
}

/// The limits of an axis of its values and the ends it is given.
///
/// An end which is given is the end exactly; with one end given, the other is derived from the
/// values cut at it and the end itself. On a logarithmic axis an end which is not positive is
/// not taken, and a pair which is no span is not taken either.
fn limits_of(
    values: &[f64],
    log: bool,
    min: Option<f64>,
    max: Option<f64>,
    derive: impl Fn(&[f64]) -> (f64, f64),
) -> (f64, f64) {
    let usable = |e: Option<f64>| e.filter(|v| v.is_finite() && (!log || *v > 0.0));
    let (mut low, mut high) = (usable(min), usable(max));
    if let (Some(l), Some(h)) = (low, high) {
        if l < h {
            return (l, h);
        }
        low = None;
        high = None;
    }
    let kept = values
        .iter()
        .copied()
        .filter(|v| v.is_finite() && (!log || *v > 0.0));
    match (low, high) {
        (None, Some(top)) => {
            let cut: Vec<f64> = kept.map(|v| v.min(top)).chain([top]).collect();
            (derive(&cut).0, top)
        }
        (Some(bottom), None) => {
            let cut: Vec<f64> = kept.map(|v| v.max(bottom)).chain([bottom]).collect();
            (bottom, derive(&cut).1)
        }
        _ => derive(values),
    }
}

/// The step of the ticks of a linear axis: 1, 2 or 5 times a power of ten.
fn nice_step(span: f64, count: usize) -> f64 {
    let raw = span / count.max(1) as f64;
    let magnitude = pow10(log10(raw).floor());
    let share = raw / magnitude;
    let step = if share <= 1.5 {
        1.0
    } else if share <= 3.0 {
        2.0
    } else if share <= 7.0 {
        5.0
    } else {
        10.0
    };
    step * magnitude
}

/// The ticks of a linear axis inside its limits.
fn linear_ticks(low: f64, high: f64, count: usize) -> Vec<f64> {
    let span = high - low;
    if span.is_nan() || span <= 0.0 {
        return vec![low];
    }
    let step = nice_step(span, count);
    let first = (low / step - 1e-9).ceil();
    let mut ticks = Vec::new();
    for i in 0..1000 {
        let v = (first + i as f64) * step;
        if v > high + step * 1e-9 {
            break;
        }
        ticks.push(v);
    }
    ticks
}

/// The values of a ladder of steps per decade inside the limits.
fn decade_ladder(low: f64, high: f64, steps: &[f64]) -> Vec<f64> {
    let mut values = Vec::new();
    let first = log10(low).floor() as i32;
    let last = log10(high).ceil() as i32;
    for e in first..=last {
        if values.len() >= 1000 {
            break;
        }
        let decade = pow10(e as f64);
        for s in steps {
            let v = s * decade;
            if v >= low * (1.0 - 1e-9) && v <= high * (1.0 + 1e-9) {
                values.push(v);
            }
        }
    }
    values
}

/// The ticks of a logarithmic axis: the finest of the ladders 1-2-5, 1-3 and the decades which
/// is no longer than asked for, every k-th decade where even those are too many.
fn log_ticks(low: f64, high: f64, count: usize) -> Vec<f64> {
    let ladders: Vec<Vec<f64>> = [&[1.0, 2.0, 5.0][..], &[1.0, 3.0], &[1.0]]
        .iter()
        .map(|steps| decade_ladder(low, high, steps))
        .filter(|l| !l.is_empty())
        .collect();
    if ladders.is_empty() {
        return vec![low, high];
    }
    let mut i = 0;
    while i < ladders.len() - 1 && ladders[i].len() > count {
        i += 1;
    }
    if (ladders[i].len() as f64) < LEAST_TICKS && i > 0 {
        return ladders[i - 1].clone();
    }
    let fitted = &ladders[i];
    if fitted.len() <= count {
        return fitted.clone();
    }
    let every = fitted.len().div_ceil(count).max(1);
    fitted.iter().copied().step_by(every).collect()
}

/// Whether the labels of the ticks of a log axis are written with powers of ten: a tick of
/// 1e5 or more, or below 1e-3.
fn exponential_ticks(ticks: &[f64]) -> bool {
    ticks.iter().any(|&v| {
        let m = v.abs();
        v != 0.0 && v.is_finite() && (m >= 1e5 || m < 1e-3)
    })
}

/// The decimals the labels of the ticks need: of their smallest step and of their smallest.
fn tick_decimals(ticks: &[f64]) -> i32 {
    let smallest = ticks
        .iter()
        .filter(|v| **v != 0.0 && v.is_finite())
        .map(|v| v.abs())
        .fold(f64::INFINITY, f64::min);
    let mut step = ticks
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(f64::INFINITY, f64::min);
    if !step.is_finite() {
        step = match ticks.first() {
            Some(v) if *v != 0.0 => v.abs(),
            _ => 1.0,
        };
    }
    let of = |v: f64| (-log10(v)).ceil() as i32;
    let needed = of(step).max(if smallest.is_finite() {
        of(smallest)
    } else {
        0
    });
    needed.clamp(0, MOST_DIGITS)
}

/// The exponent of a power of ten as a label writes it, with the minus sign U+2212.
fn exponent(n: i64) -> String {
    if n < 0 {
        format!("\u{2212}{}", -n)
    } else {
        n.to_string()
    }
}

/// `10^{n}`, a power of ten with its exponent as a superscript (see `text::runs`).
fn power(n: i64) -> String {
    format!("10^{{{}}}", exponent(n))
}

/// A number of a label without trailing zeros, `0` for zero in either form: plain with
/// `digits` decimals, or with `scientific` as `m×10^{n}` with `digits` decimals of the
/// mantissa, `10^{n}` for a mantissa of 1 (matplotlib's `LogFormatterSciNotation`).
pub(crate) fn format_number(value: f64, digits: i32, scientific: bool) -> String {
    if !value.is_finite() || value == 0.0 {
        return "0".into();
    }
    let digits = digits.clamp(0, MOST_DIGITS) as usize;
    let text = if scientific {
        let s = format!("{value:.digits$e}");
        let (mantissa, e) = s.split_once('e').unwrap_or((&s, "0"));
        let n: i64 = e.parse().unwrap_or(0);
        match trim_zeros(mantissa) {
            "1" => power(n),
            "-1" => format!("-{}", power(n)),
            m => format!("{m}\u{d7}{}", power(n)),
        }
    } else {
        trim_zeros(&format!("{value:.digits$}")).to_string()
    };
    if text == "-0" { "0".into() } else { text }
}

/// A decimal without its trailing zeros and without a trailing point.
fn trim_zeros(s: &str) -> &str {
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.')
    } else {
        s
    }
}

/// The labels of ticks written with `digits` decimals and more, grown digit by digit until
/// neighbours differ.
fn grown(ticks: &[f64], mut digits: i32, scientific: bool) -> Vec<String> {
    let write = |d: i32| -> Vec<String> {
        ticks
            .iter()
            .map(|v| format_number(*v, d, scientific))
            .collect()
    };
    let mut labels = write(digits);
    while digits < MOST_DIGITS
        && ticks
            .windows(2)
            .zip(labels.windows(2))
            .any(|(t, l)| t[0] != t[1] && l[0] == l[1])
    {
        digits += 1;
        labels = write(digits);
    }
    labels
}

/// The order of magnitude of the multiplier of the ticks of a linear axis, matplotlib's
/// `ScalarFormatter` with its `axes.formatter.limits` of (-5, 6): of the largest tick, when it
/// is at least 1e6 or below 1e-4.
fn order_of_magnitude(ticks: &[f64]) -> Option<i64> {
    let largest = ticks
        .iter()
        .filter(|v| v.is_finite())
        .map(|v| v.abs())
        .fold(0.0, f64::max);
    if largest == 0.0 {
        return None;
    }
    let oom = log10(largest).floor() as i64;
    (oom >= 6 || oom <= -5).then_some(oom)
}

/// Whether a value is a power of ten, within a relative 1e-9.
fn is_decade(v: f64) -> bool {
    v > 0.0 && {
        let e = log10(v);
        (e - e.round()).abs() < 1e-9
    }
}

/// The labels of the ticks of an axis and their multiplier (since 0.31.0).
///
/// - A log axis whose ticks are decades writes them `10^{n}`, as matplotlib's
///   `LogFormatterSciNotation`; one of other ticks (the ladders 1-2-5 and 1-3) whose ticks
///   reach 1e5 or below 1e-3 writes them `m×10^{n}` and its decades `10^{n}`, else plain.
/// - A linear axis whose largest tick is at least 1e6 or below 1e-4 writes the ticks divided by
///   the power of ten of that tick, and with `multiplier` that power as the multiplier
///   `×10^{n}` of the axis, else every label with it (`m×10^{n}`), as matplotlib's
///   `ScalarFormatter`; ticks which need more than 6 decimals are written `m×10^{n}`; the
///   others plain.
///
/// The labels grow digit by digit until neighbours differ.
pub(crate) fn axis_labels(
    ticks: &[f64],
    log: bool,
    multiplier: bool,
) -> (Vec<String>, Option<String>) {
    let decimals = tick_decimals(ticks);
    if log {
        if !ticks.is_empty() && ticks.iter().all(|v| is_decade(*v)) {
            let labels = ticks
                .iter()
                .map(|v| power(log10(*v).round() as i64))
                .collect();
            return (labels, None);
        }
        if exponential_ticks(ticks) || decimals > PLAIN_DECIMALS {
            return (grown(ticks, 0, true), None);
        }
        return (grown(ticks, decimals, false), None);
    }
    if let Some(oom) = order_of_magnitude(ticks) {
        let scale = pow10(oom as f64);
        let scaled: Vec<f64> = ticks.iter().map(|v| v / scale).collect();
        let labels = grown(&scaled, tick_decimals(&scaled), false);
        let factor = power(oom);
        if multiplier {
            return (labels, Some(format!("\u{d7}{factor}")));
        }
        let labels = labels
            .into_iter()
            .map(|l| match l.as_str() {
                "0" => l,
                "1" => factor.clone(),
                _ => format!("{l}\u{d7}{factor}"),
            })
            .collect();
        return (labels, None);
    }
    if decimals > PLAIN_DECIMALS {
        return (grown(ticks, EXPONENTIAL_DIGITS, true), None);
    }
    (grown(ticks, decimals, false), None)
}

/// The labels of ticks of a linear scale without a multiplier of the axis, each with its power
/// of ten where the axis would have one (see [`axis_labels`]): the labels of the given ticks
/// of a colour bar.
#[cfg(test)]
pub(crate) fn tick_labels(ticks: &[f64]) -> Vec<String> {
    axis_labels(ticks, false, false).0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels() {
        assert_eq!(tick_labels(&[0.0, 0.5, 1.0]), ["0", "0.5", "1"]);
        assert_eq!(format_number(-0.0001, 2, false), "0");
        // the decades of a log axis as powers of ten, the others plain or with their power
        let (l, m) = axis_labels(&[1e-4, 1e-3, 1e-2, 1e-1], true, true);
        assert_eq!(
            l,
            [
                "10^{\u{2212}4}",
                "10^{\u{2212}3}",
                "10^{\u{2212}2}",
                "10^{\u{2212}1}"
            ]
        );
        assert_eq!(m, None);
        assert_eq!(
            axis_labels(&[0.001, 1.0], true, true).0,
            ["10^{\u{2212}3}", "10^{0}"]
        );
        assert_eq!(
            axis_labels(&[0.5, 1.0, 2.0, 5.0], true, true).0,
            ["0.5", "1", "2", "5"]
        );
        assert_eq!(
            axis_labels(&[1e-4, 2e-4, 5e-4, 1e-3], true, true).0,
            [
                "10^{\u{2212}4}",
                "2\u{d7}10^{\u{2212}4}",
                "5\u{d7}10^{\u{2212}4}",
                "10^{\u{2212}3}"
            ]
        );
        // a linear axis of large values: the mantissas and the multiplier
        let (l, m) = axis_labels(&[0.0, 2e6, 4e6], false, true);
        assert_eq!(l, ["0", "2", "4"]);
        assert_eq!(m.as_deref(), Some("\u{d7}10^{6}"));
        assert_eq!(
            tick_labels(&[0.0, 2e6, 4e6]),
            ["0", "2\u{d7}10^{6}", "4\u{d7}10^{6}"]
        );
        // below 1e6 and above 1e-5 plain, as matplotlib
        assert_eq!(tick_labels(&[0.0, 2e5, 4e5]), ["0", "200000", "400000"]);
        assert_eq!(
            tick_labels(&[0.0, 0.0001, 0.0002]),
            ["0", "0.0001", "0.0002"]
        );
        let (l, m) = axis_labels(&[0.0, 2e-5, 4e-5], false, true);
        assert_eq!(l, ["0", "2", "4"]);
        assert_eq!(m.as_deref(), Some("\u{d7}10^{\u{2212}5}"));
    }

    #[test]
    fn limits() {
        assert_eq!(linear_limits(&[]), (0.0, 1.0));
        assert_eq!(linear_limits(&[0.0, 0.0]), (-1.0, 1.0));
        let (lo, hi) = log_limits(&[1e-12, 1e2], 6.0);
        assert!(lo < 1e-4 && lo > 1e-5 && hi > 1e2);
        let (lo, hi) = log_limits(&[5.0], 6.0);
        assert!(lo < 1.0 && hi > 10.0);
        let ends = limits_of(&[1.0, 30.0], false, Some(0.0), Some(24.0), linear_limits);
        assert_eq!(ends, (0.0, 24.0));
        let ends = limits_of(&[1.0, 30.0], false, Some(0.0), None, linear_limits);
        assert_eq!(ends.0, 0.0);
        assert!(ends.1 > 30.0);
    }

    #[test]
    fn sticky_ends() {
        // bars from 0: the lower end sticks to 0 instead of its margin below it
        let (lo, hi) = stick(&[0.0, 3.0], &[0.0], false, (None, None), (-0.15, 3.15));
        assert_eq!((lo, hi), (0.0, 3.15));
        // a value below the base keeps the margin, a given end is kept
        let ends = stick(&[-1.0, 3.0], &[0.0], false, (None, None), (-1.2, 3.2));
        assert_eq!(ends, (-1.2, 3.2));
        let ends = stick(&[0.0, 3.0], &[0.0], false, (Some(-1.0), None), (-1.0, 3.15));
        assert_eq!(ends, (-1.0, 3.15));
        // bars all of height 0 keep a span; a log axis takes no sticky 0
        assert_eq!(
            stick(&[0.0], &[0.0], false, (None, None), (-1.0, 1.0)),
            (0.0, 1.0)
        );
        assert_eq!(
            stick(&[1.0], &[0.0], true, (None, None), (0.1, 10.0)),
            (0.1, 10.0)
        );
    }

    #[test]
    fn ticks() {
        assert_eq!(
            linear_ticks(0.0, 1.0, 5),
            [0.0, 0.2, 0.4, 0.6000000000000001, 0.8, 1.0]
        );
        // a step below the resolution of a double: the ticks round onto the same values, each
        // label is written once
        let t = linear_ticks(1e16, 1e16 + 8.0, 8);
        let labels = tick_labels(&t);
        assert!(labels.windows(2).any(|w| w[0] == w[1]), "{labels:?}");
        let kept = distinct_ticks(t, labels);
        assert!(kept.windows(2).all(|w| w[0].1 != w[1].1), "{kept:?}");
        assert!(kept.len() >= 2, "{kept:?}");
        let t = log_ticks(5e-5, 200.0, 8);
        assert_eq!(t.first().copied(), Some(1e-4));
        assert!(t.len() <= 8);
    }
}
