//! The statistics of the analyses (`sbmlsim-stats`) in a browser: every function of
//! `sbmlsim_core.stats` of python, which `js/stats.mjs` calls by its name.
//!
//! One call of the C ABI, `stats_call(request, request_len, values, values_len, out)`, runs a
//! function: the request is the UTF-8 JSON of `{"function": <the name of python>, <its
//! arguments by their names of python>...}`, and the values are the little-endian bytes of the
//! doubles of the arguments, one after the other. The JSON carries the structure and no double:
//!
//! - a double, alone or of an array of any shape, is `{"at": <its index in the values>,
//!   "shape": [...]}`, a double alone of the shape `[]`, so every double crosses as its bits,
//!   `NaN`, the infinities and `-0.0` included (of at most 64 dimensions, as numpy),
//! - an integer is a string of its decimal digits, so a seed of 64 bits crosses exactly,
//! - a boolean, a name (a scale, a design, a side, a family, a kind) and a text are JSON,
//! - an argument which is absent (or `null`) takes the default of python.
//!
//! The call writes four words to `out`, as [`crate::compile`]: for a result 1 and the JSON of
//! the answer and the bytes of its doubles, in the same form (a tuple of python an array, a
//! dictionary an object, `None` `null`, an integer a JSON number); for an error 0 and the JSON
//! of `{"kind": <the name of the ErrorKind of the crate>, "message": ...}` and no values. The
//! kind of a request the binding cannot read (an unknown function, a missing or unknown
//! argument, an index past the values) is `TypeError`, as python raises one for such a call.
//!
//! The arguments are checked as the python binding checks them, by [`sbmlsim_stats::binding`],
//! so an argument the python refuses is refused with its message. `{"function": "constants"}`
//! gives the constants of `sbmlsim_core.stats`.

use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value, json};

use sbmlsim_stats::correlation::{self, NEAREST_MAX_ITERATIONS, NEAREST_TOL, PSD_RTOL};
use sbmlsim_stats::draws::{self, Design};
use sbmlsim_stats::fisher::{self, DEFAULT_RANK_TOLERANCE};
use sbmlsim_stats::testing::{self, MARGIN, Side};
use sbmlsim_stats::{Scale, StatsError, binding, metrics, posterior, uncertainty};

use crate::{bytes, give};

/// The error of a call: the name of the kind of the error (of [`sbmlsim_stats::ErrorKind`], or
/// `TypeError` of a request the binding cannot read) and its message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    /// The name of the kind: `ValueError`, `OverflowError`, `ZeroDivisionError`,
    /// `IndexError` or `TypeError`.
    pub kind: &'static str,
    /// The message.
    pub message: String,
}

impl From<StatsError> for Failure {
    fn from(error: StatsError) -> Self {
        Self {
            kind: error.kind.name(),
            message: error.message,
        }
    }
}

/// A request the binding cannot read.
fn type_error(message: impl Into<String>) -> Failure {
    Failure {
        kind: "TypeError",
        message: message.into(),
    }
}

/// The answer of a call: the JSON and the doubles it refers to.
#[derive(Debug, Clone, PartialEq)]
pub struct Answer {
    /// The JSON of the answer, every double an `{"at", "shape"}` of [`Answer::values`].
    pub json: String,
    /// The doubles of the answer.
    pub values: Vec<f64>,
}

/// A double or an array of doubles of the request: its index in the values and its shape.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Doubles {
    at: usize,
    shape: Vec<usize>,
}

/// An integer of the request, from the string of its digits.
#[derive(Deserialize)]
#[serde(try_from = "String")]
struct Integer(i128);

impl TryFrom<String> for Integer {
    type Error = String;

    fn try_from(digits: String) -> Result<Self, String> {
        digits
            .parse()
            .map(Self)
            .map_err(|_| format!("{digits:?} is not an integer"))
    }
}

impl Integer {
    /// The integer as an `i64`, which every integer argument of python is but the seed.
    fn i64(&self) -> Result<i64, Failure> {
        i64::try_from(self.0)
            .map_err(|_| type_error(format!("the integer {} is out of range", self.0)))
    }

    /// The seed of a stream: a negative one is the error of numpy.
    fn seed(&self) -> Result<u64, Failure> {
        if self.0 < 0 {
            return Err(StatsError::value("expected non-negative integer").into());
        }
        u64::try_from(self.0)
            .map_err(|_| type_error(format!("the seed {} is out of range", self.0)))
    }
}

/// The most dimensions of an array of numpy.
const MAX_DIMENSIONS: usize = 64;

/// The doubles of a request.
struct Values(Vec<f64>);

impl Values {
    /// The doubles of the little-endian bytes.
    fn of(bytes: &[u8]) -> Result<Self, Failure> {
        if !bytes.len().is_multiple_of(8) {
            return Err(type_error(format!(
                "the values are {} bytes, not a multiple of 8",
                bytes.len()
            )));
        }
        Ok(Self(
            bytes
                .as_chunks::<8>()
                .0
                .iter()
                .map(|&chunk| f64::from_le_bytes(chunk))
                .collect(),
        ))
    }

    /// The doubles of an array of the request and its shape.
    fn array<'a>(&'a self, doubles: &'a Doubles) -> Result<(&'a [f64], &'a [usize]), Failure> {
        // the dimensions numpy holds, which bounds the recursion of a message of nested lists
        if doubles.shape.len() > MAX_DIMENSIONS {
            return Err(StatsError::value(format!(
                "maximum supported dimension for an ndarray is currently {MAX_DIMENSIONS}, found {}",
                doubles.shape.len()
            ))
            .into());
        }
        let len = doubles
            .shape
            .iter()
            .try_fold(1_usize, |len, &n| len.checked_mul(n));
        let end = len.and_then(|len| doubles.at.checked_add(len));
        match end {
            Some(end) if end <= self.0.len() => Ok((&self.0[doubles.at..end], &doubles.shape)),
            _ => Err(type_error(format!(
                "the array at {} of the shape {:?} is past the {} values",
                doubles.at,
                doubles.shape,
                self.0.len()
            ))),
        }
    }

    /// The doubles of an array of any shape, in the order numpy ravels it.
    fn vector<'a>(&'a self, doubles: &'a Doubles) -> Result<&'a [f64], Failure> {
        Ok(self.array(doubles)?.0)
    }

    /// A double alone.
    fn number(&self, doubles: &Doubles) -> Result<f64, Failure> {
        match self.array(doubles)? {
            (&[x], []) => Ok(x),
            (_, shape) => Err(type_error(format!(
                "a number is a double of the shape [], not {shape:?}"
            ))),
        }
    }

    /// A double alone which may be absent, the default then.
    fn number_or(&self, doubles: Option<&Doubles>, default: f64) -> Result<f64, Failure> {
        doubles.map_or(Ok(default), |doubles| self.number(doubles))
    }

    /// Booleans of any shape, `np.asarray(values, dtype=bool)`: a value which is not zero is
    /// true, a not a number included.
    fn booleans(&self, doubles: &Doubles) -> Result<Vec<bool>, Failure> {
        Ok(self.vector(doubles)?.iter().map(|&x| x != 0.0).collect())
    }
}

/// The answer under construction: its doubles.
#[derive(Default)]
struct Out(Vec<f64>);

impl Out {
    /// An array of the shape, as `{"at", "shape"}` of the doubles of the answer.
    fn array(&mut self, values: Vec<f64>, shape: &[usize]) -> Value {
        let at = self.0.len();
        self.0.extend(values);
        json!({ "at": at, "shape": shape })
    }

    /// A vector.
    fn vector(&mut self, values: Vec<f64>) -> Value {
        let n = values.len();
        self.array(values, &[n])
    }

    /// A double alone.
    fn number(&mut self, x: f64) -> Value {
        self.array(vec![x], &[])
    }
}

/// The name a JSON value is, a string; `None` of another value.
fn name(value: &Value) -> Option<&str> {
    value.as_str()
}

/// The text a message names a value by which is not a name: a string as python writes it,
/// `repr`, another value as its JSON.
fn written(value: &Value) -> String {
    match value {
        Value::String(text) => binding::string_repr(text),
        other => other.to_string(),
    }
}

/// The member of a name, `parse` of its string, or the error of the python of another value.
fn named<T>(
    value: &Value,
    parse: fn(&str) -> Option<T>,
    unknown: fn(&str) -> StatsError,
) -> Result<T, Failure> {
    name(value)
        .and_then(parse)
        .ok_or_else(|| unknown(&written(value)).into())
}

/// The scales of the parameters of their names.
fn scales(names: &[Value]) -> Result<Vec<Scale>, Failure> {
    names
        .iter()
        .map(|value| named(value, Scale::from_name, binding::unknown_scale))
        .collect()
}

/// The arguments of a function, the fields of its request but `function`.
fn arguments<T: DeserializeOwned>(
    function: &str,
    request: Map<String, Value>,
) -> Result<T, Failure> {
    serde_json::from_value(Value::Object(request))
        .map_err(|error| type_error(format!("{function}(): {error}")))
}

// The arguments of the functions, by their names of python.

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Residuals {
    residuals: Doubles,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FromMse {
    mse: Doubles,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Criterion {
    mse: Doubles,
    n: Integer,
    k: Integer,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Predictions {
    y_observed: Doubles,
    y_predicted: Doubles,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Fold {
    y_observed: Doubles,
    y_predicted: Doubles,
    fold: Option<Doubles>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SdAccepted {
    y_observed: Doubles,
    y_predicted: Doubles,
    y_sd: Doubles,
    k: Option<Doubles>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Nres {
    nres: Doubles,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Observed {
    observed: Doubles,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Ratios {
    observed: Doubles,
    predicted: Doubles,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Jacobian {
    jacobian: Doubles,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FisherArguments {
    information: Doubles,
    cost: Doubles,
    n: Integer,
    likelihood: Option<bool>,
    rank_tolerance: Option<Doubles>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Information {
    information: Doubles,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OfDecomposition {
    spectrum: Doubles,
    inverse: Doubles,
    cost: Doubles,
    n: Integer,
    likelihood: Option<bool>,
    rank_tolerance: Option<Doubles>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OfX {
    x: Doubles,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OfQ {
    q: Doubles,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OfXDf {
    x: Doubles,
    df: Doubles,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OfQDf {
    q: Doubles,
    df: Doubles,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Intervals {
    values: Doubles,
    standard_errors: Doubles,
    scales: Vec<Value>,
    level: Option<Doubles>,
    dof: Option<Doubles>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RelativeErrors {
    values: Doubles,
    standard_errors: Doubles,
    scales: Vec<Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Laplace {
    precision: Doubles,
    pids: Option<Vec<Value>>,
    opid: Option<String>,
    rank_tolerance: Option<Doubles>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Correlation {
    r: Doubles,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LogSd {
    scale: Value,
    sd: Doubles,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LognormalCv {
    sigma: Doubles,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Draws {
    x: Doubles,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Quantile {
    x: Doubles,
    q: Doubles,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Quantiles {
    x: Doubles,
    qs: Doubles,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct QuantileCi {
    x: Doubles,
    q: Doubles,
    level: Option<Doubles>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FirstOrder {
    x: Doubles,
    y: Doubles,
    n_bins: Option<Integer>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Precise {
    values: Doubles,
    rel_tol: Doubles,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct IntervalParameters {
    low: Doubles,
    high: Doubles,
    level: Option<Doubles>,
    lognormal: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UnitPoints {
    d: Integer,
    n: Integer,
    design: Option<Value>,
    seed: Integer,
    start: Option<Integer>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Copula {
    u: Doubles,
    r: Doubles,
}

/// A marginal of a covariance, `(family, mean, sd)` of `_moments`.
#[derive(Deserialize)]
struct Moments(String, Doubles, Doubles);

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NormalScoreRho {
    m: Doubles,
    a: Moments,
    b: Moments,
    scale: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Nearest {
    matrix: Doubles,
    max_iterations: Option<Integer>,
    tol: Option<Doubles>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct IsPsd {
    matrix: Doubles,
    rtol: Option<Doubles>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Validate {
    targets: Vec<String>,
    matrix: Doubles,
    kind: Option<String>,
    /// The number of the units, `tuple(units)` of python, which the validation counts.
    units: Option<Integer>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PValues {
    satisfied: Doubles,
    p0: Doubles,
    side: Value,
    margin: Option<Doubles>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfidenceSequence {
    satisfied: Doubles,
    level: Option<Doubles>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Decide {
    p_values: Doubles,
    alpha: Doubles,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DecisionArguments {
    p_greater: Doubles,
    p_less: Doubles,
    p_equivalent: Doubles,
    alpha: Doubles,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Needed {
    estimate: Doubles,
    n: Integer,
    p0: Doubles,
    margin: Doubles,
    alpha: Doubles,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FamilyLevel {
    level: Doubles,
    m: Integer,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GroupMeans {
    values: Doubles,
    size: Integer,
    groups: Integer,
    geometric: Option<bool>,
    seed: Integer,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Bands {
    distribution: Doubles,
    qs: Doubles,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NoArguments {}

/// Runs the function of a request on its values: the answer, or the error.
///
/// # Errors
///
/// The error of the function, or a `TypeError` of a request the binding cannot read.
pub fn call(request: &[u8], values: &[u8]) -> Result<Answer, Failure> {
    let request: Value = serde_json::from_slice(request)
        .map_err(|error| type_error(format!("the request is not JSON: {error}")))?;
    let Value::Object(mut request) = request else {
        return Err(type_error("the request is not an object"));
    };
    let function = match request.remove("function") {
        Some(Value::String(function)) => function,
        _ => return Err(type_error("the request names no function")),
    };
    let v = Values::of(values)?;
    let mut out = Out::default();
    let answer = run(&function, request, &v, &mut out)?;
    Ok(Answer {
        json: answer.to_string(),
        values: out.0,
    })
}

/// The answer of a function of its arguments.
#[allow(clippy::too_many_lines)]
fn run(
    function: &str,
    request: Map<String, Value>,
    v: &Values,
    out: &mut Out,
) -> Result<Value, Failure> {
    let answer = match function {
        "constants" => {
            let NoArguments {} = arguments(function, request)?;
            json!({
                "BISECTIONS": testing::BISECTIONS,
                "DEFAULT_RANK_TOLERANCE": out.number(DEFAULT_RANK_TOLERANCE),
                "EPS": out.number(draws::EPS),
                "MARGIN": out.number(MARGIN),
                "MAX_NEEDED": testing::MAX_NEEDED,
                "MIN_POINTS_SUBSET_AGREEMENT": metrics::MIN_POINTS_SUBSET_AGREEMENT,
                "MIXTURE": [out.number(testing::MIXTURE.0), out.number(testing::MIXTURE.1)],
                "PSD_RTOL": out.number(PSD_RTOL),
            })
        }
        "sse" => {
            let a: Residuals = arguments(function, request)?;
            out.number(metrics::sse(v.vector(&a.residuals)?))
        }
        "mse" => {
            let a: Residuals = arguments(function, request)?;
            out.number(metrics::mse(v.vector(&a.residuals)?)?)
        }
        "rmse_from_mse" => {
            let a: FromMse = arguments(function, request)?;
            out.number(metrics::rmse_from_mse(v.number(&a.mse)?)?)
        }
        "aic_from_mse" | "bic_from_mse" => {
            let a: Criterion = arguments(function, request)?;
            let (mse, n, k) = (v.number(&a.mse)?, a.n.i64()?, a.k.i64()?);
            let criterion = if function == "aic_from_mse" {
                metrics::aic_from_mse(mse, n, k)?
            } else {
                metrics::bic_from_mse(mse, n, k)?
            };
            out.number(criterion)
        }
        "r_squared" | "gmfe" | "afe" | "subset_agreement" => {
            let a: Predictions = arguments(function, request)?;
            let (y, f) = (v.vector(&a.y_observed)?, v.vector(&a.y_predicted)?);
            match function {
                "r_squared" => out.number(metrics::r_squared(y, f)?),
                "gmfe" => out.number(metrics::gmfe(y, f)?),
                "afe" => out.number(metrics::afe(y, f)?),
                _ => match metrics::subset_agreement(y, f)? {
                    Some((bias, half, ci_bias, ci_limit)) => json!([
                        out.number(bias),
                        out.number(half),
                        out.number(ci_bias),
                        out.number(ci_limit)
                    ]),
                    None => Value::Null,
                },
            }
        }
        "fold_counts" | "fold_accepted" => {
            let a: Fold = arguments(function, request)?;
            let (y, f) = (v.vector(&a.y_observed)?, v.vector(&a.y_predicted)?);
            let fold = v.number_or(a.fold.as_ref(), 2.0)?;
            if function == "fold_counts" {
                let (within, with_ratio) = metrics::fold_counts(y, f, fold)?;
                json!([within, with_ratio])
            } else {
                out.vector(metrics::fold_accepted(y, f, fold)?)
            }
        }
        "sd_accepted" => {
            let a: SdAccepted = arguments(function, request)?;
            let (y, f, sd) = (
                v.vector(&a.y_observed)?,
                v.vector(&a.y_predicted)?,
                v.vector(&a.y_sd)?,
            );
            let k = v.number_or(a.k.as_ref(), 2.0)?;
            out.vector(metrics::sd_accepted(y, f, sd, k)?)
        }
        "nrmse" => {
            let a: Nres = arguments(function, request)?;
            out.number(metrics::nrmse(v.vector(&a.nres)?))
        }
        "guest_limits" => {
            let a: Observed = arguments(function, request)?;
            let (ratios, shape) = v.array(&a.observed)?;
            let (low, high): (Vec<f64>, Vec<f64>) = ratios
                .iter()
                .map(|&ratio| metrics::guest_limits(ratio))
                .unzip();
            json!([out.array(low, shape), out.array(high, shape)])
        }
        "within_guest" => {
            let a: Ratios = arguments(function, request)?;
            let (observed, observed_shape) = v.array(&a.observed)?;
            let (predicted, predicted_shape) = v.array(&a.predicted)?;
            out.number(binding::within_guest(
                observed,
                observed_shape,
                predicted,
                predicted_shape,
            )?)
        }
        "gram" => {
            let a: Jacobian = arguments(function, request)?;
            let (jacobian, shape) = v.array(&a.jacobian)?;
            let (gram, k) = binding::gram(jacobian, shape)?;
            out.array(gram, &[k, k])
        }
        "fisher_statistics" => {
            let a: FisherArguments = arguments(function, request)?;
            let (information, shape) = v.array(&a.information)?;
            let rank_tolerance = v.number_or(a.rank_tolerance.as_ref(), DEFAULT_RANK_TOLERANCE)?;
            let (s, k) = binding::fisher_statistics(
                information,
                shape,
                v.number(&a.cost)?,
                a.n.i64()?,
                a.likelihood.unwrap_or(false),
                rank_tolerance,
            )?;
            fisher_json(out, s, k)
        }
        "fisher_decomposition" => {
            let a: Information = arguments(function, request)?;
            let (information, shape) = v.array(&a.information)?;
            let (spectrum, inverse, k) = binding::fisher_decomposition(information, shape)?;
            json!([out.vector(spectrum), out.array(inverse, &[k, k])])
        }
        "fisher_statistics_of_decomposition" => {
            let a: OfDecomposition = arguments(function, request)?;
            let (spectrum, spectrum_shape) = v.array(&a.spectrum)?;
            let (inverse, inverse_shape) = v.array(&a.inverse)?;
            let rank_tolerance = v.number_or(a.rank_tolerance.as_ref(), DEFAULT_RANK_TOLERANCE)?;
            let (s, k) = binding::fisher_statistics_of_decomposition(
                spectrum,
                spectrum_shape,
                inverse,
                inverse_shape,
                v.number(&a.cost)?,
                a.n.i64()?,
                a.likelihood.unwrap_or(false),
                rank_tolerance,
            )?;
            fisher_json(out, s, k)
        }
        "norm_cdf" | "erf" | "gamma" => {
            let a: OfX = arguments(function, request)?;
            unary(function, v, out, &a.x)?
        }
        "norm_ppf" => {
            let a: OfQ = arguments(function, request)?;
            unary(function, v, out, &a.q)?
        }
        "t_cdf" | "chi2_cdf" => {
            let a: OfXDf = arguments(function, request)?;
            binary(function, v, out, &a.x, &a.df)?
        }
        "t_ppf" | "chi2_ppf" => {
            let a: OfQDf = arguments(function, request)?;
            binary(function, v, out, &a.q, &a.df)?
        }
        "fisher_intervals" => {
            let a: Intervals = arguments(function, request)?;
            let (values, se) = (v.vector(&a.values)?, v.vector(&a.standard_errors)?);
            let scales = scales(&a.scales)?;
            let level = v.number_or(a.level.as_ref(), 0.95)?;
            let dof = a.dof.as_ref().map(|dof| v.number(dof)).transpose()?;
            let (lower, upper): (Vec<f64>, Vec<f64>) =
                fisher::intervals(values, se, &scales, level, dof)?
                    .into_iter()
                    .unzip();
            json!([out.vector(lower), out.vector(upper)])
        }
        "fisher_relative_errors" => {
            let a: RelativeErrors = arguments(function, request)?;
            let (values, se) = (v.vector(&a.values)?, v.vector(&a.standard_errors)?);
            let scales = scales(&a.scales)?;
            out.vector(fisher::relative_errors(values, se, &scales)?)
        }
        "posterior_laplace" => {
            let a: Laplace = arguments(function, request)?;
            let (precision, shape) = v.array(&a.precision)?;
            let rank_tolerance = v.number_or(a.rank_tolerance.as_ref(), DEFAULT_RANK_TOLERANCE)?;
            // a pid as python writes it in a list
            let names = a.pids.map(|pids| pids.iter().map(written).collect());
            let (covariance, k) = binding::posterior_laplace(
                precision,
                shape,
                names,
                a.opid.as_deref(),
                rank_tolerance,
            )?;
            out.array(covariance, &[k, k])
        }
        "rank_correlation" | "normal_correlation" => {
            let a: Correlation = arguments(function, request)?;
            let (r, shape) = v.array(&a.r)?;
            let (correlation, k) = if function == "rank_correlation" {
                binding::rank_correlation(r, shape)?
            } else {
                binding::normal_correlation(r, shape)?
            };
            out.array(correlation, &[k, k])
        }
        "log_sd" => {
            let a: LogSd = arguments(function, request)?;
            let scale = named(&a.scale, Scale::from_name, binding::unknown_scale)?;
            out.number(posterior::log_sd(scale, v.number(&a.sd)?))
        }
        "lognormal_cv" => {
            let a: LognormalCv = arguments(function, request)?;
            out.number(posterior::lognormal_cv(v.number(&a.sigma)?))
        }
        "mean_se" | "sd_se" => {
            let a: Draws = arguments(function, request)?;
            let x = v.vector(&a.x)?;
            let (statistic, se, n) = if function == "mean_se" {
                uncertainty::mean_se(x)
            } else {
                uncertainty::sd_se(x)?
            };
            json!([out.number(statistic), out.number(se), n])
        }
        "quantile" => {
            let a: Quantile = arguments(function, request)?;
            out.number(uncertainty::quantile(v.vector(&a.x)?, v.number(&a.q)?)?)
        }
        "quantiles" => {
            let a: Quantiles = arguments(function, request)?;
            out.vector(uncertainty::quantiles(v.vector(&a.x)?, v.vector(&a.qs)?)?)
        }
        "quantile_ci" => {
            let a: QuantileCi = arguments(function, request)?;
            let level = v.number_or(a.level.as_ref(), 0.95)?;
            let (q, low, high, n) =
                uncertainty::quantile_ci(v.vector(&a.x)?, v.number(&a.q)?, level)?;
            json!([out.number(q), out.number(low), out.number(high), n])
        }
        "first_order_given_data" => {
            let a: FirstOrder = arguments(function, request)?;
            let n_bins = a.n_bins.as_ref().map(Integer::i64).transpose()?;
            out.number(binding::first_order_given_data(
                v.vector(&a.x)?,
                v.vector(&a.y)?,
                n_bins,
            )?)
        }
        "precise" => {
            let a: Precise = arguments(function, request)?;
            let (precise, worst) =
                uncertainty::precise(v.vector(&a.values)?, v.number(&a.rel_tol)?)?;
            json!([precise, out.number(worst)])
        }
        "interval_parameters" => {
            let a: IntervalParameters = arguments(function, request)?;
            let level = v.number_or(a.level.as_ref(), 0.95)?;
            let (first, second) = uncertainty::interval_parameters(
                v.number(&a.low)?,
                v.number(&a.high)?,
                level,
                a.lognormal,
            )?;
            json!([out.number(first), out.number(second)])
        }
        "unit_points" => {
            let a: UnitPoints = arguments(function, request)?;
            // the arguments in the order of python, which reads them (the seed refused when it
            // is negative) before it refuses a design which is no name; `null` is the default
            let (d, n) = (a.d.i64()?, a.n.i64()?);
            let seed = a.seed.seed()?;
            let start = a.start.as_ref().map_or(Ok(0), Integer::i64)?;
            let design = match &a.design {
                Some(value) => named(value, Design::from_name, binding::unknown_design)?,
                None => Design::Sobol,
            };
            let (points, n, d) = binding::unit_points(d, n, design, seed, start)?;
            out.array(points, &[n, d])
        }
        "copula" => {
            let a: Copula = arguments(function, request)?;
            let (u, u_shape) = v.array(&a.u)?;
            let (r, r_shape) = v.array(&a.r)?;
            let mut points = u.to_vec();
            let (n, d) = binding::copula(&mut points, u_shape, r, r_shape)?;
            out.array(points, &[n, d])
        }
        "normal_score_rho" => {
            let a: NormalScoreRho = arguments(function, request)?;
            let scale = a.scale.as_deref().unwrap_or("linear");
            let marginal = |Moments(family, mean, sd): &Moments| -> Result<_, Failure> {
                Ok(binding::marginal(
                    family,
                    v.number(mean)?,
                    v.number(sd)?,
                    scale,
                )?)
            };
            let (a_marginal, b_marginal) = (marginal(&a.a)?, marginal(&a.b)?);
            out.number(correlation::normal_score_rho(
                v.number(&a.m)?,
                a_marginal,
                b_marginal,
            )?)
        }
        "nearest_correlation" => {
            let a: Nearest = arguments(function, request)?;
            let (m, shape) = v.array(&a.matrix)?;
            let max_iterations = match &a.max_iterations {
                Some(iterations) => iterations.i64()?,
                None => NEAREST_MAX_ITERATIONS as i64,
            };
            let tol = v.number_or(a.tol.as_ref(), NEAREST_TOL)?;
            let (nearest, k) = binding::nearest_correlation(m, shape, max_iterations, tol)?;
            out.array(nearest, &[k, k])
        }
        "is_psd" => {
            let a: IsPsd = arguments(function, request)?;
            let (m, shape) = v.array(&a.matrix)?;
            json!(binding::is_psd(
                m,
                shape,
                v.number_or(a.rtol.as_ref(), PSD_RTOL)?
            )?)
        }
        "validate_covariance" => {
            let a: Validate = arguments(function, request)?;
            let (m, shape) = v.array(&a.matrix)?;
            let targets: Vec<&str> = a.targets.iter().map(String::as_str).collect();
            let is_correlation = a.kind.as_deref() == Some("correlation");
            let units = a
                .units
                .as_ref()
                .map(|units| {
                    usize::try_from(units.0)
                        .map_err(|_| type_error(format!("{} units are no count", units.0)))
                })
                .transpose()?;
            correlation::validate(m, shape, &targets, is_correlation, units)?;
            Value::Null
        }
        "p_values" => {
            let a: PValues = arguments(function, request)?;
            let side = named(&a.side, Side::from_name, binding::unknown_side)?;
            let satisfied = v.booleans(&a.satisfied)?;
            let margin = v.number_or(a.margin.as_ref(), MARGIN)?;
            out.vector(testing::p_values(
                &satisfied,
                v.number(&a.p0)?,
                side,
                margin,
            ))
        }
        "confidence_sequence" => {
            let a: ConfidenceSequence = arguments(function, request)?;
            let satisfied = v.booleans(&a.satisfied)?;
            let level = v.number_or(a.level.as_ref(), 0.95)?;
            let (low, high) = testing::confidence_sequence(&satisfied, 1.0 - level)?;
            json!([out.number(low), out.number(high)])
        }
        "decide" => {
            let a: Decide = arguments(function, request)?;
            json!(testing::decide(v.vector(&a.p_values)?, v.number(&a.alpha)?))
        }
        "decision" => {
            let a: DecisionArguments = arguments(function, request)?;
            let (decision, n) = testing::decision(
                v.vector(&a.p_greater)?,
                v.vector(&a.p_less)?,
                v.vector(&a.p_equivalent)?,
                v.number(&a.alpha)?,
            );
            json!([decision.name(), n])
        }
        "needed" => {
            let a: Needed = arguments(function, request)?;
            json!(binding::needed(
                v.number(&a.estimate)?,
                a.n.i64()?,
                v.number(&a.p0)?,
                v.number(&a.margin)?,
                v.number(&a.alpha)?,
            )?)
        }
        "family_level" => {
            let a: FamilyLevel = arguments(function, request)?;
            out.number(testing::family_level(v.number(&a.level)?, a.m.i64()?))
        }
        "group_means" => {
            let a: GroupMeans = arguments(function, request)?;
            let (values, shape) = v.array(&a.values)?;
            let (statistics, rows, points) = binding::group_means(
                values,
                shape,
                a.size.i64()?,
                a.groups.i64()?,
                a.geometric.unwrap_or(false),
                a.seed.seed()?,
            )?;
            out.array(statistics, &[rows, points])
        }
        "bands" => {
            let a: Bands = arguments(function, request)?;
            let (distribution, shape) = v.array(&a.distribution)?;
            let (bands, rows, columns) = binding::bands(distribution, shape, v.vector(&a.qs)?)?;
            out.array(bands, &[rows, columns])
        }
        _ => {
            return Err(type_error(format!(
                "sbmlsim_core.stats has no function {function:?}"
            )));
        }
    };
    Ok(answer)
}

/// The statistics of a Fisher information of `k` parameters as the dictionary of
/// `fisher_statistics`.
fn fisher_json(out: &mut Out, s: fisher::FisherStatistics, k: usize) -> Value {
    json!({
        "sigma2": out.number(s.sigma2),
        "spectrum": out.vector(s.spectrum),
        "rank": s.rank,
        "condition": out.number(s.condition),
        "covariance": out.array(s.covariance, &[k, k]),
        "standard_errors": out.vector(s.standard_errors),
        "correlation": out.array(s.correlation, &[k, k]),
    })
}

/// A function of one argument of any shape, value by value, by its name.
fn unary(function: &str, v: &Values, out: &mut Out, x: &Doubles) -> Result<Value, Failure> {
    let unary = binding::Unary::from_name(function)
        .ok_or_else(|| type_error(format!("{function} is no function of one argument")))?;
    let (x, shape) = v.array(x)?;
    Ok(out.array(binding::unary(unary, x)?, shape))
}

/// A function of a value and the degrees of freedom, broadcast as scipy broadcasts them, by
/// its name.
fn binary(
    function: &str,
    v: &Values,
    out: &mut Out,
    x: &Doubles,
    df: &Doubles,
) -> Result<Value, Failure> {
    let binary = binding::Binary::from_name(function)
        .ok_or_else(|| type_error(format!("{function} is no function of two arguments")))?;
    let (x, x_shape) = v.array(x)?;
    let (df, df_shape) = v.array(df)?;
    let (values, shape) = binding::binary(binary, x, x_shape, df, df_shape)?;
    Ok(out.array(values, &shape))
}

/// [`call`] of the C ABI: writes the JSON of the answer and the little-endian bytes of its
/// doubles to the four words at `out` and gives 1, or the JSON of the error (`kind`,
/// `message`), no values and 0.
///
/// # Safety
///
/// `request` is a buffer of [`crate::allocate`] of `request_len` bytes, `values` one of
/// `values_len` bytes (or null of none) and `out` four words of the caller.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn stats_call(
    request: *const u8,
    request_len: usize,
    values: *const u8,
    values_len: usize,
    out: *mut usize,
) -> u32 {
    // SAFETY: the buffers are the caller's, as documented
    let (request, values) = unsafe { (bytes(request, request_len), bytes(values, values_len)) };
    let (status, text, values) = match call(request, values) {
        Ok(answer) => {
            let bytes: Vec<u8> = answer.values.iter().flat_map(|x| x.to_le_bytes()).collect();
            (1, answer.json, bytes)
        }
        Err(failure) => (
            0,
            json!({ "kind": failure.kind, "message": failure.message }).to_string(),
            Vec::new(),
        ),
    };
    // SAFETY: the caller gives four words of its own memory
    unsafe {
        give(text.into_bytes(), out);
        give(values, out.add(2));
    }
    status
}
