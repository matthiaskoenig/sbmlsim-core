//! The one implementation of every math function of a model.
//!
//! Every `model.wasm` imports these functions under the module name `math`, so a result is bitwise identical on every wasm host. The functions are those of the `libm` crate, never the builtin math of a host or of the compiler (no fast math). The crate does no file system or OS access and `mathlib.wasm` imports nothing.

/// The math functions as plain Rust functions, for native use.
pub mod f {
    /// `e^x`.
    pub fn math_exp(x: f64) -> f64 {
        libm::exp(x)
    }
    /// Natural logarithm.
    pub fn math_log(x: f64) -> f64 {
        libm::log(x)
    }
    /// Base 10 logarithm.
    pub fn math_log10(x: f64) -> f64 {
        libm::log10(x)
    }
    /// `x^y`.
    pub fn math_pow(x: f64, y: f64) -> f64 {
        libm::pow(x, y)
    }
    /// Square root.
    pub fn math_sqrt(x: f64) -> f64 {
        libm::sqrt(x)
    }
    /// Sine.
    pub fn math_sin(x: f64) -> f64 {
        libm::sin(x)
    }
    /// Cosine.
    pub fn math_cos(x: f64) -> f64 {
        libm::cos(x)
    }
    /// Tangent.
    pub fn math_tan(x: f64) -> f64 {
        libm::tan(x)
    }
    /// Arc sine.
    pub fn math_asin(x: f64) -> f64 {
        libm::asin(x)
    }
    /// Arc cosine.
    pub fn math_acos(x: f64) -> f64 {
        libm::acos(x)
    }
    /// Arc tangent.
    pub fn math_atan(x: f64) -> f64 {
        libm::atan(x)
    }
    /// Hyperbolic sine.
    pub fn math_sinh(x: f64) -> f64 {
        libm::sinh(x)
    }
    /// Hyperbolic cosine.
    pub fn math_cosh(x: f64) -> f64 {
        libm::cosh(x)
    }
    /// Hyperbolic tangent.
    pub fn math_tanh(x: f64) -> f64 {
        libm::tanh(x)
    }
    /// Inverse hyperbolic sine.
    pub fn math_asinh(x: f64) -> f64 {
        libm::asinh(x)
    }
    /// Inverse hyperbolic cosine.
    pub fn math_acosh(x: f64) -> f64 {
        libm::acosh(x)
    }
    /// Inverse hyperbolic tangent.
    pub fn math_atanh(x: f64) -> f64 {
        libm::atanh(x)
    }
    /// Largest integer not above `x`.
    pub fn math_floor(x: f64) -> f64 {
        libm::floor(x)
    }
    /// Smallest integer not below `x`.
    pub fn math_ceil(x: f64) -> f64 {
        libm::ceil(x)
    }
    /// Remainder of `x / y` with the sign of `x` (C `fmod`, Rust `%`).
    pub fn math_fmod(x: f64, y: f64) -> f64 {
        libm::fmod(x, y)
    }
    /// `x!` as the running product `1 * 2 * ... * x`, exact in doubles.
    ///
    /// Defined for non-negative integers only: a negative or non-integer `x` (and NaN) gives NaN, and `x >= 171` gives infinity, the overflow of `171!`.
    pub fn math_factorial(x: f64) -> f64 {
        if x.is_nan() || x < 0.0 || libm::floor(x) != x {
            return f64::NAN;
        }
        if x >= 171.0 {
            return f64::INFINITY;
        }
        let mut p = 1.0;
        let mut i = 2.0;
        while i <= x {
            p *= i;
            i += 1.0;
        }
        p
    }
}

macro_rules! exports {
    ($($name:ident($($arg:ident),+);)+) => {
        $(
            #[unsafe(no_mangle)]
            pub extern "C" fn $name($($arg: f64),+) -> f64 {
                f::$name($($arg),+)
            }
        )+
    };
}

exports! {
    math_exp(x); math_log(x); math_log10(x); math_pow(x, y); math_sqrt(x);
    math_sin(x); math_cos(x); math_tan(x); math_asin(x); math_acos(x); math_atan(x);
    math_sinh(x); math_cosh(x); math_tanh(x); math_asinh(x); math_acosh(x); math_atanh(x);
    math_floor(x); math_ceil(x); math_factorial(x); math_fmod(x, y);
}
