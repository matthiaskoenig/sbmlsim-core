"""The API of sbmlsim_core used as a caller uses it, which `ty` checks against the stub.

The file is checked and never run (`ty check python/typecheck`, the job `python bindings`).
A line which must be refused carries an ignore of its rule: with `error-on-warning` the
ignore of a line which is accepted is a diagnostic, so the signatures are held in both
directions.

What it cannot check: the default values (`optimize=True`, `jacobian="auto"`) are invisible to a
type checker, and `jacobian` is a plain `str` in the stub, so an invalid value is not refused.
Those are held by the tests of `python/tests`.
"""

from pathlib import Path
from typing import Any

import numpy
import sbmlsim_core
from sbmlsim_core import (
    CompiledModel,
    CompileError,
    FitProblem,
    Instance,
    IntegrationError,
    InternalError,
    LoadedModel,
    LoadError,
    PointError,
    PreparedSimulation,
    RequestError,
    Runtime,
    SbmlsimCoreError,
    SimulationError,
    SimulationOutput,
    UnsupportedError,
)


def use(sbml: str, request: dict[str, Any], fit: dict[str, Any]) -> None:
    """Calls every function and member of the package."""
    contract: str = sbmlsim_core.MODEL_CONTRACT
    version: str = sbmlsim_core.__version__
    simulation: str = sbmlsim_core.SIMULATION_CONTRACT
    fitting: str = sbmlsim_core.FIT_CONTRACT

    compiled: CompiledModel = sbmlsim_core.compile_sbml(sbml)
    sbmlsim_core.compile_sbml(sbml, optimize=False, jacobian="difference_quotient")
    sbmlsim_core.compile_sbml(sbml, parameter_jacobian=False)
    wasm: bytes = compiled.wasm
    description: dict[str, Any] = compiled.description

    runtime: Runtime = Runtime()
    Runtime(max_memory_bytes=1 << 30)
    Runtime(None)
    Runtime(cache=False)
    Runtime(cache_dir=Path("wasmtime-cache"))
    Runtime(cache_dir="wasmtime-cache")
    cache_dir: Path | None = runtime.cache_dir
    model: LoadedModel = runtime.load(wasm, description)
    loaded: dict[str, Any] = model.description
    values: dict[str, float] = model.initial_values()

    instance: Instance = model.instance()
    out: SimulationOutput = instance.simulate(request)
    columns: list[str] = out.columns
    table: numpy.typing.NDArray[numpy.float64] = out.values
    local = out.local_errors
    if local is not None:
        states: list[str] = local.states
        errors: numpy.typing.NDArray[numpy.float64] = local.errors
        weights: numpy.typing.NDArray[numpy.float64] = local.weights
        print(states, errors, weights)
    stats: dict[str, int | float] = instance.stats()
    prepared: PreparedSimulation = model.prepare(request, varying=["k"])
    n_values: int = prepared.n_values
    instance.simulate_prepared(prepared, [0.5])
    instance.simulate_prepared(model.prepare(request))

    answer: dict[str, Any] = model.fit(fit)
    model.fit(fit, threads=2)
    model.fit(fit, threads=None)

    objective: FitProblem = model.fit_problem(fit, threads=2)
    searched: list[str] = objective.parameters
    residuals_of: int = objective.n_residuals
    on: int = objective.threads
    from_point: list[float] = objective.start
    bounds: tuple[list[float], list[float]] = objective.bounds
    at_point: list[float] = objective.values(from_point)
    relative: float = objective.step(0, from_point)
    r: numpy.typing.NDArray[numpy.float64] = objective.residuals(from_point)
    c: float = objective.cost(from_point)
    j: numpy.typing.NDArray[numpy.float64] = objective.jacobian(from_point)
    of: str = objective.jacobian_of
    ran: int = objective.simulations

    results: list[SimulationOutput | PointError] = model.simulate_many([request])
    model.simulate_many([request], threads=4)
    model.simulate_many([request], threads=None)
    with model.batch_executor(threads=2) as executor:
        executor.simulate_many([request])
        executor.simulate_prepared(prepared, [[0.5], [0.8]])
        executor.simulate_plans([(prepared, [0.5]), (prepared, [0.8])])
    executor.close()
    for result in results:
        if isinstance(result, PointError):
            kind: str = result.kind
            message: str = result.message
            flag: int | None = result.flag
            time: float | None = result.time
        else:
            rows: list[str] = result.columns

    print(
        contract, version, simulation, loaded, values, columns, table, stats, cache_dir
    )


def errors(request: dict[str, Any], instance: Instance) -> None:
    """The hierarchy of the errors and the members of an integration error."""
    try:
        instance.simulate(request)
    except IntegrationError as error:
        flag: int = error.flag
        time: float = error.time
        print(flag, time)
    except SimulationError as error:
        print(error)
    except CompileError as error:
        base: SbmlsimCoreError = error
        print(base)


def hierarchy() -> None:
    """The base of every error, held by assignment (a changed base class fails here)."""
    simulation: SimulationError = IntegrationError()
    simulation = RequestError()
    simulation = UnsupportedError()
    simulation = InternalError()
    base: SbmlsimCoreError = simulation
    base = CompileError()
    base = LoadError()
    base = SimulationError()
    print(base)


def refused(model: LoadedModel, request: dict[str, Any], sbml: str) -> None:
    """What the stub must refuse."""
    model.simulate_many([request], 4)  # ty: ignore[too-many-positional-arguments]
    sbmlsim_core.compile_sbml(sbml, False)  # ty: ignore[too-many-positional-arguments]
    sbmlsim_core.compile_sbml(1)  # ty: ignore[invalid-argument-type]
    sbmlsim_core.compile_sbml(sbml, optimize="yes")  # ty: ignore[invalid-argument-type]
    Runtime(max_memory_bytes="big")  # ty: ignore[invalid-argument-type]
    model.simulate_many(request)  # ty: ignore[invalid-argument-type]
    model.simulate_many([request], threads=1.5)  # ty: ignore[invalid-argument-type]
    model.instance().simulate("request")  # ty: ignore[invalid-argument-type]
    wrong: int = model.initial_values()  # ty: ignore[invalid-assignment]
    print(wrong)


def the_runtime_and_the_inline_build() -> None:
    """The runtime and the inline build which a page of one file embeds."""
    from sbmlsim_core import web

    files: dict[str, bytes] = sbmlsim_core.runtime_wasm()
    engine: bytes = files["engine"]
    plot: bytes = sbmlsim_core.plot_wasm()
    sbmlsim_core.plot_wasm("svg")  # ty: ignore[too-many-positional-arguments]
    notices: str = sbmlsim_core.THIRD_PARTY
    script: str = web.script()
    page: str = web.figure_script()
    print(engine, plot, notices, script, page)


def a_polynomial_chaos_expansion(u: numpy.typing.NDArray[numpy.float64]) -> None:
    """The basis of an expansion, its design matrix and its surrogate."""
    basis: numpy.typing.NDArray[numpy.int64] = sbmlsim_core.chaos_expansion(
        22, 4, q=0.75, max_interaction=2
    )
    x: numpy.typing.NDArray[numpy.float64] = sbmlsim_core.chaos_design_matrix(
        u, basis, threads=4
    )
    coefficients = numpy.ones((len(basis), 3))
    values: numpy.typing.NDArray[numpy.float64] = sbmlsim_core.chaos_predict(
        u, basis, coefficients
    )
    sbmlsim_core.chaos_expansion(22, 4, 0.75)  # ty: ignore[too-many-positional-arguments]
    sbmlsim_core.chaos_design_matrix(u, basis, threads=1.5)  # ty: ignore[invalid-argument-type]
    print(x, values)


def the_figures(values: list[float]) -> None:
    """The SVG, PNG, geometry and picks of a figure and its colour scales."""
    contract: str = sbmlsim_core.FIGURE_CONTRACT
    spec = "{}"
    svg: str = sbmlsim_core.render_svg(spec)
    png: bytes = sbmlsim_core.render_png(spec, 72.0)
    default_png: bytes = sbmlsim_core.render_png(spec)
    geometry: dict[str, Any] = sbmlsim_core.figure_geometry(spec)
    hit: dict[str, Any] | None = sbmlsim_core.pick(spec, 1.0, 2.0, radius=4.0)
    colors: list[str] = sbmlsim_core.color_scale(
        "viridis", 0.0, 1.0, values, center=0.5, log=False, start=0.0, end=0.85
    )
    sbmlsim_core.render_png(spec, dpi="x")  # ty: ignore[invalid-argument-type]
    sbmlsim_core.render_svg(1)  # ty: ignore[invalid-argument-type]
    sbmlsim_core.pick(spec, 1.0)  # ty: ignore[missing-argument]
    sbmlsim_core.color_scale("viridis", 0.0, 1.0)  # ty: ignore[missing-argument]
    sbmlsim_core.color_scale("viridis", 0.0, 1.0, values, end="1")  # ty: ignore[invalid-argument-type]
    width: float = sbmlsim_core.text_width("Smith2001", 13.0, bold=True)
    sbmlsim_core.text_width("x")  # ty: ignore[missing-argument]
    print(contract, svg, png, default_png, geometry, hit, colors, width)
