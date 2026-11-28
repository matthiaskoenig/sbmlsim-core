//! The points of a scan: its dimensions, their product in C order (the last dimension
//! fastest), a block of their flat indices and its chunks, sbmlsim's `Simulator._compile`,
//! `_Compiled.chunks` and `worker.point_plan`.

use crate::error::ScanError;
use crate::model::ModelInfo;
use crate::plan::{Plan, compile_simulation, model_time};
use crate::request::{
    DimensionDefinition, DimensionKind, Ordered, ScanDefinition, SimulationDefinition, Value,
};
use crate::text;
use crate::units::target_values;

/// The most points of a chunk of the engine, sbmlsim's `CORE_MAX_CHUNK`: a chunk is one batch
/// of the engine, which answers with the table of every one of its points at once.
pub const CORE_MAX_CHUNK: usize = 250;

/// The names of the dimensions and variables of a result, which no dimension takes.
pub const RESERVED: [&str; 5] = ["time", "_point", "_index", "statistic", "status"];

/// The names of the result as python's `sorted(RESERVED)` writes them in a message.
pub(crate) fn reserved_text() -> String {
    let mut names = RESERVED;
    names.sort_unstable();
    text::list(&names)
}

/// A dimension of values in the units of a model.
#[derive(Debug, Clone)]
pub struct Vector {
    /// Target to the value of every point of the dimension, in the order of the request.
    pub values: Vec<(String, Vec<f64>)>,
}

/// The scan compiled against its models: a plan per simulation and model, the values of the
/// dimensions in the units of every model and their times.
///
/// The plans are by their key `s * n_models + m` of the simulation `s` and the model `m`, the
/// order of sbmlsim's `plans` (every model of a simulation, then the next simulation); without
/// a dimension of models there is one model, the model of the run, and without a dimension of
/// simulations one simulation.
#[derive(Debug, Clone)]
pub struct Expanded {
    /// The number of points of every dimension.
    pub shape: Vec<usize>,
    /// The position of the dimension of simulations, `None` without one.
    pub simulation_axis: Option<usize>,
    /// The position of the dimension of models, `None` without one.
    pub model_axis: Option<usize>,
    /// The number of models, 1 without a dimension of models.
    pub n_models: usize,
    /// The plan of every simulation and model, by its key.
    pub plans: Vec<Plan>,
    /// Per model, every dimension's values in its units (empty for a dimension of
    /// simulations or of models).
    pub vectors: Vec<Vec<Vector>>,
    /// Per plan, the time of every dimension with `at` in the time unit of its model.
    pub at_times: Vec<Vec<Option<f64>>>,
}

/// The values of a point: the values of the dimensions without a time, and per time of a
/// dimension with `at` the values of its change, as `worker.point_plan` applies them.
#[derive(Debug, Clone, PartialEq)]
pub struct PointValues {
    /// Target to value, in the order of the dimensions and of their targets.
    pub values: Vec<(String, f64)>,
    /// Time to target to value, sorted by time.
    pub timed: Vec<(f64, Vec<(String, f64)>)>,
}

impl Expanded {
    /// Compiles a scan against its models: `models` is the model of the run, or the model of
    /// every point of the dimension of models in its order, `labels` names them in a message
    /// (`""` for the model of the run), and `changes` are the changes of every model, the
    /// defaults of the pre-initialization of its simulations (empty for none).
    ///
    /// The scan is checked by the caller first ([`check_scan`]), since the models
    /// of a dimension of models are resolved from them before.
    ///
    /// # Errors
    ///
    /// A scan which does not fit a model, see the checks of sbmlsim's `Simulator._compile`.
    pub(crate) fn new(
        scan: &ScanDefinition,
        models: &[ModelInfo],
        labels: &[String],
        changes: &[Ordered<Value>],
    ) -> Result<Expanded, ScanError> {
        let axis = |kind: DimensionKind| scan.dimensions.iter().position(|d| d.kind == kind);
        let simulation_axis = axis(DimensionKind::Simulations);
        let model_axis = axis(DimensionKind::Models);
        let simulations: Vec<(String, &SimulationDefinition)> = match simulation_axis {
            None => vec![(String::new(), &scan.simulation)],
            Some(axis) => {
                let dimension = &scan.dimensions[axis];
                dimension
                    .labels
                    .iter()
                    .zip(dimension.simulations.iter())
                    .map(|(label, (_, simulation))| (text::label(label), simulation))
                    .collect()
            }
        };
        let mut plans = Vec::new();
        let mut at_times = Vec::new();
        for (simulation_label, simulation) in &simulations {
            for (m, (model, model_label)) in models.iter().zip(labels).enumerate() {
                let defaults = changes.get(m).filter(|c| !c.is_empty());
                let changed = defaults.map(|c| simulation.with_preinit_defaults(c));
                let simulation = changed.as_ref().unwrap_or(*simulation);
                // sbmlsim's `_where`
                let mut place = "The simulation".to_string();
                if simulation_axis.is_some() {
                    place.push_str(&format!(" '{simulation_label}'"));
                }
                if !model_label.is_empty() {
                    place.push_str(&format!(" on the model '{model_label}'"));
                }
                let plan = compile_simulation(simulation, model).map_err(|e| e.within(&place))?;
                at_times.push(
                    dimension_times(scan, simulation, &plan, model)
                        .map_err(|e| e.within(&place))?,
                );
                plans.push(plan);
            }
        }
        let mut vectors = Vec::new();
        for (model, label) in models.iter().zip(labels) {
            let on = if label.is_empty() {
                "the model".to_string()
            } else {
                format!("the model '{label}'")
            };
            let of_model = scan
                .dimensions
                .iter()
                .map(|dimension| {
                    let values = dimension
                        .values
                        .iter()
                        .map(|(target, values)| {
                            Ok((target.to_string(), target_values(model, target, values)?))
                        })
                        .collect::<Result<Vec<_>, ScanError>>()
                        .map_err(|err| {
                            err.within(&format!(
                                "The dimension '{}' does not fit {on}",
                                dimension.id
                            ))
                        })?;
                    Ok(Vector { values })
                })
                .collect::<Result<Vec<_>, ScanError>>()?;
            vectors.push(of_model);
        }
        Ok(Expanded {
            shape: scan.dimensions.iter().map(|d| d.labels.len()).collect(),
            simulation_axis,
            model_axis,
            n_models: models.len(),
            plans,
            vectors,
            at_times,
        })
    }

    /// The number of points of the scan, 1 without dimensions.
    pub fn size(&self) -> usize {
        self.shape.iter().product()
    }

    /// The index of a point along every dimension, of its flat index in C order.
    pub fn position(&self, index: usize) -> Vec<usize> {
        let mut position = vec![0; self.shape.len()];
        let mut rest = index;
        for (axis, &n) in self.shape.iter().enumerate().rev() {
            position[axis] = rest % n;
            rest /= n;
        }
        position
    }

    /// The simulation of a point.
    pub fn simulation_of(&self, position: &[usize]) -> usize {
        self.simulation_axis.map_or(0, |axis| position[axis])
    }

    /// The model of a point, its index in the models of the scan.
    pub fn model_of(&self, position: &[usize]) -> usize {
        self.model_axis.map_or(0, |axis| position[axis])
    }

    /// The plan of a point, its key in [`Expanded::plans`].
    pub fn plan_of(&self, position: &[usize]) -> usize {
        self.simulation_of(position) * self.n_models + self.model_of(position)
    }

    /// The model of a plan, by its key.
    pub fn model_of_plan(&self, key: usize) -> usize {
        key % self.n_models
    }

    /// The values of a point.
    pub fn point_values(&self, position: &[usize]) -> PointValues {
        let key = self.plan_of(position);
        let vectors = &self.vectors[self.model_of(position)];
        let mut values: Vec<(String, f64)> = Vec::new();
        let mut timed: Vec<(f64, Vec<(String, f64)>)> = Vec::new();
        for (axis, vector) in vectors.iter().enumerate() {
            let at = self.at_times[key][axis];
            for (target, column) in &vector.values {
                let value = column[position[axis]];
                let into = match at {
                    None => &mut values,
                    Some(at) => match timed.iter().position(|(t, _)| *t == at) {
                        Some(k) => &mut timed[k].1,
                        None => {
                            timed.push((at, Vec::new()));
                            &mut timed.last_mut().expect("just pushed").1
                        }
                    },
                };
                match into.iter_mut().find(|(t, _)| t == target) {
                    Some(entry) => entry.1 = value,
                    None => into.push((target.clone(), value)),
                }
            }
        }
        timed.sort_by(|a, b| a.0.total_cmp(&b.0));
        PointValues { values, timed }
    }

    /// The plan of a point, sbmlsim's `worker.point_plan`: the values of the dimensions without
    /// a time applied first, then the changes of the dimensions with one in time order, so a
    /// target in both has the value of the change at its time. `models` are the models of the
    /// scan, of which the point takes its own.
    ///
    /// # Errors
    ///
    /// A target which is no target of the model.
    pub fn point_plan(&self, position: &[usize], models: &[ModelInfo]) -> Result<Plan, ScanError> {
        let model = &models[self.model_of(position)];
        let plan = &self.plans[self.plan_of(position)];
        let point = self.point_values(position);
        let mut plan = plan.with_values(&point.values, None, model)?;
        for (at, values) in &point.timed {
            plan = plan.with_values(values, Some(*at), model)?;
        }
        Ok(plan)
    }

    /// The position of the first point of a plan, by its key: the first point in C order of its
    /// simulation and model, at the index 0 along every other dimension.
    pub fn first_point(&self, key: usize) -> Vec<usize> {
        let mut position = vec![0; self.shape.len()];
        if let Some(axis) = self.simulation_axis {
            position[axis] = key / self.n_models;
        }
        if let Some(axis) = self.model_axis {
            position[axis] = key % self.n_models;
        }
        position
    }

    /// The chunks of a block of points, sbmlsim's `_Compiled.chunks` of a run of the engine
    /// core: the points of every plan in chunks of at most `most` points, in the order of
    /// their first point.
    pub fn chunks(&self, block: &std::ops::Range<usize>, most: usize) -> Vec<Vec<usize>> {
        let size = most.min(block.len()).max(1);
        let mut rows: Vec<Vec<usize>> = vec![Vec::new(); self.plans.len()];
        for index in block.clone() {
            rows[self.plan_of(&self.position(index))].push(index);
        }
        let mut chunks: Vec<Vec<usize>> = Vec::new();
        for rows in &rows {
            chunks.extend(rows.chunks(size).map(<[usize]>::to_vec));
        }
        chunks.sort_by_key(|chunk| chunk[0]);
        chunks
    }
}

/// The checks of a scan which do not need a model: its type, and the checks of sbmlsim's
/// `Dimension` and of `_validate` of its `Scan` of its dimensions, with their messages.
pub(crate) fn check_scan(scan: &ScanDefinition) -> Result<(), ScanError> {
    if scan.kind != "Scan" {
        return Err(ScanError::request(format!(
            "A scan has the type 'Scan', not '{}'.",
            scan.kind
        )));
    }
    check_dimensions(&scan.dimensions)
}

/// The checks of the dimensions of sbmlsim's `Dimension` and of `_validate` of its `Scan`
/// which do not need the model, with their messages.
fn check_dimensions(dimensions: &[DimensionDefinition]) -> Result<(), ScanError> {
    for dimension in dimensions {
        check_dimension(dimension)?;
    }
    let ids: Vec<&str> = dimensions.iter().map(|d| d.id.as_str()).collect();
    let mut duplicates: Vec<&str> = ids
        .iter()
        .copied()
        .filter(|id| ids.iter().filter(|other| *other == id).count() > 1)
        .collect();
    duplicates.sort_unstable();
    duplicates.dedup();
    if !duplicates.is_empty() {
        return Err(ScanError::request(format!(
            "The dimensions {} appear more than once in the scan.",
            text::list(&duplicates)
        )));
    }
    let mut reserved: Vec<&str> = ids
        .iter()
        .copied()
        .filter(|id| RESERVED.contains(id))
        .collect();
    reserved.sort_unstable();
    if !reserved.is_empty() {
        return Err(ScanError::request(format!(
            "The dimension ids {} are names of the result ({}), choose other ids.",
            text::list(&reserved),
            reserved_text()
        )));
    }
    let mut clash: Vec<&str> = ids
        .iter()
        .copied()
        .filter(|id| dimensions.iter().any(|d| d.values.contains(id)))
        .collect();
    clash.sort_unstable();
    if !clash.is_empty() {
        return Err(ScanError::request(format!(
            "The dimension ids {} are changed targets, which are coordinates of the result, \
             choose other ids.",
            text::list(&clash)
        )));
    }
    for (kind, name) in [
        (DimensionKind::Simulations, "simulations"),
        (DimensionKind::Models, "models"),
    ] {
        let count = dimensions.iter().filter(|d| d.kind == kind).count();
        if count > 1 {
            return Err(ScanError::request(format!(
                "A scan has at most one dimension of {name}, this one has {count}."
            )));
        }
    }
    Ok(())
}

/// The checks of sbmlsim's `Dimension` of one dimension.
fn check_dimension(dimension: &DimensionDefinition) -> Result<(), ScanError> {
    let id = dimension.id.as_str();
    let (kind, n) = match dimension.kind {
        DimensionKind::Values => ("values", dimension.values.len()),
        DimensionKind::Simulations => ("simulations", dimension.simulations.len()),
        DimensionKind::Models => ("models", dimension.models.len()),
    };
    let others = [
        ("values", dimension.values.len()),
        ("simulations", dimension.simulations.len()),
        ("models", dimension.models.len()),
    ];
    let given: Vec<&str> = others
        .iter()
        .filter(|(_, k)| *k > 0)
        .map(|(name, _)| *name)
        .collect();
    if given.len() > 1 {
        return Err(ScanError::request(format!(
            "The dimension '{id}' needs exactly one of 'values', 'simulations' and 'models', it \
             has {}.",
            text::list(&given)
        )));
    }
    if n == 0 {
        return Err(ScanError::request(format!(
            "The dimension '{id}' has no {kind}."
        )));
    }
    let points = match dimension.kind {
        DimensionKind::Values => {
            let lengths: Vec<(&str, usize)> = dimension
                .values
                .iter()
                .map(|(target, v)| (target, v.magnitudes().len()))
                .collect();
            if let Some((target, _)) = lengths.iter().find(|(_, n)| *n == 0) {
                return Err(ScanError::request(format!(
                    "The values of '{target}' are empty."
                )));
            }
            if lengths.iter().any(|(_, n)| *n != lengths[0].1) {
                let entries: Vec<String> = lengths
                    .iter()
                    .map(|(target, n)| format!("{}: {n}", text::repr(target)))
                    .collect();
                return Err(ScanError::request(format!(
                    "The values of the dimension '{id}' have different lengths {{{}}}: point k \
                     sets the k-th value of every target.",
                    entries.join(", ")
                )));
            }
            lengths[0].1
        }
        _ => {
            if dimension.at.is_some() {
                return Err(ScanError::request(format!(
                    "The dimension '{id}' of {kind} has a time 'at', which only a dimension of \
                     values has."
                )));
            }
            n
        }
    };
    if dimension.labels.len() != points {
        return Err(ScanError::request(format!(
            "The dimension '{id}' has {points} points, its labels {} do not fit.",
            text::labels(&dimension.labels)
        )));
    }
    if has_repeats(&dimension.labels) {
        return Err(ScanError::request(format!(
            "The labels {} of the dimension '{id}' are not unique.",
            text::labels(&dimension.labels)
        )));
    }
    if dimension.kind != DimensionKind::Values
        && (dimension.design.is_some() || !dimension.coordinates.is_empty())
    {
        return Err(ScanError::request(format!(
            "The dimension '{id}' of {kind} has a design or coordinates, which only a dimension \
             of values has."
        )));
    }
    for (name, values) in dimension.coordinates.iter() {
        if dimension.values.contains(name) {
            return Err(ScanError::request(format!(
                "The coordinate '{name}' of the dimension '{id}' is a target of its values; a \
                 coordinate is never set on a model."
            )));
        }
        let length = values.magnitudes().len();
        if length != points {
            return Err(ScanError::request(format!(
                "The coordinate '{name}' of the dimension '{id}' has the length {length}, the \
                 dimension has {points} points."
            )));
        }
    }
    Ok(())
}

/// Whether a label is given twice (two labels which are equal as JSON values), in a time
/// linear in the number of labels: a label is compared only with the earlier labels of its
/// bucket, a key which equal labels share (a number by its value, a text by itself), so a
/// dimension of many points is checked as fast as python's set of its labels.
fn has_repeats(labels: &[serde_json::Value]) -> bool {
    use serde_json::Value as Json;
    let key = |label: &Json| -> String {
        match label {
            // equal numbers have the same value as a double, `-0.0` the one of `0.0`
            Json::Number(n) => {
                let x = n.as_f64().unwrap_or(f64::NAN);
                format!("n{}", if x == 0.0 { 0 } else { x.to_bits() })
            }
            Json::String(text) => format!("s{text}"),
            Json::Bool(b) => format!("b{b}"),
            Json::Null => "z".to_string(),
            Json::Array(_) | Json::Object(_) => "c".to_string(),
        }
    };
    let mut buckets: std::collections::HashMap<String, Vec<usize>> =
        std::collections::HashMap::new();
    for (k, label) in labels.iter().enumerate() {
        let bucket = buckets.entry(key(label)).or_default();
        if bucket.iter().any(|&j| labels[j] == *label) {
            return true;
        }
        bucket.push(k);
    }
    false
}

/// The time of every dimension with `at` in the time unit of the model for a simulation,
/// sbmlsim's `_at_times`.
fn dimension_times(
    scan: &ScanDefinition,
    simulation: &SimulationDefinition,
    plan: &Plan,
    model: &ModelInfo,
) -> Result<Vec<Option<f64>>, ScanError> {
    let mut times = Vec::new();
    let mut seen: Vec<(f64, &str, &str)> = Vec::new();
    for dimension in &scan.dimensions {
        let Some(at) = &dimension.at else {
            // a target set before the initialization by two dimensions
            for target in dimension.values.keys() {
                if let Some((_, _, other)) =
                    seen.iter().find(|(t, s, _)| t.is_nan() && *s == target)
                {
                    return Err(ScanError::request(format!(
                        "The target '{target}' is set before the initialization by the \
                         dimensions '{other}' and '{}': a point sets a target once.",
                        dimension.id
                    )));
                }
                seen.push((f64::NAN, target, &dimension.id));
            }
            times.push(None);
            continue;
        };
        let at_model = model_time(simulation, at, model)?;
        if !(plan.start <= at_model && at_model <= plan.end) {
            return Err(ScanError::request(format!(
                "The time {at} of the dimension '{}' is outside of the simulation [{}, {}] in \
                 the time unit of the model.",
                dimension.id,
                text::number(plan.start),
                text::number(plan.end)
            )));
        }
        for target in dimension.values.keys() {
            if let Some((_, _, other)) =
                seen.iter().find(|(t, s, _)| *t == at_model && *s == target)
            {
                return Err(ScanError::request(format!(
                    "The dimensions '{other}' and '{}' set '{target}' at the same time {}.",
                    dimension.id,
                    text::number(at_model)
                )));
            }
            seen.push((at_model, target, &dimension.id));
        }
        times.push(Some(at_model));
    }
    Ok(times)
}

#[cfg(test)]
mod label_tests {
    use super::has_repeats;
    use serde_json::json;

    #[test]
    fn a_label_given_twice_is_a_repeat() {
        assert!(!has_repeats(&[json!(0), json!(1), json!("a"), json!(1.5)]));
        assert!(has_repeats(&[json!(0), json!("a"), json!("a")]));
        assert!(has_repeats(&[json!(2.5), json!(1), json!(2.5)]));
        assert!(has_repeats(&[json!(0.0), json!(-0.0)]));
        assert!(has_repeats(&[
            json!([1, 2]),
            json!({"a": 1}),
            json!([1, 2])
        ]));
        // labels of JSON values: the number 1 is not the number 1.0, nor the text "1"
        assert!(!has_repeats(&[
            json!(1),
            json!(1.0),
            json!("1"),
            json!(true)
        ]));
        let many: Vec<_> = (0..100_000).map(|k| json!(k)).collect();
        assert!(!has_repeats(&many));
    }
}
