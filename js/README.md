# sbmlsim-core

Simulate SBML models in browsers and in Node. [sbmlsim-core](https://matthiaskoenig.github.io/sbmlsim-core/) compiles an SBML model into a WebAssembly module (`model.wasm` with its description `model.json`) and simulates it with its engine (`engine.wasm`: CVODE and IDA of SUNDIALS with the sparse solver KLU), with the semantics of [sbmlsim](https://matthiaskoenig.github.io/sbmlsim/) and bitwise the same results on every host. This package is its JavaScript host: plain ES modules without dependencies, the engine and its math library included.

| module | import | what it is |
|---|---|---|
| `loader.mjs` | `sbmlsim-core` | runs `engine.wasm`, `mathlib.wasm` and a compiled `model.wasm` |
| `pool.mjs` (with `worker.mjs`) | `sbmlsim-core/pool` | a pool of Web Workers (worker threads in Node) which run the loader off the main thread |
| `panels.mjs` | `sbmlsim-core/panels` | the routes of the panel service of sbmlsim, answered inside a page from an exported bundle |
| `simulate.mjs` | the command `sbmlsim-core-simulate` | the CSV of a simulation in Node |

A model is compiled ahead of time with the command line of the repository (`sbmlsim-core compile model.xml -o DIR`); compiling SBML in the browser is not part of this package yet.

## Install

```bash
npm install sbmlsim-core
```

or import the modules from a CDN, which serves them with the headers every origin may read:

```js
import { createPanelEndpoint } from "https://cdn.jsdelivr.net/npm/sbmlsim-core@0.11.0/panels.mjs";
```

A pool started from a module of another origin starts its workers through a module of the page's own origin, so the CDN works for the workers as well. The engine needs fixed-width SIMD of WebAssembly: Chrome 91, Firefox 89, Safari 16.4 or newer, Node 20 or newer.

## Simulate a model

```js
import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { createRuntime } from "sbmlsim-core";

// the engine and its math library are files of the package; a page fetches them from where it
// serves them
const file = (specifier) => readFile(fileURLToPath(import.meta.resolve(specifier)));
const runtime = await createRuntime({
  engine: await file("sbmlsim-core/runtime/engine.wasm"),
  mathlib: await file("sbmlsim-core/runtime/mathlib.wasm"),
});
// model.wasm and model.json of `sbmlsim-core compile model.xml -o model`
const instance = await runtime.instantiate(
  await readFile("model/model.wasm"),
  await readFile("model/model.json", "utf8"),
);
const { columns, nRows, values } = instance.simulate({
  contract: "simulation/1",
  start: 0,
  end: 100,
  output: { uniform: { steps: 100 } },
  preinit: [["k1", 0.5]],
  changes: [],
  steady_state: null,
  selections: ["time", "[S1]"],
  settings: {
    relative_tolerance: 1e-8,
    absolute_tolerance: { scalar: 1e-12 },
    maximum_num_steps: 20000,
    maximum_time_step: 0,
    initial_time_step: 0,
    stiff: true,
    linear_solver: "auto",
    jacobian: "auto",
  },
});
// values: a Float64Array of nRows rows of columns.length values, row major
```

A simulation is one synchronous call; in a page it belongs into a worker:

```js
import { createPool } from "sbmlsim-core/pool";

const pool = await createPool({ engine, mathlib }); // WebAssembly.Modules or their bytes
await pool.addModel("model", modelWasm, modelJson);
const result = await pool.simulate("model", request, { signal });
pool.close();
```

## A static app of interactive panels

`python -m sbmlsim.serve --model id=module:attribute --export DIR` of sbmlsim writes the panels and the app of a model as a bundle; `createPanelEndpoint` answers the routes of the service from it, with the simulations on a pool of workers, and the components of [sbmlsim-web](https://matthiaskoenig.github.io/sbmlsim-web/) draw them:

```html
<!-- the IIFE build of sbmlsim-web, a release with registerEndpoint -->
<script src="sbmlsim-web.iife.js"></script>
<script type="module">
  import { createPanelEndpoint } from "https://cdn.jsdelivr.net/npm/sbmlsim-core@0.11.0/panels.mjs";

  sbmlsimWeb.registerEndpoint(
    "local:glimepiride",
    createPanelEndpoint({ bundle: new URL("./bundle/bundle.json", location.href) }),
  );
</script>
<sbmlsim-app endpoint="local:glimepiride" model="glimepiride"></sbmlsim-app>
```

The [glimepiride app](https://matthiaskoenig.github.io/sbmlsim-web/examples/glimepiride/) is built this way. The [JavaScript page](https://matthiaskoenig.github.io/sbmlsim-core/javascript/) of the documentation describes every module, option and route.

## License

The JavaScript is MIT licensed. `runtime/engine.wasm` contains SUNDIALS (BSD-3-Clause) and KLU with AMD, COLAMD and BTF of SuiteSparse (LGPL-2.1-or-later), whose notices and sources are named in `runtime/THIRD_PARTY.md`; the package as a whole is `MIT AND BSD-3-Clause AND LGPL-2.1-or-later`.
