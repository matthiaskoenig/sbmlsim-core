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
    pub fn tick_coordinates(&self) -> Vec<(f64, String)> {
        let (lo, hi) = self.domain();
        self.ticks
            .iter()
            .filter_map(|(v, label)| {
                let u = self.u(*v);
                u.is_finite().then(|| (u.clamp(lo, hi), label.clone()))
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
pub(crate) fn resolve(axis: &Axis, values: &[f64], length: f64, room: f64) -> ResolvedAxis {
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
            let (low, high) = limits_of(values, log, axis.min, axis.max, derive);
            let ticks = if log {
                log_ticks(low, high, count)
            } else {
                linear_ticks(low, high, count)
            };
            let labels = tick_labels(&ticks);
            ResolvedAxis {
                transform: if log {
                    Transform::Log
                } else {
                    Transform::Linear
                },
                reverse: axis.reverse,
                low,
                high,
                ticks: ticks.into_iter().zip(labels).collect(),
            }
        }
    }
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

/// Whether the labels of the ticks are written in the exponential form.
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

/// A number of a label without trailing zeros, `0` for zero in either form.
pub(crate) fn format_number(value: f64, digits: i32, exponential: bool) -> String {
    if !value.is_finite() || value == 0.0 {
        return "0".into();
    }
    let digits = digits.clamp(0, MOST_DIGITS) as usize;
    let text = if exponential {
        let s = format!("{value:.digits$e}");
        let (mantissa, exponent) = s.split_once('e').unwrap_or((&s, "0"));
        let exponent = if exponent.starts_with('-') {
            exponent.to_string()
        } else {
            format!("+{exponent}")
        };
        format!("{}e{exponent}", trim_zeros(mantissa))
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

/// The labels of the ticks of an axis, grown digit by digit until neighbours differ.
fn tick_labels(ticks: &[f64]) -> Vec<String> {
    let decimals = tick_decimals(ticks);
    let exponential = exponential_ticks(ticks) || decimals > PLAIN_DECIMALS;
    let mut digits = if exponential {
        EXPONENTIAL_DIGITS
    } else {
        decimals
    };
    let write = |d: i32| -> Vec<String> {
        ticks
            .iter()
            .map(|v| format_number(*v, d, exponential))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels() {
        assert_eq!(tick_labels(&[0.0, 0.5, 1.0]), ["0", "0.5", "1"]);
        assert_eq!(tick_labels(&[1e-4, 1e-3, 1e2]), ["1e-4", "1e-3", "1e+2"]);
        assert_eq!(tick_labels(&[0.0, 2e5, 4e5]), ["0", "2e+5", "4e+5"]);
        assert_eq!(format_number(-0.0001, 2, false), "0");
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
    fn ticks() {
        assert_eq!(
            linear_ticks(0.0, 1.0, 5),
            [0.0, 0.2, 0.4, 0.6000000000000001, 0.8, 1.0]
        );
        let t = log_ticks(5e-5, 200.0, 8);
        assert_eq!(t.first().copied(), Some(1e-4));
        assert!(t.len() <= 8);
    }
}
