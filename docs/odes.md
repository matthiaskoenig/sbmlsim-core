# ODE source exports

`sbmlsim-core` derives one equation system from SBML and renders Python/NumPy,
Diffrax/JAX, Julia, R, Markdown, LaTeX and Typst. The Rust crate `sbmlsim-ode`
owns the analysis, expression printers and templates. Native Python, the CLI,
and import-free browser WASM use that implementation. Generation needs neither
libSBML nor a Python template engine.

## Python

```python
from pathlib import Path
from sbmlsim_core import ODE_FORMATS, OdeSystem

system = OdeSystem.from_sbml(Path("model.xml"))
code = system.render("python", simulator=False)
system.write("model.py")
system.write("model_jax.py", fmt="diffrax")
system.write("model.md", symbols="name", standalone=False)
```

`ODE_FORMATS` describes the formats, suffixes and supported options. Code formats
accept `simulator` (default `True`). Document formats accept `standalone`
(default `True`) and `symbols` (`"id"` or `"name"`). Unknown options and incorrect
types fail. Both Python and Diffrax use `.py`; choose Diffrax explicitly.

Pass original XML text, bytes or a path. Original bytes preserve double precision;
serializing a libSBML document first can round its values. A trusted file path
resolves local external `comp` documents in Rust. HTTP sources are never fetched.
Raw XML accepts only companions supplied explicitly:

```python
system = OdeSystem.from_sbml(
    Path("main.xml").read_bytes(),
    location="./models/main.xml",
    documents={"./parts/child.xml": Path("parts/child.xml").read_bytes()},
    max_expanded_elements=10000,
)
```

Source locations resolve within the supplied root. The source set is limited to
100 MiB and 1,000 documents. The expanded-model budget also applies to nested
submodels. The portable core performs no filesystem or network access.

`system.typeset(links)` returns linked LaTeX equations as JSON-compatible data;
callers may supply opaque symbol, local-parameter and metadata links. It does not
require a report implementation. `system.render_template(path, fmt="python")`
renders a custom MiniJinja template with the same target context and embedded
format templates. A template has no filesystem or interpreter access.

## Command line

```sh
sbmlsim-core export model.xml --format python --no-simulator -o model.py
sbmlsim-core export model.xml --format markdown --symbols name -o model.md
```

Without `-o`, the CLI prints the generated content. `--fragment` omits the document
preamble; `--no-simulator` omits the integration driver.

## Browser and Node

```javascript
import { createCompiler } from "sbmlsim-core/compile";

const bytes = await fetch("compiler.wasm").then(response => response.arrayBuffer());
const compiler = await createCompiler({ compiler: bytes });
const file = compiler.exportOde(await sourceFile.arrayBuffer(), "diffrax", {
  simulator: false,
});
// file.filename and file.content are ready to save or display.
```

`exportOde` accepts source text or a BufferSource. Its options also include
`documents` (a mapping of source locations to XML strings), `location` and
`maxExpandedElements`. Reuse the compiler instance across exports. In an
interactive page, run generation in a worker so large models do not block input.
The module has no host imports and uses the same fixtures as the native bindings.

## Mathematical behavior and migration

States retain their declared amount or concentration conventions. Concentration
states include dilution for changing compartments. Initial assignments, local
kinetic parameters, assignment and rate rules, functions and simultaneous event
assignments share one resolved plan across targets. Generated event drivers retain
trigger-time values, delays, priorities and compartment-size conversion.

Documents report unsupported constructs with the equations. Executable exports
refuse constructs their generated solver cannot implement, including algebraic
rules and delay functions. This export layer does not change the existing
`model/2` simulation compiler or its amount-state convention.

This replaces the former `sbmlode` generator. Import `OdeSystem` from
`sbmlsim_core`. The migration fixtures preserve 56 complete generated files,
14,672 executable expression cases, and the LaTeX and Typst math corpus. Headers
now identify `sbmlsim-core`. Numerical tests execute the generated Python event
and variable-compartment models against their former outputs.
