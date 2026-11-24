//! The basis of a polynomial chaos expansion: its multi-indices, its design matrix at the
//! points of a sample and the surrogate an expansion is.
//!
//! A polynomial chaos expansion writes an output of a model as a series of polynomials of its
//! inputs which are orthonormal under the distribution of the inputs. sbmlsim fits one to the
//! outputs of a scan (`sbmlsim.sensitivity.chaos`) in the coordinates of the unit cube of its
//! design, so the basis is the one of the uniform distribution of the unit cube: the products
//! of the shifted Legendre polynomials normalized to one, `sqrt(2k + 1) P_k(2u - 1)`.
//!
//! This crate is the part of the expansion which is regular and cheap per entry: the
//! multi-indices of a truncated basis ([`expansion`]), the design matrix of a basis at the
//! points of a sample ([`Basis::design_matrix`]) and the values of an expansion at any points,
//! its surrogate ([`Basis::predict`]). The least squares of the fit stay with sbmlsim, whose
//! decomposition is a blocked, threaded QR of LAPACK (#86).
//!
//! The multi-indices are those of sbmlsim in its order, and a value of the design matrix is
//! the product of the same factors in the same order, so the design matrix is bitwise the one
//! of `sbmlsim.sensitivity.chaos.design_matrix`. A term reads few of the inputs (at most
//! `max_interaction` of them), and its product skips the factors of the inputs it does not
//! read, which are exactly 1, so a basis of many inputs costs per term what its own inputs
//! cost.

use std::ops::Range;

/// The largest basis [`expansion`] builds, a guard against a degree of a high dimension which
/// no number of points supports; the `MAX_TERMS` of sbmlsim.
pub const MAX_TERMS: usize = 20000;

/// The relative slack of the hyperbolic truncation, so that a term whose norm is the degree
/// in exact arithmetic is kept whatever the rounding of its sum of powers.
const NORM_SLACK: f64 = 1e-12;

/// The fewest points a thread of [`Basis::design_matrix`] and [`Basis::predict`] evaluates,
/// below which a thread costs more than the points it would take.
const POINTS_PER_THREAD: usize = 64;

/// A basis which cannot be built, or points or coefficients which do not fit it. The
/// messages are the ones of sbmlsim.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ChaosError {
    /// A basis of no input.
    #[error("The basis needs at least one input, not {0}.")]
    Inputs(usize),
    /// An exponent of the hyperbolic truncation outside of `(0, 1]`.
    #[error("The hyperbolic truncation q must be in (0, 1], not {0}.")]
    Truncation(f64),
    /// A cap on the interaction of no input.
    #[error("The interaction order must be at least 1, not {0}.")]
    Interaction(usize),
    /// A basis of more than [`MAX_TERMS`] terms.
    #[error(
        "The basis of the degree {degree} over {inputs} inputs has more than {MAX_TERMS} \
         terms; lower the degree, lower q or cap max_interaction."
    )]
    Terms {
        /// The degree of the basis.
        degree: u32,
        /// Its inputs.
        inputs: usize,
    },
    /// Multi-indices, points or coefficients whose shapes do not fit each other.
    #[error("{0}")]
    Shape(String),
}

/// The terms of a polynomial chaos expansion over a number of inputs.
///
/// A term is the product of the polynomials of the inputs it reads, each of its own degree:
/// its multi-index. It is kept sparse, as the pairs of an input and a degree of the inputs it
/// reads, in the order of the inputs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Basis {
    inputs: usize,
    /// Where the factors of a term start in `factors`, one more than the terms.
    starts: Vec<usize>,
    /// The input and the degree of every factor of every term.
    factors: Vec<(u32, u32)>,
    /// The largest degree of a factor, 0 for a basis of the constant term alone.
    max_degree: u32,
}

/// The multi-indices of a truncated basis, ordered by their total degree and then by the
/// inputs they read, as `itertools.combinations_with_replacement` enumerates them, which is
/// the order of sbmlsim; the first term is the constant one.
///
/// A term is kept when it reads at most `max_interaction` inputs and its hyperbolic norm
/// `(sum alpha_i^q)^(1/q)` is at most `degree`, so `q = 1` keeps every term of the degree
/// (Blatman and Sudret 2010).
///
/// # Errors
///
/// For no input, a `q` outside of `(0, 1]`, a `max_interaction` of 0 and a basis of more than
/// [`MAX_TERMS`] terms.
pub fn expansion(
    inputs: usize,
    degree: u32,
    q: f64,
    max_interaction: Option<usize>,
) -> Result<Basis, ChaosError> {
    if inputs < 1 {
        return Err(ChaosError::Inputs(inputs));
    }
    // a NaN is none of the interval either
    if !(q > 0.0 && q <= 1.0) {
        return Err(ChaosError::Truncation(q));
    }
    if max_interaction == Some(0) {
        return Err(ChaosError::Interaction(0));
    }
    let mut walk = Walk {
        basis: Basis {
            inputs,
            starts: vec![0],
            factors: Vec::new(),
            max_degree: 0,
        },
        alpha: vec![0; inputs],
        degree,
        q,
        max_interaction: max_interaction.unwrap_or(inputs),
    };
    for total in 0..=degree {
        walk.grow(0, total, 0)?;
    }
    Ok(walk.basis)
}

/// The enumeration of the multi-indices of [`expansion`]: the nondecreasing sequences of the
/// inputs of a total degree in their lexicographic order, which are its combinations with
/// replacement, as the counts `alpha` of every input.
struct Walk {
    basis: Basis,
    alpha: Vec<u32>,
    degree: u32,
    q: f64,
    max_interaction: usize,
}

impl Walk {
    /// Adds the multi-indices whose next `left` inputs are `from` or after it, `read` the
    /// inputs `alpha` reads so far.
    ///
    /// The interaction and the norm of a term only grow with the inputs added to it, so a
    /// prefix which breaks either has no term after it which keeps it.
    fn grow(&mut self, from: usize, left: u32, read: usize) -> Result<(), ChaosError> {
        if left == 0 {
            return self.keep();
        }
        for input in from..self.alpha.len() {
            let read = read + usize::from(self.alpha[input] == 0);
            if read > self.max_interaction {
                continue;
            }
            self.alpha[input] += 1;
            let kept = self.q == 1.0 || self.norm_kept();
            let grown = if kept {
                self.grow(input, left - 1, read)
            } else {
                Ok(())
            };
            self.alpha[input] -= 1;
            grown?;
        }
        Ok(())
    }

    /// Whether the hyperbolic norm of `alpha` is at most the degree.
    fn norm_kept(&self) -> bool {
        let sum: f64 = self
            .alpha
            .iter()
            .filter(|&&a| a > 0)
            .map(|&a| f64::from(a).powf(self.q))
            .sum();
        sum.powf(1.0 / self.q) <= f64::from(self.degree) * (1.0 + NORM_SLACK)
    }

    /// Adds `alpha` to the basis.
    fn keep(&mut self) -> Result<(), ChaosError> {
        let basis = &mut self.basis;
        if basis.len() == MAX_TERMS {
            return Err(ChaosError::Terms {
                degree: self.degree,
                inputs: basis.inputs,
            });
        }
        for (input, &a) in self.alpha.iter().enumerate() {
            if a > 0 {
                basis.factors.push((input as u32, a));
                basis.max_degree = basis.max_degree.max(a);
            }
        }
        basis.starts.push(basis.factors.len());
        Ok(())
    }
}

impl Basis {
    /// The basis of dense multi-indices over `inputs` inputs, a row of `inputs` degrees per
    /// term, as [`Basis::multi_indices`] gives them.
    ///
    /// # Errors
    ///
    /// For no input and for multi-indices which are no whole number of rows.
    pub fn from_multi_indices(inputs: usize, multi_indices: &[u32]) -> Result<Basis, ChaosError> {
        if inputs < 1 {
            return Err(ChaosError::Inputs(inputs));
        }
        if !multi_indices.len().is_multiple_of(inputs) {
            return Err(ChaosError::Shape(format!(
                "The multi-indices of {} values are no rows of {inputs} inputs.",
                multi_indices.len()
            )));
        }
        let mut basis = Basis {
            inputs,
            starts: vec![0],
            factors: Vec::new(),
            max_degree: 0,
        };
        for row in multi_indices.chunks(inputs) {
            for (input, &a) in row.iter().enumerate() {
                if a > 0 {
                    basis.factors.push((input as u32, a));
                    basis.max_degree = basis.max_degree.max(a);
                }
            }
            basis.starts.push(basis.factors.len());
        }
        Ok(basis)
    }

    /// The inputs of the basis.
    pub fn inputs(&self) -> usize {
        self.inputs
    }

    /// The terms of the basis.
    pub fn len(&self) -> usize {
        self.starts.len() - 1
    }

    /// Whether the basis has no term.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The input and the degree of every input a term reads, in the order of the inputs.
    pub fn term(&self, term: usize) -> &[(u32, u32)] {
        &self.factors[self.starts[term]..self.starts[term + 1]]
    }

    /// The multi-indices of the basis, a row of `inputs` degrees per term.
    pub fn multi_indices(&self) -> Vec<u32> {
        let mut dense = vec![0; self.len() * self.inputs];
        for (term, row) in dense.chunks_mut(self.inputs).enumerate() {
            for &(input, degree) in self.term(term) {
                row[input as usize] = degree;
            }
        }
        dense
    }

    /// The design matrix of the basis at `points`, a row of `inputs` coordinates of the unit
    /// cube per point: a row per point and a column per term, row major.
    ///
    /// The points are spread over at most `threads` threads, and a value does not depend on
    /// them: it is bitwise the one of `sbmlsim.sensitivity.chaos.design_matrix`.
    ///
    /// # Errors
    ///
    /// For points which are no whole number of rows of the inputs of the basis.
    pub fn design_matrix(&self, points: &[f64], threads: usize) -> Result<Vec<f64>, ChaosError> {
        let n = self.points(points)?;
        let terms = self.len();
        Ok(self.rows(n, terms, threads, |rows, out| {
            let mut table = self.table();
            for (point, row) in rows.zip(out.chunks_mut(terms.max(1))) {
                self.evaluate(
                    &points[point * self.inputs..][..self.inputs],
                    &mut table,
                    row,
                );
            }
        }))
    }

    /// The values of the expansion of the coefficients at `points`, a row of `inputs`
    /// coordinates of the unit cube per point: a row per point and a column per element, row
    /// major, for the coefficients of a row per term and a column per element.
    ///
    /// It is the product of the design matrix of the points and the coefficients, without the
    /// design matrix: a point evaluates its row of the basis and adds up its terms, so a scan
    /// of a surrogate of a million points holds no more than its result. The points are spread
    /// over at most `threads` threads, and a value does not depend on them.
    ///
    /// # Errors
    ///
    /// For points which are no whole number of rows of the inputs of the basis and for
    /// coefficients which are no whole number of rows of `elements` per term.
    pub fn predict(
        &self,
        points: &[f64],
        coefficients: &[f64],
        elements: usize,
        threads: usize,
    ) -> Result<Vec<f64>, ChaosError> {
        let n = self.points(points)?;
        let terms = self.len();
        if coefficients.len() != terms * elements {
            return Err(ChaosError::Shape(format!(
                "The basis has {terms} terms, the coefficients of {elements} elements have {} \
                 values, not {}.",
                coefficients.len(),
                terms * elements
            )));
        }
        Ok(self.rows(n, elements, threads, |rows, out| {
            let mut table = self.table();
            let mut basis = vec![0.0; terms];
            for (point, values) in rows.zip(out.chunks_mut(elements.max(1))) {
                self.evaluate(
                    &points[point * self.inputs..][..self.inputs],
                    &mut table,
                    &mut basis,
                );
                for (&psi, row) in basis.iter().zip(coefficients.chunks(elements.max(1))) {
                    for (value, &c) in values.iter_mut().zip(row) {
                        *value += psi * c;
                    }
                }
            }
        }))
    }

    /// The number of points of `points`, which are rows of the inputs of the basis.
    fn points(&self, points: &[f64]) -> Result<usize, ChaosError> {
        if !points.len().is_multiple_of(self.inputs) {
            return Err(ChaosError::Shape(format!(
                "The points of {} values are no rows of the {} inputs of the basis.",
                points.len(),
                self.inputs
            )));
        }
        Ok(points.len() / self.inputs)
    }

    /// The table of the polynomials of one point, a row of every degree per input.
    fn table(&self) -> Vec<f64> {
        vec![0.0; self.inputs * (self.max_degree as usize + 1)]
    }

    /// The values of every term of the basis at a point into `row`, with `table` the room of
    /// the polynomials of the point.
    fn evaluate(&self, point: &[f64], table: &mut [f64], row: &mut [f64]) {
        let width = self.max_degree as usize + 1;
        for (&u, values) in point.iter().zip(table.chunks_mut(width)) {
            legendre(u, values);
        }
        for (term, value) in row.iter_mut().enumerate() {
            // the factors of the inputs the term does not read are exactly 1, and the
            // product of sbmlsim multiplies them in the order of the inputs from 1
            *value = self
                .term(term)
                .iter()
                .fold(1.0, |product, &(input, degree)| {
                    product * table[input as usize * width + degree as usize]
                });
        }
    }

    /// A matrix of `n` rows of `width` values, zeros first, whose rows `fill` writes: the rows
    /// are split into one contiguous range per thread of at most `threads`.
    fn rows(
        &self,
        n: usize,
        width: usize,
        threads: usize,
        fill: impl Fn(Range<usize>, &mut [f64]) + Sync,
    ) -> Vec<f64> {
        let mut out = vec![0.0; n * width];
        let threads = threads.min(n.div_ceil(POINTS_PER_THREAD)).max(1);
        if threads == 1 || width == 0 {
            fill(0..n, &mut out);
            return out;
        }
        let per_thread = n.div_ceil(threads);
        std::thread::scope(|scope| {
            for (at, chunk) in out.chunks_mut(per_thread * width).enumerate() {
                let fill = &fill;
                let start = at * per_thread;
                scope.spawn(move || fill(start..start + chunk.len() / width, chunk));
            }
        });
        out
    }
}

/// The orthonormal Legendre polynomials of the unit interval at `u` into `values`, one per
/// degree from 0: `sqrt(2k + 1) P_k(2u - 1)`, from the recurrence of `P_k` in the operations
/// and their order of `sbmlsim.sensitivity.chaos.legendre_values`.
pub fn legendre(u: f64, values: &mut [f64]) {
    let t = 2.0 * u - 1.0;
    if let Some(first) = values.first_mut() {
        *first = 1.0;
    }
    if let Some(second) = values.get_mut(1) {
        *second = t;
    }
    for k in 1..values.len().saturating_sub(1) {
        let k_f = k as f64;
        values[k + 1] = ((2.0 * k_f + 1.0) * t * values[k] - k_f * values[k - 1]) / (k_f + 1.0);
    }
    for (k, value) in values.iter_mut().enumerate() {
        *value *= (2.0 * k as f64 + 1.0).sqrt();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The number of terms of the full basis of a degree, `(p + d)! / (p! d!)`.
    fn full(d: u64, p: u64) -> usize {
        ((1..=d).fold(1u64, |acc, i| acc * (p + i) / i)) as usize
    }

    #[test]
    fn the_full_basis_has_every_term_of_its_degree() {
        for (d, p) in [(1, 0), (1, 5), (3, 2), (5, 4), (22, 2)] {
            let basis = expansion(d, p, 1.0, None).unwrap();
            assert_eq!(basis.len(), full(d as u64, u64::from(p)), "d {d}, p {p}");
        }
    }

    #[test]
    fn the_terms_are_ordered_by_degree_and_then_by_their_inputs() {
        let basis = expansion(3, 2, 1.0, None).unwrap();
        // combinations_with_replacement(range(3), k) for k = 0, 1, 2, as counts
        let expected: [[u32; 3]; 10] = [
            [0, 0, 0],
            [1, 0, 0],
            [0, 1, 0],
            [0, 0, 1],
            [2, 0, 0],
            [1, 1, 0],
            [1, 0, 1],
            [0, 2, 0],
            [0, 1, 1],
            [0, 0, 2],
        ];
        assert_eq!(basis.multi_indices(), expected.concat());
    }

    #[test]
    fn the_truncations_drop_the_terms_of_many_inputs() {
        // q = 0.5 keeps (2, 0) (norm 2) and drops (1, 1) (norm 4) of the degree 2
        let basis = expansion(2, 2, 0.5, None).unwrap();
        assert_eq!(basis.multi_indices(), [0, 0, 1, 0, 0, 1, 2, 0, 0, 2]);
        // the additive basis: no term reads two inputs
        let additive = expansion(4, 3, 1.0, Some(1)).unwrap();
        assert_eq!(additive.len(), 1 + 4 * 3);
        assert!((0..additive.len()).all(|term| additive.term(term).len() <= 1));
        let pairs = expansion(5, 4, 1.0, Some(2)).unwrap();
        assert!((0..pairs.len()).all(|term| pairs.term(term).len() <= 2));
        assert!((0..pairs.len()).any(|term| pairs.term(term).len() == 2));
    }

    #[test]
    fn a_basis_out_of_range_is_refused() {
        assert_eq!(expansion(0, 2, 1.0, None), Err(ChaosError::Inputs(0)));
        assert_eq!(expansion(2, 2, 0.0, None), Err(ChaosError::Truncation(0.0)));
        assert_eq!(expansion(2, 2, 1.5, None), Err(ChaosError::Truncation(1.5)));
        assert!(matches!(
            expansion(2, 2, f64::NAN, None),
            Err(ChaosError::Truncation(_))
        ));
        assert_eq!(
            expansion(2, 2, 1.0, Some(0)),
            Err(ChaosError::Interaction(0))
        );
        // the degree 5 over 22 inputs has 80730 terms
        assert_eq!(
            expansion(22, 5, 1.0, None),
            Err(ChaosError::Terms {
                degree: 5,
                inputs: 22
            })
        );
    }

    #[test]
    fn the_multi_indices_give_their_basis_back() {
        let basis = expansion(6, 3, 0.75, Some(2)).unwrap();
        let dense = basis.multi_indices();
        assert_eq!(Basis::from_multi_indices(6, &dense).unwrap(), basis);
        assert!(matches!(
            Basis::from_multi_indices(6, &dense[1..]),
            Err(ChaosError::Shape(_))
        ));
    }

    #[test]
    fn the_polynomials_are_orthonormal_on_the_unit_interval() {
        // Gauss-Legendre of 8 nodes integrates a polynomial of the degree 15 exactly
        let nodes = [
            (0.183_434_642_495_649_8, 0.362_683_783_378_362),
            (0.525_532_409_916_329, 0.313_706_645_877_887_3),
            (0.796_666_477_413_626_7, 0.222_381_034_453_374_5),
            (0.960_289_856_497_536_3, 0.101_228_536_290_376_26),
        ];
        let mut gram = [[0.0; 6]; 6];
        let mut values = [0.0; 6];
        for (x, w) in nodes {
            for x in [x, -x] {
                legendre((x + 1.0) / 2.0, &mut values);
                for i in 0..6 {
                    for j in 0..6 {
                        gram[i][j] += w / 2.0 * values[i] * values[j];
                    }
                }
            }
        }
        for (i, row) in gram.iter().enumerate() {
            for (j, &g) in row.iter().enumerate() {
                let expected = if i == j { 1.0 } else { 0.0 };
                assert!((g - expected).abs() < 1e-12, "<P{i}, P{j}> = {g}");
            }
        }
    }

    /// Points of the unit cube which are no lattice.
    fn points(n: usize, d: usize) -> Vec<f64> {
        (0..n * d)
            .map(|k| ((k as f64 + 0.5) * 0.618_033_988_749_894_9).fract())
            .collect()
    }

    #[test]
    fn a_value_of_the_design_matrix_is_the_product_of_its_polynomials() {
        let basis = expansion(3, 3, 1.0, None).unwrap();
        let u = points(5, 3);
        let x = basis.design_matrix(&u, 1).unwrap();
        let dense = basis.multi_indices();
        let mut values = [0.0; 4];
        for point in 0..5 {
            for term in 0..basis.len() {
                let mut product = 1.0;
                for input in 0..3 {
                    legendre(u[point * 3 + input], &mut values);
                    product *= values[dense[term * 3 + input] as usize];
                }
                assert_eq!(x[point * basis.len() + term].to_bits(), product.to_bits());
            }
        }
    }

    #[test]
    fn the_threads_do_not_change_a_bit() {
        let basis = expansion(8, 3, 0.75, Some(2)).unwrap();
        let u = points(1000, 8);
        let coefficients: Vec<f64> = (0..basis.len() * 3).map(|k| 1.0 / (k + 1) as f64).collect();
        let one = basis.design_matrix(&u, 1).unwrap();
        let one_predicted = basis.predict(&u, &coefficients, 3, 1).unwrap();
        for threads in [2, 3, 7, 64] {
            assert_eq!(basis.design_matrix(&u, threads).unwrap(), one);
            assert_eq!(
                basis.predict(&u, &coefficients, 3, threads).unwrap(),
                one_predicted
            );
        }
    }

    #[test]
    fn the_prediction_is_the_design_matrix_times_the_coefficients() {
        let basis = expansion(4, 3, 1.0, None).unwrap();
        let u = points(50, 4);
        let elements = 2;
        let coefficients: Vec<f64> = (0..basis.len() * elements)
            .map(|k| ((k * 7 % 11) as f64 - 5.0) / 3.0)
            .collect();
        let x = basis.design_matrix(&u, 1).unwrap();
        let predicted = basis.predict(&u, &coefficients, elements, 1).unwrap();
        for point in 0..50 {
            for element in 0..elements {
                let expected: f64 = (0..basis.len())
                    .map(|term| {
                        x[point * basis.len() + term] * coefficients[term * elements + element]
                    })
                    .sum();
                let value = predicted[point * elements + element];
                assert!((value - expected).abs() <= 1e-12 * expected.abs().max(1.0));
            }
        }
    }

    #[test]
    fn the_mean_of_an_expansion_is_its_constant_term() {
        // the polynomials of a degree above 0 have the mean 0 under the uniform distribution,
        // so the midpoint rule over a fine grid of one input gives the constant term back
        let basis = expansion(1, 4, 1.0, None).unwrap();
        let n = 20000;
        let u: Vec<f64> = (0..n).map(|k| (k as f64 + 0.5) / n as f64).collect();
        let predicted = basis
            .predict(&u, &[0.25, 1.0, -2.0, 0.5, 3.0], 1, 4)
            .unwrap();
        let mean = predicted.iter().sum::<f64>() / n as f64;
        assert!((mean - 0.25).abs() < 1e-6, "{mean}");
    }

    #[test]
    fn shapes_which_do_not_fit_are_refused() {
        let basis = expansion(3, 2, 1.0, None).unwrap();
        assert!(matches!(
            basis.design_matrix(&[0.5; 4], 1),
            Err(ChaosError::Shape(_))
        ));
        assert!(matches!(
            basis.predict(&[0.5; 3], &[1.0; 3], 2, 1),
            Err(ChaosError::Shape(_))
        ));
        // no point and no element are shapes as well
        assert!(basis.design_matrix(&[], 4).unwrap().is_empty());
        assert!(basis.predict(&[0.5; 6], &[], 0, 4).unwrap().is_empty());
    }
}
