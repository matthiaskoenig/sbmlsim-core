//! The reductions of a curve: a value of the simulation of a condition which is no value of a
//! column at the row of a measurement but a function of the whole column over the time of the
//! simulation.
//!
//! A curve of a fit reads a column of its condition at the row of every measurement. The data
//! of a study is often a value per simulation instead, the largest concentration of a curve,
//! its mean over a dosing interval, the amount at the end of a collection or a parameter of the
//! non-compartmental analysis of a concentration (the area under the curve, the half-life),
//! and such a value depends on every row of the column and on the times of the rows. A curve
//! with a [`Reduction`] reads that value: every measurement of it is compared with the reduced
//! value of the column, or, for [`Reduction::At`], with the column interpolated at the time of
//! the measurement.
//!
//! The rows of such a curve are not resolved when the problem is built, so its condition may
//! output the steps of the integrator, whose times depend on the parameters: the reduction
//! reads whatever rows the simulation of an evaluation has. That is the output a host keeps for
//! a reduction, since its value depends on the rows it is computed over.
//!
//! The derivative of a reduction in the parameters is the reduction of the forward
//! sensitivities of the column, which the condition carries for the jacobian of the
//! sensitivities ([`crate::JacobianChoice::Sensitivities`]):
//!
//! - [`Reduction::Max`] and [`Reduction::Min`]: the sensitivity of the column at the row the
//!   extremum is at, the first one of a tie. That is the derivative at an extremum which stays
//!   at its row, which is every extremum of a column of the steps of the integrator up to the
//!   resolution of those steps, and the one-sided derivative of an extremum at the end of the
//!   simulation.
//! - [`Reduction::Mean`]: the trapezoid mean of the sensitivities over the same intervals.
//! - [`Reduction::At`]: the sensitivities interpolated with the weights of the value.
//! - [`Reduction::Pk`]: the analysis is no linear function of the column, so its derivative is
//!   the central difference of the analysis along the sensitivity of every parameter,
//!   `(P(c + h s) - P(c - h s)) / 2h`, of the column `c` and its sensitivity `s`, with a step
//!   `h` which moves the column by [`PK_STEP`] of its largest value. The analysis of a column is
//!   cheap against a simulation, so this is two analyses per parameter and no simulation, where
//!   a difference quotient of the condition would be one simulation per parameter.
//!
//! Every time a reduction reads is the time of the table plus its `time_shift`: a host whose
//! simulation runs on the time of its plan and whose data is on a shifted axis computes the
//! reduction on the shifted times, and so does the engine.

use pkpdutils_core::pkpdutils_abi::{AnalysisResult, Batch, NcaOptions, TerminalMethod};
use sbmlsim_timecourse::{Weight, grid_weights, kept_rows, padded_length};
use serde::{Deserialize, Serialize};

/// The relative size of the move of a column the derivative of a [`Reduction::Pk`] is the
/// central difference over: the step along the sensitivity of a parameter is chosen so that
/// the largest change of the column is this share of its largest absolute value.
///
/// The analysis is a piecewise smooth function of the column (the point of the maximum, the
/// points of the terminal regression), so the step is small enough that a move of the column
/// changes none of those choices and large enough that its rounding does not swamp the
/// difference.
pub const PK_STEP: f64 = 1e-6;

/// The factor of a reduction which does not give one.
fn one() -> f64 {
    1.0
}

/// How a curve reads a value of the whole column of its condition, see the module.
///
/// The value of every kind is multiplied by its `factor`, e.g. the factor of a declared unit
/// of a host, after it is reduced: `max(c) * factor`, which is the value a host computes when
/// it converts the value of a formula into the unit of its observable.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Reduction {
    /// The largest value of the column over the rows of a finite time, ignoring `NaN`.
    Max {
        /// What the reduced value is multiplied by, one when it is absent.
        #[serde(default = "one")]
        factor: f64,
    },
    /// The smallest value of the column over the rows of a finite time, ignoring `NaN`.
    Min {
        /// What the reduced value is multiplied by, one when it is absent.
        #[serde(default = "one")]
        factor: f64,
    },
    /// The mean of the column over the time: the trapezoid integral of the column over its
    /// rows of a finite time, divided by the time between the first and the last of them; the
    /// value of the last row for a simulation of one time point. The two rows of the states
    /// before and after a change at its time are an interval without time, which adds nothing.
    Mean {
        /// What the reduced value is multiplied by, one when it is absent.
        #[serde(default = "one")]
        factor: f64,
        /// What is added to the time of every row, zero when it is absent.
        #[serde(default)]
        time_shift: f64,
    },
    /// The column interpolated linearly at the time of every measurement of the curve, its
    /// `times`, with the rule of the jumps of sbmlsim (`grid_weights` of
    /// `sbmlsim.result.timecourse`): the rows of a time which appears twice, at a change or
    /// at an event, are a jump, and a time of a measurement before a jump is interpolated
    /// toward the state before it, a time at a jump sees the state after it, unless its flag
    /// of `before` says it reads the state before it. A time outside of the rows of the
    /// simulation has no value (`NaN`).
    At {
        /// What the interpolated value is multiplied by, one when it is absent.
        #[serde(default = "one")]
        factor: f64,
        /// What is added to the time of every row, zero when it is absent.
        #[serde(default)]
        time_shift: f64,
        /// The resolution of the time: rows at most this far apart after a jump are a chain
        /// of jumps, see `grid_weights`; zero when it is absent, which makes a time which
        /// appears twice the only jump.
        #[serde(default)]
        tolerance: f64,
        /// The times of the changes of the condition after its start, in the time of the
        /// reduction; none when it is absent.
        #[serde(default)]
        changes: Vec<f64>,
        /// Whether the measurement at a jump reads the state before it, one flag per time;
        /// every measurement reads the state after a jump when it is absent.
        #[serde(default)]
        before: Option<Vec<bool>>,
    },
    /// A parameter of the non-compartmental analysis of the column, the analysis of
    /// pkpdutils (`pkpdutils_core`).
    Pk(Box<PkReduction>),
}

/// A parameter of the non-compartmental analysis of a column, see [`Reduction::Pk`].
///
/// The analysis is the one a host runs on the simulation of the condition: the batch of one
/// sample of the contract `batch/1` of pkpdutils, whose time points and values are the rows of
/// the column, and the options of the analysis. The host gives the batch it analyses, with its
/// units, its doses and its route, and the engine replaces the time points and the values of
/// its one sample with the ones of every evaluation:
///
/// - the state before a change at the time of one of `changes` is left out, the analysis takes
///   a time once (`without_states_before` of sbmlsim), and so is every row of a time which is
///   not finite;
/// - the sample is padded with `NaN` to the next power of two of its number of time points,
///   and to at least [`MIN_PADDED_LENGTH`], as sbmlsim pads it (`evaluate_pk`), unless the
///   terminal phase of the options is given by the indices of its points, which count from
///   the end of the sample.
///
/// The value is the column of `parameter` in the result, `NaN` where the analysis has none.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PkReduction {
    /// What the parameter is multiplied by, one when it is absent.
    #[serde(default = "one")]
    pub factor: f64,
    /// What is added to the time of every row, zero when it is absent.
    #[serde(default)]
    pub time_shift: f64,
    /// The times of the changes of the condition after its start, in the time of the
    /// reduction, whose states before the change are left out; none when it is absent.
    #[serde(default)]
    pub changes: Vec<f64>,
    /// The name of the parameter of the result, e.g. `cmax` or `auc_inf_obs`.
    pub parameter: String,
    /// The batch of one sample of the analysis, see the struct; its time points and values
    /// are replaced.
    pub batch: Batch,
    /// The options of the analysis.
    pub options: NcaOptions,
}

/// The kinds of a [`Reduction`] this build fits, which a host reads to know whether it may
/// send a curve of a reduction: a release before the field does not deny it, see
/// [`crate::CurveRequest::reduction`].
pub const REDUCTIONS: &[&str] = &["max", "min", "mean", "at", "pk"];

/// The shortest length of the padded sample of a [`Reduction::Pk`], `MIN_PADDED_LENGTH` of
/// `sbmlsim.simulator.pk`.
pub const MIN_PADDED_LENGTH: usize = sbmlsim_timecourse::MIN_PADDED_LENGTH;

/// The rows of one column of a table, with the time of every row in the time of a reduction.
pub(crate) struct Column<'a> {
    /// The time of every row, shifted.
    pub times: Vec<f64>,
    /// The table, row major.
    pub values: &'a [f64],
    /// The number of columns of the table.
    pub columns: usize,
    /// The column which is reduced.
    pub column: usize,
}

impl Column<'_> {
    /// The value of the column at a row.
    fn at(&self, row: usize) -> f64 {
        self.values[row * self.columns + self.column]
    }

    /// The rows of the table.
    fn rows(&self) -> usize {
        self.times.len()
    }
}

impl Reduction {
    /// The factor the reduced value is multiplied by.
    fn factor(&self) -> f64 {
        match self {
            Reduction::Max { factor }
            | Reduction::Min { factor }
            | Reduction::Mean { factor, .. }
            | Reduction::At { factor, .. } => *factor,
            Reduction::Pk(pk) => pk.factor,
        }
    }

    /// What is added to the time of a row.
    pub(crate) fn time_shift(&self) -> f64 {
        match self {
            Reduction::Max { .. } | Reduction::Min { .. } => 0.0,
            Reduction::Mean { time_shift, .. } | Reduction::At { time_shift, .. } => *time_shift,
            Reduction::Pk(pk) => pk.time_shift,
        }
    }

    /// Checks the reduction of a curve of `values` measurements at `times`, which are the
    /// times of an [`Reduction::At`] and none for every other kind.
    pub(crate) fn check(&self, times: &[f64], values: usize) -> Result<(), String> {
        if !self.factor().is_finite() {
            return Err(format!(
                "has the factor {} of its reduction, which is not finite",
                self.factor()
            ));
        }
        if !self.time_shift().is_finite() {
            return Err(format!(
                "has the time shift {} of its reduction, which is not finite",
                self.time_shift()
            ));
        }
        match self {
            Reduction::At {
                tolerance, before, ..
            } => {
                if times.len() != values {
                    return Err(format!("has {} times and {} values", times.len(), values));
                }
                if !tolerance.is_finite() || *tolerance < 0.0 {
                    return Err(format!(
                        "has the tolerance {tolerance} of its reduction, which is no \
                         resolution of a time"
                    ));
                }
                if let Some(before) = before
                    && before.len() != times.len()
                {
                    return Err(format!(
                        "has {} times and {} flags of the state before a jump",
                        times.len(),
                        before.len()
                    ));
                }
                if let Some(time) = times.iter().find(|time| !time.is_finite()) {
                    return Err(format!("measures at t = {time}, which is not finite"));
                }
            }
            _ if !times.is_empty() => {
                return Err(
                    "reduces its column to one value and has times: the measurements of such \
                     a reduction have none"
                        .to_string(),
                );
            }
            Reduction::Pk(pk) => pk.check()?,
            _ => {}
        }
        Ok(())
    }

    /// The value of every measurement of a curve on a column, with the factor applied.
    ///
    /// `grid` is the times of the measurements of a [`Reduction::At`]; every other kind has
    /// one value, which every one of its `n` measurements is compared with.
    pub(crate) fn values(
        &self,
        column: &Column<'_>,
        grid: &[f64],
        n: usize,
    ) -> Result<Vec<f64>, String> {
        let factor = self.factor();
        let scaled = |value: f64| if factor == 1.0 { value } else { value * factor };
        let one = |value: f64| vec![scaled(value); n];
        Ok(match self {
            Reduction::Max { .. } => one(extreme(column, true).0),
            Reduction::Min { .. } => one(extreme(column, false).0),
            Reduction::Mean { .. } => one(mean(column, |row| column.at(row))),
            Reduction::At { .. } => self
                .weights(column, grid)
                .iter()
                .map(|weight| {
                    scaled(weight.apply(column.at(weight.lower), column.at(weight.upper)))
                })
                .collect(),
            Reduction::Pk(pk) => one(pk.analyse(column, |row| column.at(row))?),
        })
    }

    /// The derivative of every measurement of a curve on a column in every target of the
    /// sensitivities, `n * targets` row major, with the factor applied.
    ///
    /// `sensitivity(row)` is the derivative of the column at a row in every target.
    pub(crate) fn derivatives<'s>(
        &self,
        column: &Column<'_>,
        grid: &[f64],
        n: usize,
        targets: usize,
        sensitivity: impl Fn(usize) -> &'s [f64],
    ) -> Result<Vec<f64>, String> {
        let factor = self.factor();
        let mut one = vec![0.0; targets];
        match self {
            Reduction::Max { .. } | Reduction::Min { .. } => {
                let max = matches!(self, Reduction::Max { .. });
                if let (_, Some(row)) = extreme(column, max) {
                    one.copy_from_slice(sensitivity(row));
                }
            }
            Reduction::Mean { .. } => {
                for (target, value) in one.iter_mut().enumerate() {
                    *value = mean(column, |row| sensitivity(row)[target]);
                }
            }
            Reduction::At { .. } => {
                let weights = self.weights(column, grid);
                let mut out = vec![0.0; n * targets];
                for (at, weight) in weights.iter().enumerate() {
                    if !weight.valid {
                        continue;
                    }
                    let (lower, upper) = (sensitivity(weight.lower), sensitivity(weight.upper));
                    for target in 0..targets {
                        out[at * targets + target] =
                            weight.apply(lower[target], upper[target]) * factor;
                    }
                }
                return Ok(out);
            }
            Reduction::Pk(pk) => {
                let scale = (0..column.rows())
                    .filter(|&row| column.times[row].is_finite())
                    .map(|row| column.at(row).abs())
                    .filter(|value| value.is_finite())
                    .fold(0.0, f64::max);
                for (target, value) in one.iter_mut().enumerate() {
                    let largest = (0..column.rows())
                        .map(|row| sensitivity(row)[target].abs())
                        .filter(|value| value.is_finite())
                        .fold(0.0, f64::max);
                    if largest == 0.0 || scale == 0.0 {
                        continue;
                    }
                    let h = PK_STEP * scale / largest;
                    let above =
                        pk.analyse(column, |row| column.at(row) + h * sensitivity(row)[target])?;
                    let below =
                        pk.analyse(column, |row| column.at(row) - h * sensitivity(row)[target])?;
                    *value = (above - below) / (2.0 * h);
                }
            }
        }
        let mut out = Vec::with_capacity(n * targets);
        for _ in 0..n {
            out.extend(one.iter().map(|value| value * factor));
        }
        Ok(out)
    }

    /// The weights of the times of the measurements of a [`Reduction::At`] on the rows of a
    /// column; none for every other kind.
    fn weights(&self, column: &Column<'_>, grid: &[f64]) -> Vec<Weight> {
        let Reduction::At {
            tolerance,
            changes,
            before,
            ..
        } = self
        else {
            return Vec::new();
        };
        let left = before.clone().unwrap_or_else(|| vec![false; grid.len()]);
        grid_weights(&column.times, grid, *tolerance, changes, Some(&left))
    }
}

impl PkReduction {
    /// Checks the batch of the analysis: one sample of the contract of the kernel, whose time
    /// points the engine sets, so nothing of it may be a value per time point.
    fn check(&self) -> Result<(), String> {
        let batch = &self.batch;
        if batch.n_samples() != 1 {
            return Err(format!(
                "analyses a batch of {} samples, where its reduction is one sample",
                batch.n_samples()
            ));
        }
        for (field, set) in [
            ("sd", batch.sd.is_some()),
            ("se", batch.se.is_some()),
            ("n", batch.n.is_some()),
            ("nominal_time", batch.nominal_time.is_some()),
        ] {
            if set {
                return Err(format!(
                    "analyses a batch with the values per time point '{field}', which no \
                     column of a simulation has"
                ));
            }
        }
        Ok(())
    }

    /// The parameter of the analysis of the values of the rows of a column, with the time
    /// points of the column; `NaN` where the analysis has no value of it.
    fn analyse(&self, column: &Column<'_>, value: impl Fn(usize) -> f64) -> Result<f64, String> {
        Ok(self
            .run(column, value)?
            .and_then(|result| {
                result
                    .parameters
                    .into_iter()
                    .find(|parameter| parameter.name == self.parameter)
            })
            .and_then(|parameter| parameter.values.first().copied())
            .unwrap_or(f64::NAN))
    }

    /// Whether the analysis of a column has the parameter of the reduction; a column of no
    /// time point has no analysis and is taken to have it.
    pub(crate) fn derives(&self, column: &Column<'_>) -> Result<bool, String> {
        Ok(self
            .run(column, |row| column.at(row))?
            .is_none_or(|result| {
                result
                    .parameters
                    .iter()
                    .any(|parameter| parameter.name == self.parameter)
            }))
    }

    /// The analysis of the sample of a column, padded as sbmlsim pads it; `None` for a sample
    /// of no time point, which sbmlsim does not analyse.
    fn run(
        &self,
        column: &Column<'_>,
        value: impl Fn(usize) -> f64,
    ) -> Result<Option<AnalysisResult>, String> {
        let (times, values) = self.sample(&column.times, value);
        let points = times.len();
        if points == 0 {
            return Ok(None);
        }
        let manual = self.options.terminal.method == TerminalMethod::Manual;
        let length = if manual {
            points
        } else {
            padded_length(points)
        };
        let mut batch = self.batch.clone();
        batch.n_time = length;
        batch.times = times;
        batch.values = values;
        batch.times.resize(length, f64::NAN);
        batch.values.resize(length, f64::NAN);
        pkpdutils_core::analyze_batch_in_shards(&batch, &self.options, 1)
            .map(Some)
            .map_err(|error| format!("the analysis of its column failed: {error}"))
    }

    /// The time points and the values of the sample of a column: the rows of a finite time
    /// without the state before a change at the time of one of the changes, which is the first
    /// of two rows of the same time (`without_states_before` of sbmlsim).
    fn sample(&self, times: &[f64], value: impl Fn(usize) -> f64) -> (Vec<f64>, Vec<f64>) {
        let rows = kept_rows(times, &self.changes);
        (
            rows.iter().map(|&row| times[row]).collect(),
            rows.iter().map(|&row| value(row)).collect(),
        )
    }
}

/// The largest or the smallest value of a column over its rows of a finite time and the first
/// row it is at, see [`sbmlsim_timecourse::extreme`].
fn extreme(column: &Column<'_>, max: bool) -> (f64, Option<usize>) {
    sbmlsim_timecourse::extreme(&column.times, max, |row| column.at(row))
}

/// The trapezoid mean of the values of the rows of a column, see [`sbmlsim_timecourse::mean`].
fn mean(column: &Column<'_>, value: impl Fn(usize) -> f64) -> f64 {
    sbmlsim_timecourse::mean(&column.times, value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_extreme_and_a_mean_of_a_column_ignore_the_rows_of_no_finite_time() {
        let values = [1.0, f64::NAN, 3.0, 9.0];
        let column = Column {
            times: vec![0.0, 1.0, 2.0, f64::INFINITY],
            values: &values,
            columns: 1,
            column: 0,
        };
        assert_eq!(extreme(&column, true), (3.0, Some(2)));
        assert_eq!(extreme(&column, false), (1.0, Some(0)));
        let values = [1.0, 1.0, 3.0, 3.0];
        let column = Column {
            times: vec![0.0, 1.0, 1.0, 2.0],
            values: &values,
            columns: 1,
            column: 0,
        };
        assert_eq!(mean(&column, |row| values[row]), 2.0);
    }
}
