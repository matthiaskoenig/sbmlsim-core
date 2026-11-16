"""The API of sbmlsim_core used as a caller uses it, which `ty` checks against the stub.

The file is checked and never run (`ty check python/typecheck`, the job `python bindings`).
A line which must be refused carries an ignore of its rule: with `error-on-warning` the
ignore of a line which is accepted is a diagnostic, so the signatures are held in both
directions.

What it cannot check: the default values (`optimize=True`, `jacobian="auto"`) are invisible to a
type checker, and `jacobian` is a plain `str` in the stub, so an invalid value is not refused.
Those are held by the tests of `python/tests`.
"""

from typing import Any

import numpy
import sbmlsim_core
from sbmlsim_core import (
    CompiledModel,
    CompileError,
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


def use(sbml: str, request: dict[str, Any]) -> None:
    """Calls every function and member of the package."""
    contract: str = sbmlsim_core.MODEL_CONTRACT
    version: str = sbmlsim_core.__version__
    simulation: str = sbmlsim_core.SIMULATION_CONTRACT

    compiled: CompiledModel = sbmlsim_core.compile_sbml(sbml)
    sbmlsim_core.compile_sbml(sbml, optimize=False, jacobian="difference_quotient")
    wasm: bytes = compiled.wasm
    description: dict[str, Any] = compiled.description

    runtime: Runtime = Runtime()
    Runtime(max_memory_bytes=1 << 30)
    Runtime(None)
    model: LoadedModel = runtime.load(wasm, description)
    loaded: dict[str, Any] = model.description
    values: dict[str, float] = model.initial_values()

    instance: Instance = model.instance()
    out: SimulationOutput = instance.simulate(request)
    columns: list[str] = out.columns
    table: numpy.typing.NDArray[numpy.float64] = out.values
    stats: dict[str, int] = instance.stats()
    prepared: PreparedSimulation = model.prepare(request, varying=["k"])
    n_values: int = prepared.n_values
    instance.simulate_prepared(prepared, [0.5])
    instance.simulate_prepared(model.prepare(request))

    results: list[SimulationOutput | PointError] = model.simulate_many([request])
    model.simulate_many([request], threads=4)
    model.simulate_many([request], threads=None)
    with model.batch_executor(threads=2) as executor:
        executor.simulate_many([request])
        executor.simulate_prepared(prepared, [[0.5], [0.8]])
    executor.close()
    for result in results:
        if isinstance(result, PointError):
            kind: str = result.kind
            message: str = result.message
            flag: int | None = result.flag
            time: float | None = result.time
        else:
            rows: list[str] = result.columns

    print(contract, version, simulation, loaded, values, columns, table, stats)


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
