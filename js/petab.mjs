// PEtab v2 in a JavaScript host: a problem read, simulated and evaluated in a page.
//
// Plain ECMAScript without dependencies and without APIs of Node, so it runs in browsers as well:
//
//   import { createCompiler } from "sbmlsim-core/compile";
//   import { createRuntime } from "sbmlsim-core";
//   import { openPetab } from "sbmlsim-core/petab";
//   const compiler = await createCompiler({ compiler: compilerWasm });
//   const runtime = await createRuntime({ engine, mathlib }); // or a pool of `pool.mjs`
//   const problem = await openPetab({ compiler, runtime, files }); // {name: text or bytes}
//   const { llh, chi2, logPrior, simulationTable } = await problem.evaluate(); // the nominal values
//   const simulations = await problem.simulations(x); // the results of the requests at x
//   problem.close();
//
// What a problem is, its plan and its likelihood are the Rust of `sbmlsim-petab` in
// `compiler.wasm` (the calls `petab_open`, `petab_model`, `petab_requests`, `petab_evaluate`
// and `petab_close` of `crates/wasm/src/petab.rs`), the problem of the command line
// (`sbmlsim-core petab`) and of python (`sbmlsim_core.petab`); this module only simulates the
// requests of the plan with the engine of the page, the loader of `loader.mjs` on the thread
// of the caller or the workers of `pool.mjs`. Since that engine is `engine.wasm`, which the
// command line runs as well, an evaluation in a page gives the bits of the command line.

import { EngineError } from "./loader.mjs";

/** The exports of `compiler.wasm` a problem needs. */
const PETAB_EXPORTS = [
  "allocate",
  "deallocate",
  "memory",
  "petab_open",
  "petab_model",
  "petab_requests",
  "petab_evaluate",
  "petab_close",
];

/** The most simulations of a problem one evaluation queues on a pool at once. */
const POOL_WINDOW = 256;

/** A problem which `compiler.wasm` refuses to read, plan or compile, with its message. */
export class PetabError extends Error {
  constructor(message) {
    super(message);
    this.name = "PetabError";
  }
}

/** A number of an answer of the module: a number, or `"NaN"`, `"Infinity"`, `"-Infinity"`. */
function number(value) {
  return typeof value === "number" ? value : Number(value);
}

/** The numbers of an answer as a `Float64Array`. */
function numbers(values) {
  return Float64Array.from(values, number);
}

/** The text of a file: a string as it is, bytes as UTF-8. */
function fileText(name, content) {
  if (typeof content === "string") return content;
  if (content instanceof ArrayBuffer || ArrayBuffer.isView(content)) {
    return new TextDecoder("utf-8", { fatal: true }).decode(content);
  }
  throw new TypeError(`the file ${name} is neither text nor bytes`);
}

/** The name of the YAML file of a problem: the one given, or the one file of `.yaml`/`.yml`. */
function yamlName(names, yaml) {
  if (yaml !== undefined) {
    if (!names.includes(yaml)) throw new PetabError(`the files have no ${yaml}`);
    return yaml;
  }
  const candidates = names.filter((name) => /\.ya?ml$/i.test(name));
  if (candidates.length !== 1) {
    throw new PetabError(
      `name the YAML file of the problem (yaml), the files have ${candidates.length} of them`,
    );
  }
  return candidates[0];
}

/** The options of `petab_open` from those of `openPetab`. */
function openOptions({ relativeTolerance, absoluteTolerance, dataAtChange, extension, ...rest }) {
  const unknown = Object.keys(rest);
  if (unknown.length > 0) throw new TypeError(`unknown options ${unknown.join(", ")}`);
  const options = {};
  if (relativeTolerance !== undefined) options.relative_tolerance = relativeTolerance;
  if (absoluteTolerance !== undefined) options.absolute_tolerance = absoluteTolerance;
  if (dataAtChange !== undefined) options.data_at_change = dataAtChange;
  if (extension !== undefined) options.extension = extension;
  return options;
}

/** The calls of the PEtab interface of one instance of `compiler.wasm`. */
class Interface {
  constructor(compiler) {
    const exports = compiler?.exports;
    const missing = PETAB_EXPORTS.filter((name) => exports?.[name] === undefined);
    if (missing.length > 0) {
      throw new Error(
        `compiler.wasm does not export ${missing.join(", ")}: it is built without the feature petab`,
      );
    }
    this.exports = exports;
  }

  /** Copies bytes into a new buffer of the module: `[pointer, length]`, `[0, 0]` for none. */
  put(bytes) {
    if (bytes.length === 0) return [0, 0];
    const pointer = this.exports.allocate(bytes.length);
    new Uint8Array(this.exports.memory.buffer, pointer, bytes.length).set(bytes);
    return [pointer, bytes.length];
  }

  /** The bytes of a buffer the module gave, copied, and the buffer freed. */
  take(pointer, length) {
    if (pointer === 0 || length === 0) return new Uint8Array(0);
    const bytes = new Uint8Array(this.exports.memory.buffer, pointer, length).slice();
    this.exports.deallocate(pointer, length);
    return bytes;
  }

  /**
   * Calls a function of the module which writes `words` words of buffers to `out`, with the
   * arguments `head` and the buffers of `inputs`; gives the status and the buffers it wrote.
   */
  call(name, head, inputs, words) {
    const { allocate, deallocate } = this.exports;
    const buffers = inputs.map((bytes) => this.put(bytes));
    const out = allocate(4 * words);
    try {
      const status = this.exports[name](...head, ...buffers.flat(), out);
      const header = Array.from(new Uint32Array(this.exports.memory.buffer, out, words));
      const answers = [];
      for (let k = 0; k < words; k += 2) answers.push(this.take(header[k], header[k + 1]));
      return { status, answers };
    } finally {
      deallocate(out, 4 * words);
      for (const [pointer, length] of buffers) if (pointer !== 0) deallocate(pointer, length);
    }
  }

  /** A call of a JSON answer: the parsed answer, or the message as a `PetabError`. */
  json(name, head, inputs) {
    const { status, answers } = this.call(name, head, inputs, 2);
    const text = new TextDecoder().decode(answers[0]);
    if (status !== 1) throw new PetabError(text || `${name} failed`);
    return JSON.parse(text);
  }
}

/** Runs `work` over `items` with at most `window` of them at once, in order of the results. */
async function windowed(items, window, work) {
  const results = new Array(items.length);
  let next = 0;
  const lane = async () => {
    while (next < items.length) {
      const k = next;
      next += 1;
      results[k] = await work(items[k], k);
    }
  };
  await Promise.all(Array.from({ length: Math.min(window, items.length) }, lane));
  return results;
}

/** Whether an error of a simulation is a failure of the simulation, not of the host. */
function isFailure(error) {
  return error instanceof EngineError || error?.name === "EngineError";
}

let nextPoolKey = 0;

/** A problem of PEtab v2 opened in a page, see `openPetab`. */
class PetabProblem {
  constructor(api, summary, engine) {
    this.api = api;
    this.handle = summary.handle;
    /** The ids of the models of the problem. */
    this.models = summary.models;
    /** The ids of the estimated parameters, in the order of a point. */
    this.parameters = summary.parameters;
    /** Their nominal values, the point of an evaluation without one. */
    this.nominal = numbers(summary.nominal);
    /** Their lower and upper bounds. */
    this.lower = numbers(summary.lower);
    this.upper = numbers(summary.upper);
    /** The number of the simulations of an evaluation, one per experiment of a model. */
    this.nSimulations = summary.simulations;
    /** The number of the measurements. */
    this.nMeasurements = summary.measurements;
    this.engine = engine;
    this.closed = false;
  }

  /** Throws for a closed problem. */
  checkOpen() {
    if (this.closed) throw new Error("the problem is closed");
  }

  /** The JSON of a point: `null` for the nominal values. */
  point(x) {
    if (x === undefined || x === null) return null;
    const values = Array.from(x);
    if (values.length !== this.parameters.length) {
      throw new TypeError(
        `the problem estimates ${this.parameters.length} parameters and the point has ${values.length} values`,
      );
    }
    if (values.some((value) => typeof value !== "number" || !Number.isFinite(value))) {
      throw new TypeError("the values of a point are finite numbers");
    }
    return values;
  }

  /**
   * The requests of `simulation/1` of the simulations at a point (the nominal values without
   * one), each `{ model, request }` with the index of its model.
   */
  requests(x) {
    this.checkOpen();
    const point = this.point(x);
    const text = point === null ? "" : JSON.stringify(point);
    return this.api.json("petab_requests", [this.handle], [new TextEncoder().encode(text)]);
  }

  /**
   * Simulates every simulation of the problem at a point (the nominal values without one):
   * `[{ model, request, result }]` in the order of the simulations, the result
   * `{ columns, nRows, values }` of the loader or `{ error }`, the message of an engine which
   * failed. `signal` aborts the simulations not yet done.
   */
  async simulations(x, { signal } = {}) {
    const planned = this.requests(x);
    const results = await this.engine.simulate(planned, signal);
    return planned.map(({ model, request }, k) => ({ model, request, result: results[k] }));
  }

  /**
   * Evaluates the problem at a point (the nominal values without one), from the simulations of
   * `simulations` at that point when given, else from its own: `llh`, `chi2`, `logPrior`,
   * `logPriors` (by parameter), `unnormLogPosterior`, `simulated` and `sigma` (per measurement,
   * `Float64Array`s), `failures` (`[{ simulation, message }]`) and `simulationTable`, the text
   * of the simulation table of PEtab. A simulation which failed gives `NaN` for its
   * measurements, and so a log-likelihood of `NaN`.
   */
  async evaluate(x, { simulations, signal } = {}) {
    const point = this.point(x);
    const done = simulations ?? (await this.simulations(point ?? undefined, { signal }));
    return this.evaluateSimulations(point, done);
  }

  /** The evaluation from the simulations at a point, see `evaluate`; synchronous. */
  evaluateSimulations(x, simulations) {
    this.checkOpen();
    const point = this.point(x);
    if (!Array.isArray(simulations) || simulations.length !== this.nSimulations) {
      throw new TypeError(`the problem has ${this.nSimulations} simulations`);
    }
    let n = 0;
    const results = simulations.map(({ result }) => {
      if (result.error !== undefined) return { error: String(result.error) };
      n += result.values.length;
      return { columns: result.columns, n_rows: result.nRows };
    });
    const values = new Float64Array(n);
    let at = 0;
    for (const { result } of simulations) {
      if (result.error === undefined) {
        values.set(result.values, at);
        at += result.values.length;
      }
    }
    const bytes = new Uint8Array(values.buffer);
    if (!LITTLE_ENDIAN) {
      const view = new DataView(values.buffer);
      for (let k = 0; k < values.length; k += 1) view.setFloat64(8 * k, values[k], true);
    }
    const input = new TextEncoder().encode(JSON.stringify({ x: point, results }));
    const answer = this.api.json("petab_evaluate", [this.handle], [input, bytes]);
    return {
      llh: number(answer.llh),
      chi2: number(answer.chi2),
      logPrior: number(answer.log_prior),
      logPriors: Object.fromEntries(answer.log_priors.map(([id, value]) => [id, number(value)])),
      unnormLogPosterior: number(answer.unnorm_log_posterior),
      simulated: numbers(answer.simulated),
      sigma: numbers(answer.sigma),
      failures: answer.failures.map(([simulation, message]) => ({ simulation, message })),
      simulationTable: answer.simulation_table,
    };
  }

  /** Drops the problem, its plan in `compiler.wasm` and its models in the engine. */
  close() {
    if (this.closed) return;
    this.closed = true;
    this.api.exports.petab_close(this.handle);
    this.engine.close();
  }
}

/** Whether this host stores numbers little-endian, as WebAssembly does. */
const LITTLE_ENDIAN = new Uint8Array(new Float64Array([1]).buffer)[7] === 0x3f;

/** The engine of a problem on the thread of the caller: an instance of the loader per model. */
class RuntimeEngine {
  constructor(runtime, compiled) {
    this.runtime = runtime;
    this.compiled = compiled;
    this.instances = [];
  }

  async open() {
    for (const { wasm, description } of this.compiled) {
      this.instances.push(await this.runtime.instantiate(wasm, description));
    }
  }

  async simulate(planned, signal) {
    const results = [];
    for (const { model, request } of planned) {
      signal?.throwIfAborted();
      let instance = this.instances[model];
      if (instance.trapped) {
        // an instance which trapped is not used again: a new one of the same model
        const { wasm, description } = this.compiled[model];
        instance = await this.runtime.instantiate(wasm, description);
        this.instances[model] = instance;
      }
      try {
        results.push(instance.simulate(request));
      } catch (error) {
        if (!isFailure(error)) throw error;
        results.push({ error: error.message });
      }
    }
    return results;
  }

  close() {
    this.instances = [];
  }
}

/** The engine of a problem on a pool of workers: the models added to the pool. */
class PoolEngine {
  constructor(pool, compiled) {
    this.pool = pool;
    this.compiled = compiled;
    const id = nextPoolKey;
    nextPoolKey += 1;
    this.keys = compiled.map((_, k) => `petab-${id}-${k}`);
  }

  async open() {
    for (const [k, { wasm, description }] of this.compiled.entries()) {
      await this.pool.addModel(this.keys[k], wasm, description);
    }
  }

  async simulate(planned, signal) {
    return windowed(planned, POOL_WINDOW, async ({ model, request }) => {
      try {
        return await this.pool.simulate(this.keys[model], request, { signal });
      } catch (error) {
        if (!isFailure(error)) throw error;
        return { error: error.message };
      }
    });
  }

  close() {
    for (const key of this.keys) {
      try {
        this.pool.removeModel(key);
      } catch {
        // a pool closed before the problem
      }
    }
  }
}

/**
 * Opens a problem of PEtab v2 from the contents of its files.
 *
 * @param {object} options
 * @param {object} options.compiler a compiler of `compile.mjs` whose `compiler.wasm` is built
 *   with the feature `petab` (the default), which reads, plans and evaluates the problem
 * @param {object} [options.runtime] a runtime of `loader.mjs`, which simulates on the thread of
 *   the caller
 * @param {object} [options.pool] a pool of `pool.mjs`, which simulates on its workers; one of
 *   `runtime` and `pool`
 * @param {Record<string, string|BufferSource>|Map<string, string|BufferSource>} options.files
 *   the files of the problem by the names its YAML file and its tables give them: the YAML
 *   file, the tables and the SBML of the models, as text or as the bytes of their UTF-8
 * @param {string} [options.yaml] the name of the YAML file, without it the one file of `.yaml`
 *   or `.yml`
 * @param {{relativeTolerance?: number, absoluteTolerance?: number, dataAtChange?: "after"|"before", extension?: "read"|"ignore"}} [options.options]
 *   the tolerances of the integrator, the state a measurement at a change reads and whether
 *   the extension of sbmlsim is read, as `sbmlsim_core.petab.Problem.from_files` takes them
 * @returns {Promise<PetabProblem>} the problem, its models compiled and loaded
 * @throws {PetabError} for a problem which is not read, planned or compiled
 */
export async function openPetab({ compiler, runtime, pool, files, yaml, options = {} }) {
  const api = new Interface(compiler);
  if ((runtime === undefined) === (pool === undefined)) {
    throw new TypeError("openPetab simulates with a runtime or a pool, one of them");
  }
  if (!files || typeof files !== "object") throw new TypeError("openPetab needs the files");
  const entries = files instanceof Map ? [...files.entries()] : Object.entries(files);
  const texts = Object.fromEntries(entries.map(([name, content]) => [name, fileText(name, content)]));
  const request = {
    yaml: yamlName(Object.keys(texts), yaml),
    files: texts,
    options: openOptions(options),
  };
  const summary = api.json("petab_open", [], [new TextEncoder().encode(JSON.stringify(request))]);
  const compiled = [];
  try {
    for (let k = 0; k < summary.models.length; k += 1) {
      const { status, answers } = api.call("petab_model", [summary.handle, k], [], 4);
      const text = new TextDecoder().decode(answers[0]);
      if (status !== 1) throw new PetabError(text || `the model ${summary.models[k]} does not compile`);
      compiled.push({ wasm: answers[1], description: JSON.parse(text) });
    }
    const engine = runtime !== undefined ? new RuntimeEngine(runtime, compiled) : new PoolEngine(pool, compiled);
    try {
      await engine.open();
    } catch (error) {
      engine.close();
      throw error;
    }
    return new PetabProblem(api, summary, engine);
  } catch (error) {
    api.exports.petab_close(summary.handle);
    throw error;
  }
}
