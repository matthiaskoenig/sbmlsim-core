// The scan of sbmlsim in a JavaScript host (since 0.43.0): a simulation and its dimensions with
// the observables of the run, the request `scan/1`, planned, simulated and assembled into the
// result `scanresult/1` in a page, as sbmlsim's `Simulator.run` and `sbmlsim_core.scan.run` do.
//
// Plain ECMAScript without dependencies and without APIs of Node, so it runs in browsers as well:
//
//   import { createCompiler } from "sbmlsim-core/compile";
//   import { createRuntime } from "sbmlsim-core";
//   import { runScan } from "sbmlsim-core/scan";
//   const compiler = await createCompiler({ compiler: compilerWasm });
//   const runtime = await createRuntime({ engine, mathlib }); // or a pool of `pool.mjs`
//   const model = compiler.compile(sbml); // { wasm, description }
//   const result = await runScan(request, { compiler, runtime, model });
//   for (const { name, shape, unit, values } of result.variables) console.log(name, shape, unit, values);
//
// A scan with a dimension of models (sbmlsim's `Dimension(models=...)`) takes the compiled model
// of every key of the dimension instead of the model of the run:
// `runScan(request, { compiler, runtime, models: { fast, slow } })`.
//
// The planner is the Rust of `sbmlsim-scan` in `compiler.wasm` (the calls `scan_open`,
// `scan_ask`, `scan_tell`, `scan_observed`, `scan_result` and `scan_close` of
// `crates/wasm/src/scan.rs`), the planner of python (`sbmlsim_core.scan`); this module only
// simulates the requests of its rounds with the engine of the page, the loader of `loader.mjs`
// on the thread of the caller or the workers of `pool.mjs`, and calls the observables of the
// host (sbmlsim's `Custom`). Every array of numbers crosses the boundary of the module as the
// bytes of its doubles, so the values of the result are `Float64Array`s with `NaN` and the
// infinities as the values they are.

import { EngineError } from "./loader.mjs";

/** The contract of the request of a scan. */
export const SCAN_CONTRACT = "scan/1";

/** The contract of the result of a scan. */
export const SCAN_RESULT_CONTRACT = "scanresult/1";

/** The exports of `compiler.wasm` a scan needs, the feature `scan` of `sbmlsim-wasm`. */
const SCAN_EXPORTS = [
  "allocate",
  "deallocate",
  "memory",
  "scan_open",
  "scan_ask",
  "scan_tell",
  "scan_observed",
  "scan_result",
  "scan_close",
];

/** The most simulations of a round a pool queues at once. */
const POOL_WINDOW = 256;

/**
 * A scan which cannot be planned or which failed: `kind` is `request` (a request which does not
 * fit itself or the model, or a call out of turn), `units` (a time or a value which cannot be
 * converted into the units of the model), `unsupported` (a simulation which is not supported)
 * or `point` (a point of a scan of `on_error: "raise"` which failed), and `point` the flat
 * index of that point, `null` for the other kinds.
 */
export class ScanError extends Error {
  constructor(kind, message, point = null) {
    super(message);
    this.name = "ScanError";
    this.kind = kind;
    this.point = point ?? null;
  }
}

/** Whether this host stores numbers little-endian, as WebAssembly does. */
const LITTLE_ENDIAN = new Uint8Array(new Float64Array([1]).buffer)[7] === 0x3f;

/** The doubles of little-endian bytes. */
function doubles(bytes) {
  if (bytes.length === 0) return new Float64Array(0);
  if (LITTLE_ENDIAN) return new Float64Array(bytes.buffer, bytes.byteOffset, bytes.length / 8);
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.length);
  return Float64Array.from({ length: bytes.length / 8 }, (_, k) => view.getFloat64(8 * k, true));
}

/** The little-endian bytes of doubles, the arrays one after the other. */
function bytesOf(arrays) {
  const n = arrays.reduce((sum, values) => sum + values.length, 0);
  const all = new Float64Array(n);
  let at = 0;
  for (const values of arrays) {
    all.set(values, at);
    at += values.length;
  }
  if (!LITTLE_ENDIAN) {
    const view = new DataView(all.buffer);
    for (let k = 0; k < n; k += 1) view.setFloat64(8 * k, all[k], true);
  }
  return new Uint8Array(all.buffer);
}

/** A number of the JSON of a result: a number, `null` for `NaN`, `"inf"` and `"-inf"`. */
function number(value) {
  if (value === null || value === "NaN") return NaN;
  if (value === "inf") return Infinity;
  if (value === "-inf") return -Infinity;
  return value;
}

/** The exports of `compiler.wasm` of a compiler of `compile.mjs` or of an instance. */
function exportsOf(compiler) {
  const exports = compiler?.exports;
  const missing = SCAN_EXPORTS.filter((name) => exports?.[name] === undefined);
  if (missing.length > 0) {
    throw new Error(
      `compiler.wasm does not export ${missing.join(", ")}: it is built without the feature scan`,
    );
  }
  return exports;
}

/** The text of a description: a text as it is, an object as its JSON. */
function descriptionText(description) {
  return typeof description === "string" ? description : JSON.stringify(description);
}

/**
 * The JSON text of the descriptions of the models of a dimension of models by their keys, `""`
 * for none.
 */
function modelsText(models) {
  if (models === undefined || models === null) return "";
  const entries = Object.entries(models).map(([key, model]) => {
    if (model?.description === undefined) throw new TypeError(`the model '${key}' is no compiled model`);
    return `${JSON.stringify(key)}:${descriptionText(model.description)}`;
  });
  return `{${entries.join(",")}}`;
}

/** A scan open in an instance of `compiler.wasm`, see `openScan`. */
class OpenScan {
  constructor(exports, request, description, models) {
    this.exports = exports;
    this.closed = false;
    const encoder = new TextEncoder();
    const text = typeof request === "string" ? request : JSON.stringify(request);
    const answer = this.call("scan_open", [], [
      encoder.encode(text),
      encoder.encode(description === undefined ? "" : descriptionText(description)),
      encoder.encode(modelsText(models)),
    ]);
    /** The handle of the scan in the module. */
    this.handle = answer.json.handle;
    /** The number of the points which run. */
    this.points = answer.json.points;
    /** The points which ran and the points which run, `[done, total]`. */
    this.progress = [0, this.points];
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
   * Calls a function of the module with the arguments `head` and the buffers of `inputs`, which
   * writes its JSON and the bytes of its numbers to four words; gives both, or throws the
   * `ScanError` it answered with.
   */
  call(name, head, inputs) {
    const { allocate, deallocate } = this.exports;
    const buffers = inputs.map((bytes) => this.put(bytes));
    const out = allocate(16);
    let status;
    let text;
    let values;
    try {
      status = this.exports[name](...head, ...buffers.flat(), out);
      const header = Array.from(new Uint32Array(this.exports.memory.buffer, out, 4));
      text = new TextDecoder().decode(this.take(header[0], header[1]));
      values = this.take(header[2], header[3]);
    } finally {
      deallocate(out, 16);
      for (const [pointer, length] of buffers) if (pointer !== 0) deallocate(pointer, length);
    }
    const json = JSON.parse(text);
    if (status !== 1) throw new ScanError(json.kind, json.message, json.point);
    if (json.progress !== undefined) this.progress = json.progress;
    return { json, values };
  }

  /** Throws for a closed scan. */
  checkOpen() {
    if (this.closed) throw new ScanError("request", "The scan is closed.");
  }

  /**
   * The next round of the scan:
   *
   * - `{ simulate: [request, ...], model, finished }`: the requests of `simulation/1` of a
   *   batch on the model of the key `model` in the dimension of models (`null` for the model of
   *   the run), which `tell` answers in their order, and the keys of the models whose points
   *   have all run (`finished`), whose engines a host may drop;
   * - `{ observe: { observables: [id, ...], points: [{ index, time, columns }, ...] } }`: the
   *   observables of the host of the points of a chunk, `time` the finite time points of a point
   *   and `columns` every symbol they read to its values (`Float64Array`s), `scalars` the
   *   symbols which are a value per simulation (one value), which `tellObserved` answers;
   * - `{ done: true }`: the scan is done, or failed, which `result` throws.
   *
   * A round which was asked and not told is asked again.
   */
  ask() {
    this.checkOpen();
    const { json, values } = this.call("scan_ask", [this.handle], []);
    if (json.simulate !== undefined) {
      return { simulate: json.simulate, model: json.model ?? null, finished: json.finished ?? [] };
    }
    if (json.observe !== undefined) {
      const numbers = doubles(values);
      let at = 0;
      const next = (n) => {
        const slice = numbers.slice(at, at + n);
        at += n;
        return slice;
      };
      const points = json.observe.points.map(({ index, time, columns }) => {
        const t = next(time);
        return { index, time: t, columns: Object.fromEntries(columns.map(([symbol, n]) => [symbol, next(n)])) };
      });
      return {
        observe: { observables: json.observe.observables, points, scalars: json.observe.scalars },
      };
    }
    return { done: true };
  }

  /**
   * Takes the answers of a round of simulations, one per request in their order: a table of the
   * loader (`{ columns, nRows, values }`) or the failure of its simulation, an `EngineError` or
   * `{ error, kind, flag, time }`.
   *
   * @throws {ScanError} of the kind `point` when a point of a scan of `on_error: "raise"` failed
   */
  tell(results) {
    this.checkOpen();
    const tables = [];
    const shapes = results.map((result) => {
      if (result instanceof Error || result?.error !== undefined) {
        const error = result instanceof Error ? result : null;
        const answer = {
          error: error ? (error.detail ?? error.message) : String(result.error),
          kind: error ? error.kind : result.kind,
        };
        const flag = error ? error.flag : result.flag;
        const time = error ? error.time : result.time;
        if (flag !== undefined) answer.flag = flag;
        if (Number.isFinite(time)) answer.time = time;
        return answer;
      }
      tables.push(result.values);
      return { columns: result.columns, n_rows: result.nRows };
    });
    const input = new TextEncoder().encode(JSON.stringify({ results: shapes }));
    this.call("scan_tell", [this.handle], [input, bytesOf(tables)]);
  }

  /**
   * Takes the answer of a round of the observables of the host:
   * `{ values: { id: [value of every point] }, errors: [{ observable, index, message }] }`, a
   * value a number for an observable of the kind `scalar` and a value per time point for one of
   * the kind `timecourse`, in the order of the points of the question; a point of an error fails
   * alone.
   */
  tellObserved({ values = {}, errors = [], ...rest }) {
    this.checkOpen();
    const unknown = Object.keys(rest);
    if (unknown.length > 0) throw new TypeError(`unknown keys of the answer ${unknown.join(", ")}`);
    const arrays = [];
    const observables = Object.entries(values).map(([id, points]) => [
      id,
      Array.from(points, (value) => {
        const array = typeof value === "number" ? [value] : Array.from(value);
        arrays.push(array);
        return array.length;
      }),
    ]);
    const input = new TextEncoder().encode(JSON.stringify({ observables, errors }));
    this.call("scan_observed", [this.handle], [input, bytesOf(arrays)]);
  }

  /**
   * The result `scanresult/1` once `ask` says the scan is done; a scan gives it once. The fields
   * of the contract: `sizes`, `dims`, `layout`, `coords` (the values `{ numbers: Float64Array }`,
   * `{ labels: [...] }` or `{ indices: [...] }`, an array of integers), `variables` (the values a `Float64Array` of
   * the product of the `shape`, flat in C order), `units` (pairs), `changes` (a
   * `Float64Array`), `errors` (`null` for a scan of `on_error: "raise"`) and `block`.
   *
   * @throws {ScanError} the failure of the scan, and one of the kind `request` for a scan which
   *   is not done
   */
  result() {
    this.checkOpen();
    const { json, values } = this.call("scan_result", [this.handle], []);
    const numbers = doubles(values);
    let at = 0;
    for (const variable of json.variables) {
      const n = variable.shape.reduce((a, b) => a * b, 1);
      variable.values = numbers.slice(at, at + n);
      at += n;
    }
    for (const coord of json.coords) {
      if (coord.values.numbers !== undefined) coord.values = { numbers: Float64Array.from(coord.values.numbers, number) };
    }
    json.changes = Float64Array.from(json.changes, number);
    return json;
  }

  /** Drops the scan in the module; further calls throw. */
  close() {
    if (!this.closed) {
      this.closed = true;
      this.exports.scan_close(this.handle);
    }
  }
}

/**
 * Opens a scan: plans the request `scan/1` against a compiled model in `compiler.wasm`, for a
 * host which asks for its rounds and tells their answers itself (`runScan` does both).
 *
 * @param {object|string} request the request `scan/1`, an object (whose mappings keep their
 *   order in its JSON) or its JSON text
 * @param {object} options
 * @param {object} options.compiler a compiler of `compile.mjs` whose `compiler.wasm` is built
 *   with the feature `scan` (the default)
 * @param {{description: object|string}} [options.model] the compiled model of the run, its
 *   description `model/2` (the object or the text of `model.json`)
 * @param {Object<string, {description: object|string}>} [options.models] the compiled model of
 *   every key of the dimension of models of the scan, which replaces the model of the run
 * @returns {OpenScan} the scan: `ask`, `tell`, `tellObserved`, `result`, `close`
 * @throws {ScanError} for a request which does not fit itself or its models
 */
export function openScan(request, { compiler, model, models }) {
  if (model !== undefined && model?.description === undefined) throw new TypeError("openScan needs the compiled model");
  if (model === undefined && models === undefined) throw new TypeError("openScan needs the compiled model or models");
  return new OpenScan(exportsOf(compiler), request, model?.description, models);
}

/** Whether an error of a simulation is a failure of the simulation, not of the host. */
function isFailure(error) {
  return error instanceof EngineError || error?.name === "EngineError";
}

let nextPoolKey = 0;

/** The engine of a scan on the thread of the caller: one instance of the loader. */
async function runtimeEngine(runtime, model) {
  let instance = await runtime.instantiate(model.wasm, model.description);
  return {
    async simulate(requests, signal) {
      const results = [];
      for (const request of requests) {
        signal?.throwIfAborted();
        if (instance.trapped) instance = await runtime.instantiate(model.wasm, model.description);
        try {
          results.push(instance.simulate(request));
        } catch (error) {
          if (!isFailure(error)) throw error;
          results.push(error);
        }
      }
      return results;
    },
    close() {},
  };
}

/**
 * The engine of a scan on a pool of workers: the model added to the pool, at most `window`
 * simulations of a round queued at once.
 */
async function poolEngine(pool, model, window) {
  const key = `scan-${nextPoolKey}`;
  nextPoolKey += 1;
  await pool.addModel(key, model.wasm, model.description);
  return {
    async simulate(requests, signal) {
      const results = new Array(requests.length);
      let next = 0;
      const lane = async () => {
        while (next < requests.length) {
          const k = next;
          next += 1;
          try {
            results[k] = await pool.simulate(key, requests[k], { signal });
          } catch (error) {
            if (!isFailure(error)) {
              // an error of the host (an abort, a pool which was closed) ends the round: the
              // other lanes take no further request
              next = requests.length;
              throw error;
            }
            results[k] = error;
          }
        }
      };
      // every lane settles before the round answers, so no simulation of it is in flight when
      // the scan removes its model from the pool
      const lanes = await Promise.allSettled(Array.from({ length: Math.min(window, requests.length) }, lane));
      const failed = lanes.find((lane) => lane.status === "rejected");
      if (failed !== undefined) throw failed.reason;
      return results;
    },
    close() {
      try {
        pool.removeModel(key);
      } catch {
        // a pool closed before the scan
      }
    },
  };
}

/**
 * Runs a scan and gives its result `scanresult/1`: the planner in `compiler.wasm`, the
 * simulations of its rounds on the engine of the page. A request carries everything of its
 * simulation, so the engine keeps nothing between the points: on the thread of the caller one
 * instance of the model simulates them one after the other (made again only after a trap),
 * and on a pool each worker simulates on its own, so the result does not depend on the workers
 * of a pool.
 *
 * @param {object|string} request the request `scan/1` (sbmlsim's `Scan.to_dict()` with the
 *   observables and the options of the run), an object or its JSON text
 * @param {object} options
 * @param {object} options.compiler a compiler of `compile.mjs` whose `compiler.wasm` is built
 *   with the feature `scan` (the default)
 * @param {object} [options.runtime] a runtime of `loader.mjs`, which simulates on the thread of
 *   the caller
 * @param {object} [options.pool] a pool of `pool.mjs`, which simulates on its workers; one of
 *   `runtime` and `pool`
 * @param {{wasm: BufferSource|WebAssembly.Module, description: object|string}} [options.model]
 *   the compiled model of the run, as `compiler.compile` gives it
 * @param {Object<string, {wasm: BufferSource|WebAssembly.Module, description: object|string}>}
 *   [options.models] the compiled model of every key of the dimension of models of the scan,
 *   which replaces the model of the run; the requests of a model run on an engine of that model,
 *   made at the first round of the model (on a pool: the model added) and dropped once every
 *   point of the model ran
 * @param {(question: object) => object|Promise<object>} [options.observe] computes the
 *   observables of the host (sbmlsim's `Custom`) of a round, see `OpenScan.ask` and
 *   `OpenScan.tellObserved`; a scan with such observables needs it
 * @param {(done: number, total: number) => void} [options.onBatch] called with the points which
 *   ran and the points which run whenever a batch of them finished
 * @param {number} [options.poolWindow] the most simulations of a round queued on a `pool` at
 *   once (256)
 * @param {AbortSignal} [options.signal] aborts the simulations not yet done
 * @returns {Promise<object>} the result, see `OpenScan.result`
 * @throws {ScanError} for a request which does not fit itself or the model, and for a failed
 *   point of a scan of `on_error: "raise"`
 */
export async function runScan(
  request,
  { compiler, runtime, pool, model, models, observe, onBatch, signal, poolWindow = POOL_WINDOW } = {},
) {
  if ((runtime === undefined) === (pool === undefined)) {
    throw new TypeError("runScan simulates with a runtime or a pool, one of them");
  }
  if (!Number.isInteger(poolWindow) || poolWindow < 1) throw new TypeError("poolWindow is a positive integer");
  const scan = openScan(request, { compiler, model, models });
  // the engine of every model, made at its first round and dropped (its model removed from a
  // pool) once every point of the model ran: `null` the model of the run
  const engines = new Map();
  const engineOf = async (key) => {
    if (!engines.has(key)) {
      const compiled = key === null ? model : models?.[key];
      if (compiled === undefined) throw new TypeError(`runScan has no model '${key}'`);
      engines.set(
        key,
        runtime !== undefined ? await runtimeEngine(runtime, compiled) : await poolEngine(pool, compiled, poolWindow),
      );
    }
    return engines.get(key);
  };
  try {
    let reported = 0;
    for (;;) {
      const [done, total] = scan.progress;
      if (done > reported) {
        reported = done;
        onBatch?.(done, total);
      }
      const round = scan.ask();
      if (round.done) break;
      if (round.simulate !== undefined) {
        for (const key of round.finished) {
          engines.get(key)?.close();
          engines.delete(key);
        }
        const engine = await engineOf(round.model);
        scan.tell(await engine.simulate(round.simulate, signal));
      } else {
        if (observe === undefined) {
          throw new ScanError(
            "request",
            "The scan has observables of the host (Custom) and runScan was given no observe function to compute them.",
          );
        }
        scan.tellObserved(await observe(round.observe));
      }
    }
    return scan.result();
  } finally {
    for (const engine of engines.values()) engine?.close();
    scan.close();
  }
}
