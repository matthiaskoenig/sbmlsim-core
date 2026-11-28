//! The request of a scan, contract `scan/1`: the JSON of sbmlsim's `Scan.to_dict()` with the
//! observables, the options of the run, the settings of the integrator and the selections of
//! the model.
//!
//! The types read what sbmlsim writes and nothing else (unknown fields are refused). A number
//! is a JSON number, or the string `"inf"`, `"-inf"` or `"NaN"` (and `null` for NaN), since
//! JSON has none of them. A mapping keeps the order of the request ([`Ordered`]): the order of
//! the values of a pre-initialization, of a change and of a dimension is the order of the
//! values of the requests the planner sends, and the order of the simulations of a dimension
//! is the order of its points.

use std::fmt;
use std::marker::PhantomData;

use sbmlsim_abi::IntegratorSettings;
use serde::de::{self, MapAccess, Visitor};
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value as Json;

/// The version of the request of a scan.
pub const SCAN_CONTRACT: &str = "scan/1";

/// A mapping of the request in the order it was written, which JSON objects keep and a map of
/// sorted keys would lose.
#[derive(Debug, Clone, PartialEq)]
pub struct Ordered<V>(pub Vec<(String, V)>);

impl<V> Default for Ordered<V> {
    fn default() -> Self {
        Ordered(Vec::new())
    }
}

impl<V> Ordered<V> {
    /// The value of a key.
    pub fn get(&self, key: &str) -> Option<&V> {
        self.0.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    /// Whether the mapping has a key.
    pub fn contains(&self, key: &str) -> bool {
        self.get(key).is_some()
    }

    /// The keys in their order.
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.0.iter().map(|(k, _)| k.as_str())
    }

    /// The entries in their order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &V)> {
        self.0.iter().map(|(k, v)| (k.as_str(), v))
    }

    /// The number of entries.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether the mapping is empty.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl<V: Serialize> Serialize for Ordered<V> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.0.len()))?;
        for (k, v) in &self.0 {
            map.serialize_entry(k, v)?;
        }
        map.end()
    }
}

impl<'de, V: Deserialize<'de>> Deserialize<'de> for Ordered<V> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Entries<V>(PhantomData<V>);
        impl<'de, V: Deserialize<'de>> Visitor<'de> for Entries<V> {
            type Value = Ordered<V>;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a mapping")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut access: A) -> Result<Self::Value, A::Error> {
                let mut entries: Vec<(String, V)> = Vec::new();
                while let Some((key, value)) = access.next_entry::<String, V>()? {
                    if entries.iter().any(|(k, _)| *k == key) {
                        return Err(de::Error::custom(format!("the key {key:?} appears twice")));
                    }
                    entries.push((key, value));
                }
                Ok(Ordered(entries))
            }
        }
        deserializer.deserialize_map(Entries(PhantomData))
    }
}

/// A number of the request, which may be infinite or NaN.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Num(pub f64);

impl Num {
    /// The number of a JSON value: a number, `"inf"`, `"-inf"`, `"NaN"` or `null` (NaN).
    fn of(value: &Json) -> Result<f64, String> {
        match value {
            Json::Number(n) => n.as_f64().ok_or_else(|| format!("{n} is no number")),
            Json::Null => Ok(f64::NAN),
            Json::String(s) => match s.as_str() {
                "inf" | "Infinity" => Ok(f64::INFINITY),
                "-inf" | "-Infinity" => Ok(f64::NEG_INFINITY),
                "NaN" | "nan" => Ok(f64::NAN),
                other => Err(format!(
                    "{other:?} is no number (a number, \"inf\", \"-inf\" or \"NaN\")"
                )),
            },
            other => Err(format!("{other} is no number")),
        }
    }

    /// The JSON of a number, a string for one which is not finite.
    fn json(x: f64) -> Json {
        if x.is_finite() {
            serde_json::Number::from_f64(x).map_or(Json::Null, Json::Number)
        } else if x.is_nan() {
            Json::String("NaN".into())
        } else if x > 0.0 {
            Json::String("inf".into())
        } else {
            Json::String("-inf".into())
        }
    }
}

impl Serialize for Num {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        Num::json(self.0).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Num {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Json::deserialize(deserializer)?;
        Num::of(&value).map(Num).map_err(de::Error::custom)
    }
}

/// The keys of a quantity, `{"value": x, "unit": u}`.
fn quantity_parts(map: &serde_json::Map<String, Json>) -> Option<(&Json, &str)> {
    if map.len() == 2
        && let (Some(value), Some(Json::String(unit))) = (map.get("value"), map.get("unit"))
    {
        return Some((value, unit.as_str()));
    }
    None
}

/// A time: a number in the time unit of the simulation (of the model without one) or a
/// quantity.
#[derive(Debug, Clone, PartialEq)]
pub enum Time {
    /// A number.
    Number(f64),
    /// A quantity, its magnitude and its unit as pint writes it.
    Quantity {
        /// The magnitude.
        value: f64,
        /// The unit.
        unit: String,
    },
}

impl Default for Time {
    fn default() -> Self {
        Time::Number(0.0)
    }
}

impl Time {
    fn of(value: &Json) -> Result<Time, String> {
        if let Json::Object(map) = value {
            let (magnitude, unit) = quantity_parts(map)
                .ok_or_else(|| format!("{value} is no time (a number or a quantity)"))?;
            return Ok(Time::Quantity {
                value: Num::of(magnitude)?,
                unit: unit.to_string(),
            });
        }
        Num::of(value).map(Time::Number)
    }

    fn json(&self) -> Json {
        match self {
            Time::Number(x) => Num::json(*x),
            Time::Quantity { value, unit } => {
                serde_json::json!({"value": Num::json(*value), "unit": unit})
            }
        }
    }
}

impl fmt::Display for Time {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Time::Number(x) => f.write_str(&crate::text::number(*x)),
            Time::Quantity { value, unit } => {
                write!(f, "{} {unit}", crate::text::number(*value))
            }
        }
    }
}

/// A value of a change: a number in the unit of its target in the model, a quantity or a
/// formula of the math of PEtab.
///
/// As JSON a number which is not finite is the text `"inf"`, `"-inf"` or `"NaN"` (or `null`
/// for NaN), as sbmlsim writes a request and [`Num`] writes a number, so such a text is the
/// number and no formula: the change of `inf` sets the number infinity, as sbmlsim's
/// compiler of plans did.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// A number.
    Number(f64),
    /// A quantity.
    Quantity {
        /// The magnitude.
        value: f64,
        /// The unit.
        unit: String,
    },
    /// A formula over the symbols of the model, `[S]` the concentration of a species.
    Formula(String),
    /// An uncertain value of sbmlsim, whose nominal value a simulation simulates; the rest of
    /// its declaration is kept as it was written.
    Uncertain {
        /// The nominal value.
        nominal: Box<Value>,
        /// The declaration, `{"nominal", "distribution", ...}`.
        declaration: Json,
    },
}

/// The texts of the numbers which are not finite, which a value reads as numbers.
const NON_FINITE: [&str; 3] = ["inf", "-inf", "NaN"];

impl Value {
    fn of(value: &Json) -> Result<Value, String> {
        match value {
            Json::String(text) if NON_FINITE.contains(&text.as_str()) => {
                Num::of(value).map(Value::Number)
            }
            Json::String(formula) => Ok(Value::Formula(formula.clone())),
            Json::Object(map) => {
                if let Some((magnitude, unit)) = quantity_parts(map) {
                    return Ok(Value::Quantity {
                        value: Num::of(magnitude)?,
                        unit: unit.to_string(),
                    });
                }
                if let (1, Some(declaration)) = (map.len(), map.get("uncertain")) {
                    let nominal = declaration.get("nominal").ok_or_else(|| {
                        format!("the uncertain value {declaration} has no nominal")
                    })?;
                    return Ok(Value::Uncertain {
                        nominal: Box::new(Value::of(nominal)?),
                        declaration: declaration.clone(),
                    });
                }
                Err(format!(
                    "{value} is no value (a number, a quantity, a formula or an uncertain value)"
                ))
            }
            other => Num::of(other).map(Value::Number),
        }
    }

    fn json(&self) -> Json {
        match self {
            Value::Number(x) => Num::json(*x),
            Value::Quantity { value, unit } => {
                serde_json::json!({"value": Num::json(*value), "unit": unit})
            }
            Value::Formula(formula) => Json::String(formula.clone()),
            Value::Uncertain { declaration, .. } => serde_json::json!({"uncertain": declaration}),
        }
    }

    /// The value a simulation simulates, the nominal value of an uncertain one.
    pub fn plain(&self) -> &Value {
        match self {
            Value::Uncertain { nominal, .. } => nominal.plain(),
            other => other,
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Value::Number(x) => f.write_str(&crate::text::number(*x)),
            Value::Quantity { value, unit } => {
                write!(f, "{} {unit}", crate::text::number(*value))
            }
            Value::Formula(formula) => f.write_str(formula),
            Value::Uncertain { nominal, .. } => write!(f, "{nominal}"),
        }
    }
}

/// The values of a dimension or a grid of times: numbers, or a quantity of an array.
#[derive(Debug, Clone, PartialEq)]
pub enum Values {
    /// Numbers.
    Numbers(Vec<f64>),
    /// A quantity of an array.
    Quantity {
        /// The magnitudes.
        value: Vec<f64>,
        /// The unit.
        unit: String,
    },
}

impl Values {
    fn numbers(value: &Json) -> Result<Vec<f64>, String> {
        match value {
            Json::Array(items) => items.iter().map(Num::of).collect(),
            other => Err(format!("{other} is no array of numbers")),
        }
    }

    fn of(value: &Json) -> Result<Values, String> {
        if let Json::Object(map) = value {
            let (magnitude, unit) = quantity_parts(map)
                .ok_or_else(|| format!("{value} is no array of numbers or quantity of one"))?;
            return Ok(Values::Quantity {
                value: Values::numbers(magnitude)?,
                unit: unit.to_string(),
            });
        }
        Values::numbers(value).map(Values::Numbers)
    }

    fn json(&self) -> Json {
        let array = |xs: &[f64]| Json::Array(xs.iter().map(|x| Num::json(*x)).collect());
        match self {
            Values::Numbers(xs) => array(xs),
            Values::Quantity { value, unit } => {
                serde_json::json!({"value": array(value), "unit": unit})
            }
        }
    }

    /// The magnitudes.
    pub fn magnitudes(&self) -> &[f64] {
        match self {
            Values::Numbers(xs) => xs,
            Values::Quantity { value, .. } => value,
        }
    }

    /// The unit of a quantity, `None` for numbers.
    pub fn unit(&self) -> Option<&str> {
        match self {
            Values::Numbers(_) => None,
            Values::Quantity { unit, .. } => Some(unit),
        }
    }
}

/// Serde of a type read from and written as a JSON value.
macro_rules! json_serde {
    ($type:ty) => {
        impl Serialize for $type {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                self.json().serialize(serializer)
            }
        }

        impl<'de> Deserialize<'de> for $type {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let value = Json::deserialize(deserializer)?;
                <$type>::of(&value).map_err(de::Error::custom)
            }
        }
    };
}

json_serde!(Time);
json_serde!(Value);
json_serde!(Values);

/// The request of a scan, see the module.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScanRequest {
    /// The contract, [`SCAN_CONTRACT`].
    pub contract: String,
    /// The scan, sbmlsim's `Scan.to_dict()`.
    pub scan: ScanDefinition,
    /// The observables of the run, sbmlsim's `Observable.to_dict()` of each.
    #[serde(default)]
    pub observables: Vec<Observable>,
    /// The outputs of the result, every one by default; without observables the selections of
    /// the model it keeps.
    #[serde(default)]
    pub keep: Option<Vec<String>>,
    /// A grid of times every timecourse of the result is interpolated onto, numbers in the
    /// time unit of the model or a quantity; the output times of the simulations without it.
    #[serde(default)]
    pub time: Option<Values>,
    /// What a point which fails does.
    #[serde(default)]
    pub on_error: OnError,
    /// The block of the flat indices of the points in C order which run, `[start, stop)`;
    /// every point of the scan without it.
    #[serde(default)]
    pub points: Option<[usize; 2]>,
    /// The settings of the integrator of every request.
    pub settings: IntegratorSettings,
    /// The settings of the requests of the models of a dimension of models by their key, which
    /// differ by model (the absolute tolerance of every state of a model); a model without
    /// them takes `settings`.
    #[serde(default, skip_serializing_if = "Ordered::is_empty")]
    pub model_settings: Ordered<IntegratorSettings>,
    /// The changes of the model of the run (sbmlsim's `AbstractModel.changes`, target to
    /// value), the defaults of the pre-initialization of every simulation of the scan
    /// ([`SimulationDefinition::with_preinit_defaults`]); none with a dimension of models.
    #[serde(default, skip_serializing_if = "Ordered::is_empty")]
    pub changes: Ordered<Value>,
    /// The changes of the models of a dimension of models by their key (sbmlsim's
    /// `AbstractModel.changes`, target to value), the defaults of the pre-initialization of
    /// every simulation of the model ([`SimulationDefinition::with_preinit_defaults`]).
    #[serde(default, skip_serializing_if = "Ordered::is_empty")]
    pub model_changes: Ordered<Ordered<Value>>,
    /// The selections of the model, sbmlsim's `SBMLModel.selections`, `time` among them.
    pub selections: Vec<String>,
}

/// What a point which fails does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnError {
    /// The scan fails with the error of the first point in the order of the scan which fails.
    #[default]
    Raise,
    /// The values of a point which fails are NaN and its status says it failed.
    Flag,
}

/// A scan: a simulation and its dimensions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScanDefinition {
    /// The type, `Scan`.
    #[serde(rename = "type")]
    pub kind: String,
    /// The simulation, which a dimension of simulations replaces.
    pub simulation: SimulationDefinition,
    /// The dimensions, the last one fastest.
    #[serde(default)]
    pub dimensions: Vec<DimensionDefinition>,
}

/// A simulation, sbmlsim's `Simulation.to_dict()`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SimulationDefinition {
    /// The type, `Simulation`.
    #[serde(rename = "type")]
    pub kind: String,
    /// The unit of the times given as numbers, the time unit of the model without one.
    #[serde(default)]
    pub time_unit: Option<String>,
    /// The start, negative times allowed.
    #[serde(default)]
    pub start: Time,
    /// The end.
    pub end: Time,
    /// The changes before the initialization of the model, target to value.
    #[serde(default)]
    pub preinit_changes: Ordered<Value>,
    /// The changes at times in `[start, end]`.
    #[serde(default)]
    pub changes: Vec<ChangeDefinition>,
    /// A pre-equilibration before the start.
    #[serde(default)]
    pub presimulation: Option<SteadyStateDefinition>,
    /// Exact output times, the steps of the integrator without them and without `steps`;
    /// `inf` is the steady state after the end.
    #[serde(default)]
    pub times: Option<Vec<Time>>,
    /// An equidistant output of `steps + 1` points.
    #[serde(default)]
    pub steps: Option<u64>,
    /// Added to the time of the result.
    #[serde(default)]
    pub time_shift: Time,
}

impl SimulationDefinition {
    /// The simulation with default pre-initialization changes, sbmlsim's
    /// `Simulation.with_preinit_defaults`: a default applies unless the simulation sets its
    /// target before the initialization itself, in its own `preinit_changes` or in those of its
    /// presimulation, and comes before them; a change at a time does not replace it. This is how
    /// the changes of a model reach its simulations.
    pub fn with_preinit_defaults(&self, values: &Ordered<Value>) -> SimulationDefinition {
        let own = |target: &str| {
            self.preinit_changes.contains(target)
                || self
                    .presimulation
                    .as_ref()
                    .is_some_and(|p| p.preinit_changes.contains(target))
        };
        let mut preinit: Vec<(String, Value)> = values
            .iter()
            .filter(|(target, _)| !own(target))
            .map(|(target, value)| (target.to_string(), value.clone()))
            .collect();
        preinit.extend(self.preinit_changes.0.iter().cloned());
        SimulationDefinition {
            preinit_changes: Ordered(preinit),
            ..self.clone()
        }
    }
}

/// Changes of the model at one or several times, sbmlsim's `Change.to_dict()`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChangeDefinition {
    /// The times.
    pub times: Vec<Time>,
    /// Target to value, set at every time.
    pub values: Ordered<Value>,
}

/// A pre-equilibration, sbmlsim's `SteadyState.to_dict()`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SteadyStateDefinition {
    /// The changes before the initialization of the steady state.
    #[serde(default)]
    pub preinit_changes: Ordered<Value>,
    /// The absolute tolerance of the rates of change.
    #[serde(default = "steady_atol")]
    pub absolute_tolerance: f64,
    /// The relative tolerance of the rates of change.
    #[serde(default = "steady_rtol")]
    pub relative_tolerance: f64,
    /// The longest pre-equilibration in the time unit of the model.
    #[serde(default = "steady_max_time")]
    pub max_time: f64,
}

fn steady_atol() -> f64 {
    1e-8
}

fn steady_rtol() -> f64 {
    1e-6
}

fn steady_max_time() -> f64 {
    1e8
}

/// What a dimension varies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DimensionKind {
    /// Values of targets.
    Values,
    /// Simulations.
    Simulations,
    /// Models, the SBML of every point as the host gives its compiled model.
    Models,
}

/// A dimension of a scan, sbmlsim's `Dimension.to_dict()`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DimensionDefinition {
    /// The id, its name in the result.
    pub id: String,
    /// What it varies.
    pub kind: DimensionKind,
    /// The labels of its points.
    pub labels: Vec<Json>,
    /// Target to the values of the points, numbers in the unit of the target in the model or a
    /// quantity.
    #[serde(default)]
    pub values: Ordered<Values>,
    /// Label to the simulation of the point, in the order of the points.
    #[serde(default)]
    pub simulations: Ordered<SimulationDefinition>,
    /// Key to the model of the point (its SBML), in the order of the points; the host compiles
    /// the models and gives their descriptions by these keys.
    #[serde(default)]
    pub models: Ordered<Json>,
    /// The time of the values, which makes them a change at that time.
    #[serde(default)]
    pub at: Option<Time>,
    /// The record of the design which created the dimension.
    #[serde(default)]
    pub design: Option<Json>,
    /// Coordinates of the dimension which are never set on a model.
    #[serde(default)]
    pub coordinates: Ordered<Values>,
}

/// An observable of the run, sbmlsim's `Observable.to_dict()` (evaluated by the planner in a
/// later version; its symbols decide the selections of the requests).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", deny_unknown_fields)]
pub enum Observable {
    /// A formula of the math of PEtab over the selections and other observables.
    Formula {
        /// The id.
        id: String,
        /// The formula.
        formula: String,
        /// The declared unit.
        #[serde(default)]
        unit: Option<String>,
    },
    /// The non-compartmental analysis of a timecourse.
    #[serde(rename = "PK")]
    Pk {
        /// The id.
        id: String,
        /// The timecourse.
        selection: String,
        /// The target which the simulations dose, or a fixed dose.
        #[serde(default)]
        dose: Option<Dose>,
        /// The route.
        #[serde(default)]
        route: Option<String>,
        /// The options of the analysis of pkpdutils.
        #[serde(default)]
        options: Option<Json>,
        /// The parameters of the result, every one without them.
        #[serde(default)]
        parameters: Option<Vec<String>>,
    },
    /// A function of the host.
    Custom {
        /// The id.
        id: String,
        /// The function, `module:name`.
        function: String,
        /// The unit.
        unit: String,
        /// The symbols the function reads.
        symbols: Vec<String>,
        /// `scalar` or `timecourse`.
        kind: String,
    },
}

impl Observable {
    /// The id.
    pub fn id(&self) -> &str {
        match self {
            Observable::Formula { id, .. }
            | Observable::Pk { id, .. }
            | Observable::Custom { id, .. } => id,
        }
    }
}

/// The dose of a PK observable: a target of the model or a fixed quantity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Dose {
    /// The target whose values in the plan are the doses.
    Target(String),
    /// A fixed dose.
    Quantity {
        /// The magnitude.
        value: f64,
        /// The unit.
        unit: String,
    },
}
