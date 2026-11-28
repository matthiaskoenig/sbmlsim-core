"""Tests of `compile_sbml`."""

from pathlib import Path

import pytest
import sbmlsim_core

ROOT = Path(__file__).resolve().parents[2]

ALGEBRAIC_SBML = """<?xml version="1.0" encoding="UTF-8"?>
<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" level="3" version="2">
  <model id="m">
    <listOfParameters>
      <parameter id="k" value="1" constant="true"/>
    </listOfParameters>
    <listOfRules>
      <algebraicRule id="A">
        <math xmlns="http://www.w3.org/1998/Math/MathML"><ci>k</ci></math>
      </algebraicRule>
    </listOfRules>
  </model>
</sbml>"""


LEVEL2_SBML = """<?xml version="1.0" encoding="UTF-8"?>
<sbml xmlns="http://www.sbml.org/sbml/level2/version4" level="2" version="4">
  <model id="decay">
    <listOfParameters>
      <parameter id="y" value="1" constant="false"/>
      <parameter id="k" value="0.5"/>
    </listOfParameters>
    <listOfRules>
      <rateRule variable="y">
        <math xmlns="http://www.w3.org/1998/Math/MathML">
          <apply><times/><apply><minus/><ci>k</ci></apply><ci>y</ci></apply>
        </math>
      </rateRule>
    </listOfRules>
  </model>
</sbml>"""


@pytest.fixture(scope="module")
def decay_sbml() -> str:
    """The SBML of the decay model of the host tests."""
    return (ROOT / "crates/host/tests/decay.xml").read_text(encoding="utf-8")


@pytest.fixture(scope="module")
def level2_sbml() -> str:
    """The decay model as a level 2 version 4 document."""
    return LEVEL2_SBML


def test_compile_gives_a_module_and_a_description(decay_sbml: str) -> None:
    compiled = sbmlsim_core.compile_sbml(decay_sbml)
    assert compiled.wasm[:4] == b"\0asm"
    assert compiled.description["contract"] == sbmlsim_core.MODEL_CONTRACT
    assert [s["id"] for s in compiled.description["states"]] == ["y"]


def test_compile_options_do_not_change_the_description(decay_sbml: str) -> None:
    plain = sbmlsim_core.compile_sbml(
        decay_sbml, optimize=False, jacobian="difference_quotient"
    )
    assert plain.wasm[:4] == b"\0asm"
    assert plain.description["contract"] == sbmlsim_core.MODEL_CONTRACT


def test_compile_refuses_an_unknown_jacobian(decay_sbml: str) -> None:
    with pytest.raises(ValueError, match="jacobian"):
        sbmlsim_core.compile_sbml(decay_sbml, jacobian="symbolic")


def test_parameter_jacobian_can_be_omitted(decay_sbml: str) -> None:
    with_jacobian = sbmlsim_core.compile_sbml(decay_sbml)
    without_jacobian = sbmlsim_core.compile_sbml(decay_sbml, parameter_jacobian=False)
    assert with_jacobian.description.get("jacobian_p") is not None
    assert without_jacobian.description.get("jacobian_p") is None
    assert len(without_jacobian.wasm) < len(with_jacobian.wasm)


def test_compile_names_the_entities_an_algebraic_rule_cannot_determine() -> None:
    with pytest.raises(
        sbmlsim_core.CompileError,
        match=r"A determines no variable.*k is constant",
    ):
        sbmlsim_core.compile_sbml(ALGEBRAIC_SBML)


def test_compile_reads_a_level_2_document(level2_sbml: str) -> None:
    compiled = sbmlsim_core.compile_sbml(level2_sbml)
    assert compiled.wasm[:4] == b"\0asm"
    assert [s["id"] for s in compiled.description["states"]] == ["y"]


def test_loading_a_model_which_is_no_module_raises_a_load_error(
    decay_sbml: str,
) -> None:
    compiled = sbmlsim_core.compile_sbml(decay_sbml)
    with pytest.raises(sbmlsim_core.LoadError):
        sbmlsim_core.Runtime().load(b"not a module", compiled.description)


def test_the_errors_are_one_family() -> None:
    assert issubclass(sbmlsim_core.CompileError, sbmlsim_core.SbmlsimCoreError)
    assert issubclass(sbmlsim_core.LoadError, sbmlsim_core.SbmlsimCoreError)
    assert issubclass(sbmlsim_core.SbmlsimCoreError, Exception)


def test_the_stub_names_what_the_module_exposes() -> None:
    import ast

    from sbmlsim_core import _core

    stub = ast.parse(
        (ROOT / "python/sbmlsim_core/_core.pyi").read_text(encoding="utf-8")
    )
    names = set()
    for node in stub.body:
        if isinstance(node, ast.AnnAssign) and isinstance(node.target, ast.Name):
            names.add(node.target.id)
        elif isinstance(node, (ast.ClassDef, ast.FunctionDef)):
            names.add(node.name)
    reported = {"__version__", "__allocator__", "_EquationSystem"}
    exposed = {n for n in dir(_core) if not n.startswith("_") or n in reported}
    assert names == exposed
    stub_class = next(
        n
        for n in stub.body
        if isinstance(n, ast.ClassDef) and n.name == "CompiledModel"
    )
    members = {n.name for n in stub_class.body if isinstance(n, ast.FunctionDef)}
    assert members == {"wasm", "description"}


UPTAKE_SBML = """<?xml version="1.0" encoding="UTF-8"?>
<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" level="3" version="2">
  <model id="uptake">
    <listOfCompartments>
      <compartment id="blood" size="0.15" constant="true"/>
      <compartment id="cell" size="1" constant="true"/>
    </listOfCompartments>
    <listOfSpecies>
      <species id="X" compartment="blood" initialConcentration="2"
               hasOnlySubstanceUnits="false" boundaryCondition="false" constant="false"/>
      <species id="C" compartment="cell" initialConcentration="0"
               hasOnlySubstanceUnits="false" boundaryCondition="false" constant="false"/>
    </listOfSpecies>
    <listOfParameters>
      <parameter id="k" value="0.7" constant="true"/>
    </listOfParameters>
    <listOfReactions>
      <reaction id="R" reversible="false">
        <listOfReactants>
          <speciesReference species="X" stoichiometry="1" constant="true"/>
        </listOfReactants>
        <listOfProducts>
          <speciesReference species="C" stoichiometry="1" constant="true"/>
        </listOfProducts>
        <kineticLaw>
          <math xmlns="http://www.w3.org/1998/Math/MathML">
            <apply><times/><ci>k</ci><ci>blood</ci><ci>X</ci></apply>
          </math>
        </kineticLaw>
      </reaction>
    </listOfReactions>
  </model>
</sbml>"""


def test_compile_drives_species() -> None:
    compiled = sbmlsim_core.compile_sbml(UPTAKE_SBML, driven=["X"])
    description = compiled.description
    assert [s["id"] for s in description["states"]] == ["C", "X__exchange"]
    parameters = [p["id"] for p in description["parameters"]]
    (driven,) = description["driven"]
    assert driven["id"] == "X"
    assert parameters[driven["value"]] == "X__value"
    assert parameters[driven["slope"]] == "X__slope"
    assert parameters[description["t_ref"]] == "driven__t_ref"
    assert description["parameters"][driven["value"]]["initial"] == 2.0
    # without driven species the description has none
    assert "driven" not in sbmlsim_core.compile_sbml(UPTAKE_SBML).description
    # the exchange over [0, 2] along the path 2 + 0.5 t is -k blood times its integral
    model = sbmlsim_core.Runtime().load(compiled.wasm, description)
    result = model.instance().simulate(
        {
            "contract": sbmlsim_core.SIMULATION_CONTRACT,
            "start": 0.0,
            "end": 2.0,
            "output": {"uniform": {"steps": 2}},
            "preinit": [["X__slope", 0.5]],
            "changes": [],
            "steady_state": None,
            "selections": ["X__exchange", "C"],
            "settings": {
                "relative_tolerance": 1e-10,
                "absolute_tolerance": {"scalar": 1e-14},
                "maximum_num_steps": 20000,
                "maximum_time_step": 0.0,
                "initial_time_step": 0.0,
                "stiff": True,
                "linear_solver": "auto",
                "jacobian": "auto",
            },
        }
    )
    exchange = result.values[-1, result.columns.index("X__exchange")]
    exact = -0.7 * 0.15 * (2.0 * 2.0 + 0.5 * 2.0**2 / 2.0)
    assert exchange == pytest.approx(exact, rel=1e-8)
    assert result.values[-1, result.columns.index("C")] == pytest.approx(
        -exact, rel=1e-8
    )


def test_compile_refuses_an_unknown_driven_species() -> None:
    with pytest.raises(sbmlsim_core.CompileError, match="Q"):
        sbmlsim_core.compile_sbml(UPTAKE_SBML, driven=["Q"])
    with pytest.raises(TypeError):
        sbmlsim_core.compile_sbml(UPTAKE_SBML, driven="X")
