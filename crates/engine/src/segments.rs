//! The integration of a segment, a time interval without changes, and the rows of its output.

use sbmlsim_abi::IntegratorSettings;
use sbmlsim_csolvers::ffi::{CV_NORMAL, CV_ONE_STEP, CV_TSTOP_RETURN};

use crate::cvode::{Cvode, Problem};
use crate::model::ModelFns;
use crate::{EngineError, OUTPUT_LIMIT_FLAG, STALL_FLAG};

/// A column of the result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Column {
    /// The time.
    Time,
    /// The selection at this position of the output of [`ModelFns::selections`].
    Selection(usize),
}

/// The rows of a result: every row is the time and the selections of the model after
/// [`ModelFns::assign`], computed on copies of the state and the parameters, so the output does
/// not change the integration.
pub(crate) struct Recorder<'a> {
    model: &'a dyn ModelFns,
    columns: Vec<Column>,
    y: Vec<f64>,
    p: Vec<f64>,
    selections: Vec<f64>,
    /// The values of the rows, row major.
    pub values: Vec<f64>,
    /// The number of rows.
    pub n_rows: usize,
    /// The most values the result may hold.
    cap: usize,
}

impl<'a> Recorder<'a> {
    /// A recorder of the columns; `n_selections` is the length of the output of
    /// [`ModelFns::selections`].
    pub fn new(
        model: &'a dyn ModelFns,
        columns: Vec<Column>,
        n_selections: usize,
        cap: usize,
    ) -> Self {
        Recorder {
            model,
            columns,
            y: Vec::new(),
            p: Vec::new(),
            selections: vec![0.0; n_selections],
            values: Vec::new(),
            n_rows: 0,
            cap,
        }
    }

    /// Reserves the values of `rows` more rows.
    pub fn reserve(&mut self, rows: usize) {
        self.values.reserve_exact(rows * self.columns.len());
    }

    /// Appends the row of the time `t`, the state `y` and the parameters `p`.
    ///
    /// Fails with [`OUTPUT_LIMIT_FLAG`] when the row would exceed the cap of values of the
    /// result.
    pub fn row(&mut self, t: f64, y: &[f64], p: &[f64]) -> Result<(), EngineError> {
        if self.values.len() + self.columns.len() > self.cap {
            return Err(EngineError::Integration {
                flag: OUTPUT_LIMIT_FLAG,
                time: t,
                message: format!(
                    "the result would exceed the limit of {} values ({} rows of {} columns)",
                    self.cap,
                    self.n_rows,
                    self.columns.len()
                ),
            });
        }
        self.y.clear();
        self.y.extend_from_slice(y);
        self.p.clear();
        self.p.extend_from_slice(p);
        self.model.assign(t, &mut self.y, &mut self.p);
        self.model
            .selections(t, &self.y, &self.p, &mut self.selections);
        for column in &self.columns {
            self.values.push(match column {
                Column::Time => t,
                Column::Selection(index) => self.selections[*index],
            });
        }
        self.n_rows += 1;
        Ok(())
    }
}

/// The output of a segment.
#[derive(Debug, Clone, Copy)]
pub(crate) enum SegmentOutput<'a> {
    /// The given times, non-decreasing, within the segment.
    Times(&'a [f64]),
    /// The start, every step of the integrator and the end.
    Steps,
}

/// Whether `t` is too close to `t0` for CVODE to integrate from `t0` to it (the test of
/// `cvHin`: a distance below two units of rounding of the larger magnitude); the state at `t`
/// is then the state at `t0`.
pub(crate) fn too_close(t0: f64, t: f64) -> bool {
    t == t0 || (t - t0).abs() < 2.0 * f64::EPSILON * t0.abs().max(t.abs())
}

/// Integrates the segment from `t0` with the state `y0` to `end` and records its output.
///
/// `cvode` is `None` for a model without states, whose rows are the selections at the times
/// of the output. The stop time of CVODE is `end`, so the integration never passes it; with
/// [`SegmentOutput::Times`] it ends at the last time of the output.
#[allow(clippy::too_many_arguments)]
pub(crate) fn integrate(
    cvode: Option<&mut Cvode>,
    problem: &mut Problem<'_>,
    settings: &IntegratorSettings,
    t0: f64,
    y0: &[f64],
    end: f64,
    output: SegmentOutput<'_>,
    recorder: &mut Recorder<'_>,
) -> Result<(), EngineError> {
    let p = problem.p;
    let Some(cvode) = cvode else {
        match output {
            SegmentOutput::Times(times) => {
                for t in times {
                    recorder.row(*t, y0, p)?;
                }
            }
            SegmentOutput::Steps => {
                recorder.row(t0, y0, p)?;
                if end != t0 {
                    recorder.row(end, y0, p)?;
                }
            }
        }
        return Ok(());
    };
    match output {
        SegmentOutput::Times(times) => {
            let mut started = false;
            for &tout in times {
                if !started && too_close(t0, tout) {
                    recorder.row(tout, y0, p)?;
                    continue;
                }
                if !started {
                    cvode.start(t0, y0, end, settings)?;
                    started = true;
                }
                cvode.integrate(problem, tout, CV_NORMAL)?;
                recorder.row(tout, cvode.state(), p)?;
            }
        }
        SegmentOutput::Steps => {
            recorder.row(t0, y0, p)?;
            if end == t0 {
                return Ok(());
            }
            if too_close(t0, end) {
                recorder.row(end, y0, p)?;
                return Ok(());
            }
            cvode.start(t0, y0, end, settings)?;
            let mut previous = t0;
            loop {
                let (t, flag) = cvode.integrate(problem, end, CV_ONE_STEP)?;
                // a step which does not advance the time (CVODE's `t + h = t`) never ends
                if flag != CV_TSTOP_RETURN && t <= previous {
                    return Err(EngineError::Integration {
                        flag: STALL_FLAG,
                        time: t,
                        message: format!(
                            "the integrator stalled: a step did not advance the time past {previous}"
                        ),
                    });
                }
                previous = t;
                // CVODE returns the stop time exactly when it reaches it
                let t = if flag == CV_TSTOP_RETURN { end } else { t };
                recorder.row(t, cvode.state(), p)?;
                if flag == CV_TSTOP_RETURN || t >= end {
                    break;
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use sbmlsim_abi::{IntegratorSettings, Tolerance};

    use super::*;
    use crate::cvode::{Config, Linear, Method};
    use crate::model::Decay;

    #[test]
    fn steps_stop_at_the_output_limit() {
        let model = Decay { k: 1.0 };
        let desc = model.description();
        let (p, y0) = (vec![1.0, 2f64.ln()], vec![1.0]);
        let settings = IntegratorSettings {
            relative_tolerance: 1e-10,
            absolute_tolerance: Tolerance::Scalar(1e-14),
            ..IntegratorSettings::default()
        };
        let config = Config {
            n: 1,
            method: Method::Bdf(Linear::Dense { analytic: true }),
        };
        let mut cvode = Cvode::new(config).unwrap();
        let mut problem = Problem {
            n: 1,
            model: &model,
            p: &p,
            pattern: Some(crate::pattern_of(&desc)),
            jac_values: vec![0.0],
        };
        let mut run = |cap: usize| {
            let columns = vec![Column::Time, Column::Selection(0)];
            let mut recorder = Recorder::new(&model, columns, 2, cap);
            let result = integrate(
                Some(&mut cvode),
                &mut problem,
                &settings,
                0.0,
                &y0,
                50.0,
                SegmentOutput::Steps,
                &mut recorder,
            );
            (result, recorder)
        };
        // the whole run is more than 20 rows, so a cap of 40 values stops it
        let (result, recorder) = run(1 << 20);
        result.unwrap();
        assert!(recorder.n_rows > 20, "{}", recorder.n_rows);
        match run(40) {
            (
                Err(EngineError::Integration {
                    flag,
                    time,
                    message,
                }),
                recorder,
            ) => {
                assert_eq!(flag, OUTPUT_LIMIT_FLAG);
                assert!(flag < -99, "distinct from the flags of CVODE");
                assert!(time > 0.0 && time < 50.0, "{time}");
                assert!(message.contains("limit of 40"), "{message}");
                assert_eq!(recorder.n_rows, 20);
                assert_eq!(recorder.values.len(), 40);
            }
            (other, _) => panic!("{other:?}"),
        }
    }
}
