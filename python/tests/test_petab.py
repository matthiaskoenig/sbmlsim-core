"""The PEtab v2 problems of `sbmlsim_core.petab`."""

import math
from pathlib import Path

import pytest
from sbmlsim_core import UnsupportedError, petab

MODEL = """<?xml version="1.0" encoding="UTF-8"?>
<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" level="3" version="2">
  <model id="decay">
    <listOfCompartments><compartment id="c" size="1" constant="true"/></listOfCompartments>
    <listOfSpecies>
      <species id="A" compartment="c" initialConcentration="1"
        hasOnlySubstanceUnits="false" boundaryCondition="false" constant="false"/>
    </listOfSpecies>
    <listOfParameters><parameter id="k" value="0.5" constant="true"/></listOfParameters>
    <listOfReactions>
      <reaction id="decay" reversible="false">
        <listOfReactants>
          <speciesReference species="A" stoichiometry="1" constant="true"/>
        </listOfReactants>
        <kineticLaw><math xmlns="http://www.w3.org/1998/Math/MathML">
          <apply><times/><ci>k</ci><ci>A</ci></apply></math></kineticLaw>
      </reaction>
    </listOfReactions>
  </model>
</sbml>"""

YAML = """format_version: 2.0.0
parameter_files: [parameters.tsv]
model_files:
  decay: {location: model.xml, language: sbml}
observable_files: [observables.tsv]
measurement_files: [measurements.tsv]
"""

TIMES = [1.0, 2.0, 4.0]


def files(k: float = 0.3) -> dict[str, bytes]:
    """The problem of a decay of the rate `k` measured without noise."""
    measurements = "observableId\ttime\tmeasurement\n" + "".join(
        f"obs_a\t{t}\t{math.exp(-k * t)!r}\n" for t in TIMES
    )
    return {
        "problem.yaml": YAML.encode(),
        "model.xml": MODEL.encode(),
        "observables.tsv": b"observableId\tobservableFormula\tnoiseFormula\nobs_a\tA\t0.1\n",
        "measurements.tsv": measurements.encode(),
        "parameters.tsv": (
            b"parameterId\tlowerBound\tupperBound\tnominalValue\testimate\n"
            b"k\t0.01\t10\t0.5\ttrue\n"
        ),
    }


def test_a_problem_is_evaluated_at_its_nominal_values() -> None:
    problem = petab.Problem.from_files(
        files(), "problem.yaml", relative_tolerance=1e-10, absolute_tolerance=1e-12
    )
    assert problem.parameters == ["k"]
    assert problem.nominal == [0.5]
    assert problem.n_measurements == 3
    evaluation = problem.evaluate()
    simulated = [math.exp(-0.5 * t) for t in TIMES]
    assert evaluation["simulated"] == pytest.approx(simulated, rel=1e-7)
    measured = [math.exp(-0.3 * t) for t in TIMES]
    chi2 = sum(((m - y) / 0.1) ** 2 for m, y in zip(measured, simulated, strict=True))
    llh = -0.5 * chi2 - 3 * math.log(math.sqrt(2 * math.pi) * 0.1)
    assert evaluation["chi2"] == pytest.approx(chi2, rel=1e-6)
    assert evaluation["llh"] == pytest.approx(llh, rel=1e-6)
    assert problem.llh([0.5]) == evaluation["llh"]
    assert evaluation["simulation_table"].startswith("observableId\texperimentId")
    assert evaluation["failures"] == []


def test_a_problem_is_read_from_its_directory(tmp_path: Path) -> None:
    for name, content in files().items():
        (tmp_path / name).write_bytes(content)
    problem = petab.Problem.from_yaml(tmp_path / "problem.yaml")
    assert problem.n_simulations == 1
    (request,) = problem.requests()
    assert request["contract"] == "simulation/1"
    assert ["k", 0.5] in [list(value) for value in request["preinit"]]


def test_a_fit_finds_the_rate_of_the_data() -> None:
    problem = petab.Problem.from_files(files(), "problem.yaml")
    answer = problem.fit({"max_iterations": 100}, threads=2)
    assert answer["x"] == pytest.approx([0.3], rel=1e-5)
    # the data is the simulation: the objective is the normalization of the densities
    assert answer["objective"] == pytest.approx(
        3 * math.log(math.sqrt(2 * math.pi) * 0.1), abs=1e-6
    )
    fit_problem = problem.fit_problem(threads=2)
    cost = fit_problem.cost(fit_problem.start) - problem.cost_offset
    assert cost == pytest.approx(-problem.llh(), rel=1e-9)


def test_a_required_extension_is_refused() -> None:
    content = files()
    content["problem.yaml"] += (
        b"extensions:\n  other: {version: 1.0.0, required: true}\n"
    )
    with pytest.raises(UnsupportedError, match="'other'"):
        petab.Problem.from_files(content, "problem.yaml")


def test_a_problem_whose_parts_do_not_fit_is_refused() -> None:
    content = files()
    content["observables.tsv"] = (
        b"observableId\tobservableFormula\tnoiseFormula\nobs_a\tA * unknown\t0.1\n"
    )
    with pytest.raises(ValueError, match="'unknown'"):
        petab.Problem.from_files(content, "problem.yaml")
