//! Compiles SUNDIALS CVODE and SuiteSparse KLU from the submodules in `third_party` with `cc`.
//!
//! For a native target the C compiler is the one `cc` finds (`CC`, else `cc`), for a target of
//! MSVC without `CC` the `clang` of LLVM on the path, since `cl` reads neither the GNU attributes
//! of the configuration of SUNDIALS nor the flags below; for `wasm32-wasip1`
//! it is `$WASI_SDK_PATH/bin/clang` with the sysroot of wasi-sdk and `-msimd128`. Every compile
//! is `-O3 -DNDEBUG -ffp-contract=off -fno-fast-math`, so neither the optimization level of the
//! cargo profile, nor SIMD, nor the contraction into fused multiply-adds changes a result.
//! `sundials_config.h` is generated per target into `OUT_DIR`, the configuration CMake writes
//! with `SUNDIALS_INDEX_SIZE=32` and KLU enabled.
//!
//! The flags of the environment are not used: `cc` appends `CFLAGS` (and `CFLAGS_<target>`,
//! `TARGET_CFLAGS`, `HOST_CFLAGS`) after the flags of the crate, so a stray `-ffast-math` there
//! would change the bits of every result, of a distributed `engine.wasm` too. The build script
//! removes them from its environment before `cc` reads it, and `cc` does not inherit flags of
//! rustc either.

use std::env;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

/// The release of SUNDIALS in `third_party/sundials`.
const SUNDIALS_VERSION: (u32, u32, u32) = (7, 9, 0);

/// The sources of SUNDIALS, relative to `third_party/sundials/src`.
const SUNDIALS_SOURCES: &[&str] = &[
    // the core library, `sundials_SOURCES` of `src/sundials/CMakeLists.txt` without MPI
    "sundials/sundatanode/sundatanode_inmem.c",
    "sundials/sundials_adaptcontroller.c",
    "sundials/sundials_adjointcheckpointscheme.c",
    "sundials/sundials_adjointstepper.c",
    "sundials/sundials_band.c",
    "sundials/sundials_cli.c",
    "sundials/sundials_context.c",
    "sundials/sundials_dense.c",
    "sundials/sundials_datanode.c",
    "sundials/sundials_direct.c",
    "sundials/sundials_errors.c",
    "sundials/sundials_domeigestimator.c",
    "sundials/sundials_futils.c",
    "sundials/sundials_hashmap.c",
    "sundials/sundials_iterative.c",
    "sundials/sundials_linearsolver.c",
    "sundials/sundials_logger.c",
    "sundials/sundials_math.c",
    "sundials/sundials_matrix.c",
    "sundials/sundials_memory.c",
    "sundials/sundials_nonlinearsolver.c",
    "sundials/sundials_nvector_senswrapper.c",
    "sundials/sundials_nvector.c",
    "sundials/sundials_stepper.c",
    "sundials/sundials_profiler.c",
    "sundials/sundials_version.c",
    // CVODE, `cvode_SOURCES` without the band preconditioners `cvode_bandpre.c` and
    // `cvode_bbdpre.c`, which need the band matrix and are for the iterative linear solvers
    "cvode/cvode.c",
    "cvode/cvode_cli.c",
    "cvode/cvode_diag.c",
    "cvode/cvode_io.c",
    "cvode/cvode_ls.c",
    "cvode/cvode_nls.c",
    "cvode/cvode_proj.c",
    "cvode/cvode_resize.c",
    // the vector, the matrices (the band matrix for the difference quotient Jacobian of
    // `cvode_ls.c`), the linear and the nonlinear solvers
    "nvector/serial/nvector_serial.c",
    "sunmatrix/band/sunmatrix_band.c",
    "sunmatrix/dense/sunmatrix_dense.c",
    "sunmatrix/sparse/sunmatrix_sparse.c",
    "sunlinsol/dense/sunlinsol_dense.c",
    "sunlinsol/klu/sunlinsol_klu.c",
    "sunnonlinsol/newton/sunnonlinsol_newton.c",
    "sunnonlinsol/fixedpoint/sunnonlinsol_fixedpoint.c",
];

/// The sources of SuiteSparse, relative to `third_party/SuiteSparse`: the real `int32_t`
/// variants (without `_l`) of KLU, AMD, COLAMD and BTF, which `SUNLinSol_KLU` calls with
/// `SUNDIALS_INT32_T`, their version functions and SuiteSparse_config.
const SUITESPARSE_SOURCES: &[&str] = &[
    "SuiteSparse_config/SuiteSparse_config.c",
    "AMD/Source/amd_1.c",
    "AMD/Source/amd_2.c",
    "AMD/Source/amd_aat.c",
    "AMD/Source/amd_control.c",
    "AMD/Source/amd_defaults.c",
    "AMD/Source/amd_dump.c",
    "AMD/Source/amd_info.c",
    "AMD/Source/amd_order.c",
    "AMD/Source/amd_postorder.c",
    "AMD/Source/amd_post_tree.c",
    "AMD/Source/amd_preprocess.c",
    "AMD/Source/amd_valid.c",
    "AMD/Source/amd_version.c",
    "COLAMD/Source/colamd.c",
    "COLAMD/Source/colamd_version.c",
    "BTF/Source/btf_maxtrans.c",
    "BTF/Source/btf_order.c",
    "BTF/Source/btf_strongcomp.c",
    "BTF/Source/btf_version.c",
    "KLU/Source/klu.c",
    "KLU/Source/klu_analyze.c",
    "KLU/Source/klu_analyze_given.c",
    "KLU/Source/klu_defaults.c",
    "KLU/Source/klu_diagnostics.c",
    "KLU/Source/klu_dump.c",
    "KLU/Source/klu_extract.c",
    "KLU/Source/klu_factor.c",
    "KLU/Source/klu_free_numeric.c",
    "KLU/Source/klu_free_symbolic.c",
    "KLU/Source/klu_kernel.c",
    "KLU/Source/klu_memory.c",
    "KLU/Source/klu_refactor.c",
    "KLU/Source/klu_scale.c",
    "KLU/Source/klu_solve.c",
    "KLU/Source/klu_sort.c",
    "KLU/Source/klu_tsolve.c",
    "KLU/Source/klu_version.c",
];

/// The C sources of this crate, relative to its directory: the KLU linear solver which keeps its
/// symbolic analysis across the initializations of CVODE (`sbmlsim_SUNLinSol_KLUKept`).
const CRATE_SOURCES: &[&str] = &["csrc/sunlinsol_klu_kept.c"];

/// The include directories of SuiteSparse, relative to `third_party/SuiteSparse`.
const SUITESPARSE_INCLUDES: &[&str] = &[
    "SuiteSparse_config",
    "AMD/Include",
    "COLAMD/Include",
    "BTF/Include",
    "KLU/Include",
];

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let third_party = manifest.join("../../third_party");
    let sundials = third_party.join("sundials");
    let suitesparse = third_party.join("SuiteSparse");
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let target = env::var("TARGET").unwrap();
    let wasm = target == "wasm32-wasip1";
    // windows has no POSIX timers: SUNDIALS profiles with the timers of windows and SuiteSparse
    // without a timer (`NTIMER`), as their CMake builds configure it; no solver reads a timer
    let windows = env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows");

    for source in SUNDIALS_SOURCES {
        require(&sundials.join("src").join(source));
    }
    for source in SUITESPARSE_SOURCES {
        require(&suitesparse.join(source));
    }
    for source in CRATE_SOURCES {
        require(&manifest.join(source));
    }

    for key in c_flag_variables() {
        // SAFETY: the build script is single threaded here (`cc` starts its threads later)
        unsafe { env::remove_var(key) };
    }
    let mut build = cc::Build::new();
    build.inherit_rustflags(false);
    let compiler_name = if wasm {
        println!("cargo:rerun-if-env-changed=WASI_SDK_PATH");
        let sdk = PathBuf::from(env::var("WASI_SDK_PATH").unwrap_or_else(|_| {
            panic!("building for wasm32-wasip1 needs WASI_SDK_PATH, the directory of wasi-sdk 34")
        }));
        require(&sdk.join("bin/clang"));
        build
            .compiler(sdk.join("bin/clang"))
            .archiver(sdk.join("bin/llvm-ar"))
            .flag(format!(
                "--sysroot={}",
                sdk.join("share/wasi-sysroot").display()
            ))
            .flag("-msimd128");
        "Clang"
    } else {
        if env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") && !compiler_given() {
            // cl of MSVC reads neither the GNU attributes of the configuration of SUNDIALS nor
            // the flags which fix the bits (`-ffp-contract=off`), so a windows build compiles the
            // C with clang of LLVM for the target of MSVC unless a compiler is given
            build.compiler("clang");
        }
        if build.get_compiler().is_like_clang() {
            "Clang"
        } else {
            "GNU"
        }
    };
    let flags = if wasm {
        "-O3 -DNDEBUG -ffp-contract=off -fno-fast-math -msimd128"
    } else {
        "-O3 -DNDEBUG -ffp-contract=off -fno-fast-math"
    };
    build
        .opt_level(3)
        .debug(false)
        .define("NDEBUG", None)
        .flag("-ffp-contract=off")
        .flag("-fno-fast-math")
        .flag("-std=gnu11")
        // SUNDIALS as a static library
        .define("SUNDIALS_STATIC_DEFINE", None)
        // KLU without CHOLMOD, SuiteSparse without BLAS (as the CMake build configures it)
        .define("NCHOLMOD", None)
        .define("BLAS_Generic", None)
        // `-w`: the warnings of the C of SUNDIALS and SuiteSparse are not ours; what the compiler
        // still writes, the errors, reaches the output of cargo
        .warnings(false)
        .cargo_warnings(true);

    let include = out.join("include");
    if windows {
        build.define("NTIMER", None);
    }
    write_config(&include, compiler_name, flags, !windows);
    build
        .include(&include)
        .include(sundials.join("include"))
        .include(sundials.join("src/sundials"));
    for dir in SUITESPARSE_INCLUDES {
        build.include(suitesparse.join(dir));
    }
    for source in SUNDIALS_SOURCES {
        build.file(sundials.join("src").join(source));
    }
    for source in SUITESPARSE_SOURCES {
        build.file(suitesparse.join(source));
    }
    for source in CRATE_SOURCES {
        build.file(manifest.join(source));
    }
    build.compile("sbmlsim_csolvers");
    if !wasm && env::var("CARGO_CFG_TARGET_FAMILY").as_deref() == Ok("unix") {
        println!("cargo:rustc-link-lib=m");
    }

    println!("cargo:rerun-if-changed=build.rs");
    for dir in [
        manifest.join("csrc"),
        sundials.join("include"),
        sundials.join("src/sundials"),
        sundials.join("src/cvode"),
        sundials.join("src/nvector/serial"),
        sundials.join("src/sunmatrix/band"),
        sundials.join("src/sunmatrix/dense"),
        sundials.join("src/sunmatrix/sparse"),
        sundials.join("src/sunlinsol/dense"),
        sundials.join("src/sunlinsol/klu"),
        sundials.join("src/sunnonlinsol/newton"),
        sundials.join("src/sunnonlinsol/fixedpoint"),
        suitesparse.join("SuiteSparse_config"),
        suitesparse.join("AMD/Source"),
        suitesparse.join("AMD/Include"),
        suitesparse.join("COLAMD/Source"),
        suitesparse.join("COLAMD/Include"),
        suitesparse.join("BTF/Source"),
        suitesparse.join("BTF/Include"),
        suitesparse.join("KLU/Source"),
        suitesparse.join("KLU/Include"),
    ] {
        println!("cargo:rerun-if-changed={}", dir.display());
    }
}

/// Panics with the hint to check out the submodules when `path` does not exist.
fn require(path: &Path) {
    assert!(
        path.exists(),
        "{} does not exist; run `git submodule update --init` (and set WASI_SDK_PATH for wasm32-wasip1)",
        path.display()
    );
}

/// The variables of C flags in the environment which `cc` would append to the flags of the
/// crate: `CFLAGS` and its variants of a target and a kind (`CFLAGS_<target>`,
/// `TARGET_CFLAGS`, `HOST_CFLAGS`).
fn c_flag_variables() -> Vec<std::ffi::OsString> {
    env::vars_os()
        .map(|(key, _)| key)
        .filter(|key| {
            key.to_str().is_some_and(|key| {
                key == "CFLAGS"
                    || key.starts_with("CFLAGS_")
                    || key == "TARGET_CFLAGS"
                    || key == "HOST_CFLAGS"
            })
        })
        .collect()
}

/// Whether the environment names the C compiler `cc` takes: `CC`, its variants of a target
/// (`CC_<target>`, with `-` or `_`) and `TARGET_CC`.
fn compiler_given() -> bool {
    let target = env::var("TARGET").unwrap_or_default();
    let names = [
        "CC".to_string(),
        format!("CC_{target}"),
        format!("CC_{}", target.replace('-', "_")),
        "TARGET_CC".to_string(),
    ];
    for name in &names {
        println!("cargo:rerun-if-env-changed={name}");
    }
    names.iter().any(|name| env::var_os(name).is_some())
}

/// Writes `sundials/sundials_config.h` and `sundials/sundials_export.h` into `include`.
///
/// The configuration is the one CMake writes for this build: double precision, `int32_t`
/// indices, `long int` counters, KLU, no MPI, the logging level 0 (no logging: the logger of a
/// context reads no environment variable and writes nothing, the engine records the errors with
/// an error handler of its own), the POSIX timers of the profiler where the target has them
/// (`posix_timers`, every target but windows), and the modules of [`SUNDIALS_SOURCES`].
///
/// The indices are 32 bits, also natively, so every target computes with the same code: half
/// the memory of the index arrays of KLU and of the sparse matrix, and no conversion of a 64 bit
/// index into a 32 bit address in wasm32, which makes the KLU runs 3 to 6% faster in wasm (the
/// results are bitwise those of 64 bit indices). The engine refuses a model whose matrix would
/// not fit (`sbmlsim-engine`, `cvode::check_index_range`).
fn write_config(include: &Path, compiler: &str, flags: &str, posix_timers: bool) {
    let dir = include.join("sundials");
    fs::create_dir_all(&dir).unwrap();
    let (major, minor, patch) = SUNDIALS_VERSION;
    let mut config = String::new();
    let lines = [
        "/* Generated by the build script of sbmlsim-csolvers. */",
        "#ifndef _SUNDIALS_CONFIG_H",
        "#define _SUNDIALS_CONFIG_H",
        "#include \"sundials/sundials_export.h\"",
        "#if defined(__cplusplus)",
        "#define SUNDIALS_NOEXCEPT noexcept",
        "#else",
        "#define SUNDIALS_NOEXCEPT",
        "#endif",
        "#ifndef SUNDIALS_DEPRECATED_MSG",
        "#define SUNDIALS_DEPRECATED_MSG(msg) __attribute__((__deprecated__(msg)))",
        "#endif",
        "#ifndef SUNDIALS_DEPRECATED_EXPORT_MSG",
        "#define SUNDIALS_DEPRECATED_EXPORT_MSG(msg) SUNDIALS_EXPORT SUNDIALS_DEPRECATED_MSG(msg)",
        "#endif",
        "#ifndef SUNDIALS_DEPRECATED_NO_EXPORT_MSG",
        "#define SUNDIALS_DEPRECATED_NO_EXPORT_MSG(msg) SUNDIALS_NO_EXPORT SUNDIALS_DEPRECATED_MSG(msg)",
        "#endif",
    ];
    for line in lines {
        writeln!(config, "{line}").unwrap();
    }
    writeln!(
        config,
        "#define SUNDIALS_VERSION \"{major}.{minor}.{patch}\""
    )
    .unwrap();
    writeln!(config, "#define SUNDIALS_VERSION_MAJOR {major}").unwrap();
    writeln!(config, "#define SUNDIALS_VERSION_MINOR {minor}").unwrap();
    writeln!(config, "#define SUNDIALS_VERSION_PATCH {patch}").unwrap();
    writeln!(config, "#define SUNDIALS_VERSION_LABEL \"\"").unwrap();
    writeln!(
        config,
        "#define SUNDIALS_GIT_VERSION \"v{major}.{minor}.{patch}\""
    )
    .unwrap();
    let defines = [
        ("SUNDIALS_C_COMPILER_HAS_BUILTIN_EXPECT", ""),
        ("SUNDIALS_C_COMPILER_HAS_ATTRIBUTE_ASSUME", ""),
        ("SUNDIALS_C_COMPILER_HAS_ATTRIBUTE_UNUSED", ""),
        ("SUNDIALS_DOUBLE_PRECISION", "1"),
        ("SUNDIALS_INT32_T", "1"),
        ("SUNDIALS_INDEX_TYPE", "int32_t"),
        ("SUNDIALS_COUNTER_TYPE", "long int"),
        ("SUNDIALS_LOGGING_LEVEL", "0"),
        ("SUN_C_COMPILER", &format!("\"{compiler}\"")),
        ("SUN_C_COMPILER_VERSION", "\"\""),
        ("SUN_C_COMPILER_FLAGS", &format!("\"{flags}\"")),
        ("SUN_CXX_COMPILER", "\"\""),
        ("SUN_CXX_COMPILER_VERSION", "\"\""),
        ("SUN_CXX_COMPILER_FLAGS", "\"\""),
        ("SUN_FORTRAN_COMPILER", "\"\""),
        ("SUN_FORTRAN_COMPILER_VERSION", "\"\""),
        ("SUN_FORTRAN_COMPILER_FLAGS", "\"\""),
        ("SUN_BUILD_TYPE", "\"Release\""),
        ("SUN_JOB_ID", "\"sbmlsim-csolvers\""),
        ("SUN_JOB_START_TIME", "\"\""),
        ("SUN_TPL_LIST", "\"KLU\""),
        ("SUN_TPL_LIST_SIZE", "\"\""),
        ("SUNDIALS_SPACK_VERSION", "\"\""),
        ("SUNDIALS_KLU_ENABLED", ""),
        ("SUN_KLU_VERSION", "\"\""),
        ("SUNDIALS_MPI_ENABLED", "0"),
        ("SUNDIALS_CVODE", "1"),
        ("SUNDIALS_NVECTOR_SERIAL", "1"),
        ("SUNDIALS_SUNMATRIX_BAND", "1"),
        ("SUNDIALS_SUNMATRIX_DENSE", "1"),
        ("SUNDIALS_SUNMATRIX_SPARSE", "1"),
        ("SUNDIALS_SUNLINSOL_DENSE", "1"),
        ("SUNDIALS_SUNLINSOL_KLU", "1"),
        ("SUNDIALS_SUNNONLINSOL_NEWTON", "1"),
        ("SUNDIALS_SUNNONLINSOL_FIXEDPOINT", "1"),
    ];
    for (name, value) in defines {
        writeln!(config, "#define {name} {value}").unwrap();
    }
    if posix_timers {
        writeln!(config, "#define SUNDIALS_HAVE_POSIX_TIMERS").unwrap();
    }
    writeln!(config, "#endif /* _SUNDIALS_CONFIG_H */").unwrap();
    write_if_changed(&dir.join("sundials_config.h"), &config);

    let export = "\
/* Generated by the build script of sbmlsim-csolvers: a static library, nothing is exported. */
#ifndef SUNDIALS_EXPORT_H
#define SUNDIALS_EXPORT_H
#define SUNDIALS_EXPORT
#define SUNDIALS_NO_EXPORT
#ifndef SUNDIALS_DEPRECATED_ATTRIBUTE
#define SUNDIALS_DEPRECATED_ATTRIBUTE __attribute__((__deprecated__))
#endif
#ifndef SUNDIALS_DEPRECATED_ATTRIBUTE_EXPORT
#define SUNDIALS_DEPRECATED_ATTRIBUTE_EXPORT SUNDIALS_EXPORT SUNDIALS_DEPRECATED_ATTRIBUTE
#endif
#ifndef SUNDIALS_DEPRECATED_ATTRIBUTE_NO_EXPORT
#define SUNDIALS_DEPRECATED_ATTRIBUTE_NO_EXPORT SUNDIALS_NO_EXPORT SUNDIALS_DEPRECATED_ATTRIBUTE
#endif
#endif /* SUNDIALS_EXPORT_H */
";
    write_if_changed(&dir.join("sundials_export.h"), export);
}

/// Writes `contents` to `path` unless it has them already.
fn write_if_changed(path: &Path, contents: &str) {
    if fs::read_to_string(path).ok().as_deref() != Some(contents) {
        fs::write(path, contents).unwrap();
    }
}
