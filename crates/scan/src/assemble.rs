//! The assembly of the result of a scan from the outputs of its points, sbmlsim's `_grid`,
//! `_on_grid`, `_Assembly` and `_Compiled.assemble` (`simulator/simulator.py`,
//! `simulator/worker.py`), see the module of [`crate::result`].

use std::collections::HashSet;
use std::ops::Range;

use crate::error::ScanError;
use crate::expand::{Expanded, RESERVED};
use crate::model::ModelInfo;
use crate::observables::Graph;
use crate::plan::{Plan, time_tolerance};
use crate::request::{ScanRequest, Values};
use crate::result::{
    Block, Coord, CoordValues, FailedPoint, Layout, RESULT_CONTRACT, ScanResult, Size, Variable,
};

/// The output of a point which ran: the time and the kept timecourses (on the grid of a run
/// which interpolates), and the kept values per simulation.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PointOutput {
    /// The time first, then the kept timecourses.
    pub rows: Vec<Vec<f64>>,
    /// The kept values per simulation.
    pub scalars: Vec<f64>,
}

/// The rows of a point interpolated onto a grid, the time the grid, sbmlsim's `_on_grid`.
pub(crate) fn interpolated(rows: Vec<Vec<f64>>, grid: &[f64], plan: &Plan) -> Vec<Vec<f64>> {
    let changes: Vec<f64> = plan
        .change_times()
        .iter()
        .map(|t| t + plan.time_shift)
        .collect();
    let weights = sbmlsim_timecourse::grid_weights(
        &rows[0],
        grid,
        time_tolerance(plan.start, plan.end),
        &changes,
        None,
    );
    let mut out: Vec<Vec<f64>> = rows
        .iter()
        .map(|row| weights.iter().map(|w| w.value(row)).collect())
        .collect();
    out[0] = grid.to_vec();
    out
}

/// The factor of a unit of a grid of times into the unit of time of the model, sbmlsim's
/// `time_magnitudes`: a model without a unit of time takes numbers, and a dimensionless
/// quantity, but no quantity of a unit.
///
/// # Errors
///
/// A unit of a model without a unit of time, or one which is no unit of the time of the model.
fn time_factor(unit: &str, model: &ModelInfo) -> Result<f64, ScanError> {
    let registry = model.registry();
    let time_unit = model.time_unit_text();
    if time_unit.is_empty() || time_unit == "dimensionless" {
        return registry.factor(unit, "dimensionless").map_err(|_| {
            ScanError::units(format!(
                "The model has no unit of time, give the times as numbers, not in '{unit}'."
            ))
        });
    }
    registry.factor(unit, time_unit).map_err(|_| {
        ScanError::units(format!(
            "The times in '{unit}' cannot be converted to the unit of time '{time_unit}' of the \
             model."
        ))
    })
}

/// The grid of the result and whether the timecourses are interpolated onto it, sbmlsim's
/// `_grid`: the times of the run in the time unit of the model; without them the output times
/// the simulations share, or, where they differ only in the changes at them, their union with
/// a time as often as the simulation with the most rows at it has it; `None` otherwise.
///
/// # Errors
///
/// An empty grid, or a quantity which is no time of the model.
pub(crate) fn grid(
    time: Option<&Values>,
    expanded: &Expanded,
    model: &ModelInfo,
) -> Result<(Option<Vec<f64>>, bool), ScanError> {
    if let Some(time) = time {
        let grid = match time {
            Values::Numbers(xs) => xs.clone(),
            Values::Quantity { value, unit } => {
                let factor = time_factor(unit, model)?;
                value.iter().map(|x| x * factor).collect()
            }
        };
        if grid.is_empty() {
            return Err(ScanError::request("The grid of times 'time' is empty."));
        }
        return Ok((Some(grid), true));
    }
    // the distinct output times of the simulations, compared with `==` as python's set of
    // tuples compares them (`-0.0` is `0.0`)
    let key = |times: &Option<Vec<f64>>| -> Option<Vec<u64>> {
        times.as_ref().map(|v| {
            v.iter()
                .map(|x| if *x == 0.0 { 0.0f64 } else { *x }.to_bits())
                .collect()
        })
    };
    let mut seen: HashSet<Option<Vec<u64>>> = HashSet::new();
    let mut outputs: Vec<Option<Vec<f64>>> = Vec::new();
    for (s, plan) in expanded.plans.iter().enumerate() {
        let at: Vec<f64> = expanded.at_times[s].iter().flatten().copied().collect();
        let times = plan.output_times(&at);
        if seen.insert(key(&times)) {
            outputs.push(times);
        }
    }
    if let [Some(times)] = outputs.as_slice() {
        return Ok((Some(times.clone()), false));
    }
    let given: Vec<&Vec<f64>> = outputs.iter().flatten().collect();
    // the output times are sorted: a time and how often it appears, in order
    let runs = |times: &[f64]| -> Vec<(f64, usize)> {
        let mut out: Vec<(f64, usize)> = Vec::new();
        for &t in times {
            match out.last_mut() {
                Some((x, n)) if *x == t => *n += 1,
                _ => out.push((t, 1)),
            }
        }
        out
    };
    let distinct =
        |times: &[f64]| -> Vec<f64> { runs(times).into_iter().map(|(t, _)| t).collect() };
    let first = given.first().map(|t| distinct(t));
    if given.len() != outputs.len()
        || given.iter().any(|t| Some(distinct(t)) != first)
        || given
            .iter()
            .any(|t| t.last().is_some_and(|x| x.is_infinite()))
    {
        return Ok((None, false));
    }
    // the times are the same in every output, each as often as in the output with the most
    // rows at it
    let mut counts = runs(given[0]);
    for times in &given[1..] {
        for (count, (_, n)) in counts.iter_mut().zip(runs(times)) {
            count.1 = count.1.max(n);
        }
    }
    let grid = counts
        .into_iter()
        .flat_map(|(t, n)| std::iter::repeat_n(t, n))
        .collect();
    Ok((Some(grid), true))
}

/// What the result is assembled from.
pub(crate) struct Assembly<'a> {
    pub request: &'a ScanRequest,
    pub model: &'a ModelInfo,
    pub expanded: &'a Expanded,
    pub graph: &'a Graph,
    pub grid: Option<&'a [f64]>,
    pub block: Range<usize>,
    /// The first failed points.
    pub errors: Vec<FailedPoint>,
}

/// The times of the changes after the start in the time of the result, of every simulation and
/// of the dimensions with `at`, sbmlsim's `_Compiled.change_times`.
fn change_times(expanded: &Expanded) -> Vec<f64> {
    let mut times: Vec<f64> = Vec::new();
    for (s, plan) in expanded.plans.iter().enumerate() {
        let at = expanded.at_times[s]
            .iter()
            .flatten()
            .filter(|t| **t > plan.start)
            .map(|t| t + plan.time_shift);
        let changes = plan.change_times().into_iter().map(|t| t + plan.time_shift);
        for t in changes.chain(at) {
            if !times.contains(&t) {
                times.push(t);
            }
        }
    }
    times.sort_by(f64::total_cmp);
    times
}

/// The result, see [`crate::result`].
pub(crate) fn assemble(a: &Assembly<'_>, mut outputs: Vec<Option<PointOutput>>) -> ScanResult {
    let dimensions = &a.request.scan.dimensions;
    let n = a.block.len();
    let timecourses = a.graph.timecourses();
    let scalars = a.graph.scalars();
    let timed = !timecourses.is_empty() || a.request.observables.is_empty();
    let n_rows = match a.grid {
        Some(grid) => grid.len(),
        None => outputs
            .iter()
            .flatten()
            .map(|o| o.rows[0].len())
            .max()
            .unwrap_or(0),
    };
    if a.grid.is_some() {
        // the time of a grid is its coordinate, no variable: the time rows are not kept
        for output in outputs.iter_mut().flatten() {
            output.rows[0] = Vec::new();
        }
    }
    let block = a.request.points.is_some();
    let dims: Vec<String> = dimensions.iter().map(|d| d.id.clone()).collect();
    let (axes, shape): (Vec<String>, Vec<usize>) = if block {
        (vec!["_index".to_string()], vec![n])
    } else {
        (dims.clone(), a.expanded.shape.clone())
    };
    let positions: Vec<Vec<usize>> = a.block.clone().map(|i| a.expanded.position(i)).collect();
    let tdim = if a.grid.is_some() { "time" } else { "_point" };
    let mut units: Vec<(String, String)> = Vec::new();
    let mut variables: Vec<Variable> = Vec::new();
    // a variable at a time, its row of every point moved out of the point, so the outputs of
    // the points and the variables of the result are held once together
    let mut row_variable = |j: usize, name: &str| -> Variable {
        let mut values = Vec::with_capacity(n * n_rows);
        for output in &mut outputs {
            match output {
                Some(o) => {
                    let row = std::mem::take(&mut o.rows[j]);
                    let kept = row.len().min(n_rows);
                    values.extend_from_slice(&row[..kept]);
                    values.extend(std::iter::repeat_n(f64::NAN, n_rows - kept));
                }
                None => values.extend(std::iter::repeat_n(f64::NAN, n_rows)),
            }
        }
        let mut var_dims = axes.clone();
        var_dims.push(if j == 0 { "_point" } else { tdim }.to_string());
        let mut var_shape = shape.clone();
        var_shape.push(n_rows);
        Variable {
            name: name.to_string(),
            dims: var_dims,
            shape: var_shape,
            unit: String::new(),
            values,
        }
    };
    if timed {
        units.push(("time".to_string(), a.graph.units["time"].clone()));
        if a.grid.is_none() {
            let mut time = row_variable(0, "time");
            time.unit = a.graph.units["time"].clone();
            variables.push(time);
        }
        for (j, name) in timecourses.iter().enumerate() {
            let mut variable = row_variable(j + 1, name);
            variable.unit = a.graph.units[name].clone();
            units.push((name.clone(), variable.unit.clone()));
            variables.push(variable);
        }
    }
    for (j, name) in scalars.iter().enumerate() {
        let values = outputs
            .iter()
            .map(|o| o.as_ref().map_or(f64::NAN, |o| o.scalars[j]))
            .collect();
        let unit = a.graph.units[name].clone();
        units.push((name.clone(), unit.clone()));
        variables.push(Variable {
            name: name.clone(),
            dims: axes.clone(),
            shape: shape.clone(),
            unit,
            values,
        });
    }
    let mut coords: Vec<Coord> = Vec::new();
    let mut owners: Vec<(String, String)> = Vec::new();
    if block {
        coords.push(Coord {
            name: "_index".to_string(),
            dims: vec!["_index".to_string()],
            unit: None,
            values: CoordValues::Indices(a.block.clone().collect()),
        });
    }
    let along_dims = |k: usize| -> Vec<String> {
        if block {
            vec!["_index".to_string()]
        } else {
            vec![dimensions[k].id.clone()]
        }
    };
    let along = |k: usize, values: &[f64]| -> Vec<f64> {
        if block {
            positions.iter().map(|p| values[p[k]]).collect()
        } else {
            values.to_vec()
        }
    };
    for (k, dimension) in dimensions.iter().enumerate() {
        let labels = if block {
            positions
                .iter()
                .map(|p| dimension.labels[p[k]].clone())
                .collect()
        } else {
            dimension.labels.clone()
        };
        coords.push(Coord {
            name: dimension.id.clone(),
            dims: along_dims(k),
            unit: Some(String::new()),
            values: CoordValues::Labels(labels),
        });
        owners.push((dimension.id.clone(), dimension.id.clone()));
        units.push((dimension.id.clone(), String::new()));
        for (target, values) in dimension.values.iter() {
            let name = if variables.iter().any(|v| v.name == target) || RESERVED.contains(&target) {
                format!("{}.{target}", dimension.id)
            } else if coords.iter().any(|c| c.name == target) {
                continue;
            } else {
                target.to_string()
            };
            let unit = match values.unit() {
                Some(unit) => unit.to_string(),
                None => a.model.unit(target).unwrap_or("").to_string(),
            };
            owners.push((name.clone(), dimension.id.clone()));
            units.push((name.clone(), unit.clone()));
            coords.push(Coord {
                name,
                dims: along_dims(k),
                unit: Some(unit),
                values: CoordValues::Numbers(along(k, values.magnitudes())),
            });
        }
        for (name, values) in dimension.coordinates.iter() {
            if variables.iter().any(|v| v.name == name) || coords.iter().any(|c| c.name == name) {
                continue;
            }
            let unit = values.unit().unwrap_or("").to_string();
            owners.push((name.to_string(), dimension.id.clone()));
            units.push((name.to_string(), unit.clone()));
            coords.push(Coord {
                name: name.to_string(),
                dims: along_dims(k),
                unit: Some(unit),
                values: CoordValues::Numbers(along(k, values.magnitudes())),
            });
        }
    }
    if let (Some(grid), true) = (a.grid, timed) {
        coords.push(Coord {
            name: "time".to_string(),
            dims: vec!["time".to_string()],
            unit: Some(a.graph.units["time"].clone()),
            values: CoordValues::Numbers(grid.to_vec()),
        });
    }
    let flagged = a.request.on_error == crate::request::OnError::Flag;
    if flagged {
        let values = outputs
            .iter()
            .map(|o| if o.is_some() { 0.0 } else { 1.0 })
            .collect();
        variables.push(Variable {
            name: "status".to_string(),
            dims: axes.clone(),
            shape: shape.clone(),
            unit: String::new(),
            values,
        });
        units.push(("status".to_string(), String::new()));
    }
    let mut sizes: Vec<Size> = axes
        .iter()
        .zip(&shape)
        .map(|(name, size)| Size {
            name: name.clone(),
            size: *size,
        })
        .collect();
    if timed {
        sizes.push(Size {
            name: tdim.to_string(),
            size: n_rows,
        });
    }
    ScanResult {
        contract: RESULT_CONTRACT.to_string(),
        sizes,
        dims: dims.clone(),
        layout: if a.grid.is_some() {
            Layout::Grid
        } else {
            Layout::Ragged
        },
        coords,
        variables,
        units,
        changes: change_times(a.expanded),
        errors: flagged.then(|| a.errors.clone()),
        block: block.then(|| Block {
            start: a.block.start,
            stop: a.block.end,
            shape: a.expanded.shape.clone(),
            dims,
            coords: owners,
        }),
    }
}
