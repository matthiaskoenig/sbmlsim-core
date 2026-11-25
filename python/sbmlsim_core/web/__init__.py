"""The JavaScript host of sbmlsim-core for a page which embeds everything it runs.

`script()` is the inline build of the JavaScript host (`js/inline.mjs`, built by `js/build.mjs`):
one classic script which defines the global `sbmlsimCore` with `createInlineEndpoint`, the panel
endpoint of a bundle held in memory, whose pool starts its workers from a `blob:` URL of the
embedded worker, so a page of one file runs the models compiled with `compile_sbml` without a
server, also from `file://`. The runtime it needs is `sbmlsim_core.runtime_wasm()`, and a page
which distributes it carries `sbmlsim_core.THIRD_PARTY`, the notices of the libraries the engine
links.

`figure_script()` is the script of a static report: the inline build plus the renderer of the
figures (`sbmlsim_core.plot_wasm()`) as base64, which on `DOMContentLoaded` makes every
`<figure data-figure="KEY">` of the page interactive from the `figure/1` spec in its child
`<script type="application/json" data-figure-spec="KEY">`. A report writes it once beside its pages
(`<script src="figures.js">`); it loads nothing from the network and works from `file://`.
"""

import base64
from functools import cache
from importlib import resources

__all__ = ["SCRIPT", "figure_script", "script"]

#: the file of the inline build in this package
SCRIPT = "sbmlsim-core.js"


@cache
def script() -> str:
    """Get the inline build of the JavaScript host, see the module."""
    return resources.files(__package__).joinpath(SCRIPT).read_text(encoding="utf-8")


@cache
def figure_script() -> str:
    """Get the page script of a static report, see the module."""
    from sbmlsim_core import plot_wasm

    wasm = base64.b64encode(plot_wasm()).decode("ascii")
    return (
        script()
        + f'\n;(() => {{\nconst PLOT_WASM = "{wasm}";\n'
        + "const start = () => {\n"
        + "  const bytes = Uint8Array.from(atob(PLOT_WASM), (c) => c.charCodeAt(0));\n"
        + "  sbmlsimCore.hydrateFigures(document, { wasm: bytes }).catch((error) => console.warn(`figures stay static: ${error?.message ?? error}`));\n"
        + "};\n"
        + 'if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", start);\n'
        + "else start();\n"
        + "})();\n"
    )
