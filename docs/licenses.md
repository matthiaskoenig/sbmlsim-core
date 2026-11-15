# Licenses

The code of sbmlsim-core is MIT licensed (`LICENSE`, Copyright (c) 2026 Matthias König).

`engine.wasm` statically links libraries which the crate `sbmlsim-csolvers` compiles from the git submodules in `third_party`:

| library | version | license |
|---|---|---|
| SUNDIALS (CVODE and its vectors, matrices and linear solvers) | 7.9.0 | BSD-3-Clause |
| KLU (SuiteSparse 7.14.1) | 2.3.6 | LGPL-2.1-or-later |
| BTF (SuiteSparse 7.14.1) | 2.3.3 | LGPL-2.1-or-later |
| AMD (SuiteSparse 7.14.1) | 3.3.4 | BSD-3-Clause |
| COLAMD (SuiteSparse 7.14.1) | 3.3.5 | BSD-3-Clause |
| SuiteSparse_config | 7.14.1 | BSD-3-Clause |

So `engine.wasm` is under `MIT AND BSD-3-Clause AND LGPL-2.1-or-later`, and so are the packages which build or contain it: `sbmlsim-csolvers`, `sbmlsim-host` and `sbmlsim-cli` (which embed `engine.wasm`) and the JavaScript package, whose runtime is `engine.wasm`. A page which serves `engine.wasm` distributes it under these licenses.

KLU and BTF are under the GNU Lesser General Public License 2.1 or later and are linked statically into `engine.wasm`. Their complete source is in the submodule, and the complete source of `engine.wasm` and its build are in the repository (`cargo build`, or `SBMLSIM_CORE_WASM_DIR` for an `engine.wasm` built otherwise), so a recipient can modify KLU or BTF, build `engine.wasm` again and use it with the hosts in place of the distributed one.

Also compiled into the modules, under permissive licenses whose texts are in their sources: the Rust standard library (MIT OR Apache-2.0); in `engine.wasm` the crates serde, serde_json, itoa, rand_core, rand_pcg and thiserror (MIT OR Apache-2.0), memchr (Unlicense OR MIT) and zmij (MIT), and wasi-libc (Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT, with parts of musl under MIT); in `mathlib.wasm` the crate libm (MIT). `model.wasm` contains no third-party code.

## Third-party notices

`THIRD_PARTY.md` in the root of the repository has the versions, the commits of the submodules, the complete license texts and how to rebuild `engine.wasm` with a modified KLU or BTF. `sbmlsim-core runtime -o DIR` writes it next to `engine.wasm` and `mathlib.wasm`, so a page which serves the runtime can serve its notices next to it, and `sbmlsim_host::THIRD_PARTY_NOTICES` is its text.
