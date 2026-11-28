//! The special functions of the distributions: the logarithm of the beta function, the
//! regularized incomplete beta function and the distribution functions and quantiles of the
//! normal, the Student t and the binomial distribution, as scipy gives them.
//!
//! A function of a probability or a parameter outside of its domain, or of not a number, is not
//! a number, as in scipy. The tests hold every function to scipy (`oracles/special.json`): the
//! normal distribution to 1e-13 relative, the logarithm of the beta function to 1e-12, the
//! incomplete beta function and the t and binomial distributions to 1e-10, the binomial
//! quantile exactly.
//!
//! The range of these digits: the continued fraction of the incomplete beta function loses
//! about `a * eps` relative when both of its parameters are large, so the incomplete beta
//! function keeps them for both parameters below about 1e6 (a large one with a small one,
//! below 40, takes an expansion without the loss) and the binomial distribution for up to
//! about 1e6 draws (6.7e-11 relative at 1e6, 2.3e-10 at 1e7). The t distribution keeps them
//! for every number of degrees of freedom. Where the loss would exceed about 1e-6, both
//! parameters above [`CF_MAX`], and where a continued fraction does not converge in
//! [`MAX_STEPS`] steps, a function gives not a number instead of a wrong value.
//!
//! References:
//!
//! - Press, W. H., Teukolsky, S. A., Vetterling, W. T., Flannery, B. P. (2007). Numerical
//!   Recipes, 3rd edition, sections 6.2 (the incomplete gamma function) and 6.4 (the
//!   incomplete beta function, its continued fraction by the method of Lentz).
//! - DiDonato, A. R., Morris, A. H. (1992). Algorithm 708: Significant digit computation of the
//!   incomplete beta function ratios. ACM Transactions on Mathematical Software 18(3):360-373,
//!   the expansion BGRAT for a large first parameter and a second one up to one, and the
//!   recurrence `bup`, which takes it to a second parameter above one.
//! - Wichura, M. J. (1988). Algorithm AS 241: The percentage points of the normal distribution.
//!   Journal of the Royal Statistical Society, Series C 37(3):477-484.

use std::f64::consts::FRAC_1_SQRT_2;

/// The logarithm of the square root of two pi, the constant of the formula of Stirling.
const LN_SQRT_2PI: f64 = 0.918_938_533_204_672_8;

/// The most degrees of freedom of a t distribution which is not the normal distribution: above,
/// the two differ by less than a rounding (by about `x^4 / (4 df)` relative).
const T_NORMAL_DF: f64 = 1e20;

/// The relative difference below which a value of the binomial distribution function reaches
/// a probability, a few roundings.
const BINOM_TIE: f64 = 8.0 * f64::EPSILON;

/// The smallest first parameter `a` of the incomplete beta function `I_x(a, b)` which takes the
/// expansion BGRAT, with a second parameter below [`BGRAT_B_MAX`] and an `x` above
/// [`BGRAT_X_MIN`]: there the continued fraction, a function of `x`, loses the digits of
/// `1 - x` by a factor of about `a`, while the expansion is a function of `ln x = log1p(-y)`.
const BGRAT_A_MIN: f64 = 100.0;

/// The second parameter below which the expansion BGRAT, of a second parameter up to one, takes
/// a larger one through the recurrence `I_x(a, b + 1) = I_x(a, b) + x^a y^b / (b B(a, b))` of
/// positive terms (`bup` of DiDonato and Morris, who take it below 40 as well).
const BGRAT_B_MAX: f64 = 40.0;

/// The smallest `x` of the expansion BGRAT, which converges for an `ln x` small against `a`;
/// DiDonato and Morris take it above 0.7 as well.
const BGRAT_X_MIN: f64 = 0.7;

/// The most steps of a continued fraction or a series, far more than they take to a rounding
/// (the continued fraction of the incomplete beta function takes a few dozen below its mean,
/// about the square root of its parameters at its mean); a function which does not converge
/// in them is not a number.
pub const MAX_STEPS: usize = 1_000_000;

/// The largest smaller parameter of the continued fraction of the incomplete beta function:
/// above, it loses more than about 1e-6 relative (`a * eps`; 2e-8 at 1e9, above one at 1e16),
/// and the function is not a number.
pub const CF_MAX: f64 = 1e10;

/// The logarithm of the gamma function, of libm.
fn ln_gamma(x: f64) -> f64 {
    libm::lgamma_r(x).0
}

/// The remainder of the formula of Stirling, `ln Γ(x) - ((x - 1/2) ln x - x + ln √(2π))`, for
/// `x >= 10` by its asymptotic series, the coefficients `B_2k / (2k (2k - 1))`: the first term
/// left out is 2e-18 at 10.
fn stirling_remainder(x: f64) -> f64 {
    let r = 1.0 / x;
    let r2 = r * r;
    r * (1.0 / 12.0
        + r2 * (-1.0 / 360.0
            + r2 * (1.0 / 1260.0
                + r2 * (-1.0 / 1680.0
                    + r2 * (1.0 / 1188.0
                        + r2 * (-691.0 / 360_360.0
                            + r2 * (1.0 / 156.0 + r2 * (-3617.0 / 122_400.0))))))))
}

/// The logarithm of the beta function `B(a, b) = Γ(a) Γ(b) / Γ(a + b)` of positive `a` and
/// `b`, `scipy.special.betaln`.
///
/// A parameter of at least ten takes the formula of Stirling with its remainder, so the
/// logarithms of the gamma function, which grow as `x ln x`, do not cancel (the scheme of
/// `lbeta` of R): the result is accurate to a few roundings of itself, also for a large and a
/// small parameter, where betaln of scipy loses digits (1.5e-10 relative at 0.5 and 1e6).
/// Not a number for a parameter which is not positive, where scipy continues the function.
#[must_use]
pub fn ln_beta(a: f64, b: f64) -> f64 {
    if !(a > 0.0 && b > 0.0) {
        return f64::NAN;
    }
    let (p, q) = if a < b { (a, b) } else { (b, a) };
    if q.is_infinite() {
        return f64::NEG_INFINITY;
    }
    let ratio = p / (p + q);
    if p >= 10.0 {
        let correction = stirling_remainder(p) + stirling_remainder(q) - stirling_remainder(p + q);
        -0.5 * libm::log(q)
            + LN_SQRT_2PI
            + correction
            + (p - 0.5) * libm::log(ratio)
            + q * libm::log1p(-ratio)
    } else if q >= 10.0 {
        let correction = stirling_remainder(q) - stirling_remainder(p + q);
        ln_gamma(p) + correction + p - p * libm::log(p + q) + (q - 0.5) * libm::log1p(-ratio)
    } else {
        ln_gamma(p) + ln_gamma(q) - ln_gamma(p + q)
    }
}

/// The regularized incomplete beta function `I_x(a, b)` of positive `a` and `b` and `x` in
/// `[0, 1]`, `scipy.special.betainc`.
///
/// The continued fraction of the function by the method of Lentz, of `I_x(a, b)` below the
/// mean `(a + 1) / (a + b + 2)` and of `1 - I_(1-x)(b, a)` above it. Close to one, a large
/// first parameter with a small second one takes the expansion BGRAT of DiDonato and Morris
/// instead, whose digits do not depend on those of `x` but of `1 - x`.
///
/// Accurate to 1e-10 relative for both parameters below about 1e6, where the continued
/// fraction loses about `a * eps`; between 1e6 and [`CF_MAX`] (a large `a` with a `b` above
/// 40, outside the expansion BGRAT) it loses that silently, which only `binom_ppf` of the
/// intervals of more than 1e6 draws reaches, where it can move a rank at a near tie only; not
/// a number for both above [`CF_MAX`] and where it does
/// not converge in [`MAX_STEPS`] steps. As scipy, an infinite parameter is the limit: zero
/// below `x = 1` for an infinite `a`, one above `x = 0` for an infinite `b`, not a number for
/// both.
#[must_use]
pub fn beta_inc(a: f64, b: f64, x: f64) -> f64 {
    if !(a > 0.0 && b > 0.0 && (0.0..=1.0).contains(&x)) {
        return f64::NAN;
    }
    if a.is_infinite() && b.is_infinite() {
        return f64::NAN;
    }
    if a.is_infinite() {
        return if x == 1.0 { 1.0 } else { 0.0 };
    }
    if b.is_infinite() {
        return if x == 0.0 { 0.0 } else { 1.0 };
    }
    beta_inc_xy(a, b, x, 1.0 - x)
}

/// `I_x(a, b)` of `x` and `y = 1 - x`, both given, so a caller which knows the smaller one
/// exactly (the probability of a binomial distribution, the ratio of a t distribution) keeps
/// its digits.
fn beta_inc_xy(a: f64, b: f64, x: f64, y: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    if y <= 0.0 {
        return 1.0;
    }
    if a == 1.0 && b == 1.0 {
        return x;
    }
    // the closed forms of a parameter one: I_x(a, 1) = x^a and I_x(1, b) = 1 - y^b
    if b == 1.0 {
        return libm::exp(a * ln_of(x, y));
    }
    if a == 1.0 {
        return -libm::expm1(b * ln_of(y, x));
    }
    if x > (a + 1.0) / (a + b + 2.0) {
        1.0 - beta_inc_below_mean(b, a, y, x)
    } else {
        beta_inc_below_mean(a, b, x, y)
    }
}

/// The logarithm of `x` of `x` and `y = 1 - x`, through `log1p(-y)` for an `x` close to one.
fn ln_of(x: f64, y: f64) -> f64 {
    if x > 0.5 {
        libm::log1p(-y)
    } else {
        libm::log(x)
    }
}

/// `I_x(a, b)` of an `x` at most the mean `(a + 1) / (a + b + 2)`.
fn beta_inc_below_mean(a: f64, b: f64, x: f64, y: f64) -> f64 {
    if a >= BGRAT_A_MIN && b < BGRAT_B_MAX && x > BGRAT_X_MIN {
        return bgrat_up(a, b, x, y);
    }
    if a.min(b) > CF_MAX {
        return f64::NAN;
    }
    let front = libm::exp(a * ln_of(x, y) + b * ln_of(y, x) - ln_beta(a, b)) / a;
    front * beta_continued_fraction(a, b, x, MAX_STEPS)
}

/// `I_x(a, b)` of a large `a` and an `x` close to one: the expansion BGRAT of `I_x(a, b0)` of
/// `b0` in `(0, 1]`, `b` less a whole number `n`, and the `n` terms of the recurrence
/// `I_x(a, j + 1) = I_x(a, j) + x^a y^j / (j B(a, j))` from `b0` to `b`, all of them positive.
fn bgrat_up(a: f64, b: f64, x: f64, y: f64) -> f64 {
    let n = libm::ceil(b) - 1.0;
    let b0 = b - n;
    let mut sum = bgrat(a, b0, x, y);
    if n > 0.0 {
        let ln_x = ln_of(x, y);
        let ln_y = ln_of(y, x);
        // x^a y^j / (j B(a, j)) of j = b0, and of j + 1 its multiple y (a + j) / (j + 1)
        let mut term = libm::exp(a * ln_x + b0 * ln_y - libm::log(b0) - ln_beta(a, b0));
        let mut j = b0;
        for _ in 0..n as usize {
            sum += term;
            term *= y * (a + j) / (j + 1.0);
            j += 1.0;
        }
    }
    sum
}

/// The continued fraction of `I_x(a, b) a B(a, b) / (x^a y^b)`, by the modified method of
/// Lentz (Numerical Recipes, `betacf`), to a rounding; not a number if it does not converge in
/// `max_steps` steps.
fn beta_continued_fraction(a: f64, b: f64, x: f64, max_steps: usize) -> f64 {
    const TINY: f64 = 1e-300;
    let guard = |value: f64| if value.abs() < TINY { TINY } else { value };
    let (sum, a_plus_one, a_minus_one) = (a + b, a + 1.0, a - 1.0);
    let mut c = 1.0;
    let mut d = 1.0 / guard(1.0 - sum * x / a_plus_one);
    let mut h = d;
    for step in 1..=max_steps {
        let m = step as f64;
        let m2 = 2.0 * m;
        // the even step
        let coefficient = m * (b - m) * x / ((a_minus_one + m2) * (a + m2));
        d = 1.0 / guard(1.0 + coefficient * d);
        c = guard(1.0 + coefficient / c);
        h *= d * c;
        // the odd step
        let coefficient = -(a + m) * (sum + m) * x / ((a + m2) * (a_plus_one + m2));
        d = 1.0 / guard(1.0 + coefficient * d);
        c = guard(1.0 + coefficient / c);
        let delta = d * c;
        h *= delta;
        if (delta - 1.0).abs() <= f64::EPSILON {
            return h;
        }
    }
    f64::NAN
}

/// `I_x(a, b)` of a large `a` and a `b` up to one by the asymptotic expansion BGRAT of
/// DiDonato and Morris (1992, equations 9 to 9.6), in the arrangement of
/// `beta_small_b_large_a_series` of Boost: with `T = a + (b - 1) / 2` and `u = -T ln x`,
/// `I_x(a, b) = Γ(a + b) / (Γ(a) T^b) Σ p_n J_n`, whose first term is the regularized upper
/// incomplete gamma function `Q(b, u)`.
fn bgrat(a: f64, b: f64, x: f64, y: f64) -> f64 {
    const TERMS: usize = 30;
    let b_minus_one = b - 1.0;
    let t = a + b_minus_one / 2.0;
    let ln_x = if y < 0.35 {
        libm::log1p(-y)
    } else {
        libm::log(x)
    };
    let u = -t * ln_x;
    let ln_gamma_b = ln_gamma(b);
    // the factor of the terms past the first, u^b e^-u / Γ(b)
    let h = libm::exp(b * libm::log(u) - u - ln_gamma_b);
    // Γ(a + b) / (Γ(a) T^b), through the beta function, whose formula of Stirling keeps it
    // accurate for a large a
    let ratio = libm::exp(ln_gamma_b - ln_beta(a, b) - b * libm::log(t));

    // (2m + 1)! of m = 0..TERMS
    let mut factorials = [1.0; TERMS + 1];
    for m in 1..=TERMS {
        factorials[m] = factorials[m - 1] * (2 * m) as f64 * (2 * m + 1) as f64;
    }
    let mut p = [0.0; TERMS];
    p[0] = 1.0;
    // J_n times h, from J_0 h = Q(b, u)
    let mut j = gamma_q(b, u);
    let mut sum = j;
    let ln_x_half_squared = (ln_x / 2.0) * (ln_x / 2.0);
    let mut ln_x_power = 1.0;
    let mut b2n = b;
    let t4 = 4.0 * t * t;
    for n in 1..TERMS {
        let mut pn = 0.0;
        for m in 1..n {
            pn += (m as f64 * b - n as f64) * p[n - m] / factorials[m];
        }
        p[n] = pn / n as f64 + b_minus_one / factorials[n];
        j = (b2n * (b2n + 1.0) * j + h * (u + b2n + 1.0) * ln_x_power) / t4;
        ln_x_power *= ln_x_half_squared;
        b2n += 2.0;
        let term = p[n] * j;
        sum += term;
        if term.abs() <= f64::EPSILON * sum.abs() {
            break;
        }
    }
    ratio * sum
}

/// The regularized upper incomplete gamma function `Q(b, u)` of `b` in `(0, 1]`: the closed
/// forms of one half (`erfc √u`) and one (`e^-u`), else the series of `P = 1 - Q` below
/// `u = b + 1` and the continued fraction of `Q` above (Numerical Recipes, `gser`, `gcf`), not
/// a number if either does not converge in [`MAX_STEPS`] steps.
fn gamma_q(b: f64, u: f64) -> f64 {
    if b == 0.5 {
        return libm::erfc(libm::sqrt(u));
    }
    if b == 1.0 {
        return libm::exp(-u);
    }
    if u <= 0.0 {
        return 1.0;
    }
    let front = b * libm::log(u) - u;
    if u < b + 1.0 {
        let mut term = 1.0 / b;
        let mut sum = term;
        let mut denominator = b;
        for _ in 0..MAX_STEPS {
            denominator += 1.0;
            term *= u / denominator;
            sum += term;
            if term.abs() <= sum.abs() * f64::EPSILON {
                return 1.0 - sum * libm::exp(front - ln_gamma(b));
            }
        }
        f64::NAN
    } else {
        const TINY: f64 = 1e-300;
        let guard = |value: f64| if value.abs() < TINY { TINY } else { value };
        let mut bn = u + 1.0 - b;
        let mut c = 1.0 / TINY;
        let mut d = 1.0 / bn;
        let mut h = d;
        for step in 1..=MAX_STEPS {
            let i = step as f64;
            let an = -i * (i - b);
            bn += 2.0;
            d = 1.0 / guard(an * d + bn);
            c = guard(bn + an / c);
            let delta = d * c;
            h *= delta;
            if (delta - 1.0).abs() <= f64::EPSILON {
                return libm::exp(front - ln_gamma(b)) * h;
            }
        }
        f64::NAN
    }
}

/// The distribution function of the standard normal distribution, `scipy.stats.norm.cdf`:
/// `erf` around zero and `erfc` in the tails, the scheme of `ndtr` of scipy.
#[must_use]
pub fn normal_cdf(x: f64) -> f64 {
    let z = x * FRAC_1_SQRT_2;
    if z.abs() < FRAC_1_SQRT_2 {
        0.5 + 0.5 * libm::erf(z)
    } else {
        let tail = 0.5 * libm::erfc(z.abs());
        if z > 0.0 { 1.0 - tail } else { tail }
    }
}

/// The quantile of the standard normal distribution, `scipy.stats.norm.ppf`: the algorithm AS
/// 241 of Wichura (`sbmlsim_sensitivity::normal_quantile`), minus and plus infinity at zero
/// and one.
#[must_use]
pub fn normal_ppf(p: f64) -> f64 {
    if p == 0.0 {
        f64::NEG_INFINITY
    } else if p == 1.0 {
        f64::INFINITY
    } else {
        sbmlsim_sensitivity::normal_quantile(p)
    }
}

/// The distribution function of the Student t distribution of `df` degrees of freedom,
/// `scipy.stats.t.cdf`: `I_z(df / 2, 1 / 2) / 2` of `z = df / (df + x^2)` for a negative `x`
/// and its complement for a positive one; above 1e20 degrees of freedom the normal
/// distribution.
#[must_use]
pub fn t_cdf(x: f64, df: f64) -> f64 {
    if x.is_nan() || df.is_nan() || df <= 0.0 {
        return f64::NAN;
    }
    if df > T_NORMAL_DF {
        return normal_cdf(x);
    }
    if x == 0.0 {
        return 0.5;
    }
    let tail = t_tail(x.abs(), df);
    if x < 0.0 { tail } else { 1.0 - tail }
}

/// The lower tail `P(T <= -t)` of a positive `t`, from `z = df / (df + t^2)` and its
/// complement `t^2 / (df + t^2)`, both without a difference; far out, where `z` underflows,
/// the first term `z^a / (a B(a, 1/2))` of the series of `I_z(a, 1/2)`, `a = df / 2`, whose
/// next one is below `a z` of it.
fn t_tail(t: f64, df: f64) -> f64 {
    let ln_df = libm::log(df);
    let ln_r = ln_df - 2.0 * libm::log(t);
    if ln_r < -46.0 && ln_r + ln_df.max(0.0) < -37.0 {
        let a = df / 2.0;
        return 0.5 * libm::exp(a * ln_r - libm::log(a) - ln_beta(a, 0.5));
    }
    let (z, w) = if t * t > df {
        let r = df / t / t;
        (r / (1.0 + r), 1.0 / (1.0 + r))
    } else {
        let r = t * t / df;
        (1.0 / (1.0 + r), r / (1.0 + r))
    };
    0.5 * beta_inc_xy(df / 2.0, 0.5, z, w)
}

/// The logarithm of the density of the Student t distribution at a positive `t`.
fn t_ln_pdf(t: f64, df: f64) -> f64 {
    let ln_df = libm::log(df);
    // ln(1 + t^2 / df), without an overflow of t^2
    let ln_ratio = if t * t > df {
        2.0 * libm::log(t) - ln_df + libm::log1p(df / t / t)
    } else {
        libm::log1p(t * t / df)
    };
    -ln_beta(df / 2.0, 0.5) - 0.5 * ln_df - (df + 1.0) / 2.0 * ln_ratio
}

/// The quantile of the Student t distribution of `df` degrees of freedom, `scipy.stats.t.ppf`.
///
/// The tail of the smaller probability `q` by the method of Newton on `ln P(T <= -e^s) - ln q`
/// in `s`, the logarithm of the quantile, in which the tail of every t distribution is close to
/// a line, from the normal quantile, which bounds it from below, and guarded by bisection
/// between it and the quantile of the power law of the tail, which bounds it from above. Above
/// 1e20 degrees of freedom the normal quantile.
#[must_use]
pub fn t_ppf(p: f64, df: f64) -> f64 {
    if !((0.0..=1.0).contains(&p) && df > 0.0) {
        return f64::NAN;
    }
    if df > T_NORMAL_DF {
        return normal_ppf(p);
    }
    if p == 0.0 {
        return f64::NEG_INFINITY;
    }
    if p == 1.0 {
        return f64::INFINITY;
    }
    if p == 0.5 {
        return 0.0;
    }
    // 1 - p is exact for p of at least one half
    let (q, sign) = if p < 0.5 { (p, -1.0) } else { (1.0 - p, 1.0) };
    sign * t_tail_quantile(q, df)
}

/// The positive `t` of `P(T <= -t) = q` of a `q` below one half.
fn t_tail_quantile(q: f64, df: f64) -> f64 {
    let ln_q = libm::log(q);
    // the normal quantile is closer to zero than every t quantile
    let normal = -normal_ppf(q);
    // the tail of the density is below k (t^2 / df)^(-(df + 1) / 2), whose integral is q at
    // the bound above
    let ln_bound = ((df / 2.0 - 1.0) * libm::log(df) - ln_beta(df / 2.0, 0.5) - ln_q) / df;
    let mut low = libm::log(normal) - 1e-3;
    let mut high = ln_bound.max(low) + 1e-3;
    let mut s = low + 1e-3;
    for _ in 0..200 {
        let t = libm::exp(s);
        let tail = t_tail(t, df);
        let ln_tail = libm::log(tail);
        let f = ln_tail - ln_q;
        if f == 0.0 {
            break;
        }
        if f > 0.0 {
            low = s;
        } else {
            high = s;
        }
        // d ln P(T <= -e^s) / ds = -t pdf(t) / P(T <= -t)
        let slope = -libm::exp(s + t_ln_pdf(t, df) - ln_tail);
        let mut next = s - f / slope;
        if !(next > low && next < high) {
            next = 0.5 * (low + high);
        }
        let step = (next - s).abs();
        s = next;
        let scale = low.abs().max(high.abs()).max(1.0);
        if step <= 2.0 * f64::EPSILON * scale || high - low <= 2.0 * f64::EPSILON * scale {
            break;
        }
    }
    libm::exp(s)
}

/// The distribution function of the binomial distribution of `n` draws of the probability `p`
/// at `k`, `scipy.stats.binom.cdf`: `I_(1-p)(n - k, k + 1)`, of `p` itself as the complement,
/// so a small probability keeps its digits. Accurate to 1e-10 relative for up to about 1e6
/// draws (see [`beta_inc`]).
#[must_use]
pub fn binom_cdf(k: u64, n: u64, p: f64) -> f64 {
    if !(0.0..=1.0).contains(&p) {
        return f64::NAN;
    }
    if k >= n || p == 0.0 {
        return 1.0;
    }
    if p == 1.0 {
        return 0.0;
    }
    beta_inc_xy((n - k) as f64, (k + 1) as f64, 1.0 - p, p)
}

/// The quantile of the binomial distribution of `n` draws of the probability `p`,
/// `scipy.stats.binom.ppf`: the smallest `k` whose distribution function reaches `q`, by
/// bisection of the support, as a double; as scipy -1 at `q` zero and `n` at `q` one.
///
/// A distribution function within eight roundings of `q` (relative) reaches it: a `q`
/// which is a value of the distribution function, of a decimal probability
/// (`binom.ppf(0.81, 2, 0.1)`) or as scipy computes it, is a tie which the last digits of
/// either decide, and scipy mostly takes it as reached.
#[must_use]
pub fn binom_ppf(q: f64, n: u64, p: f64) -> f64 {
    if !((0.0..=1.0).contains(&q) && (0.0..=1.0).contains(&p)) {
        return f64::NAN;
    }
    if q == 0.0 {
        return -1.0;
    }
    if q == 1.0 {
        return n as f64;
    }
    let (mut low, mut high) = (0, n);
    while low < high {
        let middle = low + (high - low) / 2;
        if binom_cdf(middle, n, p) >= q * (1.0 - BINOM_TIE) {
            high = middle;
        } else {
            low = middle + 1;
        }
    }
    low as f64
}

#[cfg(test)]
mod tests {
    use super::{MAX_STEPS, beta_continued_fraction};

    #[test]
    fn a_continued_fraction_which_does_not_converge_is_not_a_number() {
        // at the mean of two large parameters it takes about the square root of them in steps
        let (a, b, x) = (1e6, 1e6, 0.5);
        assert!(beta_continued_fraction(a, b, x, MAX_STEPS).is_finite());
        assert!(beta_continued_fraction(a, b, x, 10).is_nan());
    }
}
