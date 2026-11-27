"""Exercise the installed native generator and the generated numerical programs."""

from __future__ import annotations

import concurrent.futures
import types
from pathlib import Path

import numpy as np
import pytest
import sbmlsim_core as core

ROOT = Path(__file__).resolve().parents[2]
FIXTURES = ROOT / "testdata" / "ode"
MODELS = ["demo", "repressilator", "events", "variable_compartment"]


@pytest.mark.parametrize("label", MODELS)
@pytest.mark.parametrize("fmt", list(core.ODE_FORMATS))
@pytest.mark.parametrize("complete", [False, True])
def test_complete_native_exports(label: str, fmt: str, complete: bool) -> None:
    """Compare each complete output through the actual compiled Python binding."""
    system = core.OdeSystem.from_sbml((FIXTURES / f"{label}.xml").read_bytes())
    option = "simulator" if core.ODE_FORMATS[fmt].kind == "code" else "standalone"
    actual = system.render(fmt, **{option: complete})
    expected = (FIXTURES / f"{label}.{fmt}_{str(complete).lower()}.txt").read_text(
        encoding="utf-8"
    )
    expected = expected.replace("sbmlode 0.3.3", f"sbmlsim-core {core.__version__}")
    assert actual == expected


def _module(source: str) -> types.ModuleType:
    """Import generated source as a user imports the downloaded model."""
    import sys

    module = types.ModuleType("generated_model")
    sys.modules[module.__name__] = module
    exec(compile(source, "generated_model.py", "exec"), module.__dict__)
    return module


@pytest.mark.parametrize("label", ["events", "variable_compartment"])
def test_generated_event_simulators_preserve_numerical_results(label: str) -> None:
    """Run actual event/dilution code against the previous complete simulator."""
    pytest.importorskip("scipy")
    pytest.importorskip("pandas")
    system = core.OdeSystem.from_sbml((FIXTURES / f"{label}.xml").read_bytes())
    current = _module(system.render("python"))
    reference = _module(
        (FIXTURES / f"{label}.python_true.txt").read_text(encoding="utf-8")
    )
    result = current.simulate(20, points=41)
    expected = reference.simulate(20, points=41)
    assert list(result.columns) == list(expected.columns)
    np.testing.assert_array_equal(result.to_numpy(), expected.to_numpy())


def test_shared_system_can_render_concurrently() -> None:
    """No interpreter or process-global printer state affects concurrent targets."""
    system = core.OdeSystem.from_sbml((FIXTURES / "events.xml").read_bytes())
    formats = list(core.ODE_FORMATS) * 3
    with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:
        actual = list(pool.map(system.render, formats))
    assert actual == [system.render(fmt) for fmt in formats]


def test_options_and_target_suffixes(tmp_path: Path) -> None:
    """Keep code/document options strict and make the shared .py suffix explicit."""
    system = core.OdeSystem.from_sbml((FIXTURES / "events.xml").read_bytes())
    with pytest.raises(ValueError, match="no option"):
        system.render("python", standalone=False)
    with pytest.raises(ValueError, match="bool"):
        system.render("python", simulator="false")
    with pytest.raises(ValueError, match="symbols"):
        system.render("latex", symbols="unknown")
    assert core.ODE_FORMATS["diffrax"].suffixes[0] == ".py"
    output = system.write(tmp_path / "model.py", fmt="diffrax", simulator=False)
    assert "import jax" in output.read_text(encoding="utf-8")


def test_typing_and_equation_links() -> None:
    """The caller decorates source identities without a report implementation here."""
    system = core.OdeSystem.from_sbml((FIXTURES / "events.xml").read_bytes())
    equations = system.typeset({"symbols": {"S": "model/Species:S"}})
    assert any(
        "\\htmlData{pk=model/Species:S}" in row["lhs"] for row in equations["odes"]
    )
    assert core.ODE_CONTRACT == "ode/1"


def test_generation_does_not_import_legacy_generator() -> None:
    """A normal installed core wheel has no dependency on the previous package."""
    import sys

    assert "sbmlode" not in sys.modules
    source = (FIXTURES / "events.xml").read_bytes()
    core.OdeSystem.from_sbml(source).render("markdown")
    assert "sbmlode" not in sys.modules


def _comp(model: str, external: str = "") -> bytes:
    return (
        '<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" '
        'xmlns:comp="http://www.sbml.org/sbml/level3/version1/comp/version1" '
        f'level="3" version="2" comp:required="true">{external}{model}</sbml>'
    ).encode()


def test_native_file_and_closed_source_comp_resolution(tmp_path: Path) -> None:
    """Trusted files resolve encoded companions; raw XML uses the explicit source set."""
    source = _comp(
        '<model id="main"><comp:listOfSubmodels><comp:submodel comp:id="child" '
        'comp:modelRef="external"/></comp:listOfSubmodels></model>',
        "<comp:listOfExternalModelDefinitions><comp:externalModelDefinition "
        'comp:id="external" comp:source="other%20model.xml"/>'
        "</comp:listOfExternalModelDefinitions>",
    )
    companion = _comp(
        '<model id="other"><listOfParameters><parameter id="p" '
        'value="1.2345678901234567" constant="false"/></listOfParameters>'
        '<listOfRules><rateRule variable="p"><math '
        'xmlns="http://www.w3.org/1998/Math/MathML"><cn>1</cn></math>'
        "</rateRule></listOfRules></model>"
    )
    path = tmp_path / "main.xml"
    path.write_bytes(source)
    other = tmp_path / "other model.xml"
    other.write_bytes(companion)
    with pytest.raises(ValueError):
        core.OdeSystem.from_sbml(source)
    closed = core.OdeSystem.from_sbml(
        source, documents={"./other model.xml": companion}
    ).render("python", simulator=False)
    native = core.OdeSystem.from_sbml(path).render("python", simulator=False)
    assert "child__p" in native
    assert "1.2345678901234567" in native
    assert "1.2345678901234567" in closed
    for uri in [other.as_uri(), other.as_uri().replace("file://", "file://localhost")]:
        path.write_bytes(source.replace(b"other%20model.xml", uri.encode()))
        assert "child__p" in core.OdeSystem.from_sbml(path).render("python")
    path.write_bytes(source.replace(b"other%20model.xml", b"missing.xml"))
    with pytest.raises(ValueError):
        core.OdeSystem.from_sbml(path)


def test_custom_templates_share_the_rust_context(tmp_path: Path) -> None:
    system = core.OdeSystem.from_sbml((FIXTURES / "events.xml").read_bytes())
    path = tmp_path / "custom.jinja"
    path.write_text(
        "{{ model.id }}: {% for state in states %}{{ state.id }};{% endfor %}"
    )
    rendered = system.render_template(path)
    assert rendered.startswith("events_model: ")
    assert "S;" in rendered
    path.write_text("{{ missing_field }}")
    with pytest.raises(ValueError, match="undefined"):
        system.render_template(path)


def test_original_double_precision_survives_source_generation() -> None:
    source = (
        (FIXTURES / "events.xml")
        .read_bytes()
        .replace(b'value="0.1"', b'value="1.2345678901234567"')
    )
    assert b"1.2345678901234567" in source
    assert "1.2345678901234567" in core.OdeSystem.from_sbml(source).render("python")


@pytest.mark.parametrize("label", ["demo", "events"])
def test_generated_diffrax_numerics_and_gradients(label: str) -> None:
    """Execute the migrated JAX solver and compare its values and derivatives."""
    jax = pytest.importorskip("jax")
    jnp = pytest.importorskip("jax.numpy")
    pytest.importorskip("diffrax")
    pytest.importorskip("pandas")
    system = core.OdeSystem.from_sbml((FIXTURES / f"{label}.xml").read_bytes())
    current = _module(system.render("diffrax"))
    reference = _module(
        (FIXTURES / f"{label}.diffrax_true.txt").read_text(encoding="utf-8")
    )
    times = jnp.linspace(0.0, 10.0, 21)
    actual = current.to_frame(current.simulate(times))
    expected = reference.to_frame(reference.simulate(times))
    assert list(actual.columns) == list(expected.columns)
    np.testing.assert_array_equal(actual.to_numpy(), expected.to_numpy())
    if label == "demo":

        def gradient(module: types.ModuleType) -> object:
            return jax.grad(lambda p: jnp.sum(module.simulate(times, p=p).x[-1]))(
                module.P0
            )

        np.testing.assert_array_equal(gradient(current), gradient(reference))
