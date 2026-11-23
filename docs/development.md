# Development

How a release of sbmlsim-core is made. The toolchain, the build of the engine and the tests are in [Installation](installation.md) and [Test suite](testsuite.md).

## Release

A release is made from `develop`. Since `develop` only accepts pull requests, the release is prepared on a branch and tagged once that pull request is merged. The tag `vx.y.z` starts `.github/workflows/publish.yml`, which publishes the python package to PyPI and the package to npm with trusted publishing and provenance, so the tag is the publication and cannot be moved or deleted afterwards.

1. branch off `develop`: `git switch -c release/x.y.z develop`
2. write the release notes in `release-notes/x.y.z.md`, which the job `check` of `publish.yml` requires
3. set the version in every place which carries it, which is not only the manifests:

    | Where | What compares it, and when |
    | --- | --- |
    | `Cargo.toml`, `[workspace.package]`, and `Cargo.lock`, which `cargo` refreshes | the job `check` of `publish.yml`, against the tag |
    | `js/package.json` and the two entries of `js/package-lock.json` | the job `check` of `publish.yml`, against the tag |
    | `CORE_VERSION` of `js/panels.mjs` | `js/prepack.mjs` at `npm pack`, which runs only while the tag publishes |
    | the `compiler` of `testdata/golden/model.json` | `cargo test --all-features`, the job `checks` of `ci.yml` |
    | the header and the `CORE_VERSION` of `python/sbmlsim_core/web/sbmlsim-core.js`, the inline build of the JavaScript host | `python/tests/test_web.py`, the job `python-bindings` of `ci.yml` |
    | the examples: the CDN of `js/README.md` and of `docs/javascript.md`, the compiler of `docs/simulating.md`, the four `tag = "vX.Y.Z"` of the git dependencies of `docs/rust.md` | nothing |

    A version left behind in the golden model fails the job `checks` and with it every job which builds a wheel, so a missed place looks like eight of the fourteen checks red while `testsuite-parity` and `testsuite-cases` stay green, the engine itself being unchanged.

4. the inline build is generated and is never edited by hand: `cd js && npm ci && node build.mjs`, then `node build.mjs --check`, which compares the committed build with its sources
5. make sure everything passes: `cargo fmt --check`, `cargo clippy --all-targets --all-features -- -D warnings` and `cargo test --all-features`, which builds `engine.wasm` and needs `WASI_SDK_PATH` (see [Installation](installation.md#building))
6. push the branch, open the pull request against `develop` and merge it once the checks are green
7. tag the merged commit on `develop` and push the tag:

    ```bash
    git switch develop
    git pull
    git tag vx.y.z
    git push origin vx.y.z
    ```

    Check the version before pushing: the tag publishes to PyPI and to npm, and a version of either registry cannot be taken back.

    The tag makes all three of a release: the python package on PyPI, the package on npm and the release of this repository, named `sbmlsim-core x.y.z` with the body of `release-notes/x.y.z.md`. The job `github_release` makes the last one once both registries carry the version, so a release of this repository never announces a version a registry does not have. Nothing of a release is made by hand.

8. bump the live demo of the documentation, which is vendored from the published npm package and so can only follow the tag: set `NPM_VERSION` of `scripts/demo_assets.py` to the new version, run `uv run scripts/demo_assets.py` to vendor the package again, compare it with `uv run scripts/demo_assets.py --check` and take `docs/demo/` through a pull request of its own. The page compiles its model itself with the vendored `compiler.wasm`, so nothing of the demo needs a local build. Nothing compares this one either, so a demo left behind keeps running the engine of the release before.

## The crate on crates.io

`sbmlsim-sbml`, the reader of SBML, is the one crate of the workspace which is published on [crates.io](https://crates.io/crates/sbmlsim-sbml): it holds nothing but its own sources and depends on two published crates, where the rest of the workspace builds `engine.wasm` and the solvers from files outside the crate directories. A reader of SBML is useful without the engine, and a git dependency is not: cargo clones the repository with its submodules, which fails on windows over the length of a path inside `third_party/sundials`.

The crate versions with the workspace, so a release publishes the version of its tag:

```bash
cargo publish -p sbmlsim-sbml
```

`cargo package -p sbmlsim-sbml` is the same without the publication: it packs the crate and compiles it outside the workspace, which is what a dependent sees. Nothing of the publication is automated yet, since [trusted publishing](https://crates.io/docs/trusted-publishing) can only be configured for a crate which exists: once it is set up for this crate, the publication belongs in `publish.yml` next to PyPI and npm, so that a tag makes all of a release.

`CORE_VERSION` of `js/panels.mjs` is the one version no check of a pull request compares, since `npm pack` runs only while the tag publishes; a release which gets it wrong fails after the tag exists.
