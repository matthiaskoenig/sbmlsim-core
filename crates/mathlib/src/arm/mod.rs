//! `exp`, `log` and `pow` of ARM optimized-routines (<https://github.com/ARM-software/optimized-routines>,
//! commit 20859d3fd9e797dde592c489c8ef7d383f778620: `math/exp.c`, `math/log.c`, `math/pow.c`,
//! `math/pow_common.h`, `math/math_config.h` and the tables of `data.rs`), the implementations of
//! glibc >= 2.28 and musl >= 1.1.20, ported to Rust.
//!
//! Copyright (c) 2018-2026, Arm Limited. SPDX-License-Identifier: MIT OR Apache-2.0 WITH LLVM-exception
//! (see `THIRD_PARTY.md`).
//!
//! The port follows the C code with `HAVE_FAST_FMA == 0`, `TOINT_INTRINSICS == 0`, `WANT_ROUNDING == 1`
//! and `EXP_USE_TOINT_NARROW == 0`: every product is a plain rounded multiplication (Dekker splits where the C
//! code would use `fma`), so the result is the same bits natively and in wasm, which has no fused multiply
//! add. The order of every floating point operation is the one of the C code. Left out are the parts which
//! only concern floating point exceptions, errno and rounding modes other than to nearest, none of which
//! changes a result in round to nearest. Worst case errors (without fma): `exp` 0.511 ulp, `log` 0.52 ulp,
//! `pow` 0.54 ulp.

mod data;

use data::*;

const SIGN_MASK: u64 = 0x8000_0000_0000_0000;
const ONE_BITS: u64 = 0x3ff0_0000_0000_0000;
const INF_BITS: u64 = 0x7ff0_0000_0000_0000;
/// `0x1p1009`.
const P1009: f64 = f64::from_bits((0x3ff + 1009) << 52);
/// `0x1p-1022`.
const PM1022: f64 = f64::MIN_POSITIVE;
/// `0x1p52`.
const P52: f64 = 4503599627370496.0;
/// `top12(0x1p-54)`.
const TOP12_P_54: u32 = 0x3ff - 54;
/// `top12(512.0)`.
const TOP12_512: u32 = 0x3ff + 9;
/// `top12(1024.0)`.
const TOP12_1024: u32 = 0x3ff + 10;
/// Number of entries of the exp table, `1 << EXP_TABLE_BITS`.
const EXP_N: u64 = 128;
/// The sign bias of `exp_inline`: added to the `k` of the exponent it flips the sign bit of the scale.
const SIGN_BIAS: u64 = 0x800 << 7;

/// Top 12 bits of a double (sign and exponent bits).
#[inline(always)]
fn top12(x: f64) -> u32 {
    (x.to_bits() >> 52) as u32
}

/// `__math_oflow(sign)`: infinity with the sign.
#[inline(always)]
fn oflow(sign: u64) -> f64 {
    if sign != 0 {
        f64::NEG_INFINITY
    } else {
        f64::INFINITY
    }
}

/// `__math_uflow(sign)`: zero with the sign.
#[inline(always)]
fn uflow(sign: u64) -> f64 {
    if sign != 0 { -0.0 } else { 0.0 }
}

/// `__math_invalid(x)`: NaN (a NaN `x` propagates).
#[inline(always)]
#[allow(clippy::eq_op)]
fn invalid(x: f64) -> f64 {
    (x - x) / (x - x)
}

/// Handles the cases of `exp_inline` which may overflow or underflow when computing the result `scale *
/// (1 + tmp)` without intermediate rounding. `sbits` is the bit representation of the scale, whose computed
/// exponent may have overflown into the sign bit. `ki` as `i32` is the `k` of the argument reduction, positive
/// when the result may overflow, negative when it may underflow.
#[inline(never)]
fn specialcase(tmp: f64, sbits: u64, ki: u64) -> f64 {
    if ki & 0x8000_0000 == 0 {
        // k > 0, the exponent of the scale might have overflowed by <= 460.
        let scale = f64::from_bits(sbits.wrapping_sub(1009 << 52));
        let y = scale + scale * tmp;
        return y * P1009;
    }
    // k < 0, need special care in the subnormal range.
    let sbits = sbits.wrapping_add(1022 << 52);
    // Note: sbits is the signed scale.
    let scale = f64::from_bits(sbits);
    let mut y = scale + scale * tmp;
    if y.abs() < 1.0 {
        // Round y to the right precision before scaling it into the subnormal range to avoid double
        // rounding that can cause 0.5+E/2 ulp error where E is the worst-case ulp error outside the
        // subnormal range.
        let one = if y < 0.0 { -1.0 } else { 1.0 };
        let lo = scale - y + scale * tmp;
        let hi = one + y;
        let lo = one - hi + y + lo;
        y = (hi + lo) - one;
        // Fix the sign of 0.
        if y == 0.0 {
            y = f64::from_bits(sbits & SIGN_MASK);
        }
    }
    PM1022 * y
}

/// `sign * exp(x + xtail)` where `|xtail| < 2^-8/N` and `|xtail| <= |x|`; `sign_bias` is `SIGN_BIAS` or 0
/// and sets the sign to -1 or 1; without `TAIL`, `xtail` is taken as 0. NaN and infinities of `x` must be
/// handled by the caller.
#[inline(always)]
fn exp_inline<const TAIL: bool>(x: f64, xtail: f64, sign_bias: u64) -> f64 {
    let mut abstop = top12(x) & 0x7ff;
    if abstop.wrapping_sub(TOP12_P_54) >= TOP12_512 - TOP12_P_54 {
        if abstop.wrapping_sub(TOP12_P_54) >= 0x8000_0000 {
            // Avoid spurious underflow for tiny x. Note: 0 is common input.
            let one = 1.0 + x;
            return if sign_bias != 0 { -one } else { one };
        }
        if abstop >= TOP12_1024 {
            return if x.to_bits() >> 63 != 0 {
                uflow(sign_bias)
            } else {
                oflow(sign_bias)
            };
        }
        // Large x is special cased below.
        abstop = 0;
    }
    // exp(x) = 2^(k/N) * exp(r), with exp(r) in [2^(-1/2N),2^(1/2N)].
    // x = ln2/N*k + r, with int k and r in [-ln2/2N, ln2/2N].
    let z = EXP_INV_LN2_N * x;
    // z - kd is in [-1, 1] in non-nearest rounding modes.
    let mut kd = z + EXP_SHIFT;
    let ki = kd.to_bits();
    kd -= EXP_SHIFT;
    let mut r = x + kd * EXP_NEG_LN2_HI_N + kd * EXP_NEG_LN2_LO_N;
    // The code assumes 2^-200 < |xtail| < 2^-8/N.
    if TAIL {
        r += xtail;
    }
    // 2^(k/N) ~= scale * (1 + tail).
    let idx = (2 * (ki % EXP_N)) as usize;
    let top = ki.wrapping_add(sign_bias) << (52 - 7);
    let tail = f64::from_bits(EXP_TAB[idx]);
    // This is only a valid scale when -1023*N < k < 1024*N.
    let sbits = EXP_TAB[idx + 1].wrapping_add(top);
    // exp(x) = 2^(k/N) * exp(r) ~= scale + scale * (tail + exp(r) - 1).
    let r2 = r * r;
    let tmp =
        tail + r + r2 * (EXP_POLY[0] + r * EXP_POLY[1]) + r2 * r2 * (EXP_POLY[2] + r * EXP_POLY[3]);
    if abstop == 0 {
        return specialcase(tmp, sbits, ki);
    }
    let scale = f64::from_bits(sbits);
    scale + scale * tmp
}

/// `e^x`.
pub fn exp(x: f64) -> f64 {
    let abstop = top12(x) & 0x7ff;
    if abstop >= TOP12_1024 {
        if x.to_bits() == f64::NEG_INFINITY.to_bits() {
            return 0.0;
        }
        if abstop >= 0x7ff {
            return 1.0 + x;
        }
        return if x.to_bits() >> 63 != 0 {
            uflow(0)
        } else {
            oflow(0)
        };
    }
    exp_inline::<false>(x, 0.0, 0)
}

/// Natural logarithm.
pub fn log(x: f64) -> f64 {
    /// `x = 2^k z` with `z` in `[OFF, 2*OFF)`.
    const OFF: u64 = 0x3fe6_0000_0000_0000;
    /// Bounds of the inputs close to 1.
    const LO: u64 = (1.0f64 - 0.0625).to_bits();
    const HI: u64 = (1.0f64 + 0.064697265625).to_bits();
    /// `0x1p27`.
    const P27: f64 = 134217728.0;
    let b = &LOG_POLY1;
    let a = &LOG_POLY;

    let mut ix = x.to_bits();
    let top = (ix >> 48) as u32;
    if ix.wrapping_sub(LO) < HI - LO {
        // Handle close to 1.0 inputs separately. Fix the sign of zero with downward rounding when x == 1.
        if ix == ONE_BITS {
            return 0.0;
        }
        let r = x - 1.0;
        let r2 = r * r;
        let r3 = r * r2;
        let mut y = r3
            * (b[1]
                + r * b[2]
                + r2 * b[3]
                + r3 * (b[4]
                    + r * b[5]
                    + r2 * b[6]
                    + r3 * (b[7] + r * b[8] + r2 * b[9] + r3 * b[10])));
        // Worst-case error is around 0.507 ULP.
        let w = r * P27;
        let rhi = r + w - w;
        let rlo = r - rhi;
        let w = rhi * rhi * b[0]; // b[0] == -0.5
        let hi = r + w;
        let mut lo = r - hi + w;
        lo += b[0] * rlo * (rhi + r);
        y += lo;
        y += hi;
        return y;
    }
    if top.wrapping_sub(0x0010) >= 0x7ff0 - 0x0010 {
        // x < 0x1p-1022 or inf or nan.
        if ix << 1 == 0 {
            return f64::NEG_INFINITY;
        }
        if ix == INF_BITS {
            return x;
        }
        if (top & 0x8000) != 0 || (top & 0x7ff0) == 0x7ff0 {
            return invalid(x);
        }
        // x is subnormal, normalize it.
        ix = (x * P52).to_bits();
        ix = ix.wrapping_sub(52 << 52);
    }
    // x = 2^k z; where z is in range [OFF,2*OFF) and exact. The range is split into N subintervals. The ith
    // subinterval contains z and c is near its center.
    let tmp = ix.wrapping_sub(OFF);
    let i = ((tmp >> (52 - 7)) % 128) as usize;
    let k = (tmp as i64) >> 52; // arithmetic shift
    let iz = ix.wrapping_sub(tmp & (0xfff << 52));
    let (invc, logc) = LOG_TAB[i];
    let z = f64::from_bits(iz);
    // log(x) = log1p(z/c-1) + log(c) + k*Ln2; r ~= z/c - 1, |r| < 1/(2*N).
    // Rounding error: 0x1p-55/N + 0x1p-66.
    let (chi, clo) = LOG_TAB2[i];
    let r = (z - chi - clo) * invc;
    let kd = k as f64;
    // hi + lo = r + log(c) + k*Ln2.
    let w = kd * LOG_LN2_HI + logc;
    let hi = w + r;
    let lo = w - hi + r + kd * LOG_LN2_LO;
    // log(x) = lo + (log1p(r) - r) + hi.
    let r2 = r * r; // rounding error: 0x1p-54/N^2
    lo + r2 * a[0] + r * r2 * (a[1] + r * a[2] + r2 * (a[3] + r * a[4])) + hi
}

/// Returns 0 if not an integer, 1 if an odd integer, 2 if an even integer (`iy` is the bit representation
/// of a finite nonzero `y`).
#[inline(always)]
fn checkint(iy: u64) -> u32 {
    let e = (iy >> 52 & 0x7ff) as u32;
    if e < 0x3ff {
        return 0;
    }
    if e > 0x3ff + 52 {
        return 2;
    }
    if iy & ((1u64 << (0x3ff + 52 - e)) - 1) != 0 {
        return 0;
    }
    if iy & (1u64 << (0x3ff + 52 - e)) != 0 {
        return 1;
    }
    2
}

/// True if `i` is the bit representation of 0, infinity or NaN.
#[inline(always)]
fn zeroinfnan(i: u64) -> bool {
    (i << 1).wrapping_sub(1) >= (INF_BITS << 1) - 1
}

/// True if `x` is a signaling NaN (IEEE 754-2008 convention).
#[inline(always)]
fn issignaling(x: f64) -> bool {
    let ix = x.to_bits();
    (ix ^ 0x0008_0000_0000_0000) << 1 > 0x7ff8_0000_0000_0000 << 1
}

/// `log(x)` as `hi + lo`, where `hi` is the rounded result and `lo` has about 15 bits more precision. `ix`
/// is the bit representation of `x`, normalized in the subnormal range using the sign bit for the exponent.
#[inline(always)]
fn pow_log_inline(ix: u64) -> (f64, f64) {
    /// `x = 2^k z` with `z` in `[OFF, 2*OFF)`.
    const OFF: u64 = 0x3fe6_9555_0000_0000;
    let a = &POW_POLY;

    // x = 2^k z; where z is in range [OFF,2*OFF) and exact. The range is split into N subintervals. The ith
    // subinterval contains z and c is near its center.
    let tmp = ix.wrapping_sub(OFF);
    let i = ((tmp >> (52 - 7)) % 128) as usize;
    let k = (tmp as i64) >> 52; // arithmetic shift
    let iz = ix.wrapping_sub(tmp & (0xfff << 52));
    let z = f64::from_bits(iz);
    let kd = k as f64;

    // log(x) = k*Ln2 + log(c) + log1p(z/c-1).
    let (invc, logc, logctail) = POW_LOG_TAB[i];

    // Note: 1/c is j/N or j/N/2 where j is an integer in [N,2N) and |z/c - 1| < 1/N, so r = z/c - 1 is
    // exactly representable. Split z such that rhi, rlo and rhi*rhi are exact and |rlo| <= |r|.
    let zhi = f64::from_bits(iz.wrapping_add(1 << 31) & (u64::MAX << 32));
    let zlo = z - zhi;
    let rhi = zhi * invc - 1.0;
    let rlo = zlo * invc;
    let r = rhi + rlo;

    // k*Ln2 + log(c) + r.
    let t1 = kd * POW_LN2_HI + logc;
    let t2 = t1 + r;
    let lo1 = kd * POW_LN2_LO + logctail;
    let lo2 = t1 - t2 + r;

    // Evaluation is optimized assuming superscalar pipelined execution.
    let ar = a[0] * r; // a[0] == -0.5
    let ar2 = r * ar;
    let ar3 = r * ar2;
    // k*Ln2 + log(c) + r + A[0]*r*r.
    let arhi = a[0] * rhi;
    let arhi2 = rhi * arhi;
    let hi = t2 + arhi2;
    let lo3 = rlo * (ar + arhi);
    let lo4 = t2 - hi + arhi2;
    // p = log1p(r) - r - A[0]*r*r.
    let p = ar3 * (a[1] + r * a[2] + ar2 * (a[3] + r * a[4] + ar2 * (a[5] + r * a[6])));
    let lo = lo1 + lo2 + lo3 + lo4 + p;
    let y = hi + lo;
    let tail = hi - y + lo;
    (y, tail)
}

/// `x^y`.
pub fn pow(x: f64, y: f64) -> f64 {
    let mut sign_bias = 0;
    let mut ix = x.to_bits();
    let iy = y.to_bits();
    let mut topx = top12(x);
    let topy = top12(y);
    if topx.wrapping_sub(0x001) >= 0x7ff - 0x001
        || (topy & 0x7ff).wrapping_sub(0x3be) >= 0x43e - 0x3be
    {
        // Note: if |y| > 1075 * ln2 * 2^53 ~= 0x1.749p62 then pow(x,y) = inf/0 and if |y| < 2^-54 / 1075 ~=
        // 0x1.e7b6p-65 then pow(x,y) = +-1.
        // Special cases: (x < 0x1p-126 or inf or nan) or (|y| < 0x1p-65 or |y| >= 0x1p63 or nan).
        if zeroinfnan(iy) {
            if iy << 1 == 0 {
                return if issignaling(x) { x + y } else { 1.0 };
            }
            if ix == ONE_BITS {
                return if issignaling(y) { x + y } else { 1.0 };
            }
            if ix << 1 > INF_BITS << 1 || iy << 1 > INF_BITS << 1 {
                return x + y;
            }
            if ix << 1 == ONE_BITS << 1 {
                return 1.0;
            }
            if (ix << 1 < ONE_BITS << 1) == (iy >> 63 == 0) {
                return 0.0; // |x|<1 && y==inf or |x|>1 && y==-inf
            }
            return y * y;
        }
        if zeroinfnan(ix) {
            let mut x2 = x * x;
            if ix >> 63 != 0 && checkint(iy) == 1 {
                x2 = -x2;
            }
            return if iy >> 63 != 0 { 1.0 / x2 } else { x2 };
        }
        // Here x and y are non-zero finite.
        if ix >> 63 != 0 {
            // Finite x < 0.
            let yint = checkint(iy);
            if yint == 0 {
                return invalid(x);
            }
            if yint == 1 {
                sign_bias = SIGN_BIAS;
            }
            ix &= !SIGN_MASK;
            topx &= 0x7ff;
        }
        if (topy & 0x7ff).wrapping_sub(0x3be) >= 0x43e - 0x3be {
            // Note: sign_bias == 0 here because y is not odd.
            if ix == ONE_BITS {
                return 1.0;
            }
            if (topy & 0x7ff) < 0x3be {
                // |y| < 2^-65, x^y ~= 1 + y*log(x).
                return if ix > ONE_BITS { 1.0 + y } else { 1.0 - y };
            }
            return if (ix > ONE_BITS) == (topy < 0x800) {
                oflow(0)
            } else {
                uflow(0)
            };
        }
        if topx == 0 {
            // Normalize subnormal x so the exponent becomes negative.
            ix = (x * P52).to_bits();
            ix &= !SIGN_MASK;
            ix = ix.wrapping_sub(52 << 52);
        }
    }

    let (hi, lo) = pow_log_inline(ix);
    let yhi = f64::from_bits(iy & (u64::MAX << 27));
    let ylo = y - yhi;
    let lhi = f64::from_bits(hi.to_bits() & (u64::MAX << 27));
    let llo = hi - lhi + lo;
    let ehi = yhi * lhi;
    let elo = ylo * lhi + y * llo; // |elo| < |ehi| * 2^-25
    exp_inline::<true>(ehi, elo, sign_bias)
}
