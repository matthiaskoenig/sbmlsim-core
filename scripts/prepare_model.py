# /// script
# requires-python = ">=3.14,<3.15"
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
"""Prepare an SBML model for `sbmlsim-core compile`.

The compiler reads SBML core of every level and version and refuses every required package but
`distrib`, whose elements it reads and ignores. A model with `comp` is prepared with libsbml:
`comp` is flattened, the elements and annotations of `distrib` are stripped (which the compiler
does not need; a document without them is smaller), and the model is converted to L3V2, which
the compiler no longer needs and which keeps a prepared model comparable to the ones of
earlier runs. The uncertainties of `distrib` (`<distrib:listOfUncertainties>` of the
parameters of a fit) are metadata and do not change a simulation; a distribution function of
`distrib` in a math (`normal(0, 1)` as a csymbol) would change it, so such a model is refused,
naming the element, as the compiler refuses it. The rest of the model (non-required packages
such as `fbc` and `layout`, notes, annotations) is kept.

    uv run scripts/prepare_model.py model.xml -o prepared.xml
    uv run scripts/prepare_model.py --bench bench/models.toml

With `--bench`, every model of the benchmark is prepared from its `source` into its `sbml`
(paths relative to the directory of the toml file, `target/bench-models/` of the
repository), with the absolute tolerance of every state as roadrunner scales the scalar one of
the model (`<name>.atol.json`, by the ids of roadrunner's state vector): `sbmlsim:<file>` is a packaged model of the installed sbmlsim, an `https://` URL is
downloaded (a URL pinned to a commit), `<checkout>:<path>` is a file of a checkout named in the
table `[checkouts]` (the environment variable `SBMLSIM_CORE_CHECKOUT_<NAME>` overrides its path)
and any other source is a path relative to the toml file. `prepared.json` next to the prepared
files records every source with its sha256 (and the commit of a checkout).
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import subprocess
import sys
import tomllib
import urllib.request
from importlib import resources
from pathlib import Path

import libsbml

DISTRIB_SYMBOLS = "http://www.sbml.org/sbml/symbols/distrib/"


class PreparationError(ValueError):
    """The model cannot be prepared."""


def _first_error(doc: libsbml.SBMLDocument) -> str:
    """Get the first error of a document as one line."""
    for index in range(doc.getNumErrors()):
        error = doc.getError(index)
        if error.getSeverity() >= libsbml.LIBSBML_SEV_ERROR:
            return " ".join(error.getMessage().split())[:300]
    return ""


def _name(element: libsbml.SBase) -> str:
    """Get a name of an element for a message: its type and its id, or the id of its parent."""
    kind = element.getElementName()
    if element.isSetIdAttribute() and element.getIdAttribute():
        return f"{kind} {element.getIdAttribute()}"
    for getter in ("getSymbol", "getVariable"):
        target = getattr(element, getter, None)
        if target is not None and target():
            return f"{kind} of {target()}"
    parent = element.getParentSBMLObject()
    while parent is not None and not (
        parent.isSetIdAttribute() and parent.getIdAttribute()
    ):
        parent = parent.getParentSBMLObject()
    if parent is not None:
        return f"{kind} of {parent.getElementName()} {parent.getIdAttribute()}"
    return kind


def _in_distrib(element: libsbml.SBase) -> bool:
    """Whether an element is an element of distrib or inside one."""
    while element is not None:
        if element.getPackageName() == "distrib":
            return True
        element = element.getParentSBMLObject()
    return False


def distrib_functions(doc: libsbml.SBMLDocument) -> list[str]:
    """Get the elements whose math uses a function of `distrib`, with the function."""
    found = []
    elements = doc.getListOfAllElements()
    for index in range(elements.getSize()):
        element = elements.get(index)
        if not hasattr(element, "isSetMath") or not element.isSetMath():
            continue
        if _in_distrib(element):
            # the math of an uncertainty of distrib describes a parameter, it is stripped
            continue
        mathml = libsbml.writeMathMLToString(element.getMath()) or ""
        position = mathml.find(DISTRIB_SYMBOLS)
        if position >= 0:
            function = mathml[position + len(DISTRIB_SYMBOLS) :].split('"', 1)[0]
            found.append(f"{_name(element)} ({function})")
    return found


def prepare(path: Path) -> str:
    """Prepare a model: flatten `comp`, strip `distrib`, convert to L3V2.

    Args:
        path: the SBML file, in any level and version.

    Returns:
        The SBML of the prepared model.

    Raises:
        PreparationError: if libsbml cannot read, flatten or convert the model, or if a math of
            the model uses a distribution function of `distrib`.
    """
    doc = libsbml.readSBMLFromFile(str(path))
    if doc.getModel() is None:
        raise PreparationError(
            f"cannot read the model: {_first_error(doc) or 'no model'}"
        )
    if doc.getPlugin("comp") is not None:
        props = libsbml.ConversionProperties()
        props.addOption("flatten comp", True)
        status = doc.convert(props)
        if status != libsbml.LIBSBML_OPERATION_SUCCESS:
            raise PreparationError(
                f"cannot flatten comp: {libsbml.OperationReturnValue_toString(status)}"
                f" {_first_error(doc)}"
            )
    used = distrib_functions(doc)
    if used:
        raise PreparationError(
            "a math uses a distribution function of distrib, which would change the "
            f"simulation: {', '.join(used)}"
        )
    for index in reversed(range(doc.getNumPlugins())):
        uri = doc.getPlugin(index).getURI()
        if "/distrib/" in uri:
            prefix = doc.getPlugin(index).getPrefix()
            if (
                doc.enablePackage(uri, prefix, False)
                != libsbml.LIBSBML_OPERATION_SUCCESS
            ):
                raise PreparationError(f"cannot strip the package distrib ({uri})")
    if (doc.getLevel(), doc.getVersion()) != (3, 2) and not doc.setLevelAndVersion(
        3, 2, False
    ):
        raise PreparationError(
            f"cannot convert L{doc.getLevel()}V{doc.getVersion()} to L3V2: {_first_error(doc)}"
        )
    sbml = libsbml.writeSBMLToString(doc)
    if not sbml:
        raise PreparationError("cannot write the prepared model")
    if "distrib" in sbml.split(">", 2)[1]:
        raise PreparationError(
            "the package distrib is still declared after stripping it"
        )
    return sbml


def sha256(path: Path) -> str:
    """Get the sha256 of a file."""
    return hashlib.sha256(path.read_bytes()).hexdigest()


def resolve_source(
    source: str, base: Path, checkouts: dict[str, str], downloads: Path
) -> tuple[Path, dict[str, str]]:
    """Get the file of a source of the benchmark and what records it."""
    record: dict[str, str] = {"source": source}
    if source.startswith("sbmlsim:"):
        import sbmlsim

        path = Path(
            str(
                resources.files("sbmlsim.resources")
                / "models"
                / source.split(":", 1)[1]
            )
        )
        record["sbmlsim"] = sbmlsim.__version__
    elif source.startswith("https://"):
        downloads.mkdir(parents=True, exist_ok=True)
        path = (
            downloads
            / hashlib.sha256(source.encode()).hexdigest()[:16]
            / Path(source).name
        )
        if not path.exists():
            path.parent.mkdir(parents=True, exist_ok=True)
            with urllib.request.urlopen(source, timeout=60) as response:
                path.write_bytes(response.read())
    elif ":" in source and source.split(":", 1)[0] in checkouts:
        name, relative = source.split(":", 1)
        variable = f"SBMLSIM_CORE_CHECKOUT_{name.upper()}"
        root = Path(os.environ.get(variable) or (base / checkouts[name])).resolve()
        path = root / relative
        commit = subprocess.run(
            ["git", "-C", str(root), "rev-parse", "HEAD"],
            check=False,
            capture_output=True,
            text=True,
        )
        if commit.returncode == 0:
            record["commit"] = commit.stdout.strip()
    else:
        path = base / source
    if not path.is_file():
        raise PreparationError(f"the source {source} is no file ({path})")
    record["sha256"] = sha256(path)
    return path, record


def atol_path(sbml: Path) -> Path:
    """Get the file of the absolute tolerances of a prepared model of the benchmark."""
    return sbml.with_suffix(".atol.json")


def roadrunner_atol(
    sbml: Path, absolute_tolerance: float, end: float, preinit: list[tuple[str, float]]
) -> dict[str, float]:
    """Get the absolute tolerance of every state as roadrunner applies a scalar one.

    roadrunner scales the absolute tolerance per state (by the size of the compartment of a
    species of a concentration, by the initial value of the variable of a rate rule), so the
    vector depends on the pre-initialization, which is applied as the benchmark applies it
    (`init(x)`, then `resetAll`); the vector of the integrator after a simulation, by the ids of
    the state vector.
    """
    import roadrunner

    options = roadrunner.LoadSBMLOptions()
    options.setLLVMBackend(roadrunner.LoadSBMLOptions.LLJIT)
    # O2 as the benchmark loads it: the volumes the vector is scaled with are computed by the
    # code of the model, whose last bits depend on the level
    options.setLLJitOptimizationLevel(roadrunner.LoadSBMLOptions.DEFAULT)
    r = roadrunner.RoadRunner(str(sbml), options)
    r.integrator.setValue("absolute_tolerance", absolute_tolerance)
    for key, value in preinit:
        r.setValue(f"init({key})", value)
    r.resetAll()
    r.simulate(0, end, 3)
    vector = r.integrator.getAbsoluteToleranceVector()
    return {r.model.getStateVectorId(i): float(vector[i]) for i in range(len(vector))}


def prepare_bench(toml: Path) -> int:
    """Prepare every model of a benchmark file; the number of models which failed."""
    config = tomllib.loads(toml.read_text(encoding="utf-8"))
    base = toml.parent
    checkouts = config.get("checkouts", {})
    failed = 0
    records = {}
    out_dirs = set()
    for name, model in config["models"].items():
        target = base / model["sbml"]
        out_dirs.add(target.parent)
        try:
            source, record = resolve_source(
                model["source"], base, checkouts, target.parent / "downloads"
            )
            sbml = prepare(source)
        except PreparationError as error:
            print(f"error: {name}: {error}", file=sys.stderr)
            failed += 1
            continue
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(sbml, encoding="utf-8")
        record["prepared_sha256"] = sha256(target)
        atol = roadrunner_atol(
            target, model["absolute_tolerance"], model["end"], model.get("preinit", [])
        )
        atol_path(target).write_text(
            json.dumps(atol, indent=1) + "\n", encoding="utf-8"
        )
        records[name] = record
        print(f"{name}: {target}")
    for out in out_dirs:
        (out / "prepared.json").write_text(
            json.dumps(records, indent=2) + "\n", encoding="utf-8"
        )
    return failed


def main() -> int:
    """Run the command line."""
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("model", nargs="?", type=Path, help="the SBML file")
    parser.add_argument("-o", "--output", type=Path, help="the prepared SBML file")
    parser.add_argument(
        "--bench", type=Path, help="prepare every model of a benchmark file"
    )
    args = parser.parse_args()
    if args.bench is not None:
        return 1 if prepare_bench(args.bench) else 0
    if args.model is None or args.output is None:
        parser.error("a model and --output, or --bench")
    try:
        sbml = prepare(args.model)
    except PreparationError as error:
        print(f"error: {args.model}: {error}", file=sys.stderr)
        return 1
    args.output.write_text(sbml, encoding="utf-8")
    return 0


if __name__ == "__main__":
    sys.exit(main())
