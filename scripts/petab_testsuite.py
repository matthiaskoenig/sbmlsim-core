# /// script
# requires-python = ">=3.12"
# dependencies = []
# ///
"""The cases of the PEtab test suite for the tests of `crates/petab`.

    python scripts/petab_testsuite.py download

Downloads the pinned commit of the PEtab test suite (`COMMIT`, the `PETAB_SUITE_COMMIT` of
sbmlsim) and unpacks its cases (`petabtests/cases`: `v1.0.0` and `v2.0.0`) into
`~/.cache/sbmlsim/petab-test-suite/<commit>`, the cache sbmlsim uses as well, where
`crates/petab/tests/testsuite.rs` reads them (`SBMLSIM_PETAB_SUITE_PATH` names another
directory of cases). A commit which is cached already is not downloaded again.

The cases of PEtab v1 are run converted to PEtab v2 (`v1.0.0-converted`), which this script does
not do: the conversion is `petab.v2.petab1to2` of python, which sbmlsim's own
`scripts/petab_testsuite.py download` runs into the same cache. Without it the test runs the
cases of PEtab v2 and of the math.
"""

import argparse
import io
import os
import shutil
import sys
import tempfile
import urllib.request
import zipfile
from pathlib import Path

COMMIT = "d09985e7bb03280b461599a24337137533d931e2"
URL = "https://github.com/PEtab-dev/petab_test_suite/archive/{commit}.zip"


def cache_path(commit: str = COMMIT) -> Path:
    """The directory of the cases of a commit."""
    if "SBMLSIM_PETAB_SUITE_PATH" in os.environ:
        return Path(os.environ["SBMLSIM_PETAB_SUITE_PATH"])
    base = Path(os.environ.get("XDG_CACHE_HOME", Path.home() / ".cache"))
    return base / "sbmlsim" / "petab-test-suite" / commit


def download(commit: str = COMMIT) -> Path:
    """Downloads and unpacks the cases of a commit, once."""
    target = cache_path(commit)
    if (target / "v2.0.0").is_dir():
        print(f"the PEtab test suite {commit} is in {target}")
        return target
    with urllib.request.urlopen(URL.format(commit=commit), timeout=120) as response:
        archive = zipfile.ZipFile(io.BytesIO(response.read()))
    target.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=target.parent) as staging:
        prefix = None
        for name in archive.namelist():
            marker = "/petabtests/cases/"
            if marker in name:
                prefix = name[: name.index(marker) + len(marker)]
                break
        if prefix is None:
            sys.exit("the archive has no directory petabtests/cases")
        unpacked = Path(staging) / "cases"
        for member in archive.infolist():
            if not member.filename.startswith(prefix) or member.is_dir():
                continue
            relative = Path(member.filename[len(prefix) :])
            if relative.is_absolute() or ".." in relative.parts:
                sys.exit(f"the archive holds the path {member.filename}")
            path = unpacked / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            with archive.open(member) as source, path.open("wb") as sink:
                shutil.copyfileobj(source, sink)
        if target.exists():
            # a cache of the cases of PEtab v1 alone, which sbmlsim converted
            for entry in unpacked.iterdir():
                if not (target / entry.name).exists():
                    entry.rename(target / entry.name)
        else:
            unpacked.rename(target)
    print(f"the PEtab test suite {commit} is unpacked into {target}")
    return target


def main() -> None:
    """The command line."""
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("command", choices=["download"])
    parser.parse_args()
    download()


if __name__ == "__main__":
    main()
