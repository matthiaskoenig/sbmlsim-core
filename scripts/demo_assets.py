# /// script
# requires-python = ">=3.14"
# dependencies = []
# ///
"""Write the assets of the live demos of the documentation into `docs/demo/`.

The demos on [JavaScript](../docs/javascript.md) simulate the repressilator and
evaluate a small PEtab problem in the page of the reader. They need the JavaScript
loader, the two modules of the engine and the module of PEtab, which are vendored from
the published npm package at `NPM_VERSION` with the integrity of the registry checked,
and the compiler of the browser, which the page compiles the SBML committed next to
them with, so nothing of the demos is compiled ahead of time. Every file is committed
and the build of the documentation generates none of them, as with the tables of the
benchmarks and the grid of the test suite.

    uv run scripts/demo_assets.py            # write them
    uv run scripts/demo_assets.py --check    # compare them with VERSION.json
"""

from __future__ import annotations

import argparse
import base64
import hashlib
import json
import tarfile
import tempfile
import urllib.request
from pathlib import Path

# the release of the npm package the demo runs, raised with a release
NPM_VERSION = "0.45.0"
REGISTRY = "https://registry.npmjs.org/sbmlsim-core"

# what is taken out of the package, by its path in the tarball
VENDORED: dict[str, str] = {
    "package/compile.mjs": "compile.mjs",
    "package/loader.mjs": "loader.mjs",
    "package/petab.mjs": "petab.mjs",
    "package/runtime/compiler.wasm": "compiler.wasm",
    "package/runtime/engine.wasm": "engine.wasm",
    "package/runtime/mathlib.wasm": "mathlib.wasm",
}

# the files of the demos which are committed rather than vendored: the model of the
# simulation and the PEtab problem, whose SBML the page compiles with the vendored compiler
COMMITTED: tuple[str, ...] = (
    "repressilator.xml",
    "conversion/problem.yaml",
    "conversion/model.xml",
    "conversion/conditions.tsv",
    "conversion/experiments.tsv",
    "conversion/observables.tsv",
    "conversion/measurements.tsv",
    "conversion/parameters.tsv",
)

DEMO_DIR = Path("docs/demo")
VERSION_FILE = "VERSION.json"


def _sha256(path: Path) -> str:
    """Get the hex SHA-256 of a file.

    Args:
        path: The file.

    Returns:
        The digest as hex.
    """
    return hashlib.sha256(path.read_bytes()).hexdigest()


def fetch_package(version: str) -> bytes:
    """Download the tarball of the npm package and check its integrity.

    Args:
        version: The version of the package.

    Returns:
        The bytes of the tarball.

    Raises:
        SystemExit: The version is not published or the integrity does not match.
    """
    with urllib.request.urlopen(REGISTRY, timeout=60) as response:
        metadata = json.load(response)
    release = metadata.get("versions", {}).get(version)
    if release is None:
        raise SystemExit(f"sbmlsim-core@{version} is not on the npm registry")
    distribution = release["dist"]
    with urllib.request.urlopen(distribution["tarball"], timeout=120) as response:
        tarball = response.read()
    algorithm, _, expected = distribution["integrity"].partition("-")
    digest = base64.b64encode(hashlib.new(algorithm, tarball).digest()).decode()
    if digest != expected:
        raise SystemExit(
            f"the tarball of sbmlsim-core@{version} does not match its integrity"
        )
    return tarball


def vendor(tarball: bytes, out: Path) -> None:
    """Extract the files of the demo from the tarball of the package.

    Args:
        tarball: The bytes of the tarball.
        out: The directory of the demo.

    Raises:
        SystemExit: A file of `VENDORED` is missing from the package.
    """
    with tempfile.TemporaryDirectory() as directory:
        archive = Path(directory) / "package.tgz"
        archive.write_bytes(tarball)
        with tarfile.open(archive) as tar:
            for member, name in VENDORED.items():
                extracted = tar.extractfile(member)
                if extracted is None:
                    raise SystemExit(f"{member} is missing from the npm package")
                (out / name).write_bytes(extracted.read())


def digests(out: Path) -> dict[str, str]:
    """Get the digests of every asset of the demo.

    Args:
        out: The directory of the demo.

    Returns:
        The file names to their hex SHA-256, sorted.
    """
    names = [*VENDORED.values(), *COMMITTED]
    return {name: _sha256(out / name) for name in sorted(names)}


def check(out: Path) -> int:
    """Compare the assets on disk with the ones `VERSION.json` records.

    Args:
        out: The directory of the demo.

    Returns:
        The exit code, 0 when they agree.
    """
    recorded = json.loads((out / VERSION_FILE).read_text(encoding="utf-8"))
    problems: list[str] = []
    if recorded.get("npm_version") != NPM_VERSION:
        problems.append(
            f"{VERSION_FILE} records npm {recorded.get('npm_version')}, "
            f"the script pins {NPM_VERSION}"
        )
    files: dict[str, str] = recorded.get("files", {})
    unlisted = set(files) ^ {*VENDORED.values(), *COMMITTED}
    if unlisted:
        problems.append(
            f"{VERSION_FILE} and the script name different files: "
            f"{', '.join(sorted(unlisted))}"
        )
    for name, digest in files.items():
        path = out / name
        if not path.exists():
            problems.append(f"{name} is missing")
        elif _sha256(path) != digest:
            problems.append(f"{name} differs from the digest of {VERSION_FILE}")
    if problems:
        for problem in problems:
            print(f"demo assets: {problem}")
        print("run `uv run scripts/demo_assets.py` to write them again")
        return 1
    print(f"demo assets: {len(files)} files, npm {NPM_VERSION}")
    return 0


def main() -> int:
    """Run the command line tool.

    Returns:
        The exit code.
    """
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument(
        "--check", action="store_true", help="compare instead of writing"
    )
    parser.add_argument("--out", type=Path, default=DEMO_DIR)
    arguments = parser.parse_args()
    out: Path = arguments.out

    if arguments.check:
        return check(out)

    out.mkdir(parents=True, exist_ok=True)
    vendor(fetch_package(NPM_VERSION), out)
    (out / VERSION_FILE).write_text(
        json.dumps(
            {"npm_version": NPM_VERSION, "files": digests(out)},
            indent=2,
            sort_keys=True,
        )
        + "\n",
        encoding="utf-8",
    )
    print(f"demo assets: written into {out}, npm {NPM_VERSION}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
