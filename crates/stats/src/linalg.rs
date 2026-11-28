//! The linear algebra of symmetric matrices: the eigendecomposition, the Cholesky factor, the
//! factor of a correlation, the pseudo-inverse and the product `JᵀJ` of a jacobian, as numpy
//! gives them (`numpy.linalg.eigh`, `eigvalsh`, `cholesky`, `pinv`).
//!
//! A matrix is a row-major slice of `k * k` doubles. The eigendecomposition is the cyclic
//! Jacobi method, which takes only additions, multiplications, divisions and square roots, so
//! one matrix gives the same bits on every target; it is accurate to the last digits of the
//! eigenvalues relative to the largest one and fast enough for the few hundred parameters of a
//! fit (a 200 x 200 matrix in about ten sweeps).
//!
//! The eigenvalues are sorted largest first, numpy's are smallest first, and an eigenvector is
//! a column of the matrix of the vectors, normalized, its largest component positive (the
//! first of the largest ones); LAPACK chooses the sign of numpy's by its own rounding.

use crate::{Result, StatsError};

/// The largest number of sweeps of the Jacobi method; a symmetric matrix converges
/// quadratically, in about ten.
const MAX_SWEEPS: usize = 100;

/// The cut of the pseudo-inverse relative to the largest absolute eigenvalue: the default
/// `rcond` of `numpy.linalg.pinv` (numpy 2.5), which `sbmlsim.fit.fisher` calls.
pub const PINV_RCOND: f64 = 1e-15;

/// The largest difference of `a[i, j]` and `a[j, i]` of a symmetric matrix, relative to the
/// largest absolute value, as the check of `sbmlsim.uncertainty.covariance`.
const SYMMETRY_TOLERANCE: f64 = 1e-12;

/// The eigenvalues and eigenvectors of the symmetric `k x k` matrix `a`.
///
/// Returns the eigenvalues largest first and the matrix of the eigenvectors, row-major,
/// column `j` the normalized eigenvector of eigenvalue `j`, its largest component positive.
/// The mean of `a[i, j]` and `a[j, i]` is decomposed.
///
/// # Errors
///
/// A slice which is not of `k * k` values, a value which is not finite, a matrix which is not
/// symmetric (`|a[i, j] - a[j, i]| > 1e-12 max|a|`) and a Jacobi method which does not
/// converge in 100 sweeps.
pub fn eigh(a: &[f64], k: usize) -> Result<(Vec<f64>, Vec<f64>)> {
    let m = symmetric(a, k)?;
    let (values, vectors) = jacobi(m, k, true, MAX_SWEEPS)?;
    let order = descending(&values);
    let mut sorted = vec![0.0; k * k];
    for (column, &j) in order.iter().enumerate() {
        // the sign: the first of the largest components positive
        let mut largest = 0.0_f64;
        for i in 0..k {
            let v = vectors[i * k + j];
            if v.abs() > largest.abs() {
                largest = v;
            }
        }
        let sign = if largest < 0.0 { -1.0 } else { 1.0 };
        for i in 0..k {
            sorted[i * k + column] = sign * vectors[i * k + j];
        }
    }
    Ok((order.iter().map(|&j| values[j]).collect(), sorted))
}

/// The eigenvalues of the symmetric `k x k` matrix `a`, largest first: those of [`eigh`],
/// to the bit, without the eigenvectors.
///
/// # Errors
///
/// As [`eigh`].
pub fn eigvalsh(a: &[f64], k: usize) -> Result<Vec<f64>> {
    let m = symmetric(a, k)?;
    let (values, _) = jacobi(m, k, false, MAX_SWEEPS)?;
    Ok(descending(&values).iter().map(|&j| values[j]).collect())
}

/// The lower Cholesky factor `L` of the positive definite `k x k` matrix `a`, `L Lᵀ = a`,
/// row-major, as `numpy.linalg.cholesky` of the lower triangle of `a`.
///
/// `None` of a matrix which is not positive definite (a pivot not positive), of a value of the
/// lower triangle which is not finite and of a slice which is not of `k * k` values.
pub fn cholesky(a: &[f64], k: usize) -> Option<Vec<f64>> {
    if a.len() != k * k {
        return None;
    }
    let mut l = vec![0.0; k * k];
    for j in 0..k {
        if !a[j * k..=j * k + j].iter().all(|v| v.is_finite()) {
            return None;
        }
        let mut pivot = a[j * k + j];
        for p in 0..j {
            pivot -= l[j * k + p] * l[j * k + p];
        }
        if pivot.is_nan() || pivot <= 0.0 {
            return None;
        }
        let diagonal = libm::sqrt(pivot);
        l[j * k + j] = diagonal;
        for i in j + 1..k {
            let mut sum = a[i * k + j];
            for p in 0..j {
                sum -= l[i * k + p] * l[j * k + p];
            }
            l[i * k + j] = sum / diagonal;
        }
    }
    Some(l)
}

/// A factor `F` of the positive semi-definite `k x k` correlation `r`, `F Fᵀ = r`, as
/// `sbmlsim.uncertainty.draws._factor`: the Cholesky factor, else, of a singular or
/// indefinite matrix, `V sqrt(clip(values, 0))` of its eigendecomposition ([`eigh`], the
/// eigenvalues largest first), the nearest positive semi-definite matrix.
///
/// # Errors
///
/// As [`eigh`]: the matrix is checked before its Cholesky factor.
pub fn factor(r: &[f64], k: usize) -> Result<Vec<f64>> {
    symmetric(r, k)?;
    if let Some(lower) = cholesky(r, k) {
        return Ok(lower);
    }
    let (values, mut vectors) = eigh(r, k)?;
    let roots: Vec<f64> = values.iter().map(|&v| libm::sqrt(v.max(0.0))).collect();
    for row in vectors.chunks_mut(k.max(1)) {
        for (v, root) in row.iter_mut().zip(&roots) {
            *v *= root;
        }
    }
    Ok(vectors)
}

/// The pseudo-inverse of the symmetric `k x k` matrix `a`, as `numpy.linalg.pinv`: the
/// eigenvalues of an absolute value at most [`PINV_RCOND`] times the largest absolute one
/// are taken as zero, the others inverted, `V diag(1/values) Vᵀ`, exactly symmetric.
///
/// # Errors
///
/// As [`eigh`].
pub fn pinv_symmetric(a: &[f64], k: usize) -> Result<Vec<f64>> {
    let (values, vectors) = eigh(a, k)?;
    Ok(pinv_of_eigh(&values, &vectors, k))
}

/// The pseudo-inverse of [`pinv_symmetric`] from the eigendecomposition of [`eigh`] (the
/// eigenvalues and the row-major matrix of the eigenvectors of a `k x k` matrix), for a caller
/// which needs the eigenvalues as well: the same cut, the same bits.
///
/// # Panics
///
/// Not `k` eigenvalues and `k * k` values of the eigenvectors.
pub fn pinv_of_eigh(values: &[f64], vectors: &[f64], k: usize) -> Vec<f64> {
    assert!(
        values.len() == k && vectors.len() == k * k,
        "an eigendecomposition of a {k} x {k} matrix has {k} eigenvalues and {} values of the \
         eigenvectors",
        k * k
    );
    let cut = PINV_RCOND * values.iter().fold(0.0_f64, |m, v| m.max(v.abs()));
    let inverse: Vec<f64> = values
        .iter()
        .map(|&v| if v.abs() > cut { 1.0 / v } else { 0.0 })
        .collect();
    let mut p = vec![0.0; k * k];
    for i in 0..k {
        for l in i..k {
            let mut sum = 0.0;
            for (j, &w) in inverse.iter().enumerate() {
                if w != 0.0 {
                    sum += vectors[i * k + j] * w * vectors[l * k + j];
                }
            }
            p[i * k + l] = sum;
            p[l * k + i] = sum;
        }
    }
    p
}

/// `JᵀJ` of the row-major `n x k` matrix `j` (a jacobian of `n` residuals and `k`
/// parameters), the `k x k` Gram matrix, exactly symmetric; a value which is not a number
/// gives not a number in its row and column, as numpy.
///
/// # Panics
///
/// A slice which is not of `n * k` values.
pub fn gram(j: &[f64], n: usize, k: usize) -> Vec<f64> {
    assert_eq!(
        j.len(),
        n * k,
        "a jacobian of {n} x {k} has {} values",
        n * k
    );
    let mut g = vec![0.0; k * k];
    for row in j.chunks(k.max(1)).take(n) {
        for a in 0..k {
            for b in a..k {
                g[a * k + b] += row[a] * row[b];
            }
        }
    }
    for a in 0..k {
        for b in 0..a {
            g[a * k + b] = g[b * k + a];
        }
    }
    g
}

/// The matrix `a` checked, of `k * k` finite values and symmetric, the mean of `a[i, j]` and
/// `a[j, i]`.
fn symmetric(a: &[f64], k: usize) -> Result<Vec<f64>> {
    if a.len() != k * k {
        return Err(StatsError::value(format!(
            "A matrix of {k} x {k} needs {} values, not {}.",
            k * k,
            a.len()
        )));
    }
    if let Some(index) = a.iter().position(|v| !v.is_finite()) {
        return Err(StatsError::value(format!(
            "The matrix is not finite: a[{}, {}] = {}.",
            index / k,
            index % k,
            a[index]
        )));
    }
    let tolerance = SYMMETRY_TOLERANCE * a.iter().fold(0.0_f64, |m, v| m.max(v.abs()));
    let mut m = a.to_vec();
    for i in 0..k {
        for j in 0..i {
            let (lower, upper) = (a[i * k + j], a[j * k + i]);
            if (lower - upper).abs() > tolerance {
                return Err(StatsError::value(format!(
                    "The matrix is not symmetric: a[{i}, {j}] = {lower:e}, a[{j}, {i}] = {upper:e}."
                )));
            }
            let mean = 0.5 * lower + 0.5 * upper;
            m[i * k + j] = mean;
            m[j * k + i] = mean;
        }
    }
    Ok(m)
}

/// The indices of `values`, the largest value first; equal values in their order.
fn descending(values: &[f64]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..values.len()).collect();
    order.sort_by(|&i, &j| values[j].total_cmp(&values[i]));
    order
}

/// The eigenvalues (unsorted) and, with `with_vectors`, the eigenvectors (columns, row-major)
/// of the symmetric `k x k` matrix `m` (both triangles set), by the cyclic Jacobi method with
/// thresholds (Rutishauser; Numerical Recipes, `jacobi`).
///
/// A sweep rotates every off-diagonal element above a threshold to zero; the first three
/// sweeps skip the elements below a fifth of the mean, the later ones set an element to zero
/// which is negligible against both of its diagonal elements. The diagonal is updated by the
/// sum of the rotations of a sweep, which keeps the small eigenvalues accurate. The matrix is
/// scaled by a power of two to a largest value in [1/2, 1) first, exactly (`scalbn` of every
/// value, which steps a power beyond the range of a double, of a subnormal largest value), so
/// neither the sum of the off-diagonal elements nor the squares of a rotation overflow or
/// underflow; an eigenvalue is scaled back with one rounding, of a subnormal one. An error
/// after `max_sweeps` sweeps without convergence.
fn jacobi(
    mut m: Vec<f64>,
    k: usize,
    with_vectors: bool,
    max_sweeps: usize,
) -> Result<(Vec<f64>, Vec<f64>)> {
    let mut v = if with_vectors {
        let mut v = vec![0.0; k * k];
        for i in 0..k {
            v[i * k + i] = 1.0;
        }
        v
    } else {
        Vec::new()
    };
    let largest = m.iter().fold(0.0_f64, |s, x| s.max(x.abs()));
    if largest == 0.0 {
        return Ok((vec![0.0; k], v));
    }
    let (_, exponent) = libm::frexp(largest);
    m.iter_mut().for_each(|x| *x = libm::scalbn(*x, -exponent));

    let mut d: Vec<f64> = (0..k).map(|i| m[i * k + i]).collect();
    let mut b = d.clone();
    let mut z = vec![0.0; k];
    let mut converged = false;
    for sweep in 1..=max_sweeps {
        let mut off = 0.0;
        for p in 0..k {
            for q in p + 1..k {
                off += m[p * k + q].abs();
            }
        }
        if off == 0.0 {
            converged = true;
            break;
        }
        let threshold = if sweep < 4 {
            0.2 * off / (k * k) as f64
        } else {
            0.0
        };
        for p in 0..k {
            for q in p + 1..k {
                let apq = m[p * k + q];
                let g = 100.0 * apq.abs();
                if sweep > 4 && d[p].abs() + g == d[p].abs() && d[q].abs() + g == d[q].abs() {
                    m[p * k + q] = 0.0;
                } else if apq.abs() > threshold {
                    let h = d[q] - d[p];
                    let t = if h.abs() + g == h.abs() {
                        apq / h
                    } else {
                        let theta = 0.5 * h / apq;
                        let t = 1.0 / (theta.abs() + libm::sqrt(1.0 + theta * theta));
                        if theta < 0.0 { -t } else { t }
                    };
                    let c = 1.0 / libm::sqrt(1.0 + t * t);
                    let s = t * c;
                    let tau = s / (1.0 + c);
                    let h = t * apq;
                    z[p] -= h;
                    z[q] += h;
                    d[p] -= h;
                    d[q] += h;
                    m[p * k + q] = 0.0;
                    let rotate = |m: &mut [f64], x: usize, y: usize| {
                        let (g, h) = (m[x], m[y]);
                        m[x] = g - s * (h + g * tau);
                        m[y] = h + s * (g - h * tau);
                    };
                    for j in 0..p {
                        rotate(&mut m, j * k + p, j * k + q);
                    }
                    for j in p + 1..q {
                        rotate(&mut m, p * k + j, j * k + q);
                    }
                    for j in q + 1..k {
                        rotate(&mut m, p * k + j, q * k + j);
                    }
                    if with_vectors {
                        for j in 0..k {
                            rotate(&mut v, j * k + p, j * k + q);
                        }
                    }
                }
            }
        }
        for p in 0..k {
            b[p] += z[p];
            d[p] = b[p];
            z[p] = 0.0;
        }
    }
    if !converged {
        return Err(StatsError::value(format!(
            "The eigendecomposition of the {k} x {k} matrix did not converge in {max_sweeps} sweeps."
        )));
    }
    Ok((d.iter().map(|&x| libm::scalbn(x, exponent)).collect(), v))
}

#[cfg(test)]
mod tests {
    use super::jacobi;

    #[test]
    fn a_jacobi_method_which_does_not_converge_in_its_sweeps_is_an_error() {
        let m = vec![4.0, 1.0, 0.5, 1.0, 3.0, 0.25, 0.5, 0.25, 2.0];
        let error = jacobi(m.clone(), 3, true, 1).unwrap_err();
        assert!(
            error.message.contains("did not converge in 1 sweeps"),
            "{error}"
        );
        assert!(jacobi(m, 3, true, super::MAX_SWEEPS).is_ok());
    }
}
