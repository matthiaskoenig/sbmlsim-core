# Licenses

The code of sbmlsim-core is MIT licensed (`LICENSE`, Copyright (c) 2026 Matthias König).

`engine.wasm` statically links libraries which the crate `sbmlsim-csolvers` compiles from the git submodules in `third_party`:

| library | version | license |
|---|---|---|
| SUNDIALS (CVODES and its vectors, matrices and linear solvers) | 7.9.0 | BSD-3-Clause |
| KLU (SuiteSparse 7.14.1) | 2.3.6 | LGPL-2.1-or-later |
| BTF (SuiteSparse 7.14.1) | 2.3.3 | LGPL-2.1-or-later |
| AMD (SuiteSparse 7.14.1) | 3.3.4 | BSD-3-Clause |
| COLAMD (SuiteSparse 7.14.1) | 3.3.5 | BSD-3-Clause |
| SuiteSparse_config | 7.14.1 | BSD-3-Clause |

So `engine.wasm` is under `MIT AND BSD-3-Clause AND LGPL-2.1-or-later`, and so are the packages which build or contain it: `sbmlsim-csolvers`, `sbmlsim-host` and `sbmlsim-cli` (which embed `engine.wasm`), the JavaScript package, whose runtime is `engine.wasm`, and the python package `sbmlsim-core`, whose extension embeds it (its wheel carries `LICENSE` and `THIRD_PARTY.md`). A page which serves `engine.wasm` distributes it under these licenses.

`plot.wasm`, the renderer of the figures of sbmlsim (the crate `sbmlsim-plot-wasm`), contains the crate charton 0.9.1 (Apache-2.0) and permissive crates whose Apache-2.0 or MIT option is used, so `sbmlsim-host`, `sbmlsim-cli` and the JavaScript package, which carry it, are under `MIT AND BSD-3-Clause AND LGPL-2.1-or-later AND Apache-2.0`. The python package also links the renderer natively with its PNG (resvg and its crates, among them arrayref under BSD-2-Clause, slotmap and zlib-rs under Zlib, yoke and zerofrom under Unicode-3.0) and embeds the font Inter (OFL-1.1), so its wheel is under `MIT AND BSD-3-Clause AND LGPL-2.1-or-later AND Apache-2.0 AND BSD-2-Clause AND Zlib AND Unicode-3.0 AND OFL-1.1`. `THIRD_PARTY.md` lists these crates and has their license texts.

KLU and BTF are under the GNU Lesser General Public License 2.1 or later and are linked statically into `engine.wasm`. Their complete source is in the submodule, and the complete source of `engine.wasm` and its build are in the repository (`cargo build`, or `SBMLSIM_CORE_WASM_DIR` for an `engine.wasm` built otherwise), so a recipient can modify KLU or BTF, build `engine.wasm` again and use it with the hosts in place of the distributed one.

Also compiled into the modules, under permissive licenses whose texts are in their sources: the Rust standard library (MIT OR Apache-2.0); in `engine.wasm` the crates serde, serde_json, itoa, rand_core, rand_pcg and thiserror (MIT OR Apache-2.0), memchr (Unlicense OR MIT) and zmij (MIT), and wasi-libc (Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT, with parts of musl under MIT); in `mathlib.wasm` the crate libm (MIT). `model.wasm` contains no third-party code.

The extension module of the python package links one library more, which no wasm module and no other package contains: mimalloc 3.3.2 (MIT, through the crate `libmimalloc-sys`), the allocator of the extension on every target but macOS, whose wheel links none (see [Python](python.md#compile-and-simulate)). Its text is in `THIRD_PARTY.md` as well, which every wheel carries whether or not its platform links it.

The native Wasmtime hosts also link Zstandard 1.5.7 and its Rust wrappers (`zstd`, `zstd-safe`, `zstd-sys`, BSD-3-Clause) for persistent compilation caching. Their notices are in `THIRD_PARTY.md`; they are in no Wasm module or JavaScript package.

## Third-party notices

`THIRD_PARTY.md` in the root of the repository has the versions, the commits of the submodules, the complete license texts and how to rebuild `engine.wasm` with a modified KLU or BTF. `sbmlsim-core runtime -o DIR` writes it next to `engine.wasm` and `mathlib.wasm`, so a page which serves the runtime can serve its notices next to it, and `sbmlsim_host::THIRD_PARTY_NOTICES` is its text.
