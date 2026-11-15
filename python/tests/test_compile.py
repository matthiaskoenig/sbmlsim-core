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


@pytest.fixture(scope="module")
def decay_sbml() -> str:
    """The SBML of the decay model of the host tests."""
    return (ROOT / "crates/host/tests/decay.xml").read_text(encoding="utf-8")


@pytest.fixture(scope="module")
def level2_sbml() -> str:
    """A level 2 document, which the reader refuses."""
    return (ROOT / "crates/sbml/tests/data/level2.xml").read_text(encoding="utf-8")


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


def test_compile_refuses_a_model_with_an_algebraic_rule() -> None:
    with pytest.raises(sbmlsim_core.CompileError, match="algebraic"):
        sbmlsim_core.compile_sbml(ALGEBRAIC_SBML)


def test_compile_refuses_a_level_2_document(level2_sbml: str) -> None:
    with pytest.raises(sbmlsim_core.CompileError, match="only level 3"):
        sbmlsim_core.compile_sbml(level2_sbml)


def test_the_errors_are_one_family() -> None:
    assert issubclass(sbmlsim_core.CompileError, sbmlsim_core.SbmlsimCoreError)
    assert issubclass(sbmlsim_core.SbmlsimCoreError, Exception)


def test_the_stub_names_what_the_module_exposes() -> None:
    import ast

    from sbmlsim_core import _core

    stub = ast.parse((ROOT / "python/sbmlsim_core/_core.pyi").read_text())
    names = set()
    for node in stub.body:
        if isinstance(node, ast.AnnAssign) and isinstance(node.target, ast.Name):
            names.add(node.target.id)
        elif isinstance(node, (ast.ClassDef, ast.FunctionDef)):
            names.add(node.name)
    exposed = {n for n in dir(_core) if not n.startswith("_") or n == "__version__"}
    assert names == exposed
    stub_class = next(
        n for n in stub.body if isinstance(n, ast.ClassDef) and n.name == "CompiledModel"
    )
    members = {n.name for n in stub_class.body if isinstance(n, ast.FunctionDef)}
    assert members == {"wasm", "description"}
