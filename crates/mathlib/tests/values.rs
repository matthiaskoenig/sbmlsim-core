use sbmlsim_mathlib::f;
use std::collections::BTreeSet;
use std::process::Command;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }
    fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.next()
    }
}

fn ulps(a: f64, b: f64) -> u64 {
    if a == b || (a.is_nan() && b.is_nan()) {
        return 0;
    }
    if a.is_nan() || b.is_nan() || a.is_infinite() != b.is_infinite() {
        return u64::MAX;
    }
    let key = |x: f64| {
        let i = x.to_bits() as i64;
        if i < 0 { i64::MIN - i } else { i }
    };
    key(a).abs_diff(key(b))
}

fn check1(name: &str, ours: fn(f64) -> f64, std: fn(f64) -> f64, lo: f64, hi: f64) {
    check1_ulp(name, ours, std, lo, hi, 1);
}

fn check1_ulp(
    name: &str,
    ours: fn(f64) -> f64,
    std: fn(f64) -> f64,
    lo: f64,
    hi: f64,
    max_ulp: u64,
) {
    let mut rng = Rng(0x9E3779B97F4A7C15);
    for _ in 0..10_000 {
        let x = rng.range(lo, hi);
        let (a, b) = (ours(x), std(x));
        assert!(
            ulps(a, b) <= max_ulp,
            "{name}({x:e}): {a:e} vs {b:e}, {} ulp",
            ulps(a, b)
        );
    }
}

#[test]
fn matches_std_within_one_ulp() {
    check1("exp", f::math_exp, f64::exp, -50.0, 50.0);
    check1("log", f::math_log, f64::ln, 1e-10, 1e10);
    check1("log10", f::math_log10, f64::log10, 1e-10, 1e10);
    check1("sqrt", f::math_sqrt, f64::sqrt, 0.0, 1e10);
    check1("sin", f::math_sin, f64::sin, -100.0, 100.0);
    check1("cos", f::math_cos, f64::cos, -100.0, 100.0);
    check1("tan", f::math_tan, f64::tan, -1.5, 1.5);
    check1("asin", f::math_asin, f64::asin, -1.0, 1.0);
    check1("acos", f::math_acos, f64::acos, -1.0, 1.0);
    check1("atan", f::math_atan, f64::atan, -100.0, 100.0);
    check1("sinh", f::math_sinh, f64::sinh, -20.0, 20.0);
    check1("cosh", f::math_cosh, f64::cosh, -20.0, 20.0);
    // libm and std differ by 2 ulp on some inputs; both are within 1 ulp of the exact value.
    check1_ulp("tanh", f::math_tanh, f64::tanh, -10.0, 10.0, 2);
    check1("asinh", f::math_asinh, f64::asinh, -1e4, 1e4);
    check1("acosh", f::math_acosh, f64::acosh, 1.0, 1e4);
    // std computes atanh as 0.5 * ln_1p(2x / (1 - x)), which loses up to ~200 ulp near |x| = 1; libm is exact
    // (see `atanh_matches_exact_values`), so std is only a sanity bound here.
    check1_ulp("atanh", f::math_atanh, f64::atanh, -0.999, 0.999, 512);
    check1("floor", f::math_floor, f64::floor, -1e6, 1e6);
    check1("ceil", f::math_ceil, f64::ceil, -1e6, 1e6);
    let mut rng = Rng(12345);
    for _ in 0..10_000 {
        let (x, y) = (rng.range(1e-3, 1e3), rng.range(-20.0, 20.0));
        let (a, b) = (f::math_pow(x, y), x.powf(y));
        assert!(ulps(a, b) <= 1, "pow({x:e},{y:e}): {a:e} vs {b:e}");
        let (x, y) = (rng.range(-1e6, 1e6), rng.range(0.1, 1e3));
        assert_eq!(f::math_fmod(x, y), x % y, "fmod({x:e},{y:e})");
    }
}

/// True if `a` and `b` are the same bits, or both NaN.
fn same(a: f64, b: f64) -> bool {
    a.to_bits() == b.to_bits() || (a.is_nan() && b.is_nan())
}

#[test]
fn pow_special_cases_match_std() {
    let tiny = f64::from_bits(1); // smallest subnormal
    let xs = [
        0.0,
        -0.0,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NAN,
        -f64::NAN,
        1.0,
        -1.0,
        0.5,
        -0.5,
        2.0,
        -2.0,
        3.0,
        -3.0,
        1.0 + f64::EPSILON,
        1.0 - f64::EPSILON / 2.0,
        -(1.0 + f64::EPSILON),
        tiny,
        -tiny,
        1e-310,
        -1e-310,
        f64::MIN_POSITIVE,
        -f64::MIN_POSITIVE,
        f64::MAX,
        f64::MIN,
        1e-300,
        1e300,
        -1e300,
        10.0,
        -10.0,
    ];
    let ys = [
        0.0,
        -0.0,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NAN,
        1.0,
        -1.0,
        2.0,
        -2.0,
        3.0,
        -3.0,
        0.5,
        -0.5,
        2.5,
        -2.5,
        1e-20,
        -1e-20,
        2f64.powi(-70),
        -(2f64.powi(-70)),
        2f64.powi(63),
        -(2f64.powi(63)),
        2f64.powi(53) - 1.0, // odd
        2f64.powi(52) + 1.0, // odd
        2f64.powi(53),       // even
        1e300,
        -1e300,
        1023.0,
        1024.0,
        -1074.0,
        -1075.0,
        tiny,
        -tiny,
        f64::MAX,
        f64::MIN,
    ];
    for &x in &xs {
        for &y in &ys {
            let (a, b) = (f::math_pow(x, y), x.powf(y));
            // special values (NaN, infinities, zeros with their sign, exact powers) are the same bits; the
            // others within the error of glibc (0.52 ulp) and of the port (0.54 ulp)
            let exact = !b.is_finite() || b == 0.0 || b.abs() == 1.0 || !a.is_finite() || a == 0.0;
            if exact {
                assert!(same(a, b), "pow({x:e}, {y:e}) = {a:e}, std {b:e}");
            } else {
                assert!(ulps(a, b) <= 1, "pow({x:e}, {y:e}) = {a:e}, std {b:e}");
            }
        }
    }
}

#[test]
fn pow_matches_std_across_ranges() {
    let mut rng = Rng(0x2545F4914F6CDD1D);
    let mut logu = |lo: f64, hi: f64| rng.range(lo.ln(), hi.ln()).exp();
    let mut cases = Vec::new();
    for _ in 0..20_000 {
        let (x, y) = (logu(1e-300, 1e300), logu(1e-3, 5.0));
        cases.push((x, y));
        cases.push((x, -y));
        // near overflow and underflow, and subnormal results
        let x = logu(1e-3, 1e3);
        let t = logu(900.0, 1080.0);
        cases.push((x, t / x.log2()));
        cases.push((x, -t / x.log2()));
        // negative bases with integer exponents
        let (x, y) = (-logu(1e-5, 1e5), logu(1.0, 60.0).round());
        cases.push((x, y));
        cases.push((x, -y));
        // x near 1 with huge exponents
        let x = 1.0 + (logu(1e-15, 1e-8) - 0.5e-8);
        let y = logu(1e8, 1e18);
        cases.push((x, y));
        cases.push((x, -y));
        // subnormal bases
        let x = logu(5e-324, 2e-308);
        cases.push((x, logu(1e-3, 1.0)));
        cases.push((x, -logu(1e-3, 1.0)));
    }
    for (x, y) in cases {
        let (a, b) = (f::math_pow(x, y), x.powf(y));
        assert!(ulps(a, b) <= 1, "pow({x:e}, {y:e}) = {a:e}, std {b:e}");
        if b == 0.0 || !b.is_finite() {
            assert!(same(a, b), "pow({x:e}, {y:e}) = {a:e}, std {b:e}");
        }
    }
}

#[test]
fn exp_log_special_cases_match_std() {
    let tiny = f64::from_bits(1);
    for x in [
        0.0,
        -0.0,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NAN,
        1e-300,
        -1e-300,
        tiny,
        709.78,
        709.79,
        710.0,
        -708.4,
        -745.13,
        -745.14,
        -746.0,
        1024.0,
        -1024.0,
        f64::MAX,
        f64::MIN,
    ] {
        let (a, b) = (f::math_exp(x), x.exp());
        assert!(ulps(a, b) <= 1, "exp({x:e}) = {a:e}, std {b:e}");
        if !b.is_finite() || b == 0.0 || b == 1.0 {
            assert!(same(a, b), "exp({x:e}) = {a:e}, std {b:e}");
        }
    }
    for x in [
        0.0,
        -0.0,
        -1.0,
        -tiny,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NAN,
        1.0,
        tiny,
        1e-310,
        f64::MIN_POSITIVE,
        f64::MAX,
        0.9375,
        1.064697265625,
        1.0 + f64::EPSILON,
        1.0 - f64::EPSILON / 2.0,
    ] {
        let (a, b) = (f::math_log(x), x.ln());
        assert!(ulps(a, b) <= 1, "log({x:e}) = {a:e}, std {b:e}");
        if !b.is_finite() || b == 0.0 {
            assert!(same(a, b), "log({x:e}) = {a:e}, std {b:e}");
        }
    }
}

#[test]
fn exp_log_pow_are_the_arm_port_without_fma() {
    // values of ARM optimized-routines compiled without fma (`HAVE_FAST_FMA=0`, `-ffp-contract=off`), where
    // glibc on x86_64 with fma differs in the last bit; the results must be the same bits on every platform
    let pow_cases: [(f64, f64, f64); 6] = [
        (565.0549497813068, 1.5627591172616029, 19991.933907224455),
        (153.42129004388946, 11.680907057032872, 3.412926091965182e25),
        (
            510.74484552606043,
            -7.909889122912066,
            3.7879714575568175e-22,
        ),
        (
            647.1296956217755,
            -1.7637981073639644,
            1.1014855739433778e-5,
        ),
        (199.71713639166168, 3.63888541065279, 234935033.80471897),
        (596.6088334305709, -3.125839835821548, 2.106874315019019e-9),
    ];
    for (x, y, want) in pow_cases {
        let got = f::math_pow(x, y);
        assert_eq!(
            got.to_bits(),
            want.to_bits(),
            "pow({x:e}, {y:e}) = {got:e}, want {want:e}"
        );
    }
    let exp_cases: [(f64, f64); 3] = [
        (-23.13037816490405, 9.007502651721375e-11),
        (-0.1058063601605852, 0.8995988293840191),
        (8.823963524003965, 6795.144037986818),
    ];
    for (x, want) in exp_cases {
        let got = f::math_exp(x);
        assert_eq!(
            got.to_bits(),
            want.to_bits(),
            "exp({x:e}) = {got:e}, want {want:e}"
        );
    }
    let log_cases: [(f64, f64); 3] = [
        (1.0545194869556107, 0.053085200572116936),
        (0.9630579463045228, -0.03764169630010693),
        (0.9487063981367649, -0.05265590849646057),
    ];
    for (x, want) in log_cases {
        let got = f::math_log(x);
        assert_eq!(
            got.to_bits(),
            want.to_bits(),
            "log({x:e}) = {got:e}, want {want:e}"
        );
    }
}

#[test]
fn atanh_matches_exact_values() {
    // correctly rounded values computed with 40 digit arithmetic
    for (x, y) in [
        (-0.9989086022321781, -3.7564486905926913),
        (-0.9975293200205587, -3.3475864739087458),
        (0.5, 0.5493061443340549),
        (0.999, 3.8002011672501994),
        (-0.123456789, -0.12408981359692224),
    ] {
        assert!(ulps(f::math_atanh(x), y) <= 1, "atanh({x:e})");
    }
}

#[test]
fn edge_values() {
    assert_eq!(f::math_log(0.0), f64::NEG_INFINITY);
    assert!(f::math_sqrt(-1.0).is_nan());
    assert_eq!(f::math_pow(0.0, 0.0), 1.0);
    assert_eq!(f::math_factorial(5.0), 120.0);
    assert_eq!(f::math_factorial(171.0), f64::INFINITY);
    assert!(f::math_factorial(f64::NAN).is_nan());
}

#[test]
fn factorial_matches_roadrunner() {
    // values of libroadrunner 2.11.1 (rr_factoriald), compared bitwise
    let cases: [(f64, f64); 16] = [
        (5.0, 120.0),
        (2.5, 1.875),
        (2.9, 4.958999999999999),
        (7.3, 3102.5166272099973),
        (13.7, 30201993205.66932),
        (0.5, 0.5),
        (1e-300, 1e-300),
        (0.0, 1.0),
        (-0.0, 1.0),
        (-1.0, 1.0),
        (-2.5, 1.0),
        (170.0, 7.257415615307994e306),
        (171.0, f64::INFINITY),
        (f64::INFINITY, 1.0),
        (f64::NEG_INFINITY, 1.0),
        (1e300, f64::INFINITY),
    ];
    for (x, want) in cases {
        let got = f::math_factorial(x);
        assert_eq!(
            got.to_bits(),
            want.to_bits(),
            "{x:e}! = {got:e}, want {want:e}"
        );
    }
}

const NAMES: [&str; 21] = [
    "math_exp",
    "math_log",
    "math_log10",
    "math_pow",
    "math_sqrt",
    "math_sin",
    "math_cos",
    "math_tan",
    "math_asin",
    "math_acos",
    "math_atan",
    "math_sinh",
    "math_cosh",
    "math_tanh",
    "math_asinh",
    "math_acosh",
    "math_atanh",
    "math_floor",
    "math_ceil",
    "math_factorial",
    "math_fmod",
];

#[test]
fn wasm_has_no_imports() {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let dir = env!("CARGO_MANIFEST_DIR");
    let target = format!("{dir}/../../target/mathlib-wasm-test");
    let status = Command::new(cargo)
        .args([
            "build",
            "-p",
            "sbmlsim-mathlib",
            "--target",
            "wasm32-unknown-unknown",
            "--release",
            "--target-dir",
            &target,
        ])
        .status()
        .unwrap();
    assert!(status.success());
    let bytes = std::fs::read(format!(
        "{target}/wasm32-unknown-unknown/release/sbmlsim_mathlib.wasm"
    ))
    .unwrap();
    let mut imports = 0;
    let mut funcs = BTreeSet::new();
    for payload in wasmparser::Parser::new(0).parse_all(&bytes) {
        match payload.unwrap() {
            wasmparser::Payload::ImportSection(s) => imports += s.count(),
            wasmparser::Payload::ExportSection(s) => {
                for e in s {
                    let e = e.unwrap();
                    if e.kind == wasmparser::ExternalKind::Func {
                        funcs.insert(e.name.to_string());
                    }
                }
            }
            _ => {}
        }
    }
    assert_eq!(imports, 0, "mathlib.wasm must import nothing");
    let want: BTreeSet<String> = NAMES.iter().map(|s| s.to_string()).collect();
    assert_eq!(funcs, want);
}
