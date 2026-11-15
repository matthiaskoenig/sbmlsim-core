"""Type stubs of the extension module `sbmlsim_core._core`."""

from collections.abc import Sequence
from typing import Any

import numpy
import numpy.typing

MODEL_CONTRACT: str
SIMULATION_CONTRACT: str
__version__: str

class SbmlsimCoreError(Exception):
    """The base class of every error of sbmlsim_core."""

class CompileError(SbmlsimCoreError):
    """An SBML model which cannot be compiled."""

class CompiledModel:
    """A model compiled to `model.wasm` and its description (`model/2`)."""

    @property
    def wasm(self) -> bytes:
        """The module `model.wasm`."""

    @property
    def description(self) -> dict[str, Any]:
        """The description of the model, `model.json` of `model/2`, as a dict."""

class SimulationError(SbmlsimCoreError):
    """The base class of the errors of a simulation."""

class RequestError(SimulationError):
    """A request which the engine refuses."""

class IntegrationError(SimulationError):
    """An integration which failed."""

    flag: int
    time: float

class UnsupportedError(SimulationError):
    """A simulation the engine does not support."""

class InternalError(SimulationError):
    """A failure of the engine itself."""

class PointError:
    """A point of a batch which failed: the error of its simulation as a value."""

    @property
    def kind(self) -> str:
        """The kind: `request`, `integration`, `unsupported` or `internal`."""

    @property
    def message(self) -> str:
        """The message of the error."""

    @property
    def flag(self) -> int | None:
        """The return flag of CVODE of an integration which failed, else none."""

    @property
    def time(self) -> float | None:
        """The time an integration which failed reached, else none."""

class SimulationOutput:
    """The table of a simulation."""

    @property
    def columns(self) -> list[str]:
        """The names of the columns, `time` first."""

    @property
    def values(self) -> numpy.typing.NDArray[numpy.float64]:
        """The values, of the shape `(n_rows, len(columns))`, in C order."""

class Instance:
    """A model in an engine of its own; simulates one request after the other."""

    def simulate(self, request: dict[str, Any]) -> SimulationOutput:
        """Simulates a request of `simulation/1` and gives its table."""

    def stats(self) -> dict[str, int]:
        """The counters of the integrator of the last simulation."""

class LoadedModel:
    """A model compiled for an engine, which makes the instances that simulate it."""

    @property
    def description(self) -> dict[str, Any]:
        """The description of the model, `model/2`, as a dict."""

    def initial_values(self) -> dict[str, float]:
        """The values of the selections of the model as loaded."""

    def instance(self) -> Instance:
        """A new instance of the model."""

    def simulate_many(
        self,
        requests: Sequence[dict[str, Any]],
        *,
        threads: int | None = None,
    ) -> list[SimulationOutput | PointError]:
        """Simulates every request on `threads` threads, in the order of the requests."""

class Runtime:
    """`engine.wasm` and `mathlib.wasm`, compiled once, which load models."""

    def __init__(self, max_memory_bytes: int | None = None) -> None:
        """The memory of an instance grows to at most `max_memory_bytes`, if given."""

    def load(self, wasm: bytes, description: dict[str, Any]) -> LoadedModel:
        """Loads `model.wasm` with its description."""

def compile_sbml(
    sbml: str, *, optimize: bool = True, jacobian: str = "auto"
) -> CompiledModel:
    """Compiles the text of an SBML Level 3 document into `model.wasm` and its description."""
