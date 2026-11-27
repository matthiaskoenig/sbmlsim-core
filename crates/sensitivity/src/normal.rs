//! The quantile of the standard normal distribution, which scales an interval.

/// The quantile of the standard normal distribution at `p`, the algorithm AS 241 (PPND16) of
/// Wichura 1988, accurate to about 1e-16: `z(conf_level)` of an interval is
/// `normal_quantile(0.5 + conf_level / 2)`, as `scipy.stats.norm.ppf` gives it to SALib.
///
/// Not a number outside of `(0, 1)` and for not a number.
///
/// Wichura, M. J. (1988). Algorithm AS 241: The percentage points of the normal distribution.
/// Journal of the Royal Statistical Society, Series C 37(3):477-484.
#[must_use]
pub fn normal_quantile(p: f64) -> f64 {
    if !(p > 0.0 && p < 1.0) {
        return f64::NAN;
    }
    let q = p - 0.5;
    if q.abs() <= 0.425 {
        let r = 0.180625 - q * q;
        return q
            * (((((((2.509_080_928_730_122_7e3 * r + 3.343_057_558_358_813e4) * r
                + 6.726_577_092_700_87e4)
                * r
                + 4.592_195_393_154_987e4)
                * r
                + 1.373_169_376_550_946e4)
                * r
                + 1.971_590_950_306_551_3e3)
                * r
                + 1.331_416_678_917_843_8e2)
                * r
                + 3.387_132_872_796_366_5)
            / (((((((5.226_495_278_852_545e3 * r + 2.872_908_573_572_194_3e4) * r
                + 3.930_789_580_009_271e4)
                * r
                + 2.121_379_430_158_659_7e4)
                * r
                + 5.394_196_021_424_751e3)
                * r
                + 6.871_870_074_920_579e2)
                * r
                + 4.231_333_070_160_091e1)
                * r
                + 1.0);
    }
    let tail = if q < 0.0 { p } else { 1.0 - p };
    let mut r = libm::sqrt(-libm::log(tail));
    let value = if r <= 5.0 {
        r -= 1.6;
        (((((((7.745_450_142_783_414e-4 * r + 2.272_384_498_926_918_4e-2) * r
            + 2.417_807_251_774_506e-1)
            * r
            + 1.270_458_252_452_368_4)
            * r
            + 3.647_848_324_763_204_5)
            * r
            + 5.769_497_221_460_691)
            * r
            + 4.630_337_846_156_546)
            * r
            + 1.423_437_110_749_683_5)
            / (((((((1.050_750_071_644_416_8e-9 * r + 5.475_938_084_995_345e-4) * r
                + 1.519_866_656_361_645_7e-2)
                * r
                + 1.481_039_764_274_800_8e-1)
                * r
                + 6.897_673_349_851e-1)
                * r
                + 1.676_384_830_183_803_8)
                * r
                + 2.053_191_626_637_759)
                * r
                + 1.0)
    } else {
        r -= 5.0;
        (((((((2.010_334_399_292_288_1e-7 * r + 2.711_555_568_743_487_6e-5) * r
            + 1.242_660_947_388_078_4e-3)
            * r
            + 2.653_218_952_657_612_4e-2)
            * r
            + 2.965_605_718_285_049e-1)
            * r
            + 1.784_826_539_917_291_3)
            * r
            + 5.463_784_911_164_114)
            * r
            + 6.657_904_643_501_103)
            / (((((((2.044_263_103_389_939_7e-15 * r + 1.421_511_758_316_446e-7) * r
                + 1.846_318_317_510_054_8e-5)
                * r
                + 7.868_691_311_456_133e-4)
                * r
                + 1.487_536_129_085_061_5e-2)
                * r
                + 1.369_298_809_227_358e-1)
                * r
                + 5.998_322_065_558_88e-1)
                * r
                + 1.0)
    };
    if q < 0.0 { -value } else { value }
}

#[cfg(test)]
mod tests {
    use super::normal_quantile;

    #[test]
    fn the_quantiles_of_the_intervals_are_those_of_scipy() {
        // scipy.stats.norm.ppf
        for (p, expected) in [
            (0.975, 1.959_963_984_540_054),
            (0.95, 1.644_853_626_951_472_2),
            (0.995, 2.575_829_303_548_900_4),
            (0.75, 0.674_489_750_196_081_7),
            (0.5, 0.0),
            (0.025, -1.959_963_984_540_054_5),
            (1e-12, -7.034_483_825_301_131),
        ] {
            let found = normal_quantile(p);
            assert!(
                (found - expected).abs() <= 4.0 * f64::EPSILON * expected.abs().max(1.0),
                "{p}: {found} != {expected}"
            );
        }
        assert!(normal_quantile(0.0).is_nan());
        assert!(normal_quantile(1.0).is_nan());
        assert!(normal_quantile(f64::NAN).is_nan());
    }
}
