// PEtab v2 in a JavaScript host (since 0.39.0): a problem read, simulated, evaluated and (since
// 0.40.0) fitted in a page.
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
//   const { x: optimum, objective } = await problem.fit({ starts: 4 }); // the fit (since 0.40.0)
//   problem.close();
//
// What a problem is, its plan and its likelihood are the Rust of `sbmlsim-petab` in
// `compiler.wasm` (the calls `petab_open`, `petab_model`, `petab_requests`, `petab_evaluate`
// and `petab_close` of `crates/wasm/src/petab.rs`), the problem of the command line
// (`sbmlsim-core petab`) and of python (`sbmlsim_core.petab`); this module only simulates the
// requests of the plan with the engine of the page, the loader of `loader.mjs` on the thread
// of the caller or the workers of `pool.mjs`. Since that engine is `engine.wasm`, which the
// command line runs as well, an evaluation in a page gives the bits of the command line.
//
// A fit (since 0.40.0) is the optimizer of `sbmlsim-fit` in `compiler.wasm` (`petab_fit_open`,
// `petab_fit_ask`, `petab_fit_tell`, `petab_fit_stop` and `petab_fit_close`), driven ask/tell:
// it asks for the simulations of its next evaluations (the `k + 1` evaluations of the finite
// differences of a jacobian, the members of a generation), this module simulates them on the
// engine of the page, on the workers of a pool next to each other, and tells their results.
// The optimizer and its evaluations are the ones of `sbmlsim-core petab --fit`.

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

/** The exports of `compiler.wasm` a fit needs, the feature `fit` of `sbmlsim-wasm`. */
const FIT_EXPORTS = [
  "petab_fit_open",
  "petab_fit_ask",
  "petab_fit_tell",
  "petab_fit_stop",
  "petab_fit_close",
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
    const { results, bytes } = encodeResults(simulations.map(({ result }) => result));
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

  /**
   * Fits the estimated parameters: the maximum likelihood estimate of a problem without priors,
   * the maximum a posteriori estimate of one with, by the optimizers of `sbmlsim-fit` in
   * `compiler.wasm` whose simulations the engine of the problem runs. A fit from the nominal
   * values with the default settings is the fit of `sbmlsim-core petab --fit`.
   *
   * The jacobian of a step is the forward differences of the simulations (`k + 1` evaluations
   * at once), as on the command line: the tables of `engine.wasm` carry no sensitivities.
   *
   * @param {object} [options]
   * @param {number|ArrayLike<number>[]} [options.starts] the points the fits start from, the
   *   values of the estimated parameters, or their number: the nominal values and points drawn
   *   uniformly in the space the optimizer searches (`log10` of a parameter of positive bounds)
   *   with `seed`; the nominal values alone without it
   * @param {number} [options.seed] the seed of the drawn starts (0)
   * @param {"trustRegion"|"levenbergMarquardt"|{differentialEvolution: object}} [options.algorithm]
   *   the optimizer, trust region reflective by default; a differential evolution takes
   *   `{ seed, population, f, cr, polish }`
   * @param {number} [options.maxIterations] the iterations of a fit (100)
   * @param {number} [options.gradientTolerance] the tolerance of the scaled gradient
   * @param {number} [options.stepTolerance] the relative tolerance of the step (`xtol`)
   * @param {number} [options.costTolerance] the relative tolerance of the cost (`ftol`)
   * @param {number} [options.maxSeconds] the time the fits may take, measured by the page,
   *   after which they stop at the best point they reached
   * @param {AbortSignal} [options.signal] stops the fits at their next step, with the best
   *   point they reached
   * @param {(step: {start: number, step: number, simulations: number, seconds: number}) => void} [options.onStep]
   *   called after the simulations of every step of every fit
   * @returns {Promise<{x: Float64Array, objective: number, cost: number, iterations: number,
   *   evaluations: number, jacobians: number, status: string, simulations: number,
   *   seconds: number, start: Float64Array, runs: object[]}>} the best fit, the one of the
   *   lowest `objective` (the negative log posterior, the negative log-likelihood of a problem
   *   without priors), and every fit as `runs`, in the order of the starts
   */
  async fit(options = {}) {
    this.checkOpen();
    const missing = FIT_EXPORTS.filter((name) => this.api.exports[name] === undefined);
    if (missing.length > 0) {
      throw new Error(
        `compiler.wasm does not export ${missing.join(", ")}: it is built without the feature fit`,
      );
    }
    const { starts, seed = 0, maxSeconds, signal, onStep, ...rest } = options;
    const settings = fitSettings(rest);
    const points = this.starts(starts, seed);
    const began = performance.now();
    const fits = [];
    try {
      for (const start of points) {
        const input = new TextEncoder().encode(JSON.stringify({ settings, start: Array.from(start) }));
        fits.push({ start, ...this.api.json("petab_fit_open", [this.handle], [input]) });
      }
    } catch (error) {
      for (const { fit } of fits) this.api.exports.petab_fit_close(fit);
      throw error;
    }
    const stopAll = () => {
      for (const { fit } of fits) this.api.exports.petab_fit_stop(fit);
    };
    if (maxSeconds !== undefined && !(maxSeconds >= 0)) {
      throw new TypeError("maxSeconds is a number of seconds");
    }
    // the clock and the signal are read between the steps of the fits, which yield to the
    // page between them, so that its events (a click on stop) are not starved by a fit on the
    // loader, whose simulations do not wait for anything
    let stopped = false;
    const check = () => {
      const late = maxSeconds !== undefined && performance.now() - began >= 1000 * maxSeconds;
      if (!stopped && (signal?.aborted || late)) {
        stopped = true;
        for (const { fit } of fits) this.api.exports.petab_fit_stop(fit);
      }
    };
    try {
      const runs = await Promise.all(fits.map((fit, k) => this.run(fit, k, { began, onStep, check })));
      const best = runs.reduce((a, b) => (b.objective < a.objective || Number.isNaN(a.objective) ? b : a));
      return { ...best, seconds: (performance.now() - began) / 1000, runs };
    } finally {
      for (const { fit } of fits) this.api.exports.petab_fit_close(fit);
    }
  }

  /** The starts of a fit: the points given, or the nominal values and drawn points. */
  starts(starts, seed) {
    if (starts === undefined) return [this.nominal];
    if (typeof starts === "number") {
      if (!Number.isInteger(starts) || starts < 1) throw new TypeError("starts is a number of starts or a list of points");
      const random = splitmix(seed);
      const points = [this.nominal];
      for (let n = 1; n < starts; n += 1) {
        points.push(
          Float64Array.from(this.parameters, (id, k) => {
            const [lower, upper] = [this.lower[k], this.upper[k]];
            if (!Number.isFinite(lower) || !Number.isFinite(upper)) {
              throw new PetabError(`the parameter ${id} has a bound which is not finite: give the starts of its fits`);
            }
            const u = random();
            // the scale of the optimizer: `log10` for positive bounds, as `Plan::definition`
            return lower > 0 ? 10 ** (Math.log10(lower) + u * (Math.log10(upper) - Math.log10(lower))) : lower + u * (upper - lower);
          }),
        );
      }
      return points;
    }
    if (!Array.isArray(starts) || starts.length === 0) throw new TypeError("starts is a number of starts or a list of points");
    return starts.map((start) => Float64Array.from(this.point(start)));
  }

  /** Runs one fit to its end: asks, simulates on the engine of the problem, tells. */
  async run({ fit, offset, start }, k, { began, onStep, check }) {
    let simulations = 0;
    for (let step = 1; ; step += 1) {
      this.checkOpen();
      check();
      const answer = this.api.json("petab_fit_ask", [fit], []);
      if (answer.done !== undefined) {
        const done = answer.done;
        return {
          x: numbers(done.x),
          objective: number(done.objective),
          cost: number(done.cost),
          offset: number(offset),
          iterations: done.iterations,
          evaluations: done.evaluations,
          jacobians: done.jacobians,
          status: done.status,
          simulations,
          start: Float64Array.from(start),
        };
      }
      // the simulations of a step run to their end: a fit is stopped between its steps
      const results = await this.engine.simulate(answer.simulate);
      simulations += results.length;
      const encoded = encodeResults(results);
      const input = new TextEncoder().encode(JSON.stringify({ results: encoded.results }));
      const { status, answers } = this.api.call("petab_fit_tell", [fit], [input, encoded.bytes], 2);
      if (status !== 1) throw new PetabError(new TextDecoder().decode(answers[0]) || "petab_fit_tell failed");
      onStep?.({ start: k, step, simulations, seconds: (performance.now() - began) / 1000 });
      await yieldToHost();
    }
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

/**
 * The results of simulations as `compiler.wasm` reads them: the shape of every table, or its
 * error and the kind of the error, as JSON and the values of the tables one after the other
 * as the little-endian bytes of their doubles.
 */
function encodeResults(simulated) {
  let n = 0;
  const results = simulated.map((result) => {
    if (result.error !== undefined) {
      return result.kind === undefined ? { error: String(result.error) } : { error: String(result.error), kind: result.kind };
    }
    n += result.values.length;
    return { columns: result.columns, n_rows: result.nRows };
  });
  const values = new Float64Array(n);
  let at = 0;
  for (const result of simulated) {
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
  return { results, bytes };
}

/** The settings of `sbmlsim_fit::Settings` from the options of `fit`. */
function fitSettings({ algorithm, maxIterations, gradientTolerance, stepTolerance, costTolerance, ...rest }) {
  const unknown = Object.keys(rest);
  if (unknown.length > 0) throw new TypeError(`unknown options ${unknown.join(", ")}`);
  const settings = {};
  if (algorithm === "trustRegion") settings.algorithm = "trust_region";
  else if (algorithm === "levenbergMarquardt") settings.algorithm = "levenberg_marquardt";
  else if (algorithm?.differentialEvolution !== undefined) {
    settings.algorithm = { differential_evolution: algorithm.differentialEvolution };
  } else if (algorithm !== undefined) {
    throw new TypeError("algorithm is trustRegion, levenbergMarquardt or { differentialEvolution }");
  }
  if (maxIterations !== undefined) settings.max_iterations = maxIterations;
  if (gradientTolerance !== undefined) settings.gradient_tolerance = gradientTolerance;
  if (stepTolerance !== undefined) settings.step_tolerance = stepTolerance;
  if (costTolerance !== undefined) settings.cost_tolerance = costTolerance;
  return settings;
}

/** Gives the event loop of the host a turn: `scheduler.yield` of a browser, else a timeout. */
function yieldToHost() {
  if (typeof globalThis.scheduler?.yield === "function") return globalThis.scheduler.yield();
  return new Promise((resolve) => setTimeout(resolve, 0));
}

/** A generator of uniform numbers in [0, 1) from a seed: SplitMix64. */
function splitmix(seed) {
  let state = BigInt.asUintN(64, BigInt(seed));
  return () => {
    state = BigInt.asUintN(64, state + 0x9e3779b97f4a7c15n);
    let z = state;
    z = BigInt.asUintN(64, (z ^ (z >> 30n)) * 0xbf58476d1ce4e5b9n);
    z = BigInt.asUintN(64, (z ^ (z >> 27n)) * 0x94d049bb133111ebn);
    z ^= z >> 31n;
    return Number(z >> 11n) / 2 ** 53;
  };
}

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
        results.push({ error: error.message, kind: error.kind });
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
        return { error: error.message, kind: error.kind };
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
