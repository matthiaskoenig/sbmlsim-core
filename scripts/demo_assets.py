# /// script
# requires-python = ">=3.14"
# dependencies = []
# ///
"""Write the assets of the live demo of the documentation into `docs/demo/`.

The demo on [JavaScript](../docs/javascript.md) simulates the repressilator in the
page of the reader. It needs the JavaScript loader and the two modules of the engine,
which are vendored from the published npm package at `NPM_VERSION` with the integrity
of the registry checked, and the compiled model, which the command line of a local
build writes from the SBML committed next to it. Every file is committed and the
build of the documentation generates none of them, as with the tables of the
benchmarks and the grid of the test suite.

    uv run scripts/demo_assets.py            # write them
    uv run scripts/demo_assets.py --check    # compare them with VERSION.json
"""

from __future__ import annotations

import argparse
import base64
import hashlib
import json
import shutil
import subprocess
import tarfile
import tempfile
import urllib.request
from pathlib import Path

# the release of the npm package the demo runs, raised with a release
NPM_VERSION = "0.14.0"
REGISTRY = "https://registry.npmjs.org/sbmlsim-core"

# what is taken out of the package, by its path in the tarball
VENDORED: dict[str, str] = {
    "package/loader.mjs": "loader.mjs",
    "package/runtime/engine.wasm": "engine.wasm",
    "package/runtime/mathlib.wasm": "mathlib.wasm",
}

# the model of the demo and what the command line compiles it into
MODEL_SBML = "repressilator.xml"
COMPILED = ("model.wasm", "model.json")

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


def compile_model(cli: Path, out: Path) -> None:
    """Compile the model of the demo with the command line of a local build.

    Args:
        cli: The `sbmlsim-core` binary.
        out: The directory of the demo, which holds the SBML.

    Raises:
        SystemExit: The compilation failed.
    """
    with tempfile.TemporaryDirectory() as directory:
        result = subprocess.run(
            [str(cli), "compile", str(out / MODEL_SBML), "-o", directory],
            capture_output=True,
            text=True,
            check=False,
        )
        if result.returncode != 0:
            raise SystemExit(f"{cli} compile failed:\n{result.stderr}")
        for name in COMPILED:
            shutil.copyfile(Path(directory) / name, out / name)


def digests(out: Path) -> dict[str, str]:
    """Get the digests of every asset of the demo.

    Args:
        out: The directory of the demo.

    Returns:
        The file names to their hex SHA-256, sorted.
    """
    names = [*VENDORED.values(), MODEL_SBML, *COMPILED]
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
    for name, digest in recorded.get("files", {}).items():
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
    print(f"demo assets: {len(recorded.get('files', {}))} files, npm {NPM_VERSION}")
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
    parser.add_argument(
        "--cli",
        type=Path,
        default=Path("target/release/sbmlsim-core"),
        help="the command line which compiles the model",
    )
    arguments = parser.parse_args()
    out: Path = arguments.out

    if arguments.check:
        return check(out)

    out.mkdir(parents=True, exist_ok=True)
    vendor(fetch_package(NPM_VERSION), out)
    if not arguments.cli.exists():
        raise SystemExit(
            f"{arguments.cli} does not exist; build it with "
            "`cargo build --release -p sbmlsim-cli` or give --cli"
        )
    compile_model(arguments.cli, out)
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
