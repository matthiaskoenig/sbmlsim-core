# /// script
# requires-python = ">=3.14"
# dependencies = [
#     "sbmlsim>=0.24.0",
#     "libroadrunner",
#     "python-libsbml",
# ]
#
# [tool.uv.sources]
# libroadrunner = { index = "roadrunner-dev" }
#
# [[tool.uv.index]]
# name = "roadrunner-dev"
# url = "https://matthiaskoenig.github.io/roadrunner-wheels/simple/"
# explicit = true
# ///
"""Prepare the timecourse cases of the SBML Test Suite for `sbmlsim-core testsuite`.

For every timecourse case of the suite at the commit `SUITE_COMMIT` of sbmlsim, the newest
encoding of the model is flattened (`comp`) and converted to SBML L3V2 with libsbml, and the
directory `<out>/<cid>/` gets `model.xml`, `case.json` and `expected.csv`. A case libsbml cannot
convert gets a `case.json` with `"not_converted": "<message>"` and no `model.xml`.

    uv run scripts/prepare_testsuite.py --out testdata/cases

The cases are generated, never committed. Rerunning replaces the directories of the cases.
"""

from __future__ import annotations

import argparse
import json
import shutil
import sys
from collections import Counter
from pathlib import Path

import libsbml
from sbmlsim.testsuite.cases import SUITE_COMMIT, SemanticCase, SemanticSuite


def _first_error(doc: libsbml.SBMLDocument) -> str:
    """Get the first error of a document as one line."""
    for index in range(doc.getNumErrors()):
        error = doc.getError(index)
        if error.getSeverity() >= libsbml.LIBSBML_SEV_ERROR:
            return " ".join(error.getMessage().split())[:300]
    return ""


def convert(path: Path) -> str:
    """Flatten and convert a model to SBML L3V2.

    Args:
        path: the SBML file, in any level and version.

    Returns:
        The SBML of the model in L3V2.

    Raises:
        ValueError: if libsbml cannot read, flatten or convert the model.
    """
    doc = libsbml.readSBMLFromFile(str(path))
    message = _first_error(doc)
    if doc.getModel() is None:
        raise ValueError(f"cannot read the model: {message or 'no model'}")
    if doc.getPlugin("comp") is not None and doc.getNumPlugins() > 0:
        props = libsbml.ConversionProperties()
        props.addOption("flatten comp", True)
        status = doc.convert(props)
        if status != libsbml.LIBSBML_OPERATION_SUCCESS:
            raise ValueError(
                f"cannot flatten comp: {libsbml.OperationReturnValue_toString(status)}"
                f" {_first_error(doc)}"
            )
    if (doc.getLevel(), doc.getVersion()) != (3, 2):
        if not doc.setLevelAndVersion(3, 2, False):
            raise ValueError(
                f"cannot convert L{doc.getLevel()}V{doc.getVersion()} to L3V2: "
                f"{_first_error(doc)}"
            )
    sbml = libsbml.writeSBMLToString(doc)
    if not sbml:
        raise ValueError("cannot write the converted model")
    return sbml


def case_json(case: SemanticCase) -> dict[str, object]:
    """Get the `case.json` of a case, without `not_converted`."""
    return {
        "cid": case.cid,
        "start": case.start,
        "duration": case.duration,
        "steps": case.steps,
        "selections": case.selections,
        "absolute_tolerance": case.absolute_tolerance,
        "relative_tolerance": case.relative_tolerance,
        "component_tags": sorted(case.component_tags),
        "test_tags": sorted(case.test_tags),
        "suite_commit": SUITE_COMMIT,
    }


def prepare(case: SemanticCase, out: Path) -> str | None:
    """Write the directory of a case.

    Args:
        case: the case.
        out: the directory of the cases.

    Returns:
        The message of a case which was not converted, else `None`.
    """
    target = out / case.cid
    if target.exists():
        shutil.rmtree(target)
    target.mkdir(parents=True)
    info = case_json(case)
    message: str | None = None
    try:
        (target / "model.xml").write_text(convert(case.model_path), encoding="utf-8")
    except Exception as err:  # every failure is an outcome of the case
        message = str(err).strip().splitlines()[0][:300] if str(err).strip() else repr(err)
        info["not_converted"] = message
    (target / "case.json").write_text(json.dumps(info, indent=2) + "\n", encoding="utf-8")
    shutil.copyfile(case.path / f"{case.cid}-results.csv", target / "expected.csv")
    return message


def main() -> int:
    """Run the command line."""
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--out", type=Path, default=Path("testdata/cases"))
    args = parser.parse_args()

    suite = SemanticSuite.load(SUITE_COMMIT)
    args.out.mkdir(parents=True, exist_ok=True)
    n_cases = 0
    reasons: Counter[str] = Counter()
    for case in suite.cases():
        n_cases += 1
        message = prepare(case, args.out)
        if message is not None:
            reasons[message] += 1
    (args.out / "SUITE_COMMIT").write_text(SUITE_COMMIT + "\n", encoding="utf-8")
    print(f"{n_cases} cases in {args.out}, {sum(reasons.values())} not converted")
    for message, count in reasons.most_common():
        print(f"  {count:4d}  {message}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
