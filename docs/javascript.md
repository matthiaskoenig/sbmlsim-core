# JavaScript

`js/` is the JavaScript host of sbmlsim-core: plain ES modules without dependencies and without APIs of Node, so they run in Node and in browsers alike.

| module | export of `js/package.json` | what it is |
|---|---|---|
| `loader.mjs` | `.` | runs `engine.wasm`, `mathlib.wasm` and a `model.wasm` with the bits of wasmtime |
| `pool.mjs` (with `worker.mjs`) | `./pool` | a pool of workers which run the loader off the main thread |
| `panels.mjs` | `./panels` | the routes of the panel service of sbmlsim, answered inside a page from a bundle |
| `simulate.mjs` | the command `sbmlsim-core-simulate` | the CSV of `sbmlsim-core simulate` in Node |

The package is private (not published to npm); a page serves the files of `js/` and the `engine.wasm` and `mathlib.wasm` of a build (`sbmlsim-core runtime -o js/runtime`, see [Installation](installation.md#building)).

## The loader

```js
import { createRuntime } from "./js/loader.mjs";

// engine and mathlib: WebAssembly.Modules or their bytes
const runtime = await createRuntime({ engine, mathlib });
// modelWasm: a Module or bytes; modelJson: the text of model.json or its object
const instance = await runtime.instantiate(modelWasm, modelJson);
// values: a Float64Array, row major
const { columns, nRows, values } = instance.simulate(request);
```

`createRuntime({ engine, mathlib, console?, maxMemoryPages? })` compiles (or takes) `engine.wasm` and `mathlib.wasm` and checks their imports, exports and the custom section `sbmlsim.engine` of `engine.wasm` (the minimum of its memory and the version of its output). `runtime.modules` are the compiled modules, which a worker gets to create a runtime of its own.

`runtime.instantiate(modelWasm, description)` instantiates a memory, mathlib, the model and the engine, as the wasmtime host links them, and loads the description into the engine. It is asynchronous: bytes compile with `WebAssembly.compile`, which browsers require for modules above a few KB on the main thread.

An instance is one model in one engine, for one thread:

| method | what it gives |
|---|---|
| `instance.simulate(request)` | a request of `simulation/1` (an object or its JSON text); `{ columns, nRows, values }`, the values a `Float64Array` whose bits are the ones of wasmtime, NaN payloads too |
| `instance.stats()` | the counters of the integrator of the last simulation, by the names of `STATS_NAMES` |
| `instance.wasiCalls()` | the calls of the engine into WASI; the engine calls none while it loads or simulates |
| `instance.memoryBytes()` | the size of the memory of the instance |

An error of the engine is an `EngineError` with `kind` (`load`, `request`, `unsupported`, `integration` with `flag` and `time`, or `internal`); an instance which trapped (`internal`) is not used again. The engine gets WASI preview1 stubs: `fd_write` writes to the console, `proc_exit` throws a `WasiExit`, the environment is empty and the clock and the random bytes throw. The module also exports `toCsv` and `formatNumber` (the CSV of the command line), `MODEL_CONTRACT`, `ENGINE_OUTPUT_FORMAT`, `MAX_MEMORY_PAGES` (2 GiB) and `STATS_NAMES`.

In Node, `js/simulate.mjs` writes the CSV of `sbmlsim-core simulate` byte for byte, with `engine.wasm` and `mathlib.wasm` of `$SBMLSIM_CORE_RUNTIME`, else of `js/runtime`:

```bash
node js/simulate.mjs crates/host/tests/decay-model crates/host/tests/decay-model/request.json [-o out.csv]
```

## The worker pool

A simulation is one synchronous call of the engine. `pool.mjs` runs them in a pool of workers, each with a runtime of its own, so a page stays responsive and the points of a scan run in parallel:

```js
import { createPool } from "./js/pool.mjs";

const pool = await createPool({ engine, mathlib }); // WebAssembly.Modules or bytes
await pool.addModel("decay", modelWasm, modelJson);
const { columns, nRows, values } = await pool.simulate("decay", request, { signal });
pool.close();
```

| call | what it does |
|---|---|
| `createPool(options)` | a Promise of the pool; the options are `engine` and `mathlib` (`WebAssembly.Module`s or bytes), `workers` is the most workers (`defaultWorkers()`: the logical processors but one, 1 to 8), `createWorker(url)` creates one (a module Web Worker without), `workerUrl` its script (`WORKER_URL`, `worker.mjs` next to `pool.mjs`) |
| `pool.addModel(key, model, description)` | adds a model under a key: a `WebAssembly.Module` or the bytes of `model.wasm`, the text of `model.json` or its object; a Promise |
| `pool.hasModel(key)` | whether the pool has a model under a key |
| `pool.simulate(key, request, options?)` | a Promise of `{ columns, nRows, values }`, as `instance.simulate`; `options.signal` drops the request |
| `pool.close()` | terminates the workers and rejects every queued and running request |
| `pool.size`, `pool.modules` | the most workers, the compiled runtime |

`engine.wasm`, `mathlib.wasm` and every model are compiled once, on the thread which creates the pool, and posted to the workers as `WebAssembly.Module`s, which a structured clone shares instead of copying. The workers are created when the queue needs them; a model is posted to a worker before the first request of it the worker runs, once per worker. The requests run in the order they were made, one at a time per worker:

- A request whose `signal` aborts is rejected with the reason of the signal: a queued one is dropped and never runs, a running one finishes in its worker and its result is discarded.
- An error of the engine (`EngineError`) rejects only its request; a worker which crashes rejects the request it ran and is replaced.
- The values are a `Float64Array` whose buffer the worker transfers. The results are bitwise the ones of the main thread (`js/test/pool.test.mjs`).

In Node a worker is a worker thread: `worker.mjs` gets its port from `node:worker_threads` (a dynamic import which never runs in a browser), and the pool takes it through `createWorker`:

```js
import { Worker } from "node:worker_threads";
const pool = await createPool({
  engine,
  mathlib,
  createWorker: (url) => new Worker(url),
});
```

## The panel endpoint

[sbmlsim](https://matthiaskoenig.github.io/sbmlsim/) declares the interactive panels of a model (knobs, curves, the study data under them) and serves them with `python -m sbmlsim.serve`; the [sbmlsim-web](https://matthiaskoenig.github.io/sbmlsim-web/) components `<sbmlsim-panel>` and `<sbmlsim-app>` draw them. `panels.mjs` answers the same routes inside a page, from a bundle the service exports, with the simulations on a pool of workers, so an app of panels is a static page without a server.

```js
import { createPanelEndpoint } from "./sbmlsim-core/panels.mjs";

const endpoint = createPanelEndpoint({ bundle: "./bundles/glimepiride/bundle.json" });
// a Response, as fetch gives it
const response = await endpoint("local:glimepiride/api/models/glimepiride/app", {});
```

`createPanelEndpoint(options)` gives a function `(url, init) => Promise<Response>` with the signature of `fetch`. It loads lazily: `bundle.json` and the runtime at the first request, a model at the first simulation which needs it. Options:

| option | what it is |
|---|---|
| `bundle` | the URL of `bundle.json` (relative to the page, or a `URL`) |
| `runtime` | `{ engine, mathlib }`, the URLs of `engine.wasm` and `mathlib.wasm` (or their `WebAssembly.Module`s or bytes); `./runtime/engine.wasm` and `./runtime/mathlib.wasm` next to `panels.mjs` without |
| `workers` | the most workers of the pool, `defaultWorkers()` without |
| `createWorker` | creates a worker of the pool, see the pool |
| `pool` | a pool of `createPool` to run on instead of one of its own, e.g. shared by the endpoints of several bundles |
| `fetch` | loads the files of the bundle and the runtime, `globalThis.fetch` without |
| `maxRunBytes` | the bound of the cache of the runs, `MAX_RUN_BYTES` (64 MiB) without |

`endpoint.close()` terminates the workers of its own pool (not of a pool it was given). The module also exports `CORE_VERSION` (the version of `js/package.json`), `ENGINE` (`"sbmlsim-core <version>"`, the engine a provenance names), `BUNDLE_CONTRACT`, `PANEL_CONTRACT`, `MAX_RUN_BYTES`, `MAX_ID`, `knobValues(knob, n)`, `canonicalJson(value)` and `sha256Hex(text)`.

### The routes

The path is the part of the URL from `/api/` on, so `local:glimepiride/api/...` and `https://example.org/any/api/...` are answered alike. The model id is the one of the bundle; the query `variant` is a key of `answers` of the bundle (none is the key `""`).

The routes hang under `/api/models/{model}`:

| route | answer |
|---|---|
| `GET /app[?variant=V]` | the app (`app/1`), a 404 for a model without one |
| `GET /panels/{panel}[?variant=V]` | the declaration of the panel (`panel/1`) |
| `GET /panels/{panel}/data[?variant=V]` | the study data of the panel (`paneldata/1`) |
| `POST /panels/{panel}/simulate` | a simulation, body `{ values?, grid?, variant? }`, answer `panel/1` |

A simulation is checked as the service checks it: a knob the panel does not declare, a value outside the bounds of a range knob or none of the choices of a choice knob, a grid of fewer than 1 or more points than the `limits` of the declaration, and a grid over a choice knob of other points than its choices are refused with a 422. The knobs of the grid span the product of their values in C order (the first knob of the declaration slowest): a range knob of `n` points runs `min`, `(min * (n - 1 - i) + max * i) / (n - 1)` for `i` from 1 to `n - 2`, and `max` (`n` 1 runs `min`), a choice knob its choices. Every other knob takes its value, or its default.

A point is one request of `simulation/1`: the start, the end and `{"uniform": {"steps": points}}` of the run of the bundle, the pre-initialization of the run, of the target of every knob (its value times its factor, in the units of the model) and of its parameter (its value), sorted by id, the selections of the model and its settings with an absolute tolerance per state. The answer:

| field | what it holds |
|---|---|
| `contract` | `"panel/1"` |
| `spec_hash` | the sha256 of the canonical JSON (sorted keys, no whitespace, every character which is not ASCII escaped) of the resolved request: the model, the panel, the query `variant`, the declaration, the sha256 of the SBML, the value of every knob (its default for one the request leaves out), the grid, the engine and the settings |
| `x` | the times of the bundle, in the unit `units.x` |
| `dims` | one axis per knob of the grid: `{ id, label, unit, values }` |
| `curves` | per curve a row per point of the grid, the values of its selection times its factor; a value which is not finite is `null` |
| `units` | the units of the run of the bundle |
| `status` | `{ ok, points, failed }`: a point which does not integrate is a row of `null` in every curve and its flat index is in `failed` |
| `provenance` | `{ model, panel, model_sha256, variant, engine, sbmlsim, seconds, created }`: the variant the bundle simulated, `sbmlsim-core <version>`, the version of sbmlsim which exported the bundle |

The runs are shared: the points of one model, end and pre-initialization are one run, so the panels of a page which show one state, each with its own curves, cost one scan. A run in flight is joined, a finished one is kept in a cache bounded by bytes, the least recently used evicted first. A request whose `signal` aborts is rejected with its reason, as `fetch` is; a run is dropped when every request which waits for it aborted.

An error is `{"detail": "..."}` with the status of the service: 404 (a route, a model or a panel which does not exist, a model without an app), 405 (a route of another method, with `Allow`), 422 (a request which is refused) and 500 (a bundle or a model which does not load, a run which fails otherwise than by an integration). `spec_hash` identifies what this endpoint simulated; it is no hash of the service, whose engine and settings differ.

### The bundle, `panelbundle/1`

`python -m sbmlsim.serve --model id=module:attribute --export DIR` writes the bundle of every model it is given into `DIR/<model id>/`:

```text
DIR/<model id>/bundle.json
DIR/<model id>/models/<model key>/model.xml     the SBML the export writes
DIR/<model id>/models/<model key>/model.wasm    sbmlsim-core compile
DIR/<model id>/models/<model key>/model.json    sbmlsim-core compile
```

`bundle.json` holds what the service answers, so the endpoint resolves nothing itself:

- `contract` (`"panelbundle/1"`), `model` (the model id of the routes), `title`, `sbmlsim` (the version which exported it) and `created`.
- `answers`: per query `variant` (`""` and every variant the app is served in), the `app` (or `null`) and per panel its `declaration`, its `data` and its `run`.
- A `run`: the `variant` and the `model` (a key of `models`) it simulates, the `model_sha256` of its SBML, `start`, `end`, `points` (intervals, `points + 1` rows), the delivered `x`, the `units`, the `curves` (per curve id the `selection` and the `factor` from the unit of the model to the delivered one), the `preinit` (pairs in the units of the model) and the `knobs` (per knob id its `target` and `factor`, its `parameter` and its `unit`).
- `models`: per model key the `sbml` and the `compiled` directory (relative to `bundle.json`), its `variant`, the `selections` every panel run on it reads, the `settings` of `simulation/1` and the `absolute_tolerances` per state id, which the endpoint turns into a `per_state` tolerance in the order of the `states` of `model.json` (the scalar one for a state without one).

Nothing of the semantics of a simulation lives in JavaScript. The formulas of the derived targets of a panel are compiled into the model by the export: one parameter `panel_knob__<knob id>` per knob a formula reads and one initial assignment per derived target, so a model key is the variant, or the variant and a hash of its derived targets. The unit conversions are factors, and the times of the output are computed by the export.

### A static panel app

1. Export the bundle with sbmlsim:

    ```bash
    python -m sbmlsim.serve --model glimepiride=pkdb_models.models.glimepiride:MODEL --export site/bundles
    ```

2. Compile every model of the bundle into its directory (prepared first, see [Compiling a model](compiling.md#preparing-a-model)):

    ```bash
    for dir in site/bundles/glimepiride/models/*/; do
      uv run scripts/prepare_model.py "$dir/model.xml" -o "$dir/prepared.xml"
      sbmlsim-core compile "$dir/prepared.xml" -o "$dir"
    done
    ```

3. Copy the JavaScript host and the runtime next to the page, the runtime in `runtime/` next to `panels.mjs`:

    ```bash
    mkdir -p site/sbmlsim-core
    cp js/loader.mjs js/pool.mjs js/worker.mjs js/panels.mjs site/sbmlsim-core/
    sbmlsim-core runtime -o site/sbmlsim-core/runtime
    ```

4. Register the endpoint with sbmlsim-web and point the elements at it. An element whose `endpoint` starts with `local:` is answered by the function registered under that name; it waits for the registration, so the module script may run after the elements connected. `sbmlsim-web.iife.js` is the IIFE build of sbmlsim-web (its documentation has the URL of a pinned release), which defines the global `sbmlsimWeb`:

    ```html
    <script src="sbmlsim-web.iife.js"></script>
    <script type="module">
      import { createPanelEndpoint } from "./sbmlsim-core/panels.mjs";
      sbmlsimWeb.registerEndpoint("local:glimepiride", createPanelEndpoint({ bundle: "./bundles/glimepiride/bundle.json" }));
    </script>
    <sbmlsim-app endpoint="local:glimepiride" model="glimepiride"></sbmlsim-app>
    ```

5. Serve the directory from any static server, e.g. GitHub Pages. The workers are module workers, so the page is served over HTTP(S), not opened as a file. `sha256Hex` takes `crypto.subtle`, which a browser has only in a secure context (HTTPS or localhost), and a digest in plain JavaScript elsewhere.

The components, their attributes and `registerEndpoint` are documented with [sbmlsim-web](https://matthiaskoenig.github.io/sbmlsim-web/), which has the glimepiride app as an example.
