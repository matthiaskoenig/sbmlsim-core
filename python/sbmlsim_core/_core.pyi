"""Type stubs of the extension module `sbmlsim_core._core`."""

from typing import Any

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

def compile_sbml(
    sbml: str, *, optimize: bool = True, jacobian: str = "auto"
) -> CompiledModel:
    """Compiles the text of an SBML Level 3 document into `model.wasm` and its description."""
