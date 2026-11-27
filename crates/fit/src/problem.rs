//! The problem of a fit: its definition, its residuals and its jacobian on the batch.

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use sbmlsim_abi::{EngineError, Output, SimulationRequest, SimulationResult};
use sbmlsim_batch::{BatchError, PointFailure, Workers};
use sbmlsim_host::{LoadedModel, NativeInstance, PreparedSimulation};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::reduction::{Column, Reduction};

/// The residual of a point whose simulation failed, the one sbmlsim scores a failed
/// evaluation with (`LIKELIHOOD_FAILURE_RESIDUAL`): a large finite number, so that the step
/// of an optimizer which reaches it is rejected and the fit goes on, where an infinite
/// residual would end the run.
pub const FAILED_RESIDUAL: f64 = 1e3;

/// The relative step of the finite differences of a jacobian, the `diff_step` of sbmlsim.
///
/// A simulation is exact only to the tolerance of its integrator, so a step below the square
/// root of that tolerance measures the noise of the integrator instead of the slope of the
/// residuals and an optimizer stops next to its start.
pub const DIFF_STEP: f64 = 1e-3;

/// What the residual of the scale of the noise of a measurement carries on top of its term,
/// the `LIKELIHOOD_OFFSET` of sbmlsim.
///
/// A least squares optimizer minimizes a sum of squares, so the negative log-likelihood of a
/// measurement is written as two residuals whose squares add up to twice it: the one of its
/// data and the one of the scale of its noise, `sqrt(2 term + offset)`. The term of a scale
/// is `ln(sigma) + c` and is negative for a small standard deviation, which has no root, so
/// every such row carries this offset. The cost of a fit is therefore the negative
/// log-likelihood plus `0.5 * offset` per measurement, a constant of the data which the host
/// subtracts: the engine never does, since the offset of a measurement of a log kind depends
/// on its own value.
pub const LIKELIHOOD_OFFSET: f64 = 100.0;

/// Half the logarithm of `2 pi`, the normalization of the density of a normal distribution,
/// the double python computes for `0.5 * np.log(2.0 * np.pi)`.
pub const LN_2PI_HALF: f64 = 0.918_938_533_204_672_7;

/// The logarithm of 2, the normalization of the density of a Laplace distribution, the
/// double python computes for `np.log(2.0)`.
pub const LN_2: f64 = std::f64::consts::LN_2;

/// What is wrong with a fit, or with the engine under it.
#[derive(Debug, Error)]
pub enum FitError {
    /// The definition has no parameter, no condition or no curve.
    #[error("the fit has no {0}")]
    Empty(&'static str),
    /// A parameter of the fit cannot be fitted.
    #[error("the parameter '{id}' {message}")]
    Parameter {
        /// The id of the parameter.
        id: String,
        /// What is wrong with it.
        message: String,
    },
    /// A condition of the fit cannot be simulated.
    #[error("the condition {at} {message}")]
    Condition {
        /// The index of the condition in the definition.
        at: usize,
        /// What is wrong with it.
        message: String,
    },
    /// A curve of the fit cannot be read off the simulation of its condition.
    #[error("the curve {at} {message}")]
    Curve {
        /// The index of the curve in the definition.
        at: usize,
        /// What is wrong with it.
        message: String,
    },
    /// A prior of a parameter of the fit cannot be evaluated.
    #[error("the prior {at} {message}")]
    Prior {
        /// The index of the prior in the definition.
        at: usize,
        /// What is wrong with it.
        message: String,
    },
    /// The correlated priors of the fit cannot be evaluated.
    #[error("the correlation of the priors {0}")]
    Correlation(String),
    /// The pool of the batch was refused, e.g. for its number of threads.
    #[error("the pool of the batch: {0}")]
    Pool(#[from] BatchError),
    /// The engine refused a request or failed in itself; a failed integration is no error of
    /// the fit, it is [`FAILED_RESIDUAL`] for the points of its curve.
    #[error("the engine: {0}")]
    Engine(#[from] EngineError),
    /// A point of the batch failed outside the engine, e.g. its thread panicked.
    #[error("a point of the batch: {0}")]
    Point(String),
    /// The optimizer refused the problem.
    #[error("the optimizer of the fit {0}")]
    Optimizer(String),
    /// The request of a fit of the contract `fit/1` does not resolve.
    #[error("the request of the fit {0}")]
    Request(String),
}

/// Where the jacobian of the residuals of a fit comes from.
///
/// The forward sensitivities of a condition are one simulation per condition and step of an
/// optimizer instead of one per parameter and condition, and the derivative of a residual
/// instead of a difference of two of them (1e-8 of itself against [`DIFF_STEP`]). What a run
/// of sensitivities does not carry (the events of a model, a discontinuity of its math, an
/// algebraic rule, a delay) is the finite differences instead.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JacobianChoice {
    /// What is fastest for the model, which is the differences today: the difference
    /// quotient of `df/dp` inside a run of sensitivities costs about as much as the
    /// integrations the run saves (`bench/OPTIMIZATION.md`), so the sensitivities are worth
    /// their accuracy and not their time until a model carries an analytic `df/dp`.
    #[default]
    Auto,
    /// The sensitivities, and a condition which does not carry them is refused.
    ///
    /// A derived value of a condition ([`Derived`]) is carried as well: its target is a
    /// varying target of the condition, whose sensitivity enters the column of every parameter
    /// of its terms times the factor of the term, the chain rule, which is exact for the linear
    /// form a derived value is. A target whose sensitivity the engine refuses refuses the
    /// condition, as the target of a parameter does.
    Sensitivities,
    /// The finite differences of the residuals, whatever the conditions carry.
    DifferenceQuotient,
}

/// The scale an optimizer searches a parameter on.
///
/// The model always sees the value of the parameter ([`Scale::to_model`]); a parameter whose
/// bounds span decades is searched as its logarithm, so that a relative step of the optimizer
/// is a relative step of the parameter, which is what a fit of a PBPK model needs.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Scale {
    /// The value itself.
    #[default]
    Lin,
    /// The natural logarithm of the value, which has to be positive.
    Log,
    /// The decadic logarithm of the value, which has to be positive.
    Log10,
}

impl Scale {
    /// The value of the parameter of a point of the search space.
    #[must_use]
    pub fn to_model(self, value: f64) -> f64 {
        match self {
            Scale::Lin => value,
            Scale::Log => value.exp(),
            Scale::Log10 => 10f64.powf(value),
        }
    }

    /// The point of the search space of the value of a parameter.
    #[must_use]
    pub fn to_search(self, value: f64) -> f64 {
        match self {
            Scale::Lin => value,
            Scale::Log => value.ln(),
            Scale::Log10 => value.log10(),
        }
    }

    /// The derivative of the value of the parameter in the point `value` of the search space,
    /// which the jacobian of the sensitivities of a run multiplies by: the chain rule of the
    /// scale, since a sensitivity is the derivative in the value the model sees.
    #[must_use]
    pub fn derivative(self, value: f64) -> f64 {
        match self {
            Scale::Lin => 1.0,
            Scale::Log => value.exp(),
            Scale::Log10 => 10f64.powf(value) * std::f64::consts::LN_10,
        }
    }

    /// Whether the scale needs a positive value.
    #[must_use]
    fn positive(self) -> bool {
        !matches!(self, Scale::Lin)
    }
}

/// The distribution of the noise of the measurements of a curve.
///
/// A curve which names one is a curve of a likelihood: the residual of a measurement is the
/// one of its data and one more of the scale of its noise, so that half the sum of their
/// squares is the negative log-likelihood of the measurement up to [`LIKELIHOOD_OFFSET`].
/// The log kinds are the ones of a measurement which is a logarithm (the log-normal noise of
/// data of a geometric standard deviation), whose density carries the measurement itself.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum NoiseKind {
    /// A normal noise of the standard deviation of the measurement.
    Normal,
    /// A normal noise of the logarithm of the measurement, whose column and values are
    /// logarithms.
    LogNormal,
    /// A Laplace noise of the scale of the measurement, which is its standard deviation over
    /// `sqrt(2)`, and whose residual is the root of twice its absolute deviation.
    Laplace,
    /// A Laplace noise of the logarithm of the measurement, whose column and values are
    /// logarithms.
    LogLaplace,
}

impl NoiseKind {
    /// Whether the measurements and the column of the kind are logarithms, whose density
    /// carries the measurement itself.
    fn logarithmic(self) -> bool {
        matches!(self, NoiseKind::LogNormal | NoiseKind::LogLaplace)
    }

    /// Whether the kind is a Laplace distribution, whose negative log-likelihood carries the
    /// absolute deviation of a measurement and not half its square.
    fn laplace(self) -> bool {
        matches!(self, NoiseKind::Laplace | NoiseKind::LogLaplace)
    }

    /// The normalization of the density of the kind, the constant of the term of the scale of
    /// its noise.
    fn constant(self) -> f64 {
        if self.laplace() { LN_2 } else { LN_2PI_HALF }
    }
}

/// The noise model of a curve, which makes its residuals the ones of a likelihood.
#[derive(Clone, Debug)]
pub struct Likelihood {
    /// The distribution of the noise of every measurement of the curve.
    pub distribution: NoiseKind,
    /// The column of the simulation of the condition which is the noise of a measurement,
    /// read in the row of that measurement, its standard deviation for a normal kind and the
    /// scale of its distribution for a Laplace one, as [`Curve::noise`] is; the `noise` of
    /// the curve when it is absent, and the two together are refused.
    pub noise_column: Option<String>,
}

/// The distribution of the prior of a parameter of a fit, the priors of PEtab the engine
/// evaluates.
///
/// Every kind takes the two parameters of PEtab, whose meaning is the one of its own kind:
///
/// ```text
/// normal(mu, s)       the mean and the standard deviation of the value
/// log-normal(mu, s)   the mean and the standard deviation of its logarithm
/// laplace(mu, b)      the location and the scale of the value
/// log-laplace(mu, b)  the location and the scale of its logarithm
/// uniform(a, b)       the bounds of the value, which increase
/// log-uniform(a, b)   the bounds of the value as well, both positive, not of its logarithm
/// ```
///
/// A kind whose name begins with `log` is the distribution of the logarithm of the value,
/// whose density carries the value itself - but the parameters of `log-uniform` are the
/// bounds of the value, as PEtab writes them, and not of its logarithm, which is the one kind
/// whose parameters are no numbers of the logarithm. A kind says nothing about the [`Scale`]
/// an optimizer searches the parameter on, which is the one of the parameter whatever its
/// prior is.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PriorKind {
    /// A normal distribution of the mean and the standard deviation of the value.
    Normal,
    /// A normal distribution of the mean and the standard deviation of its logarithm.
    LogNormal,
    /// A uniform distribution of the value between two bounds of it, whose density is zero
    /// outside of them.
    Uniform,
    /// A uniform distribution of its logarithm between two bounds of the value, not of its
    /// logarithm: the density `1 / (x (ln(b) - ln(a)))` between `a` and `b`, which is zero
    /// outside of them as well, and both bounds are positive.
    LogUniform,
    /// A Laplace distribution of the location and the scale of the value.
    Laplace,
    /// A Laplace distribution of the location and the scale of its logarithm.
    LogLaplace,
}

impl PriorKind {
    /// The name of the kind of the contract, which a refusal of a prior names it by.
    fn name(self) -> &'static str {
        match self {
            PriorKind::Normal => "normal",
            PriorKind::LogNormal => "log-normal",
            PriorKind::Uniform => "uniform",
            PriorKind::LogUniform => "log-uniform",
            PriorKind::Laplace => "laplace",
            PriorKind::LogLaplace => "log-laplace",
        }
    }
}

/// The number of parameters of a prior, the two of every kind of [`PriorKind`].
const PRIOR_PARAMETERS: usize = 2;

/// The log density of a prior at a value, the one of the kind without the truncation at the
/// bounds of its parameter, which [`Prior::normalizer`] carries.
///
/// The terms are the ones of `petab.v2`:
///
/// ```text
/// normal(mu, s)       -0.5 z^2 - ln(s) - ln(2 pi)/2            z = (x - mu)/s
/// log-normal(mu, s)   -0.5 z^2 - ln(s) - ln(2 pi)/2 - ln(x)    z = (ln(x) - mu)/s
/// laplace(mu, b)      -|x - mu|/b - ln(2 b)
/// log-laplace(mu, b)  -|ln(x) - mu|/b - ln(2 b) - ln(x)
/// uniform(a, b)       -ln(b - a) between a and b, -inf outside
/// log-uniform(a, b)   -ln(ln(b) - ln(a)) - ln(x) between a and b, -inf outside
/// ```
///
/// A value which is not positive has no logarithm, so the density of a log kind there is
/// `NaN` or an infinity and never a number: the row of such a value is `NaN`, see
/// [`Prior::row_of`].
fn log_density(kind: PriorKind, parameters: &[f64], x: f64) -> f64 {
    let (first, second) = (parameters[0], parameters[1]);
    match kind {
        PriorKind::Normal => {
            let z = (x - first) / second;
            -0.5 * z * z - second.ln() - LN_2PI_HALF
        }
        PriorKind::LogNormal => {
            let z = (x.ln() - first) / second;
            -0.5 * z * z - second.ln() - LN_2PI_HALF - x.ln()
        }
        PriorKind::Laplace => -(x - first).abs() / second - (2.0 * second).ln(),
        PriorKind::LogLaplace => -(x.ln() - first).abs() / second - (2.0 * second).ln() - x.ln(),
        PriorKind::Uniform => {
            if (first..=second).contains(&x) {
                -(second - first).ln()
            } else {
                f64::NEG_INFINITY
            }
        }
        PriorKind::LogUniform => {
            if (first..=second).contains(&x) {
                -(second.ln() - first.ln()).ln() - x.ln()
            } else {
                f64::NEG_INFINITY
            }
        }
    }
}

/// The derivative of [`log_density`] in the value, which the row of a prior of the jacobian
/// of a fit is the chain rule of:
///
/// ```text
/// normal(mu, s)       -z/s                                     z = (x - mu)/s
/// log-normal(mu, s)   -z/(s x) - 1/x                           z = (ln(x) - mu)/s
/// laplace(mu, b)      -sign(x - mu)/b
/// log-laplace(mu, b)  -sign(ln(x) - mu)/(b x) - 1/x
/// uniform(a, b)       0
/// log-uniform(a, b)   -1/x
/// ```
///
/// The density of a Laplace kind has no derivative at its location, where it is largest and
/// the row of the prior smallest: the engine takes the derivative as zero there, as it takes
/// the one of the residual of the data of a Laplace likelihood at its own minimum.
fn log_density_derivative(kind: PriorKind, parameters: &[f64], x: f64) -> f64 {
    let (first, second) = (parameters[0], parameters[1]);
    match kind {
        PriorKind::Normal => {
            let z = (x - first) / second;
            -z / second
        }
        PriorKind::LogNormal => {
            let z = (x.ln() - first) / second;
            -z / (second * x) - 1.0 / x
        }
        PriorKind::Laplace => deviation_slope(x - first, second),
        PriorKind::LogLaplace => deviation_slope(x.ln() - first, second) / x - 1.0 / x,
        PriorKind::Uniform => 0.0,
        PriorKind::LogUniform => -1.0 / x,
    }
}

/// The derivative of `-|deviation| / scale` in the deviation, zero at the kink of it.
fn deviation_slope(deviation: f64, scale: f64) -> f64 {
    if deviation == 0.0 {
        0.0
    } else {
        -deviation.signum() / scale
    }
}

/// The prior of one parameter of a fit, which is one row of the residuals of its objective.
///
/// The row is `sqrt(c - 2 log p)` of the log density `log p` of the prior truncated at
/// [`Prior::lower`] and [`Prior::upper`] and of the offset `c` the host sends, so that half
/// its square is the negative log density of the prior up to `c / 2`: the density of a narrow
/// distribution is larger than one, whose negative logarithm has no root, which is what the
/// offset of a row keeps positive, as [`LIKELIHOOD_OFFSET`] does for the scale of a noise.
/// The cost of a fit therefore carries `c / 2` per prior, a constant of the problem which the
/// host subtracts; the engine never does.
///
/// The row of a value outside the bounds, and of one whose density is zero, is `NaN`. That is
/// no claim about the cost of such a point, as an infinite noise of a measurement is: the
/// prior is not defined there, and a point of the search space an optimizer may take is
/// inside the bounds of every parameter of the fit, which the bounds of its prior cover: a
/// prior which does not cover them is refused when the fit is read.
#[derive(Clone, Debug)]
pub struct Prior {
    /// The position of the parameter in [`Definition::parameters`].
    pub parameter: usize,
    /// The distribution of the prior.
    pub kind: PriorKind,
    /// The parameters of that distribution, the two of its kind and in its units: the bounds
    /// of a `log-uniform` kind are the ones of the value and not of its logarithm, see
    /// [`PriorKind`].
    pub parameters: Vec<f64>,
    /// The offset `c` of the row, which keeps `c - 2 log p` positive between the bounds.
    pub offset: f64,
    /// The logarithm of the mass of the prior between the bounds, which the engine subtracts
    /// from the density of the kind: the host computes it, since the normalization of a
    /// truncation is a distribution function and no term of a density.
    pub normalizer: f64,
    /// The smallest value the prior is defined at, the lower bound of the parameter: a prior
    /// which does not cover the bounds of its parameter is refused.
    pub lower: f64,
    /// The largest value it is defined at, the upper bound of the parameter.
    pub upper: f64,
}

/// The residual row of a prior and its derivative, see [`Prior::row_of`].
#[derive(Clone, Copy, Debug)]
struct PriorRow {
    /// The row itself.
    residual: f64,
    /// Its derivative in the value of the parameter, zero where it has none.
    slope: f64,
}

impl Prior {
    /// The row of the prior at the value of its parameter and the derivative of that row in
    /// the value, which the residuals and the jacobian of the fit share, so that a row and
    /// its derivative cannot drift apart.
    fn row_of(&self, value: f64) -> PriorRow {
        // a value outside the bounds of the prior, `NaN` among them, is none of it, and so
        // is one whose density is zero, e.g. outside the support of a uniform prior
        let none = PriorRow {
            residual: f64::NAN,
            slope: 0.0,
        };
        if value.is_nan() || value < self.lower || value > self.upper {
            return none;
        }
        let density = log_density(self.kind, &self.parameters, value);
        if !density.is_finite() {
            return none;
        }
        let residual = (self.offset - 2.0 * (density - self.normalizer))
            .max(0.0)
            .sqrt();
        PriorRow {
            residual,
            // a density the offset of the row does not reach is a row of zero, the smallest
            // one, which has no derivative, as the row of the scale of a tiny noise of a
            // likelihood has none
            slope: if residual > 0.0 {
                -log_density_derivative(self.kind, &self.parameters, value) / residual
            } else {
                0.0
            },
        }
    }
}

/// The correlated priors of a fit, one multivariate normal of the scores of their values.
///
/// The priors of several parameters are correlated when the host gives the normal scores
/// `z_i = (v_i - location_i) / scale_i` of their values a correlation, `v` being the value of
/// the parameter or its logarithm where [`Correlation::log`] says so, i.e. a normal and a
/// log-normal prior. Their rows are the whitened scores `w = L^-1 z` of the Cholesky factor
/// `L` of that correlation, whose inverse the host sends, so that half the sum of their
/// squares is the quadratic form `z' R^-1 z / 2` of the multivariate normal, and one row more
/// per logarithmic prior for the density of its logarithm in the value, `1 / x`.
///
/// The engine normalizes no truncation of a correlated prior: the multivariate normal is
/// normalized without the bounds, and its constant, `sum(ln(scale)) + ln(det(L)) +
/// k ln(2 pi) / 2`, is a term of the offset of the cost which the host keeps and subtracts,
/// as it keeps the [`Correlation::offsets`] of the logarithmic rows. The bounds are the ones
/// of the parameters, which an optimizer searches between: they are checked when the fit is
/// read and are no term of the rows, unlike the ones of an independent [`Prior`], whose row
/// outside them is `NaN`, since a whitened score is an ordinary number of the multivariate
/// normal wherever the scores are finite.
#[derive(Clone, Debug)]
pub struct Correlation {
    /// The positions of the parameters in [`Definition::parameters`], `k` of them.
    pub parameters: Vec<usize>,
    /// Whether the score of a prior is the one of the logarithm of the value, i.e. whether
    /// it is log-normal; one flag per parameter.
    pub log: Vec<bool>,
    /// The mean of the value of every prior, or of its logarithm.
    pub location: Vec<f64>,
    /// Its standard deviation, which is positive.
    pub scale: Vec<f64>,
    /// The inverse of the lower Cholesky factor of the correlation, `k * k` row major.
    pub inverse: Vec<f64>,
    /// The offset `c` of the row `sqrt(c + 2 ln(x))` of every logarithmic prior, in the
    /// order of the logarithmic priors.
    pub offsets: Vec<f64>,
    /// The smallest value of every parameter, its lower bound.
    pub lower: Vec<f64>,
    /// Its largest value, its upper bound.
    pub upper: Vec<f64>,
}

/// The residual rows of the correlated priors of a fit and their derivatives, see
/// [`Correlation::rows_of`].
#[derive(Clone, Debug)]
struct CorrelatedRows {
    /// The rows: the `k` whitened scores and then one per logarithmic prior.
    rows: Vec<f64>,
    /// The derivative of every row in the value of every correlated parameter,
    /// `rows.len() * k` row major.
    slopes: Vec<f64>,
}

impl Correlation {
    /// The number of rows of the correlated priors: the whitened scores and one per
    /// logarithmic prior.
    fn n_rows(&self) -> usize {
        self.parameters.len() + self.offsets.len()
    }

    /// The rows of the correlated priors at the values of the parameters of the fit and the
    /// derivative of every one of them, which the residuals and the jacobian share.
    ///
    /// A whitened score is the product of a row of the inverse of the factor with the
    /// scores, accumulated from zero with one addition per column in ascending order, which
    /// is the loop the host evaluates and no dot product of a library: the two sides of the
    /// contract are compared on these numbers, so the order of the additions is part of it.
    /// The whitened scores are all `NaN` when a score is not finite, e.g. the logarithm of a
    /// value which is not positive, as they are one product of all of them, while the row of
    /// a logarithm is `NaN` only where its own term is.
    fn rows_of(&self, values: &[f64]) -> CorrelatedRows {
        let k = self.parameters.len();
        let mut scores = Vec::with_capacity(k);
        for at in 0..k {
            let value = values[self.parameters[at]];
            let value = if self.log[at] { value.ln() } else { value };
            scores.push((value - self.location[at]) / self.scale[at]);
        }
        let mut rows = vec![f64::NAN; self.n_rows()];
        let mut slopes = vec![0.0; self.n_rows() * k];
        if scores.iter().all(|score| score.is_finite()) {
            for row in 0..k {
                let mut total = 0.0;
                for column in 0..k {
                    total += self.inverse[row * k + column] * scores[column];
                }
                rows[row] = total;
                for column in 0..k {
                    let mut slope = self.inverse[row * k + column] / self.scale[column];
                    if self.log[column] {
                        slope /= values[self.parameters[column]];
                    }
                    slopes[row * k + column] = slope;
                }
            }
        }
        // the density of the logarithm of a log-normal prior in the value, `1 / x`, whose
        // row the offset of the host keeps positive as the offset of a prior does
        let mut which = 0;
        for column in 0..k {
            if !self.log[column] {
                continue;
            }
            let (at, value) = (k + which, values[self.parameters[column]]);
            let term = self.offsets[which] + 2.0 * value.ln();
            if term.is_finite() {
                rows[at] = term.max(0.0).sqrt();
                if rows[at] > 0.0 {
                    slopes[at * k + column] = 1.0 / (rows[at] * value);
                }
            }
            which += 1;
        }
        CorrelatedRows { rows, slopes }
    }
}

/// The priors of the parameters of a fit, whose rows come after every row of its curves.
#[derive(Clone, Debug, Default)]
struct Priors {
    /// The priors of the parameters which are not correlated, in the order of the fit.
    independent: Vec<Prior>,
    /// The correlated priors of the fit, if it has any.
    correlation: Option<Correlation>,
}

impl Priors {
    /// The number of rows of the priors in the residuals of the fit: one per prior of a
    /// parameter of its own and the rows of the correlated ones.
    fn n_rows(&self) -> usize {
        self.independent.len() + self.correlation.as_ref().map_or(0, Correlation::n_rows)
    }

    /// Writes the rows of the priors into the residuals of the fit, which begin at `into`.
    ///
    /// `values` is [`Problem::values`], the values of the parameters of the fit on the linear
    /// scale, which carries the ones a profile holds at a value as well: the prior of such a
    /// parameter is read where it is held.
    fn write(&self, into: &mut [f64], values: &[f64]) {
        for (at, prior) in self.independent.iter().enumerate() {
            into[at] = prior.row_of(values[prior.parameter]).residual;
        }
        if let Some(correlation) = &self.correlation {
            let rows = correlation.rows_of(values);
            into[self.independent.len()..].copy_from_slice(&rows.rows);
        }
    }

    /// Writes the rows of the priors into the jacobian of the fit, whose rows of the priors
    /// begin at `into` and which has `k` columns.
    ///
    /// The row of a prior follows the value of its parameter alone and a whitened score the
    /// values of the correlated parameters, each by the chain rule of its derivative and of
    /// the [`Scale`] of the parameter, which `factors` carries per parameter of the fit as
    /// [`Problem::jacobian_of_sensitivities`] computes it. The row of a parameter a profile
    /// holds at a value is zero: it is no column of the jacobian.
    fn write_block(
        &self,
        into: &mut [f64],
        values: &[f64],
        factors: &[Option<(usize, f64)>],
        k: usize,
    ) {
        for (at, prior) in self.independent.iter().enumerate() {
            let Some((column, derivative)) = factors[prior.parameter] else {
                continue;
            };
            into[at * k + column] = prior.row_of(values[prior.parameter]).slope * derivative;
        }
        let Some(correlation) = &self.correlation else {
            return;
        };
        let rows = correlation.rows_of(values);
        let correlated = correlation.parameters.len();
        for row in 0..correlation.n_rows() {
            for (at, parameter) in correlation.parameters.iter().enumerate() {
                let Some((column, derivative)) = factors[*parameter] else {
                    continue;
                };
                let slope = rows.slopes[row * correlated + at];
                into[(self.independent.len() + row) * k + column] = slope * derivative;
            }
        }
    }
}

/// A parameter of a fit: what it sets in the model, where it starts and where it may go.
#[derive(Clone, Debug)]
pub struct Parameter {
    /// The id of the entity the parameter sets, a selection of the model: the varying target
    /// of the prepared request of every condition.
    pub id: String,
    /// The value the fit starts from, on the linear scale.
    pub start: f64,
    /// The smallest value, on the linear scale.
    pub lower: f64,
    /// The largest value, on the linear scale.
    pub upper: f64,
    /// The scale an optimizer searches the parameter on.
    pub scale: Scale,
}

/// A curve of data of a fit: the measurements of one observable of one condition.
#[derive(Clone, Debug)]
pub struct Curve {
    /// The index of the condition in [`Definition::conditions`] whose simulation the curve
    /// is read from.
    pub condition: usize,
    /// The column of the simulation the curve is read from, a selection of its request.
    pub column: String,
    /// The times of the measurements, which have to be output times of the condition.
    pub times: Vec<f64>,
    /// The measurements, one per time.
    pub values: Vec<f64>,
    /// The standard deviation of every measurement, the residuals are divided by it; one
    /// for every point when it is absent. It is the scale of the noise of a curve whose
    /// [`Likelihood::distribution`] is a Laplace kind, whose standard deviation is `sqrt(2)`
    /// times it. A positive infinity is a measurement which carries no information: its
    /// residuals are `+-0.0` and its rows of the jacobian are zeros, the one row of a curve
    /// without a likelihood and both rows of a curve with one.
    pub noise: Option<Vec<f64>>,
    /// The weight of the curve, which multiplies its residuals.
    pub weight: f64,
    /// Whether a measurement reads the state after a change rather than the one before it,
    /// one flag per time; every measurement reads the state before a change when it is
    /// absent. See [`crate::CurveRequest::after_change`].
    pub after_change: Option<Vec<bool>>,
    /// The noise model of the curve, which makes its residuals the ones of a likelihood:
    /// two per measurement, the one of its data and the one of the scale of its noise. The
    /// weighted least squares of [`Curve::noise`] when it is absent.
    pub likelihood: Option<Likelihood>,
    /// The reduction of the column the measurements are compared with, a value of the whole
    /// column over the time of the simulation, see [`Reduction`]; the column at the row of
    /// every measurement when it is absent.
    ///
    /// The `times` of a curve of a reduction are the times of an interpolation
    /// ([`Reduction::At`]) and none for every other kind, whose measurements are all
    /// compared with the one reduced value. Its rows are not resolved, so its condition may
    /// output the steps of the integrator; it reads no state after a change
    /// ([`Curve::after_change`]) and no noise of a column ([`Likelihood::noise_column`]),
    /// which are values at a row.
    pub reduction: Option<Reduction>,
    /// The points of a dimension of a scan the curve interpolates over, whose x every
    /// measurement has, see [`Dimension`]; `None` for a curve of one condition.
    ///
    /// The [`Curve::condition`] of a curve over a dimension is the first condition of its
    /// dimension.
    pub dimension: Option<Dimension>,
}

/// The points of a dimension of a scan, every one of them a condition, which a curve
/// interpolates over.
///
/// A scan is a condition per point, and the data of a study is not always measured at a
/// point: a measurement between two points is compared with the line between what the curve
/// reads in the conditions of the two, at the x of the measurement, which is the linear
/// interpolation of sbmlsim (`numpy.interp`) operation for operation. A measurement at a
/// point is compared with what the curve reads in its condition alone. What a curve reads in
/// a condition is what a curve of that condition alone would read: the column at the row of
/// a measurement, or its [`Curve::reduction`].
///
/// The row of the jacobian of a measurement is the same interpolation of the derivatives of
/// what the curve reads in the two conditions.
#[derive(Clone, Debug)]
pub struct Dimension {
    /// The conditions of the points, by their index in [`Definition::conditions`], in the
    /// order of their values; at least two, every one of them once.
    pub conditions: Vec<usize>,
    /// The value of the dimension at every point, finite and strictly increasing.
    pub values: Vec<f64>,
    /// The value of the dimension of every measurement, between the first and the last of
    /// [`Dimension::values`].
    pub x: Vec<f64>,
    /// The reduction of the curve in the condition of every point, one per point, for a curve
    /// whose reduction is not the same at every point, e.g. the analysis of a parameter of
    /// the non-compartmental analysis whose doses are the ones of the point; empty for a
    /// curve which reads its [`Curve::reduction`], or its column at its rows, at every point.
    pub reductions: Vec<Reduction>,
}

impl Curve {
    /// Whether the curve reads its column at the rows of its measurements, a curve which
    /// reduces it neither in its condition nor in the conditions of the points of its
    /// dimension.
    fn rowed(&self) -> bool {
        self.reduction.is_none()
            && self
                .dimension
                .as_ref()
                .is_none_or(|dimension| dimension.reductions.is_empty())
    }

    /// Whether the curve reads the condition at this index: its condition, or a condition of
    /// a point of its dimension.
    fn reads(&self, condition: usize) -> bool {
        match &self.dimension {
            None => self.condition == condition,
            Some(dimension) => dimension.conditions.contains(&condition),
        }
    }
}

/// A fit as a host hands it over: the parameters, the conditions, the data and the priors.
#[derive(Clone, Debug)]
pub struct Definition {
    /// The parameters of the fit, in the order of the vector of an optimizer.
    pub parameters: Vec<Parameter>,
    /// Every condition of the fit. Its output has to be the given times or a uniform grid, so
    /// that the row of a measurement does not depend on the parameters.
    pub conditions: Vec<Condition>,
    /// The curves of data of the fit.
    pub curves: Vec<Curve>,
    /// Where the jacobian of a step of the optimizer comes from.
    pub jacobian: JacobianChoice,
    /// The priors of the parameters of the fit, one row of its residuals each, see
    /// [`Prior`]; a parameter which is correlated carries none of its own, and a fit of the
    /// least squares of its curves has none at all.
    pub priors: Vec<Prior>,
    /// The correlated priors of the fit, which are one multivariate normal of the scores of
    /// their values, see [`Correlation`].
    pub prior_correlation: Option<Correlation>,
}

/// One condition of a fit: its simulation and the values which follow the parameters.
#[derive(Clone, Debug)]
pub struct Condition {
    /// The request of the simulation, whose values are numbers.
    pub request: SimulationRequest,
    /// The id of the model the condition simulates, one of the models of the problem; `None`
    /// for a condition of a problem of one model, which simulates that one.
    pub model: Option<String>,
    /// The values which follow the parameters of the fit, set on their targets before every
    /// simulation, see [`Derived`]; empty for a condition of numbers alone.
    pub derived: Vec<Derived>,
    /// The parameters of the fit whose targets the condition holds at a value of its own, see
    /// [`Pinned`]; empty for a condition every parameter reaches.
    pub pinned: Vec<Pinned>,
}

/// A parameter of a fit whose target a condition holds at a value of its own.
///
/// The target of a parameter is set to the value the optimizer is at in every condition,
/// over the values of the request of the condition. A condition whose own value wins, e.g.
/// the point of a scan whose dimension sets the target of a parameter of the fit, pins it:
/// the target takes the value of the pin there, so the parameter has no effect on that
/// condition and its column of the jacobian of the curves of the condition is zero. A
/// [`Derived`] value of the condition reads the value of the parameter and not the pin.
#[derive(Clone, Copy, Debug)]
pub struct Pinned {
    /// The position of the parameter in the parameters of the fit.
    pub parameter: usize,
    /// The value its target holds in the condition, on the linear scale.
    pub value: f64,
}

/// A value of a condition which follows the parameters of a fit, `constant + Σ factor * p`.
///
/// A value of a request of `simulation/1` is a number, so a value which an optimizer moves,
/// e.g. the initial amount of a species which a PEtab condition assigns an estimated
/// parameter, cannot be one: a number would hold it where the fit started. The fit evaluates
/// the form at the values the optimizer is at and sets it on its target as a
/// pre-initialization of the condition, which is what a varying target of a prepared request
/// is ([`sbmlsim_core_engine::Engine::prepare`]).
///
/// The value is linear in the parameters by its shape, a constant and a factor per term, so
/// its derivative in a parameter is the factor of the term: a fit of
/// [`JacobianChoice::Sensitivities`] takes the sensitivity of its target and the chain rule
/// over its terms, and a fit of the differences moves it with the parameters it reads. A value
/// which is no linear form of the parameters, e.g. a product of two of them, is no derived
/// value, and a host keeps such a condition out of a fit of the engine.
#[derive(Clone, Debug)]
pub struct Derived {
    /// The entity of the model the value is set on, a settable selection.
    pub target: String,
    /// The constant term of the linear form.
    pub constant: f64,
    /// The terms over the parameters of the fit, by their position.
    pub terms: Vec<Term>,
}

impl From<SimulationRequest> for Condition {
    /// A condition of a simulation of numbers alone, which has no derived value.
    fn from(request: SimulationRequest) -> Self {
        Self {
            request,
            model: None,
            derived: Vec::new(),
            pinned: Vec::new(),
        }
    }
}

impl Derived {
    /// The value of the form at the values of the parameters, on the linear scale.
    fn value(&self, parameters: &[f64]) -> f64 {
        self.terms.iter().fold(self.constant, |sum, term| {
            sum + term.factor * parameters[term.parameter]
        })
    }
}

/// One term of a linear form of a [`Derived`] value.
#[derive(Clone, Copy, Debug)]
pub struct Term {
    /// The position of the parameter in the parameters of the fit.
    pub parameter: usize,
    /// What the value of that parameter is multiplied by.
    pub factor: f64,
}

/// A curve resolved against the simulation of its condition.
#[derive(Debug)]
struct Resolved {
    /// Where the residuals of the curve start in the vector of the residuals.
    offset: usize,
    /// The column of the observable in the table of the condition.
    column: usize,
    /// The row of every measurement in that table.
    rows: Vec<usize>,
    /// The measurements.
    values: Vec<f64>,
    /// The standard deviation of every measurement, which its residual is divided by; the
    /// ones of a curve whose standard deviation is a column of its condition are never read.
    noise: Vec<f64>,
    /// The weight of the curve, which every residual of it is multiplied by.
    weight: f64,
    /// The noise model of the curve, resolved against the table of its condition; `None` for
    /// the weighted least squares of a curve without one.
    likelihood: Option<ResolvedLikelihood>,
    /// The reduction of the column the measurements are compared with, `None` for a curve
    /// which reads the column at its [`Resolved::rows`], which a curve of a reduction has none
    /// of.
    reduction: Option<Reduction>,
    /// The times of the measurements of a reduction which interpolates, in the time of the
    /// reduction ([`Reduction::At`]); empty for every other curve.
    grid: Vec<f64>,
}

/// The noise model of a curve, with the column of its standard deviation resolved.
#[derive(Clone, Copy, Debug)]
struct ResolvedLikelihood {
    /// The distribution of the noise of every measurement of the curve.
    distribution: NoiseKind,
    /// The column of the standard deviation of a measurement in the table of the condition,
    /// `None` for a curve whose standard deviations are the numbers of its request.
    noise_column: Option<usize>,
}

/// The two residual rows of one measurement of a curve of a likelihood and what their
/// derivatives are built from, see [`Resolved::rows_of`].
#[derive(Clone, Copy, Debug)]
struct Rows {
    /// The standard deviation of the measurement, which is positive and may be infinite.
    sigma: f64,
    /// The deviation of the measurement over it, `(column - value) / sigma`.
    r: f64,
    /// The residual of the data, without the weight of the curve.
    data: f64,
    /// The root of `2 |r|`, which the residual of the data of a Laplace kind is with the
    /// sign of `r`, and which the derivative of that residual divides by.
    deviation_root: f64,
    /// The root of `2 term + offset`, the residual of the scale of the noise, without the
    /// weight of the curve as well; its derivative divides by it.
    scale_root: f64,
}

/// Whether a number is a standard deviation of a measurement at runtime: positive, where an
/// infinity is the one of a measurement which carries no information and zero, a negative
/// value and `NaN` are none of it.
///
/// The standard deviations a request gives are checked when the curve is resolved; a column
/// of the simulation is whatever the model computes, so a fit reads it every time.
fn usable_sigma(sigma: f64) -> bool {
    sigma > 0.0
}

impl Resolved {
    /// The number of rows of the curve in the residuals of the fit: one per measurement, and
    /// one more per measurement for the scale of the noise of a likelihood.
    fn n_rows(&self) -> usize {
        self.values.len() * if self.likelihood.is_some() { 2 } else { 1 }
    }

    /// The rows of the column of the curve in a table, with their times in the time of a
    /// reduction.
    fn column_of<'a>(&self, table: &'a SimulationResult, columns: usize, shift: f64) -> Column<'a> {
        Column {
            times: (0..table.n_rows)
                .map(|row| table.values[row * columns] + shift)
                .collect(),
            values: &table.values,
            columns,
            column: self.column,
        }
    }

    /// What every measurement of the curve is compared with: the column at its row, or the
    /// reduction of the column.
    ///
    /// # Errors
    ///
    /// If the analysis of a [`Reduction::Pk`] fails, which the start of the fit has shown it
    /// does not for the table of the condition.
    fn predictions(
        &self,
        table: &SimulationResult,
        columns: usize,
    ) -> Result<Vec<f64>, EngineError> {
        match &self.reduction {
            None => Ok(self
                .rows
                .iter()
                .map(|row| table.values[row * columns + self.column])
                .collect()),
            Some(reduction) => reduction
                .values(
                    &self.column_of(table, columns, reduction.time_shift()),
                    &self.grid,
                    self.values.len(),
                )
                .map_err(EngineError::Internal),
        }
    }

    /// The derivatives of the reduction of the curve in every varying target, `targets` per
    /// measurement, out of the sensitivities of its column (`derivatives` of a row and a
    /// column); `None` for a curve which reads its column at its rows, whose derivatives are
    /// the sensitivities of those rows themselves.
    fn reduced_derivatives<'a>(
        &self,
        table: &SimulationResult,
        columns: usize,
        targets: usize,
        derivatives: &impl Fn(usize, usize) -> &'a [f64],
    ) -> Result<Option<Vec<f64>>, EngineError> {
        let Some(reduction) = &self.reduction else {
            return Ok(None);
        };
        reduction
            .derivatives(
                &self.column_of(table, columns, reduction.time_shift()),
                &self.grid,
                self.values.len(),
                targets,
                |row| derivatives(row, self.column),
            )
            .map(Some)
            .map_err(EngineError::Internal)
    }

    /// The two residual rows of the measurement `at` of a curve of the likelihood, without
    /// the weight of the curve, `None` for a measurement whose standard deviation is none
    /// ([`FAILED_RESIDUAL`] for both of them).
    ///
    /// The row of the data is the deviation of the measurement over its standard deviation,
    /// and for a Laplace kind the root of twice its absolute value with its sign, so that
    /// half its square is the deviation the negative log-likelihood of that kind carries.
    /// The row of the scale is `sqrt(2 term + offset)` of the term `ln(sigma) + c`, which a
    /// log kind adds the measurement to, since its density carries it; the offset of the row
    /// keeps the term of a small standard deviation above zero, see [`LIKELIHOOD_OFFSET`].
    ///
    /// Both rows of a measurement of an infinite standard deviation are zero. An infinity is
    /// no claim that a measurement has infinite noise, whose density would carry the
    /// logarithm of it; it is what [`Curve::noise`] says a measurement which carries no
    /// information is, and such a measurement contributes nothing to the cost of a
    /// likelihood as it contributes nothing to the one of a weighted least squares: its two
    /// rows hold their places in the residual vector and say nothing in either of them. Its
    /// deviation is `+-0.0` by the arithmetic itself and the term of its scale is the one
    /// case of its own.
    fn rows_of(
        &self,
        table: &SimulationResult,
        columns: usize,
        likelihood: ResolvedLikelihood,
        at: usize,
        prediction: f64,
    ) -> Option<Rows> {
        // a curve of a reduction has no row and no noise column, see `resolve`
        let sigma = match likelihood.noise_column {
            Some(column) => table.values[self.rows[at] * columns + column],
            None => self.noise[at],
        };
        measurement_rows(likelihood.distribution, sigma, self.values[at], prediction)
    }
}

/// The two residual rows of a measurement of a curve of a likelihood of the noise `kind`
/// and the standard deviation `sigma`, see [`Resolved::rows_of`]; `None` for a standard
/// deviation which is none.
fn measurement_rows(kind: NoiseKind, sigma: f64, value: f64, prediction: f64) -> Option<Rows> {
    if !usable_sigma(sigma) {
        return None;
    }
    // the difference is divided by the standard deviation and not multiplied by its
    // reciprocal, which rounds once more, as the residual of a least squares is
    let r = (prediction - value) / sigma;
    let deviation_root = (2.0 * r.abs()).sqrt();
    let data = if kind.laplace() {
        r.signum() * deviation_root
    } else {
        r
    };
    let term = sigma.ln() + if kind.logarithmic() { value } else { 0.0 } + kind.constant();
    let offset = LIKELIHOOD_OFFSET
        + if kind.logarithmic() {
            (-2.0 * value).max(0.0)
        } else {
            0.0
        };
    Some(Rows {
        sigma,
        r,
        data,
        deviation_root,
        // the scale of a measurement which carries no information says nothing, where
        // the term of an infinite standard deviation would be an infinity
        scale_root: if sigma.is_finite() {
            (2.0 * term + offset).max(0.0).sqrt()
        } else {
            0.0
        },
    })
}

/// One condition of the fit: what to simulate and which residuals its table carries.
#[derive(Debug)]
struct Point {
    /// The request of the condition, with the parameters of the fit and then the targets of its
    /// derived values as its varying targets.
    prepared: PreparedSimulation,
    /// The model of the condition, its index in the models of the pool of the problem.
    model: usize,
    /// The same condition with the forward sensitivities of all of those targets, which give
    /// the block of the jacobian of the condition in one simulation ([`Point::links`]); `None`
    /// for a fit of the differences.
    sensitive: Option<PreparedSimulation>,
    /// The number of columns of its table, for the index of a value.
    columns: usize,
    /// The curves which are read off its table.
    curves: Vec<Resolved>,
    /// The number of residuals of those curves.
    n_residuals: usize,
    /// The values of the condition which follow the parameters, in the order they were
    /// appended to its varying targets, see [`Derived`]; empty for a condition of numbers.
    derived: Vec<Derived>,
    /// The parameters whose targets the condition holds at a value of its own, see
    /// [`Pinned`].
    pinned: Vec<Pinned>,
    /// What the curves over a dimension read off its table, one entry per curve and point of
    /// its dimension which is this condition, see [`Interpolated`]: the predictions of such a
    /// curve follow the residuals of the condition in its output.
    nodes: Vec<Resolved>,
    /// Where the predictions of every node begin after the residuals of the condition, in
    /// measurements.
    node_starts: Vec<usize>,
}

/// The values of the varying targets of a condition at the values of the parameters: the
/// parameters themselves, the pins of the condition in their place, and then the linear forms
/// of its derived values, which read the parameters and not the pins.
///
/// A condition without derived values and pins hands the values of the parameters through,
/// so the common fit allocates nothing per simulation.
fn varying_values<'a>(
    derived: &[Derived],
    pinned: &[Pinned],
    parameters: &'a [f64],
) -> Cow<'a, [f64]> {
    if derived.is_empty() && pinned.is_empty() {
        return Cow::Borrowed(parameters);
    }
    let mut values = Vec::with_capacity(parameters.len() + derived.len());
    values.extend_from_slice(parameters);
    for pin in pinned {
        values[pin.parameter] = pin.value;
    }
    values.extend(derived.iter().map(|value| value.value(parameters)));
    Cow::Owned(values)
}

/// How the sensitivity of a varying target of a condition enters a column of the jacobian of
/// the search space, see [`Point::links`].
///
/// The target of a parameter has one link, to the column of its parameter, and the target of
/// a derived value one per term of a parameter which is searched, so a column is the sum of
/// the sensitivities of every target which follows its parameter: the chain rule.
#[derive(Clone, Copy, Debug)]
struct Link {
    /// The varying target, its index in the sensitivities of the condition.
    target: usize,
    /// The column of the search space.
    column: usize,
    /// The derivative of the value of the target in the point of that column: the derivative
    /// of the scale of the parameter, times the factor of the term of a derived value.
    factor: f64,
}

impl Point {
    /// The values of the varying targets of the condition at the values of the parameters,
    /// in the order [`Problem::new`] prepared them in, see [`varying_values`].
    fn values<'a>(&self, parameters: &'a [f64]) -> Cow<'a, [f64]> {
        varying_values(&self.derived, &self.pinned, parameters)
    }

    /// The links of the jacobian of the condition, the chain rule from the sensitivities of its
    /// varying targets to the columns of the search space, see [`Link`].
    ///
    /// `factors` is, per parameter of the fit, the column of the search space it takes its
    /// value from and the derivative of that value in it ([`Scale::derivative`]), `None` for a
    /// parameter which is held at a value. The target of a parameter is the parameter itself,
    /// unless the condition pins it, whose value does not follow the parameter here; the
    /// target of a derived value `constant + Σ factor * p` follows every parameter of its
    /// terms with the factor of the term, which reads the parameter and not a pin.
    fn links(&self, factors: &[Option<(usize, f64)>]) -> Vec<Link> {
        let mut links = Vec::with_capacity(factors.len() + self.derived.len());
        for (target, factor) in factors.iter().enumerate() {
            if let Some((column, factor)) = factor
                && !self.pinned.iter().any(|pin| pin.parameter == target)
            {
                links.push(Link {
                    target,
                    column: *column,
                    factor: *factor,
                });
            }
        }
        for (at, value) in self.derived.iter().enumerate() {
            for term in &value.terms {
                if let Some((column, factor)) = factors[term.parameter] {
                    links.push(Link {
                        target: factors.len() + at,
                        column,
                        factor: term.factor * factor,
                    });
                }
            }
        }
        links
    }

    /// The length of the output of the condition: its residuals and the predictions of its
    /// nodes.
    fn n_outputs(&self) -> usize {
        self.n_residuals
            + self
                .nodes
                .iter()
                .map(|node| node.values.len())
                .sum::<usize>()
    }

    /// The predictions of a node of the condition in its output, see [`Point::residuals`].
    fn node_output<'a>(&self, output: &'a [f64], node: usize) -> &'a [f64] {
        &output[self.n_residuals + self.node_starts[node]..][..self.nodes[node].values.len()]
    }

    /// The prediction and its derivatives of every measurement of a node of the condition in
    /// its block of the jacobian, `k + 1` values per measurement, see [`Point::block`].
    fn node_block<'a>(&self, block: &'a [f64], node: usize, k: usize) -> &'a [f64] {
        &block[self.n_residuals * k + self.node_starts[node] * (k + 1)..]
            [..self.nodes[node].values.len() * (k + 1)]
    }

    /// The output of the condition: the residuals of its curves, in the order of the curves,
    /// and then what every node reads, its predictions, which the curves over a dimension
    /// interpolate ([`Interpolated`]).
    ///
    /// The rows of a curve of a likelihood are the residuals of its data and then the ones of
    /// the scale of their noise, which is the layout of its block of the jacobian as well.
    fn residuals(&self, table: &SimulationResult) -> Result<Vec<f64>, EngineError> {
        let mut out = Vec::with_capacity(self.n_outputs());
        for curve in &self.curves {
            if curve.likelihood.is_none() && curve.reduction.is_none() {
                for ((row, value), noise) in curve
                    .rows
                    .iter()
                    .zip(curve.values.iter())
                    .zip(curve.noise.iter())
                {
                    // the difference is divided by the standard deviation and not
                    // multiplied by its reciprocal, which rounds once more: the
                    // residual is the one of the caller bitwise, e.g. `0.9 / 2.5`
                    // is `0.36` where `0.9 * (1.0 / 2.5)` is `0.36000000000000004`
                    let residual =
                        (table.values[row * self.columns + curve.column] - value) / noise;
                    out.push(residual * curve.weight);
                }
                continue;
            }
            let predictions = curve.predictions(table, self.columns)?;
            let Some(likelihood) = curve.likelihood else {
                for ((prediction, value), noise) in predictions
                    .iter()
                    .zip(curve.values.iter())
                    .zip(curve.noise.iter())
                {
                    out.push((prediction - value) / noise * curve.weight);
                }
                continue;
            };
            // the rows of a measurement are made in both loops, which costs a division and
            // a root and holds no buffer of the rows of the scales while the data is pushed
            for (at, prediction) in predictions.iter().enumerate() {
                let rows = curve.rows_of(table, self.columns, likelihood, at, *prediction);
                out.push(rows.map_or(FAILED_RESIDUAL, |rows| rows.data * curve.weight));
            }
            for (at, prediction) in predictions.iter().enumerate() {
                let rows = curve.rows_of(table, self.columns, likelihood, at, *prediction);
                out.push(rows.map_or(FAILED_RESIDUAL, |rows| rows.scale_root * curve.weight));
            }
        }
        for node in &self.nodes {
            out.extend(node.predictions(table, self.columns)?);
        }
        Ok(out)
    }

    /// The block of the jacobian of the condition out of the sensitivities of its table: the
    /// derivative of every residual of its curves in every parameter of the search space,
    /// `n_residuals * k` row major in the order of its curves.
    ///
    /// `links` carries the sensitivity of every varying target of the condition into the
    /// columns of the search space it follows ([`Point::links`]): the derivative of a residual
    /// in a column is the sum of its sensitivities times the factor of every link to it.
    ///
    /// The two rows of a measurement of a likelihood are the chain rule of its residuals: the
    /// one of its data follows the column of the curve and, where its standard deviation is a
    /// column of the condition, that column as well, the one of the scale of its noise only
    /// the latter. The rows of a measurement whose standard deviation is none are zero, as
    /// the derivative of its constant [`FAILED_RESIDUAL`] is, and so are the rows of one
    /// whose standard deviation is infinite, whose residuals are zero.
    fn block(
        &self,
        table: &SimulationResult,
        links: &[Link],
        k: usize,
    ) -> Result<Vec<f64>, EngineError> {
        let Some(sensitivities) = &table.sensitivities else {
            return Err(EngineError::Internal(
                "the simulation of a condition of a fit answered no sensitivities".to_string(),
            ));
        };
        let targets = sensitivities.parameters.len();
        let mut out =
            vec![0.0; self.n_residuals * k + (self.n_outputs() - self.n_residuals) * (k + 1)];
        let derivatives = |row: usize, column: usize| {
            &sensitivities.values[(row * self.columns + column) * targets..][..targets]
        };
        let mut at = 0;
        for curve in &self.curves {
            // the derivative of what every measurement is compared with in every target: the
            // sensitivities of the column at its row, or their reduction
            let reduced = curve.reduced_derivatives(table, self.columns, targets, &derivatives)?;
            let dprediction = |point: usize| -> &[f64] {
                match &reduced {
                    Some(values) => &values[point * targets..][..targets],
                    None => derivatives(curve.rows[point], curve.column),
                }
            };
            let n = curve.values.len();
            let Some(likelihood) = curve.likelihood else {
                for (point, noise) in curve.noise.iter().enumerate() {
                    let scale = curve.weight / noise;
                    let values = dprediction(point);
                    for link in links {
                        out[(at + point) * k + link.column] +=
                            values[link.target] * scale * link.factor;
                    }
                }
                at += n;
                continue;
            };
            let predictions = match &curve.reduction {
                None => Vec::new(),
                Some(_) => curve.predictions(table, self.columns)?,
            };
            for point in 0..n {
                let prediction = match &curve.reduction {
                    None => table.values[curve.rows[point] * self.columns + curve.column],
                    Some(_) => predictions[point],
                };
                let Some(rows) = curve.rows_of(table, self.columns, likelihood, point, prediction)
                else {
                    continue;
                };
                let dcolumn = dprediction(point);
                let dnoise = likelihood
                    .noise_column
                    .map(|column| derivatives(curve.rows[point], column));
                for link in links {
                    let dsigma = dnoise.map_or(0.0, |values| values[link.target]);
                    let dr = (dcolumn[link.target] - rows.r * dsigma) / rows.sigma;
                    let ddata = if likelihood.distribution.laplace() {
                        // the residual of the data of a Laplace kind is the root it carries,
                        // which has no derivative where the deviation is zero, the minimum
                        // of that residual
                        if rows.r == 0.0 {
                            0.0
                        } else {
                            dr / rows.deviation_root
                        }
                    } else {
                        dr
                    };
                    let dscale = if rows.scale_root > 0.0 {
                        (dsigma / rows.sigma) / rows.scale_root
                    } else {
                        0.0
                    };
                    let weight = curve.weight * link.factor;
                    out[(at + point) * k + link.column] += ddata * weight;
                    out[(at + n + point) * k + link.column] += dscale * weight;
                }
            }
            at += 2 * n;
        }
        // what every node reads and its derivatives in the parameters of the search space,
        // which a curve over a dimension interpolates, see `Interpolated::write_block`
        let mut at = self.n_residuals * k;
        for node in &self.nodes {
            let predictions = node.predictions(table, self.columns)?;
            let reduced = node.reduced_derivatives(table, self.columns, targets, &derivatives)?;
            for (point, prediction) in predictions.iter().enumerate() {
                let values = match &reduced {
                    Some(values) => &values[point * targets..][..targets],
                    None => derivatives(node.rows[point], node.column),
                };
                out[at] = *prediction;
                for link in links {
                    out[at + 1 + link.column] += values[link.target] * link.factor;
                }
                at += k + 1;
            }
        }
        Ok(out)
    }

    /// Writes the block of the jacobian of the condition into the one of the fit.
    fn write_block(&self, into: &mut [f64], block: &[f64], k: usize) {
        let mut at = 0;
        for curve in &self.curves {
            for row in 0..curve.n_rows() {
                let (from, to) = ((at + row) * k, (curve.offset + row) * k);
                into[to..to + k].copy_from_slice(&block[from..from + k]);
            }
            at += curve.n_rows();
        }
    }

    /// Writes [`FAILED_RESIDUAL`] for every residual of the condition into the vector of the
    /// fit, the residuals of a condition whose integration failed.
    fn write_failed(&self, into: &mut [f64]) {
        for curve in &self.curves {
            into[curve.offset..curve.offset + curve.n_rows()].fill(FAILED_RESIDUAL);
        }
    }

    /// Writes the residuals of the condition into the vector of the fit.
    fn write(&self, into: &mut [f64], residuals: &[f64]) {
        let mut at = 0;
        for curve in &self.curves {
            let n = curve.n_rows();
            into[curve.offset..curve.offset + n].copy_from_slice(&residuals[at..at + n]);
            at += n;
        }
    }
}

/// A curve over the points of a dimension, resolved: every measurement is compared with the
/// interpolation of what the curve reads in the conditions of the points around its x, see
/// [`Dimension`].
///
/// What the curve reads in the condition of a point is a node of that condition
/// ([`Point::nodes`]), which is computed on the thread of the condition as its residuals are,
/// so a table never leaves its thread; the residuals of the curve are made from the nodes
/// once every condition of the evaluation is done.
#[derive(Debug)]
struct Interpolated {
    /// Where the residuals of the curve start in the vector of the residuals.
    offset: usize,
    /// The measurements.
    values: Vec<f64>,
    /// The standard deviation of every measurement, which its residual is divided by.
    noise: Vec<f64>,
    /// The weight of the curve, which every residual of it is multiplied by.
    weight: f64,
    /// The distribution of the noise of a curve of a likelihood, whose standard deviations
    /// are the numbers of its request; `None` for the weighted least squares.
    likelihood: Option<NoiseKind>,
    /// The value of the dimension at every point.
    at: Vec<f64>,
    /// The x of every measurement and the points around it.
    brackets: Vec<Bracket>,
    /// The node of every point: the index of its condition and of the node there.
    nodes: Vec<(usize, usize)>,
}

/// The points a measurement of a curve over a dimension is interpolated between.
#[derive(Clone, Copy, Debug)]
struct Bracket {
    /// The x of the measurement.
    x: f64,
    /// The last point whose value is at most `x`.
    lower: usize,
    /// The point after it, `None` for a measurement at the value of `lower` or at the last
    /// point, which is compared with what the curve reads at `lower` alone.
    upper: Option<usize>,
}

impl Bracket {
    /// The points around `x` in strictly increasing values which cover it, as `numpy.interp`
    /// finds them: the last point at or below `x`, and the one after it unless `x` is its
    /// value or it is the last.
    fn of(x: f64, at: &[f64]) -> Bracket {
        let lower = at.partition_point(|value| *value <= x).saturating_sub(1);
        let upper = (lower + 1 < at.len() && at[lower] != x).then_some(lower + 1);
        Bracket { x, lower, upper }
    }
}

/// The value at `x` of the line through `(x0, y0)` and `(x1, y1)`, `y0` without the second
/// point: the arithmetic of `numpy.interp` operation for operation, its slope first and then
/// the step from the lower point, and from the upper one where that is `NaN` (an infinite
/// value of a point), so a curve over a dimension is the interpolation of sbmlsim bitwise.
fn interpolate(x: f64, (x0, y0): (f64, f64), upper: Option<(f64, f64)>) -> f64 {
    let Some((x1, y1)) = upper else {
        return y0;
    };
    let slope = (y1 - y0) / (x1 - x0);
    let value = slope * (x - x0) + y0;
    if !value.is_nan() {
        return value;
    }
    let value = slope * (x - x1) + y1;
    if value.is_nan() && y0 == y1 {
        y0
    } else {
        value
    }
}

impl Interpolated {
    /// The number of rows of the curve in the residuals of the fit: one per measurement, and
    /// one more per measurement for the scale of the noise of a likelihood.
    fn n_rows(&self) -> usize {
        self.values.len() * if self.likelihood.is_some() { 2 } else { 1 }
    }

    /// What every measurement is compared with, the interpolation of the predictions of the
    /// nodes in the outputs of the conditions of an evaluation; `None` when the integration of
    /// a condition of a point failed.
    fn predictions(&self, points: &[Point], outputs: &[Option<Vec<f64>>]) -> Option<Vec<f64>> {
        let reads = self
            .nodes
            .iter()
            .map(|(point, node)| {
                outputs[*point]
                    .as_deref()
                    .map(|output| points[*point].node_output(output, *node))
            })
            .collect::<Option<Vec<&[f64]>>>()?;
        Some(
            self.brackets
                .iter()
                .enumerate()
                .map(|(at, bracket)| {
                    interpolate(
                        bracket.x,
                        (self.at[bracket.lower], reads[bracket.lower][at]),
                        bracket
                            .upper
                            .map(|upper| (self.at[upper], reads[upper][at])),
                    )
                })
                .collect(),
        )
    }

    /// Writes the residuals of the curve into the vector of the fit, [`FAILED_RESIDUAL`] for
    /// every one of them when a condition of a point failed.
    fn write(&self, into: &mut [f64], predictions: Option<&[f64]>) {
        let n = self.values.len();
        let rows = &mut into[self.offset..self.offset + self.n_rows()];
        let Some(predictions) = predictions else {
            rows.fill(FAILED_RESIDUAL);
            return;
        };
        for (at, ((prediction, value), noise)) in predictions
            .iter()
            .zip(self.values.iter())
            .zip(self.noise.iter())
            .enumerate()
        {
            let Some(kind) = self.likelihood else {
                rows[at] = (prediction - value) / noise * self.weight;
                continue;
            };
            match measurement_rows(kind, *noise, *value, *prediction) {
                Some(measured) => {
                    rows[at] = measured.data * self.weight;
                    rows[n + at] = measured.scale_root * self.weight;
                }
                None => {
                    rows[at] = FAILED_RESIDUAL;
                    rows[n + at] = FAILED_RESIDUAL;
                }
            }
        }
    }

    /// Writes the rows of the curve into the jacobian of the fit: the interpolation of the
    /// derivatives of the nodes, through the chain rule of its residuals. A row of the scale
    /// of a noise is zero, since the standard deviation of a measurement is a number here,
    /// and so is every row of the curve when a condition of a point failed.
    fn write_block(
        &self,
        into: &mut [f64],
        points: &[Point],
        blocks: &[Option<Vec<f64>>],
        k: usize,
    ) {
        let Some(reads) = self
            .nodes
            .iter()
            .map(|(point, node)| {
                blocks[*point]
                    .as_deref()
                    .map(|block| points[*point].node_block(block, *node, k))
            })
            .collect::<Option<Vec<&[f64]>>>()
        else {
            return;
        };
        for (at, bracket) in self.brackets.iter().enumerate() {
            let lower = &reads[bracket.lower][at * (k + 1)..][..k + 1];
            let upper = bracket
                .upper
                .map(|upper| (self.at[upper], &reads[upper][at * (k + 1)..][..k + 1]));
            let x0 = self.at[bracket.lower];
            let value = |column: usize| {
                interpolate(
                    bracket.x,
                    (x0, lower[column]),
                    upper.map(|(x1, row)| (x1, row[column])),
                )
            };
            let noise = self.noise[at];
            // the derivative of the residual of the data in the interpolated prediction
            let scale = match self.likelihood {
                None => self.weight / noise,
                Some(kind) => {
                    let Some(rows) = measurement_rows(kind, noise, self.values[at], value(0))
                    else {
                        continue;
                    };
                    if !kind.laplace() {
                        self.weight / rows.sigma
                    } else if rows.r == 0.0 {
                        // the root of a Laplace kind has no derivative at its minimum
                        0.0
                    } else {
                        self.weight / rows.sigma / rows.deviation_root
                    }
                }
            };
            let row = &mut into[(self.offset + at) * k..][..k];
            for (parameter, derivative) in row.iter_mut().enumerate() {
                *derivative = value(1 + parameter) * scale;
            }
        }
    }
}

/// One point of a batch: a condition and the values of the parameters to simulate it at.
struct Shot<'a> {
    point: &'a Point,
    values: &'a [f64],
}

/// The problem of a fit, resolved against a loaded model and the pool of its batch.
///
/// The conditions are prepared once (`prepare` of the host), so an evaluation transfers the
/// values of the parameters and nothing else, and the column and the row of every measurement
/// are resolved once as well: an evaluation is one fan out of the conditions and the
/// arithmetic of the residuals, see [`Problem::residuals`].
pub struct Problem {
    /// The parameters an optimizer searches, which are the ones of the fit without the ones
    /// a profile holds at a value ([`Problem::fixing`]).
    free: Vec<Parameter>,
    /// One slot per varying target of the prepared requests, in their order: the parameter of
    /// the search space whose value it takes, or the value it is held at.
    slots: Vec<Slot>,
    /// The conditions of the fit and the curves they carry, shared with the problems which
    /// fix a parameter of it.
    points: Arc<Vec<Point>>,
    /// The curves over a dimension, which interpolate between the nodes of the conditions of
    /// their points, shared with those problems as well.
    interpolated: Arc<Vec<Interpolated>>,
    /// The priors of the parameters, whose rows come after every row of the curves, shared
    /// with those problems as well: the prior of a parameter a profile holds at a value is
    /// read where it is held.
    priors: Arc<Priors>,
    /// The rows of the curves and of the priors together.
    n_residuals: usize,
    /// Where the jacobian of a step comes from: [`JacobianChoice::Sensitivities`] or
    /// [`JacobianChoice::DifferenceQuotient`], resolved once for the whole problem.
    jacobian: JacobianChoice,
    /// The simulations the problem ran, shared with the problems which fix a parameter of it
    /// so that a profile counts the simulations of its fits as well.
    simulations: Arc<AtomicU64>,
    /// The pool of the batch, shared with those problems as well: the instance of a thread is
    /// made once, whatever is fitted on it.
    workers: Arc<Workers>,
}

/// Where a varying target of the prepared requests takes its value from.
#[derive(Clone, Copy, Debug)]
enum Slot {
    /// The parameter at this index of the search space, on its scale.
    Free { at: usize, scale: Scale },
    /// A value the parameter is held at, on the linear scale.
    Fixed(f64),
}

impl std::fmt::Debug for Problem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Problem")
            .field("parameters", &self.free.len())
            .field("fixed", &(self.slots.len() - self.free.len()))
            .field("conditions", &self.points.len())
            .field(
                "curves",
                &self.points.iter().map(|p| p.curves.len()).sum::<usize>(),
            )
            .field("residuals", &self.n_residuals)
            .field("threads", &self.workers.threads())
            .finish()
    }
}

impl Problem {
    /// Resolves a definition against a model and the pool of its batch.
    ///
    /// Every condition is prepared with the parameters of the fit as its varying targets and
    /// simulated once at the start, which gives the columns and the output times its curves
    /// are resolved against.
    ///
    /// # Errors
    ///
    /// If the definition is empty, if a parameter cannot be fitted (its bounds, its scale or
    /// its start), if a condition cannot be prepared or simulated at the start, or if a curve
    /// does not fit the table of its condition (its column, its times or its noise).
    pub fn new(
        definition: Definition,
        model: Arc<LoadedModel>,
        threads: usize,
    ) -> Result<Problem, FitError> {
        Problem::of_models(
            definition,
            vec![(DEFAULT_MODEL.to_string(), model)],
            threads,
        )
    }

    /// Resolves a definition against several models, each with the id its conditions name it
    /// by, and the pool of its batch.
    ///
    /// This is [`Problem::new`] of a fit of more than one model, e.g. of data which is the
    /// data of two builds of a model: the condition of every simulation names the model it
    /// simulates, a condition of a fit of one model needs no name, and a thread of the pool
    /// holds an instance of every model its points met.
    ///
    /// # Errors
    ///
    /// As [`Problem::new`], and if the models are empty, if two of them share an id, or if a
    /// condition names a model the fit has not or names none although the fit has several.
    pub fn of_models(
        definition: Definition,
        models: Vec<(String, Arc<LoadedModel>)>,
        threads: usize,
    ) -> Result<Problem, FitError> {
        let Definition {
            parameters,
            conditions,
            curves,
            jacobian,
            priors,
            prior_correlation,
        } = definition;
        check_parameters(&parameters)?;
        let priors = check_priors(priors, prior_correlation, &parameters)?;
        if conditions.is_empty() {
            return Err(FitError::Empty("condition"));
        }
        if curves.is_empty() {
            return Err(FitError::Empty("curve"));
        }
        let varying: Vec<String> = parameters.iter().map(|p| p.id.clone()).collect();
        let mut varyings = Vec::with_capacity(conditions.len());
        for (at, condition) in conditions.iter().enumerate() {
            varyings.push(check_derived(at, condition, &varying)?);
            check_pinned(at, condition, parameters.len())?;
        }
        let ats = models_of(&models, &conditions)?;
        let workers = Workers::of_models(
            models.into_iter().map(|(_, model)| model).collect(),
            threads,
        )?;
        let mut prepared = Vec::with_capacity(conditions.len());
        for (((at, condition), varying), model) in conditions
            .iter()
            .enumerate()
            .zip(varyings.iter())
            .zip(ats.iter())
        {
            // the rows of a curve of a reduction are not resolved, so a condition whose
            // curves are all reductions may output the steps of the integrator
            let rowed = curves.iter().any(|curve| curve.reads(at) && curve.rowed());
            if rowed && matches!(condition.request.output, Output::IntegratorSteps) {
                return Err(FitError::Condition {
                    at,
                    message: "outputs the steps of the integrator, whose times depend on the \
                              parameters: a fit needs the times of its data or a uniform \
                              grid, unless every curve of it is a reduction"
                        .to_string(),
                });
            }
            prepared.push(
                workers.models()[*model]
                    .prepare(&condition.request, varying)
                    .map_err(|err| FitError::Condition {
                        at,
                        message: format!("cannot be prepared: {err}"),
                    })?,
            );
        }
        // the sensitivities of a condition are the ones of all of its varying targets, the
        // targets of its derived values among them, whose columns the chain rule over their
        // terms maps onto the parameters, see `Point::links`
        let (sensitive, jacobian) = if jacobian == JacobianChoice::Sensitivities {
            let prepared = sensitive_conditions(&workers, &conditions, &ats, &varyings)?;
            (prepared, JacobianChoice::Sensitivities)
        } else {
            (
                conditions.iter().map(|_| None).collect(),
                JacobianChoice::DifferenceQuotient,
            )
        };
        // the table of every condition at the start, which the curves are resolved against
        let start: Vec<f64> = parameters
            .iter()
            .map(|p| p.scale.to_model(p.scale.to_search(p.start)))
            .collect();
        let tables = simulate(&workers, &prepared, &conditions, &ats, &start)?;
        let mut points: Vec<Point> = prepared
            .into_iter()
            .zip(sensitive)
            .zip(tables.iter())
            .zip(conditions.iter())
            .zip(ats.iter())
            .map(
                |((((prepared, sensitive), table), condition), model)| Point {
                    prepared,
                    model: *model,
                    sensitive,
                    columns: table.columns.len(),
                    curves: Vec::new(),
                    n_residuals: 0,
                    derived: condition.derived.clone(),
                    pinned: condition.pinned.clone(),
                    nodes: Vec::new(),
                    node_starts: Vec::new(),
                },
            )
            .collect();
        let mut n_residuals = 0;
        let mut interpolated = Vec::new();
        for (at, curve) in curves.iter().enumerate() {
            let fault = |message: String| FitError::Curve { at, message };
            if curve.condition >= points.len() {
                return Err(fault(format!(
                    "reads the condition {}, which the fit has not",
                    curve.condition
                )));
            }
            if let Some(dimension) = &curve.dimension {
                let mut over = resolve_dimension(at, curve, dimension, &tables, n_residuals)?;
                for (index, (node, condition)) in
                    over.nodes.iter_mut().zip(&dimension.conditions).enumerate()
                {
                    // what the curve reads in the condition of the point, which is what a
                    // curve of that condition alone would read, with the reduction of the
                    // point where the points have their own
                    let resolved = match dimension.reductions.get(index) {
                        None => resolve(at, curve, &tables[*condition], 0)?,
                        Some(reduction) => {
                            let mut own = curve.clone();
                            own.reduction = Some(reduction.clone());
                            resolve(at, &own, &tables[*condition], 0)?
                        }
                    };
                    let point = &mut points[*condition];
                    *node = (*condition, point.nodes.len());
                    point
                        .node_starts
                        .push(point.nodes.iter().map(|node| node.values.len()).sum());
                    point.nodes.push(resolved);
                }
                n_residuals += over.n_rows();
                interpolated.push(over);
                continue;
            }
            let resolved = resolve(at, curve, &tables[curve.condition], n_residuals)?;
            n_residuals += resolved.n_rows();
            let point = &mut points[curve.condition];
            point.n_residuals += resolved.n_rows();
            point.curves.push(resolved);
        }
        // the rows of the priors come after every row of a curve, so that the residuals of a
        // fit of the same data are the ones of its least squares and then the ones of its
        // priors, whatever the priors are
        n_residuals += priors.n_rows();
        Ok(Problem {
            slots: (0..parameters.len())
                .map(|at| Slot::Free {
                    at,
                    scale: parameters[at].scale,
                })
                .collect(),
            free: parameters,
            points: Arc::new(points),
            interpolated: Arc::new(interpolated),
            priors: Arc::new(priors),
            n_residuals,
            jacobian,
            simulations: Arc::new(AtomicU64::new(0)),
            workers: Arc::new(workers),
        })
    }

    /// The same fit with parameters held at a value, which is the problem of a profile.
    ///
    /// The indices are the ones of [`Problem::parameters`] of this problem, and the problem
    /// it gives has the parameters around them: an optimizer searches those and the model
    /// sees the values the others are held at. The conditions, the curves and the pool of the
    /// batch are shared, so a profile of hundreds of points prepares and instantiates
    /// nothing again.
    ///
    /// # Errors
    ///
    /// If an index is none of the parameters of the fit, if the fit would have no parameter
    /// left, or if a value is outside the bounds of its parameter.
    pub fn fixing(&self, fixed: &[(usize, f64)]) -> Result<Problem, FitError> {
        let mut held: Vec<Option<f64>> = vec![None; self.free.len()];
        for (at, value) in fixed {
            let parameter = self.free.get(*at).ok_or_else(|| FitError::Parameter {
                id: format!("{at}"),
                message: format!("is none of the {} parameters of the fit", self.free.len()),
            })?;
            if !value.is_finite() || *value < parameter.lower || *value > parameter.upper {
                return Err(FitError::Parameter {
                    id: parameter.id.clone(),
                    message: format!(
                        "is held at {value}, outside its bounds [{}, {}]",
                        parameter.lower, parameter.upper
                    ),
                });
            }
            held[*at] = Some(*value);
        }
        let free: Vec<Parameter> = self
            .free
            .iter()
            .zip(held.iter())
            .filter(|(_, held)| held.is_none())
            .map(|(parameter, _)| parameter.clone())
            .collect();
        if free.is_empty() {
            return Err(FitError::Empty("parameter which is not held at a value"));
        }
        // the index of every parameter which is still searched, in the order of the fit
        let mut at_of: Vec<usize> = Vec::with_capacity(self.free.len());
        let mut next = 0;
        for value in &held {
            at_of.push(next);
            if value.is_none() {
                next += 1;
            }
        }
        let slots = self
            .slots
            .iter()
            .map(|slot| match slot {
                Slot::Fixed(value) => Slot::Fixed(*value),
                Slot::Free { at, scale } => match held[*at] {
                    Some(value) => Slot::Fixed(value),
                    None => Slot::Free {
                        at: at_of[*at],
                        scale: *scale,
                    },
                },
            })
            .collect();
        Ok(Problem {
            free,
            slots,
            points: self.points.clone(),
            interpolated: self.interpolated.clone(),
            priors: self.priors.clone(),
            n_residuals: self.n_residuals,
            jacobian: self.jacobian,
            simulations: self.simulations.clone(),
            workers: self.workers.clone(),
        })
    }

    /// The parameters of the fit, in the order of the vector of an optimizer.
    #[must_use]
    pub fn parameters(&self) -> &[Parameter] {
        &self.free
    }

    /// The number of residuals of one evaluation: the rows of the curves of the fit and
    /// then the rows of its priors.
    #[must_use]
    pub fn n_residuals(&self) -> usize {
        self.n_residuals
    }

    /// The threads of the batch of an evaluation.
    #[must_use]
    pub fn threads(&self) -> usize {
        self.workers.threads()
    }

    /// The start of the fit, in the space an optimizer searches.
    #[must_use]
    pub fn start(&self) -> Vec<f64> {
        self.free
            .iter()
            .map(|p| p.scale.to_search(p.start))
            .collect()
    }

    /// The bounds of the parameters, in the space an optimizer searches.
    #[must_use]
    pub fn bounds(&self) -> (Vec<f64>, Vec<f64>) {
        (
            self.free
                .iter()
                .map(|p| p.scale.to_search(p.lower))
                .collect(),
            self.free
                .iter()
                .map(|p| p.scale.to_search(p.upper))
                .collect(),
        )
    }

    /// The values of the parameters an optimizer searches at a point of the search space,
    /// on the linear scale: the ones of [`Problem::parameters`], in their order.
    #[must_use]
    pub fn parameter_values(&self, x: &[f64]) -> Vec<f64> {
        self.free
            .iter()
            .zip(x.iter())
            .map(|(parameter, value)| parameter.scale.to_model(*value))
            .collect()
    }

    /// The values of the varying targets of the conditions at a point of the search space:
    /// the parameters of the fit on their scale and the ones a profile holds at a value.
    #[must_use]
    pub fn values(&self, x: &[f64]) -> Vec<f64> {
        self.slots
            .iter()
            .map(|slot| match slot {
                Slot::Free { at, scale } => scale.to_model(x[*at]),
                Slot::Fixed(value) => *value,
            })
            .collect()
    }

    /// The residuals of a point of the search space: the rows of the curves of the fit, in
    /// their order, and then the rows of its priors.
    ///
    /// The conditions are one batch of the engine: every point simulates on the instance of
    /// its thread and reduces its table to the residuals of its curves there, so the tables
    /// never leave the thread which made them. The residuals of a condition whose integration
    /// failed are [`FAILED_RESIDUAL`]. The rows of the priors need no simulation and are
    /// computed from the values of the parameters themselves, see [`Priors::write`].
    ///
    /// # Errors
    ///
    /// If the engine refuses a request or fails in itself, or if a point of the batch failed
    /// outside the engine; a failed integration is no error.
    pub fn residuals(&self, x: &[f64]) -> Result<Vec<f64>, FitError> {
        let values = self.values(x);
        let shots: Vec<Shot<'_>> = self
            .points
            .iter()
            .map(|point| Shot {
                point,
                values: &values,
            })
            .collect();
        let outputs = self.run(&shots)?;
        let mut residuals = vec![0.0; self.n_residuals];
        self.write_evaluation(&mut residuals, &outputs);
        let at = self.priors_at();
        self.priors.write(&mut residuals[at..], &values);
        Ok(residuals)
    }

    /// Writes the rows of the curves of one evaluation, the outputs of every condition in the
    /// order of the conditions (`None` for a failed integration), into the residuals of the
    /// fit: the rows of the curves of every condition and then the ones of the curves over a
    /// dimension, which interpolate between the outputs of several conditions.
    fn write_evaluation(&self, into: &mut [f64], outputs: &[Option<Vec<f64>>]) {
        for (point, output) in self.points.iter().zip(outputs) {
            match output {
                Some(output) => point.write(into, output),
                None => point.write_failed(into),
            }
        }
        for curve in self.interpolated.iter() {
            curve.write(into, curve.predictions(&self.points, outputs).as_deref());
        }
    }

    /// Where the rows of the priors begin in the residuals of the fit: after every row of
    /// its curves.
    fn priors_at(&self) -> usize {
        self.n_residuals - self.priors.n_rows()
    }

    /// The cost of a point of the search space, half the sum of the squares of its residuals.
    ///
    /// # Errors
    ///
    /// As [`Problem::residuals`].
    pub fn cost(&self, x: &[f64]) -> Result<f64, FitError> {
        Ok(0.5
            * self
                .residuals(x)?
                .iter()
                .map(|value| value * value)
                .sum::<f64>())
    }

    /// Runs an optimizer over the problem, from the start of its parameters.
    ///
    /// # Errors
    ///
    /// As [`Problem::residuals`], or if the optimizer refuses the bounds of the fit.
    pub fn optimize(&self, settings: &crate::Settings) -> Result<crate::Fit, FitError> {
        crate::optimize::run(self, settings, &self.start(), &crate::Stop::new())
    }

    /// Runs an optimizer over the problem, from a point of the search space.
    ///
    /// The start of a fit of many starts, the optimum of a fit a profile steps away from and
    /// the best member of a population are such points.
    ///
    /// # Errors
    ///
    /// As [`Problem::optimize`], or if the point is not one of the parameters of the fit.
    pub fn optimize_from(
        &self,
        settings: &crate::Settings,
        from: &[f64],
    ) -> Result<crate::Fit, FitError> {
        crate::optimize::run(self, settings, from, &crate::Stop::new())
    }

    /// Runs an optimizer over the problem, from a point of the search space, until a caller
    /// stops it.
    ///
    /// # Errors
    ///
    /// As [`Problem::optimize_from`].
    pub fn optimize_with(
        &self,
        settings: &crate::Settings,
        from: &[f64],
        stop: &crate::Stop,
    ) -> Result<crate::Fit, FitError> {
        crate::optimize::run(self, settings, from, stop)
    }

    /// Runs `work` on the threads of the batch of the problem.
    pub(crate) fn install<R: Send>(&self, work: impl FnOnce() -> R + Send) -> R {
        self.workers.install(work)
    }

    /// The step of the finite difference of the parameter `k` at a point of the search space.
    ///
    /// A relative step of [`DIFF_STEP`], as the `diff_step` of sbmlsim, which points away
    /// from the bound it would cross.
    #[must_use]
    pub fn step(&self, k: usize, x: &[f64]) -> f64 {
        let parameter = &self.free[k];
        let step = DIFF_STEP * x[k].abs().max(1.0);
        let upper = parameter.scale.to_search(parameter.upper);
        if x[k] + step > upper { -step } else { step }
    }

    /// Where the jacobian of the problem comes from, resolved: the sensitivities of its
    /// conditions or the finite differences of its residuals.
    #[must_use]
    pub fn jacobian_of(&self) -> JacobianChoice {
        self.jacobian
    }

    /// The simulations the problem ran, and the ones of the problems which hold a parameter
    /// of it ([`Problem::fixing`]): what a fit costs, which the rate of an evaluation does
    /// not say once the jacobian of a step is one simulation per condition.
    #[must_use]
    pub fn simulations(&self) -> u64 {
        self.simulations.load(Ordering::Relaxed)
    }

    /// The jacobian of the residuals at a point of the search space, row major
    /// (`n_residuals * parameters`), of [`Problem::jacobian_of`].
    ///
    /// # Errors
    ///
    /// As [`Problem::residuals`].
    pub fn jacobian(&self, x: &[f64]) -> Result<Vec<f64>, FitError> {
        match self.jacobian {
            JacobianChoice::DifferenceQuotient => self.jacobian_of_differences(x),
            _ => self.jacobian_of_sensitivities(x),
        }
    }

    /// The jacobian out of the forward sensitivities of the conditions: one simulation per
    /// condition, whatever the number of the parameters, and the derivative of a residual
    /// instead of a difference of two of them. The rows of the priors are analytic as well
    /// and need no simulation, see [`Priors::write_block`].
    ///
    /// The sensitivities of a condition are the derivatives of its columns in the values of
    /// its varying targets, which the scale of a parameter turns into the derivative in the
    /// point of the search space ([`Scale::derivative`]). The block of a condition is reduced
    /// on the thread which simulated it, as its residuals are, so the sensitivities of a
    /// table never leave it either.
    ///
    /// # Errors
    ///
    /// As [`Problem::residuals`].
    fn jacobian_of_sensitivities(&self, x: &[f64]) -> Result<Vec<f64>, FitError> {
        let k = self.free.len();
        let values = self.values(x);
        let factors: Vec<Option<(usize, f64)>> = self
            .slots
            .iter()
            .map(|slot| match slot {
                Slot::Free { at, scale } => Some((*at, scale.derivative(x[*at]))),
                Slot::Fixed(_) => None,
            })
            .collect();
        let shots: Vec<Shot<'_>> = self
            .points
            .iter()
            .map(|point| Shot {
                point,
                values: &values,
            })
            .collect();
        self.simulations
            .fetch_add(shots.len() as u64, Ordering::Relaxed);
        let outcomes = self.workers.map_at(
            &shots,
            |shot| shot.point.model,
            |instance, shot| {
                let Some(prepared) = &shot.point.sensitive else {
                    return Err(EngineError::Internal(
                        "a condition of the fit carries no sensitivities".to_string(),
                    ));
                };
                let values = shot.point.values(shot.values);
                let table = instance.simulate_prepared(prepared, &values)?;
                shot.point.block(&table, &shot.point.links(&factors), k)
            },
        );
        let mut jacobian = vec![0.0; self.n_residuals * k];
        let at = self.priors_at();
        self.priors
            .write_block(&mut jacobian[at * k..], &values, &factors, k);
        let mut blocks = Vec::with_capacity(shots.len());
        for (shot, outcome) in shots.iter().zip(outcomes) {
            blocks.push(match outcome {
                Ok(block) => {
                    shot.point.write_block(&mut jacobian, &block, k);
                    Some(block)
                }
                // a failed integration is a point of the search space the fit may not take:
                // its residuals are `FAILED_RESIDUAL` and its block of the jacobian says
                // nothing, so the optimizer rejects the step by its cost
                Err(PointFailure::Engine(EngineError::Integration { .. })) => None,
                Err(PointFailure::Engine(err)) => return Err(FitError::Engine(err)),
                Err(PointFailure::Internal(message)) => return Err(FitError::Point(message)),
            });
        }
        for curve in self.interpolated.iter() {
            curve.write_block(&mut jacobian, &self.points, &blocks, k);
        }
        Ok(jacobian)
    }

    /// The jacobian by the finite differences of the residuals: the column of a parameter is
    /// the forward difference of the residuals over [`Problem::step`]. The whole jacobian is
    /// one batch of the engine, `(parameters + 1) * conditions` points which do not depend on
    /// each other, so it fans out over the threads at once instead of once per column.
    ///
    /// The rows of the priors are rows of the residuals, so they are differenced as well and
    /// are as accurate as the rows of a curve here, which is the step of a forward difference
    /// and not the rounding of a derivative: the jacobian of a fit is the one of its
    /// differences or the one of its sensitivities and never a mixture of the two. A step
    /// which leaves the bounds of its parameter, which [`Problem::step`] takes only in a box
    /// of less than twice its step, differences a row of a prior which is `NaN` there.
    ///
    /// # Errors
    ///
    /// As [`Problem::residuals`].
    fn jacobian_of_differences(&self, x: &[f64]) -> Result<Vec<f64>, FitError> {
        let k = self.free.len();
        let steps: Vec<f64> = (0..k).map(|column| self.step(column, x)).collect();
        // the values of the base point and of every column of the difference
        let mut moved: Vec<Vec<f64>> = Vec::with_capacity(k + 1);
        moved.push(self.values(x));
        for (column, step) in steps.iter().enumerate() {
            let mut ahead = x.to_vec();
            ahead[column] += step;
            moved.push(self.values(&ahead));
        }
        let mut shots: Vec<Shot<'_>> = Vec::with_capacity((k + 1) * self.points.len());
        for values in &moved {
            for point in self.points.iter() {
                shots.push(Shot { point, values });
            }
        }
        let g = self.points.len();
        let n = self.n_residuals;
        let outputs = self.run(&shots)?;
        // the residuals of the base point and of every column of the difference, one after
        // the other: the points of a column are the conditions of one evaluation
        let mut residuals = vec![0.0; (k + 1) * n];
        for (column, outputs) in outputs.chunks(g).enumerate() {
            self.write_evaluation(&mut residuals[column * n..(column + 1) * n], outputs);
        }
        // the rows of the priors are rows of the residuals, so the difference of a column
        // differences them as well
        let priors = self.priors_at();
        for (column, values) in moved.iter().enumerate() {
            self.priors.write(
                &mut residuals[column * n + priors..(column + 1) * n],
                values,
            );
        }
        let mut jacobian = vec![0.0; self.n_residuals * k];
        for row in 0..self.n_residuals {
            let at = residuals[row];
            for (column, step) in steps.iter().enumerate() {
                let ahead = residuals[(column + 1) * self.n_residuals + row];
                jacobian[row * k + column] = (ahead - at) / step;
            }
        }
        Ok(jacobian)
    }

    /// Runs the points of a batch and reduces every one of them on the thread of the point to
    /// its output ([`Point::residuals`]).
    ///
    /// The output of a point whose integration failed is `None`, whose residuals are
    /// [`FAILED_RESIDUAL`] ([`Problem::write_evaluation`]); any other failure of the engine
    /// is an error of the fit.
    fn run(&self, shots: &[Shot<'_>]) -> Result<Vec<Option<Vec<f64>>>, FitError> {
        self.simulations
            .fetch_add(shots.len() as u64, Ordering::Relaxed);
        let outcomes = self.workers.map_at(
            shots,
            |shot| shot.point.model,
            |instance, shot| {
                let values = shot.point.values(shot.values);
                let table = instance.simulate_prepared(&shot.point.prepared, &values)?;
                shot.point.residuals(&table)
            },
        );
        let mut residuals = Vec::with_capacity(shots.len());
        for outcome in outcomes {
            residuals.push(match outcome {
                Ok(residuals) => Some(residuals),
                // a failed integration is a point of the search space the fit may not take
                Err(PointFailure::Engine(EngineError::Integration { .. })) => None,
                Err(PointFailure::Engine(err)) => return Err(FitError::Engine(err)),
                Err(PointFailure::Internal(message)) => return Err(FitError::Point(message)),
            });
        }
        Ok(residuals)
    }
}

/// The id of the one model of a fit of one model, which a condition may name it by.
pub const DEFAULT_MODEL: &str = "model";

/// The model of every condition, its index in `models`, or why a condition has none.
///
/// A condition of a fit of one model needs no model of its own, and a condition of a fit of
/// several names the one it simulates; a name is the id of a model of the fit whatever the
/// number of models, so a typo is an error and not the first model.
fn models_of(
    models: &[(String, Arc<LoadedModel>)],
    conditions: &[Condition],
) -> Result<Vec<usize>, FitError> {
    if models.is_empty() {
        return Err(FitError::Empty("model"));
    }
    let mut by_id: HashMap<&str, usize> = HashMap::with_capacity(models.len());
    for (at, (id, _)) in models.iter().enumerate() {
        if by_id.insert(id.as_str(), at).is_some() {
            return Err(FitError::Request(format!(
                "two models of the fit have the id '{id}'"
            )));
        }
    }
    let mut ats = Vec::with_capacity(conditions.len());
    for (at, condition) in conditions.iter().enumerate() {
        let model = match &condition.model {
            None if models.len() == 1 => 0,
            None => {
                return Err(FitError::Condition {
                    at,
                    message: format!(
                        "names no model, and the fit has {} of them: every condition of a \
                         fit of several models names the model it simulates",
                        models.len()
                    ),
                });
            }
            Some(id) => *by_id.get(id.as_str()).ok_or_else(|| FitError::Condition {
                at,
                message: format!("simulates the model '{id}', which the fit has not"),
            })?,
        };
        ats.push(model);
    }
    Ok(ats)
}

/// Every condition of a fit prepared with the forward sensitivities of its varying targets,
/// `varyings` per condition: the targets of the parameters and of its derived values.
///
/// A condition which carries none is refused: the jacobian of a fit is the one of the
/// sensitivities or the one of the differences, and never a mixture of the two, which would
/// be a jacobian of two accuracies. So is a condition whose derived value has a target the
/// sensitivities do not carry, which the fit would otherwise have to difference.
fn sensitive_conditions(
    workers: &Workers,
    conditions: &[Condition],
    ats: &[usize],
    varyings: &[Vec<String>],
) -> Result<Vec<Option<PreparedSimulation>>, FitError> {
    // an instance per model of the conditions, made at the first condition of that model
    let mut instances: Vec<Option<NativeInstance>> =
        workers.models().iter().map(|_| None).collect();
    let mut prepared = Vec::with_capacity(conditions.len());
    for (((at, condition), model), varying) in conditions
        .iter()
        .enumerate()
        .zip(ats.iter())
        .zip(varyings.iter())
    {
        let instance = match &mut instances[*model] {
            Some(instance) => instance,
            none => {
                let made = workers.models()[*model]
                    .instantiate_native()
                    .map_err(|error| {
                        FitError::Request(format!("cannot be instantiated: {error}"))
                    })?;
                none.insert(made)
            }
        };
        let mut request = condition.request.clone();
        request.sensitivities = varying.to_vec();
        match instance.prepare(&request, varying) {
            Ok(sensitive) => prepared.push(Some(sensitive)),
            Err(EngineError::Unsupported(message)) => {
                return Err(FitError::Condition {
                    at,
                    message: format!("carries no forward sensitivities: {message}"),
                });
            }
            Err(error) => {
                return Err(FitError::Condition {
                    at,
                    message: format!("cannot be prepared with its sensitivities: {error}"),
                });
            }
        }
    }
    Ok(prepared)
}

/// Checks the pins of a condition: a parameter of the fit, pinned once, at a finite value.
fn check_pinned(at: usize, condition: &Condition, parameters: usize) -> Result<(), FitError> {
    let mut seen = vec![false; parameters];
    for pin in &condition.pinned {
        let fault = if pin.parameter >= parameters {
            Some(format!(
                "pins the parameter {} of a fit of {parameters} parameters",
                pin.parameter
            ))
        } else if std::mem::replace(&mut seen[pin.parameter], true) {
            Some(format!("pins the parameter {} twice", pin.parameter))
        } else if !pin.value.is_finite() {
            Some(format!(
                "pins the parameter {} at {}, which is not finite",
                pin.parameter, pin.value
            ))
        } else {
            None
        };
        if let Some(message) = fault {
            return Err(FitError::Condition { at, message });
        }
    }
    Ok(())
}

/// Resolves a curve over a dimension: its points, their values and the bracket of every
/// measurement, with its nodes still to be placed in the conditions of its points.
fn resolve_dimension(
    at: usize,
    curve: &Curve,
    dimension: &Dimension,
    tables: &[SimulationResult],
    offset: usize,
) -> Result<Interpolated, FitError> {
    let fault = |message: String| FitError::Curve { at, message };
    let Dimension {
        conditions,
        values,
        x,
        reductions,
    } = dimension;
    if conditions.len() < 2 {
        return Err(fault(format!(
            "interpolates over {} points of a dimension, and a curve of one point is a curve \
             of its condition",
            conditions.len()
        )));
    }
    if values.len() != conditions.len() {
        return Err(fault(format!(
            "has {} conditions and {} values of its dimension",
            conditions.len(),
            values.len()
        )));
    }
    if let Some(condition) = conditions.iter().find(|c| **c >= tables.len()) {
        return Err(fault(format!(
            "reads the condition {condition}, which the fit has not"
        )));
    }
    if curve.condition != conditions[0] {
        return Err(fault(format!(
            "reads the condition {} and a dimension whose first condition is {}",
            curve.condition, conditions[0]
        )));
    }
    for (index, condition) in conditions.iter().enumerate() {
        if conditions[..index].contains(condition) {
            return Err(fault(format!(
                "has the condition {condition} twice in its dimension"
            )));
        }
    }
    if !values.iter().all(|value| value.is_finite())
        || values.windows(2).any(|pair| pair[0] >= pair[1])
    {
        return Err(fault(format!(
            "has the values {values:?} of its dimension, which do not increase strictly or \
             are not finite"
        )));
    }
    if !reductions.is_empty() && reductions.len() != conditions.len() {
        return Err(fault(format!(
            "has {} conditions and {} reductions of its dimension",
            conditions.len(),
            reductions.len()
        )));
    }
    if !reductions.is_empty() && curve.reduction.is_some() {
        return Err(fault(
            "carries a reduction and the reductions of the points of its dimension: a \
             point reads one of them"
                .to_string(),
        ));
    }
    if x.len() != curve.values.len() {
        return Err(fault(format!(
            "has {} values and {} x of its dimension",
            curve.values.len(),
            x.len()
        )));
    }
    let (first, last) = (values[0], values[values.len() - 1]);
    if let Some(outside) = x.iter().find(|x| !(**x >= first && **x <= last)) {
        return Err(fault(format!(
            "measures at the x {outside}, which is outside of its dimension [{first}, {last}]"
        )));
    }
    if curve
        .likelihood
        .as_ref()
        .is_some_and(|likelihood| likelihood.noise_column.is_some())
    {
        return Err(fault(
            "carries a 'noise_column' and a dimension: the noise of a column is a value of one \
             condition, which a curve over a dimension interpolates between"
                .to_string(),
        ));
    }
    // the noise of every measurement is checked with the nodes, see `resolve`
    Ok(Interpolated {
        offset,
        values: curve.values.clone(),
        noise: curve
            .noise
            .clone()
            .unwrap_or_else(|| vec![1.0; curve.values.len()]),
        weight: curve.weight,
        likelihood: curve
            .likelihood
            .as_ref()
            .map(|likelihood| likelihood.distribution),
        at: values.clone(),
        brackets: x.iter().map(|x| Bracket::of(*x, values)).collect(),
        nodes: vec![(0, 0); conditions.len()],
    })
}

/// Checks the derived values of a condition and gets its varying targets.
///
/// The varying targets of a condition are the parameters of the fit and then the targets of
/// its derived values, which must be settable entities the parameters do not already set and
/// which the condition must not name twice; a term reads a parameter of the fit.
fn check_derived(
    at: usize,
    condition: &Condition,
    varying: &[String],
) -> Result<Vec<String>, FitError> {
    let mut targets = varying.to_vec();
    for value in &condition.derived {
        let fault = if value.target.is_empty() {
            Some("has a derived value without a target".to_string())
        } else if targets.contains(&value.target) {
            Some(format!(
                "sets '{}' as a derived value and as a parameter or a derived value already",
                value.target
            ))
        } else if !value.constant.is_finite() {
            Some(format!(
                "has the derived value '{}' of the constant {}, which is not finite",
                value.target, value.constant
            ))
        } else {
            value.terms.iter().find_map(|term| {
                if term.parameter >= varying.len() {
                    Some(format!(
                        "has the derived value '{}', which reads the parameter {} of a fit of \
                         {} parameters",
                        value.target,
                        term.parameter,
                        varying.len()
                    ))
                } else if !term.factor.is_finite() {
                    Some(format!(
                        "has the derived value '{}' of the factor {}, which is not finite",
                        value.target, term.factor
                    ))
                } else {
                    None
                }
            })
        };
        if let Some(message) = fault {
            return Err(FitError::Condition { at, message });
        }
        targets.push(value.target.clone());
    }
    Ok(targets)
}

/// Simulates every prepared condition once at the values of the parameters.
fn simulate(
    workers: &Workers,
    prepared: &[PreparedSimulation],
    conditions: &[Condition],
    ats: &[usize],
    values: &[f64],
) -> Result<Vec<SimulationResult>, FitError> {
    let mut tables = Vec::with_capacity(prepared.len());
    let shots: Vec<(usize, &PreparedSimulation, Cow<'_, [f64]>)> = prepared
        .iter()
        .zip(conditions.iter())
        .zip(ats.iter())
        .map(|((prepared, condition), model)| {
            let values = varying_values(&condition.derived, &condition.pinned, values);
            (*model, prepared, values)
        })
        .collect();
    let outcomes = workers.map_at(
        &shots,
        |(model, _, _)| *model,
        |instance, (_, prepared, values)| instance.simulate_prepared(prepared, values),
    );
    for (at, outcome) in outcomes.into_iter().enumerate() {
        match outcome {
            Ok(table) => tables.push(table),
            Err(PointFailure::Engine(err)) => {
                return Err(FitError::Condition {
                    at,
                    message: format!("cannot be simulated at the start of the fit: {err}"),
                });
            }
            Err(PointFailure::Internal(message)) => return Err(FitError::Point(message)),
        }
    }
    Ok(tables)
}

/// Checks the parameters of a fit, which an optimizer searches between their bounds.
fn check_parameters(parameters: &[Parameter]) -> Result<(), FitError> {
    if parameters.is_empty() {
        return Err(FitError::Empty("parameter"));
    }
    for parameter in parameters {
        let fault = if !parameter.lower.is_finite() || !parameter.upper.is_finite() {
            Some("has a bound which is not finite".to_string())
        } else if parameter.lower >= parameter.upper {
            Some(format!(
                "has the bounds [{}, {}], which are not increasing",
                parameter.lower, parameter.upper
            ))
        } else if !parameter.start.is_finite()
            || parameter.start < parameter.lower
            || parameter.start > parameter.upper
        {
            Some(format!(
                "starts at {}, outside its bounds [{}, {}]",
                parameter.start, parameter.lower, parameter.upper
            ))
        } else if parameter.scale.positive() && parameter.lower <= 0.0 {
            Some(format!(
                "is searched on a logarithmic scale and has the lower bound {}, which is not \
                 positive",
                parameter.lower
            ))
        } else {
            None
        };
        if let Some(message) = fault {
            return Err(FitError::Parameter {
                id: parameter.id.clone(),
                message,
            });
        }
    }
    Ok(())
}

/// Checks the priors of a fit and gathers them: the prior of a parameter of its own and the
/// correlated ones, which the engine evaluates at the values of the parameters.
///
/// A parameter carries one prior: a second prior of it, and a prior of a parameter which is
/// correlated, would multiply two densities into one objective, which is no prior of the host
/// and no fit a caller means.
fn check_priors(
    priors: Vec<Prior>,
    correlation: Option<Correlation>,
    parameters: &[Parameter],
) -> Result<Priors, FitError> {
    // whether a parameter carries a prior already, the correlated ones first, so that a
    // parameter which is correlated and carries one of its own is reported at that prior
    let mut carried = vec![false; parameters.len()];
    if let Some(correlation) = &correlation {
        check_correlation(correlation, parameters, &mut carried)?;
    }
    for (at, prior) in priors.iter().enumerate() {
        let fault = |message: String| FitError::Prior { at, message };
        if prior.parameter >= parameters.len() {
            return Err(fault(format!(
                "is the prior of the parameter {} of a fit of {} parameters",
                prior.parameter,
                parameters.len()
            )));
        }
        let id = &parameters[prior.parameter].id;
        if std::mem::replace(&mut carried[prior.parameter], true) {
            return Err(fault(format!(
                "is a second prior of '{id}', which carries a prior or is correlated already"
            )));
        }
        if !prior.offset.is_finite() || !prior.normalizer.is_finite() {
            return Err(fault(format!(
                "of '{id}' has the offset {} and the normalizer {}, one of which is not \
                 finite",
                prior.offset, prior.normalizer
            )));
        }
        if !prior.lower.is_finite() || !prior.upper.is_finite() || prior.lower >= prior.upper {
            return Err(fault(format!(
                "of '{id}' is defined between {} and {}, which are not increasing and finite",
                prior.lower, prior.upper
            )));
        }
        // the row of a value the prior is not defined at is `NaN`, so a prior which does not
        // cover the box an optimizer searches in would be a cost of `NaN` at points of that
        // box and no error anywhere
        let parameter = &parameters[prior.parameter];
        if prior.lower > parameter.lower || prior.upper < parameter.upper {
            return Err(fault(format!(
                "of '{id}' is defined between {} and {}, which does not cover the bounds {} \
                 and {} of the parameter: its row would be `NaN` at a point of the search \
                 space",
                prior.lower, prior.upper, parameter.lower, parameter.upper
            )));
        }
        check_kind(prior.kind, &prior.parameters)
            .map_err(|message| fault(format!("of '{id}' {message}")))?;
    }
    Ok(Priors {
        independent: priors,
        correlation,
    })
}

/// Checks the parameters of a distribution of a prior, which are the two of its kind.
fn check_kind(kind: PriorKind, parameters: &[f64]) -> Result<(), String> {
    let name = kind.name();
    if parameters.len() != PRIOR_PARAMETERS {
        return Err(format!(
            "has {} parameters, and a {name} prior takes two of them",
            parameters.len()
        ));
    }
    if let Some(value) = parameters.iter().find(|value| !value.is_finite()) {
        return Err(format!(
            "has the parameter {value} of its {name} distribution, which is not finite"
        ));
    }
    let (first, second) = (parameters[0], parameters[1]);
    match kind {
        PriorKind::Normal | PriorKind::LogNormal | PriorKind::Laplace | PriorKind::LogLaplace => {
            if second <= 0.0 {
                return Err(format!(
                    "is a {name} prior of the scale {second}, which is not positive"
                ));
            }
        }
        PriorKind::Uniform | PriorKind::LogUniform => {
            if first >= second {
                return Err(format!(
                    "is a {name} prior between {first} and {second}, which are not increasing"
                ));
            }
            if kind == PriorKind::LogUniform && first <= 0.0 {
                return Err(format!(
                    "is a {name} prior from {first}, which is not positive and has no logarithm"
                ));
            }
        }
    }
    Ok(())
}

/// Checks the correlated priors of a fit and marks their parameters in `carried`.
///
/// The lengths are the ones of a correlation of `k` parameters, so that a row of it is read
/// where the host wrote it, and every value is a number the engine can evaluate the scores
/// and the whitened rows with. The bounds are the ones of the parameters, which the engine
/// does not normalize the multivariate normal over, see [`Correlation`]; a logarithmic prior
/// needs a positive lower bound, since the score of its parameter is a logarithm.
fn check_correlation(
    correlation: &Correlation,
    parameters: &[Parameter],
    carried: &mut [bool],
) -> Result<(), FitError> {
    let fault = |message: String| FitError::Correlation(message);
    let k = correlation.parameters.len();
    if k == 0 {
        return Err(fault("has no parameter".to_string()));
    }
    for (what, length) in [
        ("log", correlation.log.len()),
        ("location", correlation.location.len()),
        ("scale", correlation.scale.len()),
        ("lower", correlation.lower.len()),
        ("upper", correlation.upper.len()),
    ] {
        if length != k {
            return Err(fault(format!(
                "has {k} parameters and {length} of '{what}'"
            )));
        }
    }
    if correlation.inverse.len() != k * k {
        return Err(fault(format!(
            "has {k} parameters and the {} values of its inverse, which is no {k} by {k} \
             matrix",
            correlation.inverse.len()
        )));
    }
    let logarithmic = correlation.log.iter().filter(|log| **log).count();
    if correlation.offsets.len() != logarithmic {
        return Err(fault(format!(
            "has {logarithmic} logarithmic priors and {} offsets of their rows",
            correlation.offsets.len()
        )));
    }
    if let Some(value) = correlation
        .inverse
        .iter()
        .chain(correlation.offsets.iter())
        .find(|value| !value.is_finite())
    {
        return Err(fault(format!(
            "has the value {value} of its inverse or of its offsets, which is not finite"
        )));
    }
    for (at, parameter) in correlation.parameters.iter().enumerate() {
        if *parameter >= parameters.len() {
            return Err(fault(format!(
                "correlates the parameter {parameter} of a fit of {} parameters",
                parameters.len()
            )));
        }
        let id = &parameters[*parameter].id;
        if std::mem::replace(&mut carried[*parameter], true) {
            return Err(fault(format!(
                "correlates '{id}' twice, and a parameter of a correlation is one of its \
                 priors"
            )));
        }
        if !correlation.location[at].is_finite() {
            return Err(fault(format!(
                "gives '{id}' the location {}, which is not finite",
                correlation.location[at]
            )));
        }
        let scale = correlation.scale[at];
        if !scale.is_finite() || scale <= 0.0 {
            return Err(fault(format!(
                "gives '{id}' the scale {scale}, which is not positive and finite"
            )));
        }
        let (lower, upper) = (correlation.lower[at], correlation.upper[at]);
        if !lower.is_finite() || !upper.is_finite() || lower >= upper {
            return Err(fault(format!(
                "defines the prior of '{id}' between {lower} and {upper}, which are not \
                 increasing and finite"
            )));
        }
        if correlation.log[at] && lower <= 0.0 {
            return Err(fault(format!(
                "gives the logarithmic prior of '{id}' the lower bound {lower}, which is not \
                 positive and has no logarithm"
            )));
        }
    }
    Ok(())
}

/// Resolves a curve against the table of its condition: its column and the row of a point.
fn resolve(
    at: usize,
    curve: &Curve,
    table: &SimulationResult,
    offset: usize,
) -> Result<Resolved, FitError> {
    let fault = |message: String| FitError::Curve { at, message };
    match &curve.reduction {
        Some(reduction) => reduction
            .check(&curve.times, curve.values.len())
            .map_err(fault)?,
        None if curve.times.len() != curve.values.len() => {
            return Err(fault(format!(
                "has {} times and {} values",
                curve.times.len(),
                curve.values.len()
            )));
        }
        None => {}
    }
    if curve.values.is_empty() {
        return Err(fault("has no measurement".to_string()));
    }
    if !curve.weight.is_finite() || curve.weight <= 0.0 {
        return Err(fault(format!(
            "has the weight {}, which is not positive and finite",
            curve.weight
        )));
    }
    let column = table
        .columns
        .iter()
        .position(|name| *name == curve.column)
        .ok_or_else(|| {
            fault(format!(
                "reads the column '{}', which its condition does not select: {:?}",
                curve.column, table.columns
            ))
        })?;
    let noise = match &curve.noise {
        None => vec![1.0; curve.values.len()],
        Some(noise) if noise.len() == curve.values.len() => noise.clone(),
        Some(noise) => {
            return Err(fault(format!(
                "has {} values and {} standard deviations",
                curve.values.len(),
                noise.len()
            )));
        }
    };
    for value in &noise {
        // an infinite standard deviation is a measurement which carries no information
        if value.is_nan() || *value <= 0.0 {
            return Err(fault(format!(
                "has the standard deviation {value}, which is no standard deviation of a curve; pass a positive value, or an infinity to drop the point"
            )));
        }
    }
    let likelihood = match &curve.likelihood {
        None => None,
        Some(likelihood) => {
            let column = match &likelihood.noise_column {
                None if curve.noise.is_none() => {
                    return Err(fault(
                        "carries a likelihood and neither a 'noise' nor a 'noise_column': a \
                         standard deviation of one is no noise model"
                            .to_string(),
                    ));
                }
                None => None,
                Some(_) if curve.noise.is_some() => {
                    return Err(fault(
                        "carries a 'noise_column' and a 'noise' as well: the standard \
                         deviation of a measurement is the column of its condition or the \
                         number of its request, not both"
                            .to_string(),
                    ));
                }
                Some(name) => Some(
                    table
                        .columns
                        .iter()
                        .position(|column| *column == *name)
                        .ok_or_else(|| {
                            fault(format!(
                                "reads the noise_column '{name}', which its condition does \
                                 not select: {:?}",
                                table.columns
                            ))
                        })?,
                ),
            };
            if column.is_some() && curve.reduction.is_some() {
                return Err(fault(
                    "carries a 'noise_column' and a reduction: the noise of a column is a \
                     value at the row of a measurement, which a reduction has none of"
                        .to_string(),
                ));
            }
            Some(ResolvedLikelihood {
                distribution: likelihood.distribution,
                noise_column: column,
            })
        }
    };
    if !curve.values.iter().all(|value| value.is_finite()) {
        return Err(fault("has a measurement which is not finite".to_string()));
    }
    if let Some(reduction) = &curve.reduction {
        if curve.after_change.is_some() {
            return Err(fault(
                "carries the flags of the state after a change and a reduction: a reduction \
                 reads the whole column, an interpolation its flags of the state before a jump"
                    .to_string(),
            ));
        }
        let resolved = Resolved {
            offset,
            column,
            rows: Vec::new(),
            values: curve.values.clone(),
            noise,
            weight: curve.weight,
            likelihood,
            reduction: Some(reduction.clone()),
            grid: curve.times.clone(),
        };
        check_reduction(&resolved, reduction, table).map_err(fault)?;
        return Ok(resolved);
    }
    let after = match &curve.after_change {
        None => vec![false; curve.times.len()],
        Some(after) if after.len() == curve.times.len() => after.clone(),
        Some(after) => {
            return Err(fault(format!(
                "has {} times and {} flags of the state after a change",
                curve.times.len(),
                after.len()
            )));
        }
    };
    let times: Vec<f64> = (0..table.n_rows)
        .map(|row| table.values[row * table.columns.len()])
        .collect();
    let mut rows = Vec::with_capacity(curve.times.len());
    for (time, after) in curve.times.iter().zip(after.iter()) {
        rows.push(row_of(*time, &times, *after).ok_or_else(|| {
            fault(format!(
                "measures at t = {time}, which is no output time of its condition"
            ))
        })?);
    }
    Ok(Resolved {
        offset,
        column,
        rows,
        values: curve.values.clone(),
        noise,
        weight: curve.weight,
        likelihood,
        reduction: None,
        grid: Vec::new(),
    })
}

/// Checks a curve of a reduction against the table of its condition at the start of the fit:
/// the times of an interpolation are within the rows of the simulation, whose rows are the
/// same span of time at every point of the search, and the analysis of a parameter of the
/// non-compartmental analysis runs and has that parameter.
fn check_reduction(
    resolved: &Resolved,
    reduction: &Reduction,
    table: &SimulationResult,
) -> Result<(), String> {
    let column = resolved.column_of(table, table.columns.len(), reduction.time_shift());
    match reduction {
        Reduction::At { .. } => {
            let finite = column.times.iter().copied().filter(|time| time.is_finite());
            let (first, last) = finite.fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), t| {
                (a.min(t), b.max(t))
            });
            if let Some(time) = resolved
                .grid
                .iter()
                .find(|time| **time < first || **time > last)
            {
                return Err(format!(
                    "interpolates at t = {time}, which is outside of the simulation of its \
                     condition [{first}, {last}] in the time of the reduction"
                ));
            }
        }
        Reduction::Pk(pk) if !pk.derives(&column)? => {
            return Err(format!(
                "reads the parameter '{}', which the analysis of its column does not derive",
                pk.parameter
            ));
        }
        _ => {}
    }
    Ok(())
}

/// The row of an output time, the nearest one within the resolution of the time.
///
/// The output of a condition has two rows of one time at a change after its start, the state
/// before the change and the state after it, and a chain of a change and the events it fired
/// has more: `after` reads the last row of the nearest time, the state after the whole chain,
/// where the default reads the first of them, the state before the change.
fn row_of(time: f64, times: &[f64], after: bool) -> Option<usize> {
    let mut best: Option<(usize, f64)> = None;
    for (row, output) in times.iter().enumerate() {
        let distance = (output - time).abs();
        let closer = best.is_none_or(|(_, other)| {
            if after {
                distance <= other
            } else {
                distance < other
            }
        });
        if closer {
            best = Some((row, distance));
        }
    }
    let (row, distance) = best?;
    // the time of a row is the one the integrator reached, so a measurement matches the row
    // of its own time to the resolution of that time and not exactly
    let tolerance = 1e-9 * time.abs().max(1.0);
    (distance <= tolerance).then_some(row)
}
