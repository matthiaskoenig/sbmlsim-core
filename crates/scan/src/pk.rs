//! The non-compartmental analysis of a `PK` observable with the kernel of pkpdutils, sbmlsim's
//! `simulator/pk.py`.
//!
//! A PK observable analyses one timecourse of every point with the doses the plan of the point
//! assigns to its dose target (or one fixed dose at the start). Every parameter the kernel
//! derives is a value per simulation `<id>.<parameter>`, and `<id>.flags` the flags of the
//! analysis. Which parameters the kernel derives depends on the dosing, so they are found by
//! an analysis of a synthetic timecourse for the plan of the first point of every simulation,
//! dosed at every time the plan assigns the dose target, a zero value included (`compile_pk`).
//!
//! The points of a chunk are analysed in groups of the same number of doses and the same
//! padded length, the next power of two of their number of time points (at least
//! `sbmlsim_timecourse::MIN_PADDED_LENGTH`), padded with `NaN`, so a parameter depends on its point alone.

use pkpdutils_core::pkpdutils_abi::{
    BATCH_CONTRACT, Batch, NcaOptions, Route, TerminalMethod, UncertaintyMethod,
};
use serde_json::Value as Json;

use crate::error::ScanError;
use crate::plan::{Assigned, Plan};
use crate::text;
use sbmlsim_timecourse::{kept_rows, padded_length};

/// The time points of the synthetic timecourse which finds the parameters.
const PROBE_POINTS: u64 = 97;

/// The parameter of the flags of the analysis.
pub(crate) const FLAGS: &str = "flags";

/// The variable of the kernel which the host of pkpdutils writes itself, no parameter.
const HOST_VARIABLES: [&str; 1] = ["n"];

/// Where the doses of a PK observable come from.
#[derive(Debug, Clone)]
pub(crate) struct DoseSpec {
    /// The target whose values the plans assign, `None` for a fixed dose.
    pub target: Option<String>,
    /// The fixed dose at the start.
    pub amount: f64,
    /// The unit of the amounts.
    pub unit: String,
}

/// A PK observable compiled against the model.
#[derive(Debug, Clone)]
pub(crate) struct PkNode {
    pub id: String,
    pub selection: String,
    pub time_unit: String,
    pub unit: String,
    pub dose: Option<DoseSpec>,
    pub route: Option<Route>,
    pub options: NcaOptions,
    /// The parameters it gives, `flags` among them.
    pub parameters: Vec<String>,
}

impl PkNode {
    /// The output of a parameter, `<id>.<parameter>`.
    pub fn output(&self, parameter: &str) -> String {
        format!("{}.{parameter}", self.id)
    }
}

/// The options of the analysis: pkpdutils' `NCAOptions()` without them, else the `model_dump`
/// of an `NCAOptions`.
pub(crate) fn options_of(id: &str, options: Option<&Json>) -> Result<NcaOptions, ScanError> {
    match options {
        None | Some(Json::Null) => Ok(NcaOptions {
            uncertainty: Some(UncertaintyMethod::None),
            ..NcaOptions::default()
        }),
        Some(json) => serde_json::from_value(json.clone()).map_err(|err| {
            ScanError::request(format!(
                "The options of the PK observable '{id}' are no options of pkpdutils: {err}"
            ))
        }),
    }
}

/// The route of a PK observable, by its name in pkpdutils.
pub(crate) fn route_of(id: &str, route: &str) -> Result<Route, ScanError> {
    let parsed: Route = serde_json::from_value(Json::String(route.to_string())).map_err(|_| {
        ScanError::request(format!(
            "The route '{route}' of the observable '{id}' is none of pkpdutils: ['iv_bolus', \
             'iv_infusion', 'oral']."
        ))
    })?;
    if parsed == Route::IvInfusion {
        return Err(ScanError::request(format!(
            "The observable '{id}' has the route of an infusion, whose duration the changes of a \
             simulation do not carry; analyse it as 'iv_bolus' or 'oral'."
        )));
    }
    Ok(parsed)
}

/// The times and amounts of the doses of the plan of a point, sbmlsim's `doses_of`: the values
/// the plan assigns to the target before the initialization (at the start) and at its changes,
/// positive ones only unless `zeros`, a change at a time replacing an earlier value of it;
/// shifted by the time shift of the plan. The error is the message of the `ValueError` of
/// sbmlsim.
pub(crate) fn doses_of(
    plan: &Plan,
    dose: &DoseSpec,
    zeros: bool,
) -> Result<(Vec<f64>, Vec<f64>), String> {
    let Some(target) = &dose.target else {
        return Ok((vec![plan.start + plan.time_shift], vec![dose.amount]));
    };
    let mut found: Vec<(f64, f64)> = Vec::new();
    let assignments = plan.preinit.iter().map(|a| (plan.start, a)).chain(
        plan.events
            .iter()
            .flat_map(|e| e.assignments.iter().map(move |a| (e.time, a))),
    );
    for (time, assignment) in assignments {
        if assignment.target != *target {
            continue;
        }
        let value = match &assignment.value {
            Assigned::Number(x) => *x,
            Assigned::Formula(formula) => {
                return Err(format!(
                    "The dose '{target}' is the formula '{formula}' in a simulation; a PK \
                     observable reads the dose as a number."
                ));
            }
        };
        match found.iter_mut().find(|(t, _)| *t == time) {
            Some(entry) => entry.1 = value,
            None => found.push((time, value)),
        }
    }
    let mut doses: Vec<(f64, f64)> = found
        .into_iter()
        .filter(|(_, v)| zeros || *v > 0.0)
        .collect();
    doses.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
    let times = doses.iter().map(|(t, _)| t + plan.time_shift).collect();
    let amounts = doses.iter().map(|(_, a)| *a).collect();
    Ok((times, amounts))
}

/// A sample of the analysis: its time points and values, padded, and its doses.
struct Sample {
    times: Vec<f64>,
    values: Vec<f64>,
    dose_times: Vec<f64>,
    dose_amounts: Vec<f64>,
}

/// The batch of samples of one length and one number of doses.
fn batch(node: &PkNode, samples: &[&Sample], length: usize, count: usize) -> Batch {
    let dosed = node.dose.is_some() && count > 0;
    let n = samples.len();
    Batch {
        contract: BATCH_CONTRACT.to_string(),
        sample_dims: vec!["_sim".to_string()],
        sample_shape: vec![n],
        n_time: length,
        times: samples
            .iter()
            .flat_map(|s| s.times.iter().copied())
            .collect(),
        values: samples
            .iter()
            .flat_map(|s| s.values.iter().copied())
            .collect(),
        sd: None,
        se: None,
        n: None,
        nominal_time: None,
        lloq: None,
        max_doses: count,
        dose_amount: samples
            .iter()
            .flat_map(|s| s.dose_amounts.iter().copied())
            .collect(),
        dose_time: samples
            .iter()
            .flat_map(|s| s.dose_times.iter().copied())
            .collect(),
        dose_duration: vec![f64::NAN; n * count],
        time_unit: node.time_unit.clone(),
        unit: node.unit.clone(),
        dose_unit: if dosed {
            node.dose.as_ref().map(|d| d.unit.clone())
        } else {
            None
        },
        substance: Some("substance".to_string()),
        substances: None,
        route: if dosed { node.route } else { None },
        routes: None,
        tissue: None,
        coords: Vec::new(),
    }
}

/// The values of every parameter of the node for every sample of an analysis.
fn analyse(
    node: &PkNode,
    samples: &[&Sample],
    length: usize,
    count: usize,
) -> Result<Vec<Vec<f64>>, String> {
    let batch = batch(node, samples, length, count);
    let result = pkpdutils_core::analyze_batch_in_shards(&batch, &node.options, 1)
        .map_err(|err| format!("ValueError: {err}"))?;
    Ok(node
        .parameters
        .iter()
        .map(|parameter| {
            if parameter == FLAGS {
                return result.flags.iter().map(|f| f64::from(*f)).collect();
            }
            result
                .parameters
                .iter()
                .find(|c| c.name == *parameter)
                .map_or_else(|| vec![f64::NAN; samples.len()], |c| c.values.clone())
        })
        .collect())
}

/// The parameters and their units of the analysis of a PK observable for the plans of the first
/// points of the simulations, `compile_pk` of sbmlsim: every time a plan assigns the dose target
/// is a dose of a synthetic timecourse, a zero value included.
pub(crate) fn probe_parameters(
    node: &PkNode,
    plans: &[Plan],
) -> Result<Vec<(String, String)>, ScanError> {
    let mut units: Vec<(String, String)> = Vec::new();
    for plan in plans {
        if plan.end <= plan.start {
            return Err(ScanError::request(format!(
                "The observable '{}' analyses a simulation without a time span, [{}, {}].",
                node.id,
                text::number(plan.start),
                text::number(plan.end)
            )));
        }
        let (times, amounts) = match &node.dose {
            None => (Vec::new(), Vec::new()),
            Some(dose) => doses_of(plan, dose, true).map_err(ScanError::request)?,
        };
        let amounts: Vec<f64> = amounts
            .iter()
            .map(|a| if *a > 0.0 { *a } else { 1.0 })
            .collect();
        let start = plan.start + plan.time_shift;
        let end = plan.end + plan.time_shift;
        let t = crate::plan::linspace(start, end, PROBE_POINTS - 1);
        let ke = 8.0 / (end - start);
        let ka = 4.0 * ke;
        let mut c = vec![0.0; t.len()];
        let at_times = if times.is_empty() {
            vec![start]
        } else {
            times.clone()
        };
        for at in at_times {
            for (ci, ti) in c.iter_mut().zip(&t) {
                if *ti >= at {
                    let tau = (ti - at).max(0.0);
                    *ci += if node.route == Some(Route::IvBolus) {
                        (-ke * tau).exp()
                    } else {
                        (-ke * tau).exp() - (-ka * tau).exp()
                    };
                }
            }
        }
        let sample = Sample {
            times: t.clone(),
            values: c,
            dose_times: times.clone(),
            dose_amounts: amounts,
        };
        let batch = batch(node, &[&sample], t.len(), times.len());
        let result = pkpdutils_core::analyze_batch_in_shards(&batch, &node.options, 1)
            .map_err(|err| ScanError::request(err.to_string()))?;
        for column in &result.parameters {
            if HOST_VARIABLES.contains(&column.name.as_str()) {
                continue;
            }
            if !units.iter().any(|(p, _)| *p == column.name) {
                units.push((column.name.clone(), column.unit.clone()));
            }
        }
        if !units.iter().any(|(p, _)| p == FLAGS) {
            units.push((FLAGS.to_string(), "dimensionless".to_string()));
        }
    }
    Ok(units)
}

/// The timecourse of one point which a PK observable analyses.
pub(crate) struct PkPoint<'a> {
    pub time: &'a [f64],
    pub values: &'a [f64],
    pub plan: &'a Plan,
}

/// The parameters of every point (a value per parameter of the node), or the failure of the
/// point, sbmlsim's `evaluate_pk`: the points in groups of their number of doses and padded
/// length; a group the kernel refuses is analysed point by point, so a point fails exactly when
/// its own analysis fails.
pub(crate) fn evaluate(node: &PkNode, points: &[PkPoint<'_>]) -> Vec<Result<Vec<f64>, String>> {
    let manual = node.options.terminal.method == TerminalMethod::Manual;
    let mut out: Vec<Result<Vec<f64>, String>> =
        vec![Ok(vec![f64::NAN; node.parameters.len()]); points.len()];
    let mut samples: Vec<Option<(Sample, usize, usize)>> = Vec::with_capacity(points.len());
    for (k, point) in points.iter().enumerate() {
        let changes: Vec<f64> = point
            .plan
            .change_times()
            .iter()
            .map(|t| t + point.plan.time_shift)
            .collect();
        let rows = kept_rows(point.time, &changes);
        let mut times: Vec<f64> = rows.iter().map(|&r| point.time[r]).collect();
        let mut values: Vec<f64> = rows.iter().map(|&r| point.values[r]).collect();
        let doses = match &node.dose {
            None => Ok((Vec::new(), Vec::new())),
            Some(dose) => doses_of(point.plan, dose, false),
        };
        let (dose_times, dose_amounts) = match doses {
            Ok(doses) => doses,
            Err(err) => {
                out[k] = Err(failure(node, &format!("ValueError: {err}")));
                samples.push(None);
                continue;
            }
        };
        let n = times.len();
        if n == 0 {
            samples.push(None);
            continue;
        }
        let length = if manual { n } else { padded_length(n) };
        times.resize(length, f64::NAN);
        values.resize(length, f64::NAN);
        let count = dose_times.len();
        samples.push(Some((
            Sample {
                times,
                values,
                dose_times,
                dose_amounts,
            },
            count,
            length,
        )));
    }
    let mut groups: Vec<(usize, usize)> = samples
        .iter()
        .flatten()
        .map(|(_, count, length)| (*count, *length))
        .collect();
    groups.sort_unstable();
    groups.dedup();
    for (count, length) in groups {
        let rows: Vec<usize> = (0..points.len())
            .filter(|&k| {
                samples[k]
                    .as_ref()
                    .is_some_and(|(_, c, l)| *c == count && *l == length)
            })
            .collect();
        let group: Vec<&Sample> = rows
            .iter()
            .map(|&k| &samples[k].as_ref().expect("a sample").0)
            .collect();
        match analyse(node, &group, length, count) {
            Ok(columns) => {
                for (r, &k) in rows.iter().enumerate() {
                    out[k] = Ok(columns.iter().map(|c| c[r]).collect());
                }
            }
            Err(_) => {
                for (&k, sample) in rows.iter().zip(&group) {
                    out[k] = match analyse(node, &[sample], length, count) {
                        Ok(columns) => Ok(columns.iter().map(|c| c[0]).collect()),
                        Err(err) => Err(failure(node, &err)),
                    };
                }
            }
        }
    }
    out
}

/// The failure of the analysis of a point.
fn failure(node: &PkNode, error: &str) -> String {
    format!("The PK observable '{}' failed: {error}", node.id)
}
