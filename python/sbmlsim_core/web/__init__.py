"""The JavaScript host of sbmlsim-core for a page which embeds everything it runs.

`script()` is the inline build of the JavaScript host (`js/inline.mjs`, built by `js/build.mjs`):
one classic script which defines the global `sbmlsimCore` with `createInlineEndpoint`, the panel
endpoint of a bundle held in memory, whose pool starts its workers from a `blob:` URL of the
embedded worker, so a page of one file runs the models compiled with `compile_sbml` without a
server, also from `file://`. The runtime it needs is `sbmlsim_core.runtime_wasm()`, and a page
which distributes it carries `sbmlsim_core.THIRD_PARTY`, the notices of the libraries the engine
links.
"""

from functools import cache
from importlib import resources

__all__ = ["SCRIPT", "script"]

#: the file of the inline build in this package
SCRIPT = "sbmlsim-core.js"


@cache
def script() -> str:
    """Get the inline build of the JavaScript host, see the module."""
    return resources.files(__package__).joinpath(SCRIPT).read_text(encoding="utf-8")
