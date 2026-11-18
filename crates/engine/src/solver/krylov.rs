//! Matrix-free GMRES with an ILU(0) left preconditioner of `I - gamma J`.
//!
//! Only the preconditioner assembles the model Jacobian. CVODE evaluates Jacobian-vector
//! products by directional differences of the right hand side at the current Newton iterate.
//! The incomplete factorization keeps the model's sparsity pattern plus its diagonal, without
//! fill or pivoting. Its storage is linear in the number of model Jacobian entries.

use super::*;

/// Symbolic structure, saved Jacobian and numeric factors, retained across simulations.
pub(crate) struct Preconditioner {
    rows: Vec<usize>,
    columns: Vec<usize>,
    diagonal: Vec<usize>,
    /// CSC source index, or `usize::MAX` for an inserted diagonal.
    source: Vec<usize>,
    jacobian: Vec<f64>,
    factors: Vec<f64>,
    positions: Vec<usize>,
    weights: N_Vector,
    pub(super) valid: bool,
}

impl Preconditioner {
    pub(super) fn new(pattern: &SparsityPattern, ctx: SUNContext) -> Result<Self, EngineError> {
        let n = pattern.n as usize;
        let mut rows = vec![0; n + 1];
        let mut has_diagonal = vec![false; n];
        let mut seen = vec![usize::MAX; n];
        for (j, present) in has_diagonal.iter_mut().enumerate() {
            let start = pattern.col_ptr[j] as usize;
            let end = pattern.col_ptr[j + 1] as usize;
            for &row in &pattern.row_idx[start..end] {
                let i = row as usize;
                if seen[i] == j {
                    return Err(EngineError::Request(format!(
                        "GMRES requires unique Jacobian entries: repeated row {i} in column {j}"
                    )));
                }
                seen[i] = j;
                rows[i + 1] += 1;
                *present |= i == j;
            }
        }
        for i in 0..n {
            rows[i + 1] += rows[i] + usize::from(!has_diagonal[i]);
        }
        let nnz = rows[n];
        let mut next = rows[..n].to_vec();
        let mut columns = vec![0; nnz];
        let mut source = vec![usize::MAX; nnz];
        let mut diagonal = vec![0; n];
        // Visiting the CSC columns in order also sorts every CSR row.
        for j in 0..n {
            let start = pattern.col_ptr[j] as usize;
            let end = pattern.col_ptr[j + 1] as usize;
            for (offset, &row) in pattern.row_idx[start..end].iter().enumerate() {
                let i = row as usize;
                let at = next[i];
                next[i] += 1;
                columns[at] = j;
                source[at] = start + offset;
                if i == j {
                    diagonal[i] = at;
                }
            }
            if !has_diagonal[j] {
                let at = next[j];
                next[j] += 1;
                columns[at] = j;
                diagonal[j] = at;
            }
        }
        // SAFETY: the context outlives this preconditioner and n fits sunindextype.
        let weights = unsafe { N_VNew_Serial(n as sunindextype, ctx) };
        if weights.is_null() {
            return Err(not_created("the GMRES error weights"));
        }
        Ok(Self {
            rows,
            columns,
            diagonal,
            source,
            jacobian: vec![0.0; nnz],
            factors: vec![0.0; nnz],
            positions: vec![usize::MAX; n],
            weights,
            valid: false,
        })
    }

    fn save(&mut self, jacobian: &[f64]) {
        for (value, &source) in self.jacobian.iter_mut().zip(&self.source) {
            *value = if source == usize::MAX {
                0.0
            } else {
                jacobian[source]
            };
        }
        self.valid = true;
    }

    /// A failed pivot is recoverable: CVODE can refresh J or retry with a smaller step.
    fn factor(&mut self, gamma: f64) -> bool {
        for (factor, &value) in self.factors.iter_mut().zip(&self.jacobian) {
            *factor = -gamma * value;
        }
        for &at in &self.diagonal {
            self.factors[at] += 1.0;
        }
        self.positions.fill(usize::MAX);
        for i in 0..self.diagonal.len() {
            let start = self.rows[i];
            let end = self.rows[i + 1];
            for k in start..end {
                self.positions[self.columns[k]] = k;
            }
            for k in start..self.diagonal[i] {
                let j = self.columns[k];
                self.factors[k] /= self.factors[self.diagonal[j]];
                let multiplier = self.factors[k];
                if !multiplier.is_finite() {
                    return false;
                }
                for q in self.diagonal[j] + 1..self.rows[j + 1] {
                    let at = self.positions[self.columns[q]];
                    if at != usize::MAX {
                        self.factors[at] -= multiplier * self.factors[q];
                    }
                }
            }
            let pivot = self.factors[self.diagonal[i]];
            if pivot == 0.0 || !pivot.is_finite() {
                return false;
            }
            for k in start..end {
                self.positions[self.columns[k]] = usize::MAX;
            }
        }
        self.factors.iter().all(|v| v.is_finite())
    }

    fn solve(&self, z: &mut [f64]) -> bool {
        for i in 0..z.len() {
            for k in self.rows[i]..self.diagonal[i] {
                z[i] -= self.factors[k] * z[self.columns[k]];
            }
        }
        for i in (0..z.len()).rev() {
            for k in self.diagonal[i] + 1..self.rows[i + 1] {
                z[i] -= self.factors[k] * z[self.columns[k]];
            }
            z[i] /= self.factors[self.diagonal[i]];
        }
        z.iter().all(|v| v.is_finite())
    }
}

impl Drop for Preconditioner {
    fn drop(&mut self) {
        // SAFETY: weights is owned here and the integrator drops us before its context.
        unsafe { N_VDestroy(self.weights) };
    }
}

/// `P_data` is the stable CVODE memory. Read the fresh Problem pointer installed for this
/// integration call, rather than retaining a pointer from an earlier mutable reborrow.
unsafe fn current_problem<'a>(mem: *mut c_void) -> Option<&'a mut Problem<'a>> {
    let mut data = ptr::null_mut();
    // SAFETY: CVODE passes its own live memory as P_data to the preconditioner callbacks.
    unsafe {
        if CVodeGetUserData(mem, &mut data) != 0 || data.is_null() {
            return None;
        }
        Some(problem(data))
    }
}

pub(super) unsafe extern "C" fn setup(
    t: f64,
    y: N_Vector,
    fy: N_Vector,
    jok: sunbooleantype,
    jcur: *mut sunbooleantype,
    gamma: f64,
    data: *mut c_void,
) -> c_int {
    // SAFETY: CVODE lends the vectors and jcur for this call. integrate installs a pointer to
    // the integrator-owned preconditioner, disjoint from the Problem and its model buffers.
    unsafe {
        let Some(problem) = current_problem(data) else {
            return -1;
        };
        callback(problem, |problem| {
            let Some(preconditioner) = problem.krylov.as_mut() else {
                return -1;
            };
            let Some(pattern) = problem.pattern else {
                return -1;
            };
            *jcur = 0;
            if jok == 0 || !preconditioner.valid {
                let code = jacobian_values(problem, pattern, t, y, fy, preconditioner.weights);
                if code != 0 {
                    return code;
                }
                preconditioner.save(&problem.jac_values);
                *jcur = 1;
            }
            if preconditioner.factor(gamma) { 0 } else { 1 }
        })
    }
}

pub(super) unsafe extern "C" fn solve(
    _t: f64,
    _y: N_Vector,
    _fy: N_Vector,
    r: N_Vector,
    z: N_Vector,
    _gamma: f64,
    _delta: f64,
    _lr: c_int,
    data: *mut c_void,
) -> c_int {
    // SAFETY: as in setup; ptr::copy also permits r and z to be the same vector.
    unsafe {
        let Some(problem) = current_problem(data) else {
            return -1;
        };
        callback(problem, |problem| {
            let Some(preconditioner) = problem.krylov.as_ref() else {
                return -1;
            };
            ptr::copy(N_VGetArrayPointer(r), N_VGetArrayPointer(z), problem.n);
            if preconditioner.solve(values(z, problem.n)) {
                0
            } else {
                1
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ilu_solves_a_tridiagonal_system_with_unsorted_rows_and_missing_diagonal() {
        let pattern = SparsityPattern {
            n: 3,
            col_ptr: vec![0, 2, 4, 6],
            row_idx: vec![1, 0, 2, 0, 2, 1],
        };
        let jacobian = [3.0, -2.0, 4.0, 1.0, -5.0, 2.0];
        let mut ctx = ptr::null_mut();
        // SAFETY: the context owns no objects when it is freed below.
        unsafe {
            assert_eq!(SUNContext_Create(SUN_COMM_NULL, &mut ctx), 0);
            let mut preconditioner = Preconditioner::new(&pattern, ctx).unwrap();
            preconditioner.save(&jacobian);
            for gamma in [0.01, 0.1, 2.0] {
                assert!(preconditioner.factor(gamma));
                let r = [1.0, 2.0, 3.0];
                let mut z = r;
                assert!(preconditioner.solve(&mut z));
                let mut product = z;
                for (j, &zj) in z.iter().enumerate() {
                    for k in pattern.col_ptr[j] as usize..pattern.col_ptr[j + 1] as usize {
                        product[pattern.row_idx[k] as usize] -= gamma * jacobian[k] * zj;
                    }
                }
                for (actual, expected) in product.iter().zip(r) {
                    assert!((actual - expected).abs() < 1e-12);
                }
            }
            drop(preconditioner);
            assert_eq!(SUNContext_Free(&mut ctx), 0);
        }
    }

    #[test]
    fn a_failed_pivot_can_recover_and_duplicate_entries_are_refused() {
        let mut ctx = ptr::null_mut();
        // SAFETY: the context outlives the preconditioner.
        unsafe {
            assert_eq!(SUNContext_Create(SUN_COMM_NULL, &mut ctx), 0);
            let mut pattern = SparsityPattern {
                n: 1,
                col_ptr: vec![0, 1],
                row_idx: vec![0],
            };
            let mut preconditioner = Preconditioner::new(&pattern, ctx).unwrap();
            preconditioner.save(&[1.0]);
            assert!(!preconditioner.factor(1.0));
            assert!(preconditioner.factor(0.5));
            let mut z = [2.0];
            assert!(preconditioner.solve(&mut z));
            assert_eq!(z, [4.0]);
            pattern.col_ptr[1] = 2;
            pattern.row_idx.push(0);
            assert!(matches!(
                Preconditioner::new(&pattern, ctx),
                Err(EngineError::Request(_))
            ));
            drop(preconditioner);
            assert_eq!(SUNContext_Free(&mut ctx), 0);
        }
    }
}
