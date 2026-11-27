//! A seed gives the same bits on every target: the designs and the estimators of fixed seeds
//! against the hashes of their bits, which this test checks natively and, in the continuous
//! integration, in wasm (`cargo test -p sbmlsim-sensitivity --target wasm32-wasip1`).

use sbmlsim_sensitivity::{
    Bootstrap, Matrix, MatrixRef, Resamples, fast_indices, fast_sample, morris_effects,
    morris_sample, saltelli, sobol_indices,
};

/// The FNV-1a hash of the bits of the values.
fn hash(values: &[f64]) -> u64 {
    values.iter().fold(0xcbf2_9ce4_8422_2325, |state, value| {
        value
            .to_bits()
            .to_le_bytes()
            .iter()
            .fold(state, |s, &byte| {
                (s ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
            })
    })
}

/// The Ishigami function and a variation of it, a column each.
fn values(points: &Matrix) -> Vec<f64> {
    let pi = std::f64::consts::PI;
    (0..points.rows)
        .flat_map(|i| {
            let x: Vec<f64> = points.row(i).iter().map(|u| u * 2.0 * pi - pi).collect();
            // products rather than `powi`, whose rounding the target may choose
            let (s0, s1, x2) = (libm::sin(x[0]), libm::sin(x[1]), x[2] * x[2]);
            [
                s0 + 7.0 * (s1 * s1) + 0.1 * (x2 * x2) * s0,
                s0 + s1 * s1 + 0.5 * (x2 * x2) * s0,
            ]
        })
        .collect()
}

fn bootstrap(seed: u64) -> Bootstrap<'static> {
    Bootstrap {
        conf_level: 0.95,
        resamples: Resamples::Seeded { count: 50, seed },
    }
}

#[test]
fn a_seed_gives_the_same_bits_everywhere() {
    let sobol = saltelli(3, 128, true, Some(42), 0).unwrap();
    let y = values(&sobol);
    let s = sobol_indices(
        MatrixRef::new(&y, sobol.rows, 2).unwrap(),
        3,
        true,
        &bootstrap(1),
        1,
    )
    .unwrap();
    let fast = fast_sample(3, 257, 4, 42).unwrap();
    let y = values(&fast);
    let f = fast_indices(
        MatrixRef::new(&y, fast.rows, 2).unwrap(),
        3,
        4,
        &bootstrap(2),
        1,
    )
    .unwrap();
    let morris = morris_sample(3, 30, 4, 42).unwrap();
    let y = values(&morris);
    let m = morris_effects(
        morris.view(),
        MatrixRef::new(&y, morris.rows, 2).unwrap(),
        4,
        &bootstrap(3),
        1,
    )
    .unwrap();
    let found = [
        hash(&sobol.data),
        hash(
            &[
                s.s1.data,
                s.s1_conf.data,
                s.st.data,
                s.st_conf.data,
                s.s2.unwrap().data,
                s.s2_conf.unwrap().data,
            ]
            .concat(),
        ),
        hash(&fast.data),
        hash(&[f.s1.data, f.s1_conf.data, f.st.data, f.st_conf.data].concat()),
        hash(&morris.data),
        hash(&[m.mu.data, m.mu_star.data, m.sigma.data, m.mu_star_conf.data].concat()),
    ];
    assert_eq!(
        found,
        [
            0xccc4_75ed_14b0_b649,
            0xd8e5_9e02_7426_b36c,
            0x32c7_6a42_fe2a_a4fa,
            0xa580_d215_c867_cbf8,
            0x54bc_f426_c501_0ac5,
            0x6a56_91ab_4a3b_8df1,
        ],
        "{found:#x?}"
    );
}

#[test]
fn the_threads_do_not_change_a_bit() {
    // 70 elements: the Ishigami function and its variation at 35 shifts of the output
    let widen = |points: &Matrix| -> Vec<f64> {
        values(points)
            .as_chunks::<2>()
            .0
            .iter()
            .flat_map(|&[a, b]| (0..35).flat_map(move |k| [a + k as f64, b * k as f64]))
            .collect()
    };
    let sobol = saltelli(3, 64, true, Some(1), 0).unwrap();
    let y = widen(&sobol);
    let y = MatrixRef::new(&y, sobol.rows, 70).unwrap();
    let fast = fast_sample(3, 129, 4, 1).unwrap();
    let z = widen(&fast);
    let z = MatrixRef::new(&z, fast.rows, 70).unwrap();
    let morris = morris_sample(3, 12, 4, 1).unwrap();
    let w = widen(&morris);
    let w = MatrixRef::new(&w, morris.rows, 70).unwrap();
    // the bits of every index, so a not a number of the lower triangle of S2 equals itself
    let sobol_bits = |threads| {
        let s = sobol_indices(y, 3, true, &bootstrap(4), threads).unwrap();
        hash(
            &[
                s.s1.data,
                s.s1_conf.data,
                s.st.data,
                s.st_conf.data,
                s.s2.unwrap().data,
                s.s2_conf.unwrap().data,
            ]
            .concat(),
        )
    };
    let one = (
        sobol_bits(1),
        fast_indices(z, 3, 4, &bootstrap(4), 1).unwrap(),
        morris_effects(morris.view(), w, 4, &bootstrap(4), 1).unwrap(),
    );
    for threads in [2, 3, 8] {
        assert_eq!(sobol_bits(threads), one.0);
        assert_eq!(
            fast_indices(z, 3, 4, &bootstrap(4), threads).unwrap(),
            one.1
        );
        assert_eq!(
            morris_effects(morris.view(), w, 4, &bootstrap(4), threads).unwrap(),
            one.2
        );
    }
}
