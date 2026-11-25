//! Fixed-pattern matrix operations agree with the SUNDIALS sparse matrix.

use std::ptr;

use sbmlsim_csolvers::ffi::*;

unsafe fn contents(matrix: SUNMatrix, n: usize) -> (Vec<i32>, Vec<i32>, Vec<u64>) {
    unsafe {
        let columns = std::slice::from_raw_parts(SUNSparseMatrix_IndexPointers(matrix), n + 1);
        let nnz = columns[n] as usize;
        let rows = std::slice::from_raw_parts(SUNSparseMatrix_IndexValues(matrix), nnz);
        let values = std::slice::from_raw_parts(SUNSparseMatrix_Data(matrix), nnz);
        (
            columns.to_vec(),
            rows.to_vec(),
            values.iter().map(|value| value.to_bits()).collect(),
        )
    }
}

#[test]
fn saved_jacobian_clones_keep_pattern_and_match_newton_matrices() {
    // Includes empty columns, unsorted rows, absent diagonals and signed zeros.
    let columns = [0, 2, 2, 4, 5];
    let rows = [3, 1, 2, 0, 0];
    let first = [-0.0, -3.0, 2.0, 0.25, 9.0];
    let second = [8.0, 1.5, -0.0, -7.0, 0.0];
    unsafe {
        let mut context = ptr::null_mut();
        assert_eq!(SUNContext_Create(SUN_COMM_NULL, &mut context), 0);
        let fixed = sbmlsim_SUNSparseMatrix_Fixed(4, 9, context);
        let plain = SUNSparseMatrix(4, 4, 9, CSC_MAT, context);
        // CVODE clones its saved Jacobian before its first callback installs a pattern.
        let saved = SUNMatClone(fixed);
        assert!(!fixed.is_null() && !plain.is_null() && !saved.is_null());
        for values in [first, second, first] {
            assert_eq!(SUNMatZero(fixed), 0);
            assert_eq!(SUNMatZero(plain), 0);
            std::slice::from_raw_parts_mut(SUNSparseMatrix_IndexPointers(plain), 5)
                .copy_from_slice(&columns);
            std::slice::from_raw_parts_mut(SUNSparseMatrix_IndexValues(plain), 5)
                .copy_from_slice(&rows);
            std::slice::from_raw_parts_mut(SUNSparseMatrix_Data(plain), 5).copy_from_slice(&values);
            assert_eq!(
                sbmlsim_SUNSparseMatrix_SetJacobian(
                    fixed,
                    5,
                    columns.as_ptr(),
                    rows.as_ptr(),
                    values.as_ptr(),
                    4,
                    1,
                    0.0,
                ),
                0
            );
            assert_eq!(SUNMatCopy(fixed, saved), 0);
            for scale in [-0.125, -1e12, 0.0] {
                assert_eq!(SUNMatScaleAddI(scale, fixed), 0);
                assert_eq!(SUNMatScaleAddI(scale, plain), 0);
                assert_eq!(contents(fixed, 4), contents(plain, 4));
                assert_eq!(SUNMatZero(fixed), 0);
                assert_eq!(SUNMatCopy(saved, fixed), 0);
                // Restore the reference model pattern before the next scale.
                assert_eq!(SUNMatZero(plain), 0);
                std::slice::from_raw_parts_mut(SUNSparseMatrix_IndexPointers(plain), 5)
                    .copy_from_slice(&columns);
                std::slice::from_raw_parts_mut(SUNSparseMatrix_IndexValues(plain), 5)
                    .copy_from_slice(&rows);
                std::slice::from_raw_parts_mut(SUNSparseMatrix_Data(plain), 5)
                    .copy_from_slice(&values);
            }
        }
        SUNMatDestroy(fixed);
        // Metadata remains alive while its saved clone exists.
        let clone = SUNMatClone(saved);
        assert!(!clone.is_null());
        assert_eq!(SUNMatCopy(saved, clone), 0);
        assert_eq!(contents(saved, 4), contents(clone, 4));
        SUNMatDestroy(clone);
        SUNMatDestroy(saved);
        SUNMatDestroy(plain);
        assert_eq!(SUNContext_Free(&mut context), 0);
    }
}

#[test]
fn ida_assembles_only_differential_diagonals_after_each_zero() {
    let columns = [0, 1, 2, 3];
    let rows = [1, 1, 0];
    let values = [4.0, 2.0, 9.0];
    unsafe {
        let mut context = ptr::null_mut();
        assert_eq!(SUNContext_Create(SUN_COMM_NULL, &mut context), 0);
        let matrix = sbmlsim_SUNSparseMatrix_Fixed(3, 5, context);
        assert!(!matrix.is_null());
        for cj in [0.5, 8.0, 0.5] {
            assert_eq!(SUNMatZero(matrix), 0);
            assert_eq!(
                sbmlsim_SUNSparseMatrix_SetJacobian(
                    matrix,
                    3,
                    columns.as_ptr(),
                    rows.as_ptr(),
                    values.as_ptr(),
                    2,
                    0,
                    cj,
                ),
                0
            );
            assert_eq!(
                contents(matrix, 3),
                (
                    vec![0, 2, 3, 4],
                    vec![0, 1, 1, 0],
                    [-cj, 4.0, 2.0 - cj, 9.0]
                        .iter()
                        .map(|value| value.to_bits())
                        .collect()
                )
            );
        }
        SUNMatDestroy(matrix);
        assert_eq!(SUNContext_Free(&mut context), 0);
    }
}
