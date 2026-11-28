# Development

How a release of sbmlsim-core is made and what the continuous integration runs. The toolchain, the build of the engine and the tests are in [Installation](installation.md) and [Test suite](testsuite.md).

## Release

A release is made from `develop`. Since `develop` only accepts pull requests, the release is prepared on a branch and tagged once that pull request is merged. The tag `vx.y.z` starts `.github/workflows/publish.yml`, which publishes the python package to PyPI and the package to npm with trusted publishing and provenance, so the tag is the publication and cannot be moved or deleted afterwards. The release builds and tests nothing again: it publishes the wheels and the runtime of the run of `ci.yml` of the push of the tagged commit to `develop`, which built and tested them on exactly this tree (see [Continuous integration](#continuous-integration)).

1. branch off `develop`: `git switch -c release/x.y.z develop`
2. write the release notes in `release-notes/x.y.z.md`, which the job `check` of `publish.yml` requires
3. set the version in every place which carries it, which is not only the manifests:

    | Where | What compares it, and when |
    | --- | --- |
    | `Cargo.toml`, `[workspace.package]`, and `Cargo.lock`, which `cargo` refreshes | the job `check` of `publish.yml`, against the tag |
    | `js/package.json` and the two entries of `js/package-lock.json` | the job `check` of `publish.yml`, against the tag |
    | `CORE_VERSION` of `js/panels.mjs` | `js/prepack.mjs` at `npm pack`, which runs only while the tag publishes |
    | the `compiler` of `testdata/golden/model.json` | `cargo test --all-features`, the job `test` of `ci.yml` |
    | the header and the `CORE_VERSION` of `python/sbmlsim_core/web/sbmlsim-core.js`, the inline build of the JavaScript host | `python/tests/test_web.py`, the job `python-bindings` of `ci.yml` |
    | the examples: the CDN of `js/README.md` and of `docs/javascript.md`, the compiler of `docs/simulating.md`, the four `tag = "vX.Y.Z"` of the git dependencies of `docs/rust.md` | nothing |

    A version left behind in the golden model fails the job `test` and with it every job which builds a wheel, so a missed place looks like half of the checks red while `testsuite-parity` and `testsuite-cases` stay green, the engine itself being unchanged.

4. the inline build is generated and is never edited by hand: `cd js && npm ci && node build.mjs`, then `node build.mjs --check`, which compares the committed build with its sources
5. make sure everything passes: `cargo fmt --check`, `cargo clippy --all-targets --all-features -- -D warnings` and `cargo test --all-features`, which builds `engine.wasm` and needs `WASI_SDK_PATH` (see [Installation](installation.md#building))
6. push the branch, open the pull request against `develop` and merge it once the checks are green; the merge starts the run of `ci.yml` on `develop`, which builds and tests all five wheels, the one of macOS x86_64 included, which no pull request builds
7. tag the merged commit on `develop` and push the tag, which may follow the merge right away:

    ```bash
    git switch develop
    git pull
    git tag vx.y.z
    git push origin vx.y.z
    ```

    Check the version before pushing: the tag publishes to PyPI and to npm, and a version of either registry cannot be taken back.

    The job `tested on develop` of `publish.yml` finds the run of `ci.yml` of the push of the tagged commit to `develop`, waits for it and fails unless it succeeded; once it does, re-run the failed jobs of the release. The release then takes the artifacts of that run (the five wheels `wheel-*`, the runtime `runtime` and the command line `cli`, kept 90 days): the job `wheels` checks that there is one wheel per platform of the version of the tag, the job `js` runs the tests of the JavaScript host with the runtime of the npm package, and only then are both registries published.

    The tag makes all three of a release: the python package on PyPI, the package on npm and the release of this repository, named `sbmlsim-core x.y.z` with the body of `release-notes/x.y.z.md`. The job `github_release` makes the last one once both registries carry the version, so a release of this repository never announces a version a registry does not have. Nothing of a release is made by hand.

8. bump the live demo of the documentation, which is vendored from the published npm package and so can only follow the tag: set `NPM_VERSION` of `scripts/demo_assets.py` to the new version, run `uv run scripts/demo_assets.py` to vendor the package again, compare it with `uv run scripts/demo_assets.py --check` and take `docs/demo/` through a pull request of its own. The page compiles its model itself with the vendored `compiler.wasm`, so nothing of the demo needs a local build. Nothing compares this one either, so a demo left behind keeps running the engine of the release before.

### A dry run of the release

`publish.yml` runs without publishing for a commit of `develop`: everything up to the publication (the version and the release notes of the commit, the run of `ci.yml` of its push to `develop`, its wheels and the tests of the JavaScript host with its runtime), without the jobs `npm`, `pypi` and `github_release`.

```bash
gh workflow run publish.yml --ref develop -f sha=<commit of develop>
```

## Continuous integration

`.github/workflows/ci.yml` runs on every pull request and every push to `develop`; `docs.yml` builds the documentation next to it.

| Job | What it runs |
| --- | --- |
| `lint` | `cargo fmt`, `cargo check` and `cargo clippy` of every target and feature, the clippy and the tests of the engine, its solvers, the sensitivity analysis and the statistics in `wasm32-wasip1` |
| `test` | `cargo test --all-features`, the command line and the math library |
| `compiler_builds_for_wasm32_unknown_unknown`, `plot_wasm_same_bytes` | the compiler and the renderer of the figures in wasm, as a page runs them |
| `same_bits_as_wasmtime` | the JavaScript host in Node against the wasmtime host, byte for byte |
| `testsuite-parity`, `testsuite-c-target`, `testsuite-cases` | the SBML Test Suite against its baselines |
| `python-bindings` | the python package on an editable install: ruff, the tests and the type check |
| `wheels` | `wheels.yml`: the runtime once (`runtime`), then one abi3 wheel per platform, each installed and tested on python 3.14 and 3.15 |

What differs between a pull request and `develop`:

- **The wheel of macOS x86_64 is built on `develop` only.** macOS runners are shared by every repository of the account and are the ones a run waits for, the Intel ones most; a pull request builds and tests the other four wheels (macOS on arm64), and the push of the merge to `develop` builds all five, which is what a release publishes.
- **A newer push to a pull request cancels its running checks**; a run of `develop` is never cancelled, since a release publishes from it.
- **Caches are saved by `develop` only.** The cache of the repository has a limit of 10 GB, and caches of pull requests evicted those of `develop`, so every rust cache is saved by a run of `develop` (`save-if`) and restored by the pull requests. Jobs which build the same profile share one cache: `test`, `testsuite-cases` and `python-bindings` the dev profile (`shared-key: dev`, saved by `test`), `same_bits_as_wasmtime`, `testsuite-parity`, `testsuite-c-target` and the job `runtime` of the wheels the release build of the command line (`shared-key: release-cli`, saved by `same_bits_as_wasmtime`). The cache of uv (`setup-uv`) is saved by `develop` alone as well. wasi-sdk and wasmtime are pinned by version and sha256 in `.github/actions/wasi-sdk` and are cached under a key of their sha256, saved by `develop` as well.

## The crate on crates.io

`sbmlsim-sbml`, the reader of SBML, is the one crate of the workspace which is published on [crates.io](https://crates.io/crates/sbmlsim-sbml): it holds nothing but its own sources and depends on two published crates, where the rest of the workspace builds `engine.wasm` and the solvers from files outside the crate directories. A reader of SBML is useful without the engine, and a git dependency is not: cargo clones the repository with its submodules, which fails on windows over the length of a path inside `third_party/sundials`.

The crate versions with the workspace, so a release publishes the version of its tag:

```bash
cargo publish -p sbmlsim-sbml
```

`cargo package -p sbmlsim-sbml` is the same without the publication: it packs the crate and compiles it outside the workspace, which is what a dependent sees. Nothing of the publication is automated yet, since [trusted publishing](https://crates.io/docs/trusted-publishing) can only be configured for a crate which exists: once it is set up for this crate, the publication belongs in `publish.yml` next to PyPI and npm, so that a tag makes all of a release.

`CORE_VERSION` of `js/panels.mjs` is the one version no check of a pull request compares, since `npm pack` runs only while the tag publishes; a release which gets it wrong fails after the tag exists.
