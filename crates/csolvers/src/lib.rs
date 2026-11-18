//! SUNDIALS CVODES/IDA and SuiteSparse KLU, compiled from the submodules in `third_party` for the
//! native target and for `wasm32-wasip1`, and the FFI declarations the engine uses ([`ffi`]).
//!
//! The build script compiles SUNDIALS v7.9.0 (CVODES/IDA, the serial vector, the dense and the sparse
//! matrix, the dense, KLU and GMRES linear solvers, the Newton and fixed point nonlinear solvers)
//! and SuiteSparse v7.14.1 (KLU 2.3.6 with AMD, COLAMD and BTF, the `int32_t` variants, without
//! CHOLMOD and BLAS) with `-O3 -DNDEBUG -ffp-contract=off` and without fast math into one static
//! library which this crate links; for `wasm32-wasip1` with the clang of wasi-sdk
//! (`WASI_SDK_PATH`) and `-msimd128`.

pub mod ffi;
