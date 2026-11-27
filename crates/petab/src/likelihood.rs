//! The densities of PEtab v2: the noise of a measurement and the prior of a parameter.
//!
//! The simulated value `y` of a measurement `m` is the median of its distribution and the
//! noise formula gives its scale `sigma` (the specification, `Noise distributions`):
//!
//! | `noiseDistribution` | log density | residual of `chi2` |
//! | --- | --- | --- |
//! | `normal` | `-ln(2 pi sigma^2) / 2 - ((m - y) / sigma)^2 / 2` | `(m - y) / sigma` |
//! | `log-normal` | `-ln(2 pi sigma^2 m^2) / 2 - ((ln m - ln y) / sigma)^2 / 2` | `(ln m - ln y) / sigma` |
//! | `laplace` | `-ln(2 sigma) - abs(m - y) / sigma` | `(m - y) / sigma` |
//! | `log-laplace` | `-ln(2 sigma m) - abs(ln m - ln y) / sigma` | `(ln m - ln y) / sigma` |
//!
//! A prior is one of the eleven distributions of the specification (`Prior distributions`),
//! truncated at the bounds of its parameter where its support reaches beyond them: its density
//! is divided by its mass between the bounds, which [`Prior::log_density`] subtracts as the
//! logarithm of that mass.

use crate::PetabError;

/// The distribution of the noise of a measurement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NoiseDistribution {
    /// `normal`.
    Normal,
    /// `log-normal`.
    LogNormal,
    /// `laplace`.
    Laplace,
    /// `log-laplace`.
    LogLaplace,
}

/// `ln(2 pi) / 2`.
const LN_SQRT_2PI: f64 = 0.918_938_533_204_672_8;

impl NoiseDistribution {
    /// The distribution of a name of the observable table.
    #[must_use]
    pub fn from_name(name: &str) -> Option<NoiseDistribution> {
        match name.trim() {
            "normal" => Some(NoiseDistribution::Normal),
            "log-normal" => Some(NoiseDistribution::LogNormal),
            "laplace" => Some(NoiseDistribution::Laplace),
            "log-laplace" => Some(NoiseDistribution::LogLaplace),
            _ => None,
        }
    }

    /// The name of the distribution in the observable table.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            NoiseDistribution::Normal => "normal",
            NoiseDistribution::LogNormal => "log-normal",
            NoiseDistribution::Laplace => "laplace",
            NoiseDistribution::LogLaplace => "log-laplace",
        }
    }

    /// Whether the distribution is the one of the logarithm of a measurement.
    #[must_use]
    pub fn logarithmic(self) -> bool {
        matches!(
            self,
            NoiseDistribution::LogNormal | NoiseDistribution::LogLaplace
        )
    }

    /// The residual of a measurement in units of the scale of its noise, whose square `chi2`
    /// sums: `(m - y) / sigma`, of the logarithms for a logarithmic distribution.
    #[must_use]
    pub fn residual(self, measured: f64, simulated: f64, sigma: f64) -> f64 {
        if self.logarithmic() {
            (measured.ln() - simulated.ln()) / sigma
        } else {
            (measured - simulated) / sigma
        }
    }

    /// The log density of a measurement at a simulated value and the scale of its noise.
    #[must_use]
    pub fn log_density(self, measured: f64, simulated: f64, sigma: f64) -> f64 {
        let r = self.residual(measured, simulated, sigma);
        match self {
            NoiseDistribution::Normal => -LN_SQRT_2PI - sigma.ln() - 0.5 * r * r,
            NoiseDistribution::LogNormal => -LN_SQRT_2PI - sigma.ln() - measured.ln() - 0.5 * r * r,
            NoiseDistribution::Laplace => -(2.0 * sigma).ln() - r.abs(),
            NoiseDistribution::LogLaplace => -(2.0 * sigma * measured).ln() - r.abs(),
        }
    }
}

/// The distribution of the prior of a parameter, the ones of PEtab v2.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PriorKind {
    /// `cauchy(location, scale)`.
    Cauchy,
    /// `chisquare(degrees of freedom)`.
    ChiSquare,
    /// `exponential(scale)`.
    Exponential,
    /// `gamma(shape, scale)`.
    Gamma,
    /// `laplace(location, scale)`.
    Laplace,
    /// `log-laplace(location, scale)` of the logarithm.
    LogLaplace,
    /// `log-normal(mean, standard deviation)` of the logarithm.
    LogNormal,
    /// `log-uniform(lower, upper)` of the value.
    LogUniform,
    /// `normal(mean, standard deviation)`.
    Normal,
    /// `rayleigh(scale)`.
    Rayleigh,
    /// `uniform(lower, upper)`.
    Uniform,
}

impl PriorKind {
    /// The kind of a name of the parameter table.
    #[must_use]
    pub fn from_name(name: &str) -> Option<PriorKind> {
        Some(match name.trim() {
            "cauchy" => PriorKind::Cauchy,
            "chisquare" => PriorKind::ChiSquare,
            "exponential" => PriorKind::Exponential,
            "gamma" => PriorKind::Gamma,
            "laplace" => PriorKind::Laplace,
            "log-laplace" => PriorKind::LogLaplace,
            "log-normal" => PriorKind::LogNormal,
            "log-uniform" => PriorKind::LogUniform,
            "normal" => PriorKind::Normal,
            "rayleigh" => PriorKind::Rayleigh,
            "uniform" => PriorKind::Uniform,
            _ => return None,
        })
    }

    /// The name of the kind in the parameter table.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            PriorKind::Cauchy => "cauchy",
            PriorKind::ChiSquare => "chisquare",
            PriorKind::Exponential => "exponential",
            PriorKind::Gamma => "gamma",
            PriorKind::Laplace => "laplace",
            PriorKind::LogLaplace => "log-laplace",
            PriorKind::LogNormal => "log-normal",
            PriorKind::LogUniform => "log-uniform",
            PriorKind::Normal => "normal",
            PriorKind::Rayleigh => "rayleigh",
            PriorKind::Uniform => "uniform",
        }
    }

    /// The number of parameters of the kind.
    fn arity(self) -> usize {
        match self {
            PriorKind::ChiSquare | PriorKind::Exponential | PriorKind::Rayleigh => 1,
            _ => 2,
        }
    }
}

/// The prior of a parameter, truncated at its bounds.
#[derive(Debug, Clone, PartialEq)]
pub struct Prior {
    /// The distribution.
    pub kind: PriorKind,
    /// Its parameters, the ones of PEtab.
    pub parameters: Vec<f64>,
    /// The lower bound of the parameter.
    pub lower: f64,
    /// The upper bound of the parameter.
    pub upper: f64,
    /// The logarithm of the mass of the distribution between the bounds, `0` for bounds which
    /// cover its support.
    pub log_mass: f64,
}

impl Prior {
    /// The prior of a kind, its parameters and the bounds of its parameter.
    ///
    /// # Errors
    ///
    /// A number of parameters the kind does not take, a parameter outside the domain of the
    /// kind (a scale which is not positive, a uniform interval which does not increase) and
    /// bounds the distribution has no mass between.
    pub fn new(
        kind: PriorKind,
        parameters: Vec<f64>,
        lower: f64,
        upper: f64,
    ) -> Result<Prior, PetabError> {
        let written = parameters
            .iter()
            .map(f64::to_string)
            .collect::<Vec<_>>()
            .join(";");
        let fault =
            |message: String| PetabError::Prior(format!("{}({written}) {message}", kind.name()));
        if parameters.len() != kind.arity() {
            return Err(fault(format!("takes {} parameters", kind.arity())));
        }
        if !parameters.iter().all(|value| value.is_finite()) {
            return Err(fault("has a parameter which is not finite".to_string()));
        }
        let positive = |k: usize| parameters[k] > 0.0;
        let valid = match kind {
            PriorKind::Uniform => parameters[0] < parameters[1],
            PriorKind::LogUniform => positive(0) && parameters[0] < parameters[1],
            PriorKind::Cauchy
            | PriorKind::Laplace
            | PriorKind::LogLaplace
            | PriorKind::LogNormal
            | PriorKind::Normal => positive(1),
            PriorKind::Gamma => positive(0) && positive(1),
            PriorKind::ChiSquare | PriorKind::Exponential | PriorKind::Rayleigh => positive(0),
        };
        if !valid {
            return Err(fault(
                "has parameters outside of the domain of the kind".to_string(),
            ));
        }
        let mut prior = Prior {
            kind,
            parameters,
            lower,
            upper,
            log_mass: 0.0,
        };
        prior.log_mass = prior.mass();
        if !prior.log_mass.is_finite() {
            return Err(fault(format!(
                "has no mass between the bounds {lower} and {upper}"
            )));
        }
        Ok(prior)
    }

    /// The support of the distribution.
    fn support(&self) -> (f64, f64) {
        match self.kind {
            PriorKind::Uniform | PriorKind::LogUniform => (self.parameters[0], self.parameters[1]),
            PriorKind::Cauchy | PriorKind::Laplace | PriorKind::Normal => {
                (f64::NEG_INFINITY, f64::INFINITY)
            }
            _ => (0.0, f64::INFINITY),
        }
    }

    /// The logarithm of the mass between the bounds, `0` for bounds which cover the support.
    fn mass(&self) -> f64 {
        let (low, high) = self.support();
        let lower = self.lower.max(low);
        let upper = self.upper.min(high);
        if lower == low && upper == high {
            return 0.0;
        }
        if lower >= upper {
            return f64::NEG_INFINITY;
        }
        // the tails cut away, each exact where it is small
        let below = if lower == low { 0.0 } else { self.cdf(lower) };
        let above = if upper == high { 0.0 } else { self.sf(upper) };
        if below + above < 0.5 {
            libm::log1p(-(below + above))
        } else {
            (self.cdf(upper) - self.cdf(lower)).ln()
        }
    }

    /// The distribution function of the kind.
    fn cdf(&self, x: f64) -> f64 {
        let p = &self.parameters;
        match self.kind {
            PriorKind::Normal => normal_cdf((x - p[0]) / p[1]),
            PriorKind::LogNormal => normal_cdf((x.ln() - p[0]) / p[1]),
            PriorKind::Laplace => laplace_cdf((x - p[0]) / p[1]),
            PriorKind::LogLaplace => laplace_cdf((x.ln() - p[0]) / p[1]),
            PriorKind::Cauchy => 0.5 + ((x - p[0]) / p[1]).atan() / std::f64::consts::PI,
            PriorKind::Uniform => ((x - p[0]) / (p[1] - p[0])).clamp(0.0, 1.0),
            PriorKind::LogUniform => {
                ((x.ln() - p[0].ln()) / (p[1].ln() - p[0].ln())).clamp(0.0, 1.0)
            }
            PriorKind::Exponential => -libm::expm1(-x / p[0]),
            PriorKind::Rayleigh => -libm::expm1(-x * x / (2.0 * p[0] * p[0])),
            PriorKind::Gamma => gamma_p(p[0], x / p[1]),
            PriorKind::ChiSquare => gamma_p(p[0] / 2.0, x / 2.0),
        }
    }

    /// The survival function of the kind, `1 - cdf`, exact in the upper tail.
    fn sf(&self, x: f64) -> f64 {
        let p = &self.parameters;
        match self.kind {
            PriorKind::Normal => normal_cdf(-(x - p[0]) / p[1]),
            PriorKind::LogNormal => normal_cdf(-(x.ln() - p[0]) / p[1]),
            PriorKind::Laplace => laplace_cdf(-(x - p[0]) / p[1]),
            PriorKind::LogLaplace => laplace_cdf(-(x.ln() - p[0]) / p[1]),
            PriorKind::Cauchy => 0.5 - ((x - p[0]) / p[1]).atan() / std::f64::consts::PI,
            PriorKind::Exponential => (-x / p[0]).exp(),
            PriorKind::Rayleigh => (-x * x / (2.0 * p[0] * p[0])).exp(),
            PriorKind::Gamma => gamma_q(p[0], x / p[1]),
            PriorKind::ChiSquare => gamma_q(p[0] / 2.0, x / 2.0),
            PriorKind::Uniform | PriorKind::LogUniform => 1.0 - self.cdf(x),
        }
    }

    /// The log density of the kind, not truncated; `-inf` outside its support.
    #[must_use]
    pub fn untruncated_log_density(&self, x: f64) -> f64 {
        let p = &self.parameters;
        let (low, high) = self.support();
        if x.is_nan() {
            return f64::NAN;
        }
        if x < low || x > high {
            return f64::NEG_INFINITY;
        }
        match self.kind {
            PriorKind::Normal => {
                let z = (x - p[0]) / p[1];
                -0.5 * z * z - p[1].ln() - LN_SQRT_2PI
            }
            PriorKind::LogNormal => {
                let z = (x.ln() - p[0]) / p[1];
                -0.5 * z * z - p[1].ln() - LN_SQRT_2PI - x.ln()
            }
            PriorKind::Laplace => -(x - p[0]).abs() / p[1] - (2.0 * p[1]).ln(),
            PriorKind::LogLaplace => -(x.ln() - p[0]).abs() / p[1] - (2.0 * p[1]).ln() - x.ln(),
            PriorKind::Cauchy => {
                let z = (x - p[0]) / p[1];
                -(std::f64::consts::PI * p[1]).ln() - libm::log1p(z * z)
            }
            PriorKind::Uniform => -(p[1] - p[0]).ln(),
            PriorKind::LogUniform => -x.ln() - (p[1].ln() - p[0].ln()).ln(),
            PriorKind::Exponential => -p[0].ln() - x / p[0],
            PriorKind::Rayleigh => x.ln() - 2.0 * p[0].ln() - x * x / (2.0 * p[0] * p[0]),
            PriorKind::Gamma => {
                (p[0] - 1.0) * x.ln() - x / p[1] - libm::lgamma(p[0]) - p[0] * p[1].ln()
            }
            PriorKind::ChiSquare => {
                let k = p[0] / 2.0;
                (k - 1.0) * x.ln() - x / 2.0 - k * std::f64::consts::LN_2 - libm::lgamma(k)
            }
        }
    }

    /// The log density of the prior truncated at the bounds, `-inf` outside of them.
    #[must_use]
    pub fn log_density(&self, x: f64) -> f64 {
        if x.is_nan() {
            return f64::NAN;
        }
        if x < self.lower || x > self.upper {
            return f64::NEG_INFINITY;
        }
        self.untruncated_log_density(x) - self.log_mass
    }
}

/// The distribution function of the standard normal distribution, exact in the lower tail.
fn normal_cdf(z: f64) -> f64 {
    0.5 * libm::erfc(-z / std::f64::consts::SQRT_2)
}

/// The distribution function of the standard Laplace distribution.
fn laplace_cdf(z: f64) -> f64 {
    if z < 0.0 {
        0.5 * z.exp()
    } else {
        1.0 - 0.5 * (-z).exp()
    }
}

/// The regularized lower incomplete gamma function `P(a, x)`.
fn gamma_p(a: f64, x: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    if x.is_infinite() {
        return 1.0;
    }
    if x < a + 1.0 {
        gamma_series(a, x)
    } else {
        1.0 - gamma_fraction(a, x)
    }
}

/// The regularized upper incomplete gamma function `Q(a, x) = 1 - P(a, x)`.
fn gamma_q(a: f64, x: f64) -> f64 {
    if x <= 0.0 {
        return 1.0;
    }
    if x.is_infinite() {
        return 0.0;
    }
    if x < a + 1.0 {
        1.0 - gamma_series(a, x)
    } else {
        gamma_fraction(a, x)
    }
}

/// `P(a, x)` by its series, which converges fast for `x < a + 1`.
fn gamma_series(a: f64, x: f64) -> f64 {
    let mut term = 1.0 / a;
    let mut sum = term;
    let mut n = a;
    for _ in 0..1000 {
        n += 1.0;
        term *= x / n;
        sum += term;
        if term.abs() < sum.abs() * f64::EPSILON * 0.25 {
            break;
        }
    }
    sum * (-x + a * x.ln() - libm::lgamma(a)).exp()
}

/// `Q(a, x)` by its continued fraction (the modified Lentz algorithm), which converges fast
/// for `x >= a + 1`.
fn gamma_fraction(a: f64, x: f64) -> f64 {
    const TINY: f64 = 1e-300;
    let mut b = x + 1.0 - a;
    let mut c = 1.0 / TINY;
    let mut d = 1.0 / b;
    let mut h = d;
    for i in 1..1000 {
        let i = f64::from(i);
        let an = -i * (i - a);
        b += 2.0;
        d = an * d + b;
        if d.abs() < TINY {
            d = TINY;
        }
        c = b + an / c;
        if c.abs() < TINY {
            c = TINY;
        }
        d = 1.0 / d;
        let delta = d * c;
        h *= delta;
        if (delta - 1.0).abs() < f64::EPSILON * 0.25 {
            break;
        }
    }
    (-x + a * x.ln() - libm::lgamma(a)).exp() * h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_noise_densities_are_the_ones_of_petab() {
        let normal = NoiseDistribution::Normal.log_density(1.0, 0.5, 0.5);
        assert!((normal - (-LN_SQRT_2PI - 0.5f64.ln() - 0.5)).abs() < 1e-15);
        let laplace = NoiseDistribution::LogLaplace.log_density(2.0, 2.0, 1.0);
        assert!((laplace - -(4.0f64).ln()).abs() < 1e-15);
    }

    #[test]
    fn the_incomplete_gamma_function_has_its_values() {
        // P(1, x) = 1 - exp(-x), P(3, 2) = 1 - 5 exp(-2)
        assert!((gamma_p(1.0, 0.7) - (1.0 - (-0.7f64).exp())).abs() < 1e-15);
        assert!((gamma_p(3.0, 2.0) - (1.0 - 5.0 * (-2.0f64).exp())).abs() < 1e-15);
        assert!((gamma_q(3.0, 20.0) - 221.0 * (-20.0f64).exp()).abs() < 1e-20);
    }

    #[test]
    fn a_prior_whose_bounds_cover_its_support_is_not_truncated() {
        let prior = Prior::new(
            PriorKind::Normal,
            vec![4.0, 2.0],
            f64::NEG_INFINITY,
            f64::INFINITY,
        )
        .unwrap();
        assert_eq!(prior.log_mass, 0.0);
        let truncated = Prior::new(PriorKind::Normal, vec![4.0, 2.0], 0.0, 10.0).unwrap();
        assert!(truncated.log_mass < 0.0);
    }
}
