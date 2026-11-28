// The statistics of the analyses in a JavaScript host (since 0.43.0): every function of
// `sbmlsim_core.stats` of python, the Rust of the crate `sbmlsim-stats` in `compiler.wasm`.
//
// Plain ECMAScript without dependencies and without APIs of Node, so it runs in browsers as well:
//
//   import { createCompiler } from "sbmlsim-core/compile";
//   import { createStats } from "sbmlsim-core/stats";
//   const stats = createStats({ compiler: await createCompiler({ compiler: compilerWasm }) });
//   const fisher = stats.fisherStatistics(stats.gram(jacobian), 0.05, 20);
//   const [low, high] = stats.fisherIntervals([1.0, 0.3], fisher.standardErrors, ["LINEAR", "LOG10"], { dof: 18 });
//   const u = stats.unitPoints(2, 1024, { design: "sobol", seed: 1 });
//   const [q, qLow, qHigh, n] = stats.quantileCi(u.map((row) => row[0] + row[1]), 0.95);
//
// The functions are those of python with their names in camelCase (`quantile_ci` is
// `quantileCi`), their values and their errors: the module only converts the arguments and the
// results, the function and the checks of its arguments are the Rust python calls
// (`stats_call` of `crates/wasm/src/stats.rs`), so a page gives the bits of python.
//
// - The required arguments of python are the arguments of the function, in its order; every
//   optional or keyword-only argument of python is a key of an object of options after them, in
//   camelCase (`fold`, `level`, `rankTolerance`, `seed`, ...). An option `undefined` or `null`
//   takes the default of python.
// - An array is any array of numbers (a typed array of numbers included, not of BigInts), nested arrays are the dimensions
//   of numpy (a matrix is an array of its rows) and a number alone is an array of no dimension;
//   it is read in the order numpy ravels it. A matrix of no rows has no nested form: `[]` is a
//   vector of no values.
// - A vector of a result is a `Float64Array`, a matrix an array of `Float64Array` rows, a tuple
//   of python an array, a dictionary an object with its keys in camelCase, `None` `null`.
// - An argument python refuses throws a `StatsError` with the message of python and `kind`, the
//   class of the exception of python (`ValueError`, `OverflowError`, `ZeroDivisionError` or
//   `IndexError`); an argument of a type python refuses throws a `TypeError`.
//
// The doubles cross the boundary as their bits, so the infinities, `-0` and `NaN` arrive as they
// are given, and the results of a page are the ones of python and of the crate natively, bit for
// bit (a `NaN` is a `NaN`: JavaScript keeps no payload of it). Where python writes a number into
// a message as it was given (`'1'` of the int 1), JavaScript cannot tell 1 from 1.0 and the
// message has the double (`'1.0'`); and `null` takes the default where python refuses `None`.

import { withBuffers } from "./compile.mjs";

/** The exports of `compiler.wasm` the statistics need, the feature `stats` of `sbmlsim-wasm`. */
const STATS_EXPORTS = ["allocate", "deallocate", "memory", "stats_call"];

/** An argument the statistics refuse, with the message of python and the kind of its error. */
export class StatsError extends Error {
  /**
   * @param {string} kind the class of the exception of python: `ValueError`, `OverflowError`,
   *   `ZeroDivisionError` or `IndexError`
   * @param {string} message the message of python
   */
  constructor(kind, message) {
    super(message);
    this.name = "StatsError";
    /** The class of the exception python raises for the argument. */
    this.kind = kind;
  }
}

/** Whether this host stores numbers little-endian, as WebAssembly does. */
const LITTLE_ENDIAN = new Uint8Array(new Float64Array([1]).buffer)[7] === 0x3f;

/** The smallest and the largest integer of python a function takes, an `i64`. */
const I64 = [-(2n ** 63n), 2n ** 63n - 1n];

/** The largest seed, of 64 bits. */
const U64_MAX = 2n ** 64n - 1n;

/** A name of python of a name in camelCase: `rankTolerance` is `rank_tolerance`. */
function snake(name) {
  return name.replace(/[A-Z]/g, (letter) => `_${letter.toLowerCase()}`);
}

/** A name in camelCase of a name of python. */
function camel(name) {
  return name.replace(/_([a-z0-9])/g, (_, letter) => letter.toUpperCase());
}

/** The doubles of the arguments of a call and the JSON which refers to them. */
class Request {
  constructor(name) {
    this.name = name;
    this.chunks = [];
    this.length = 0;
  }

  /** The values as `{at, shape}` of the request, in the order numpy ravels them. */
  doubles(value, what) {
    const shape = [];
    const flat = [];
    const walk = (item, depth) => {
      if (typeof item === "number" || typeof item === "boolean") {
        if (depth !== shape.length) throw this.ragged(what);
        flat.push(Number(item));
        return;
      }
      const isView = ArrayBuffer.isView(item) && !(item instanceof DataView);
      if (!isView && !Array.isArray(item)) {
        throw new TypeError(`${this.name}(): ${what} is neither a number nor an array of numbers`);
      }
      if (depth === shape.length) {
        if (flat.length > 0) throw this.ragged(what);
        shape.push(item.length);
      } else if (depth > shape.length || shape[depth] !== item.length) {
        throw this.ragged(what);
      }
      if (isView && typeof item[0] !== "bigint") {
        if (depth !== shape.length - 1) throw this.ragged(what);
        for (let k = 0; k < item.length; k += 1) flat.push(item[k]);
        return;
      }
      for (const element of item) walk(element, depth + 1);
    };
    walk(value, 0);
    return this.push(Float64Array.from(flat), shape);
  }

  /** The error of nested arrays which are not of one shape. */
  ragged(what) {
    return new TypeError(`${this.name}(): the nested arrays of ${what} are not of one shape`);
  }

  /** A double alone; a boolean is 1 or 0, as `float` of python converts it. */
  number(value, what) {
    if (typeof value !== "number" && typeof value !== "boolean") {
      throw new TypeError(`${this.name}(): ${what} is a number`);
    }
    return this.push(Float64Array.of(Number(value)), []);
  }

  /** Adds values of a shape: their `{at, shape}`. */
  push(values, shape) {
    const at = this.length;
    this.chunks.push(values);
    this.length += values.length;
    return { at, shape };
  }

  /** An integer as its digits, of `i64` (`u64` of a seed). */
  integer(value, what, max = I64[1]) {
    let big;
    if (typeof value === "bigint") {
      big = value;
    } else if (Number.isSafeInteger(value)) {
      big = BigInt(value);
    } else {
      throw new TypeError(`${this.name}(): ${what} is an integer`);
    }
    if (big < I64[0] || big > max) {
      throw new RangeError(`${this.name}(): ${what} ${big} is past the integers it takes`);
    }
    return big.toString();
  }

  /** The bytes of the values, the little-endian bytes of their doubles. */
  bytes() {
    const values = new Float64Array(this.length);
    let at = 0;
    for (const chunk of this.chunks) {
      values.set(chunk, at);
      at += chunk.length;
    }
    if (!LITTLE_ENDIAN) {
      const view = new DataView(values.buffer);
      for (let k = 0; k < values.length; k += 1) view.setFloat64(8 * k, values[k], true);
    }
    return new Uint8Array(values.buffer);
  }
}

/** The JSON of an argument of a kind, its doubles added to the request. */
const KINDS = {
  /** numbers of any shape */
  array: (request, value, what) => request.doubles(value, what),
  /** booleans of any shape, `np.asarray(values, dtype=bool)`: a number is true unless 0 */
  booleans: (request, value, what) => request.doubles(value, what),
  number: (request, value, what) => request.number(value, what),
  integer: (request, value, what) => request.integer(value, what),
  seed: (request, value, what) => request.integer(value, what, U64_MAX),
  boolean: (request, value, what) => {
    if (typeof value !== "boolean") throw new TypeError(`${request.name}(): ${what} is a boolean`);
    return value;
  },
  /** a name (a scale, a design, a side): python refuses another value with its message */
  name: (request, value) => value,
  /** the names of the scales of the parameters */
  names: (request, value, what) => Array.from(iterable(request, value, what)),
  /** texts, the targets of a covariance, each as `String` writes it (`str` of python) */
  strings: (request, value, what) => Array.from(iterable(request, value, what), String),
  /** the pids of a precision, which its error names */
  pids: (request, value, what) => Array.from(iterable(request, value, what)),
  text: (request, value, what) => {
    if (typeof value !== "string") throw new TypeError(`${request.name}(): ${what} is a string`);
    return value;
  },
  /** a marginal of a covariance, `[family, mean, sd]` */
  marginal: (request, value, what) => {
    const items = Array.from(iterable(request, value, what));
    if (items.length !== 3 || typeof items[0] !== "string") {
      throw new TypeError(`${request.name}(): ${what} is [family, mean, sd]`);
    }
    return [items[0], request.number(items[1], `the mean of ${what}`), request.number(items[2], `the sd of ${what}`)];
  },
  /** the units of the targets of a covariance, of which the validation counts the items */
  count: (request, value, what) => String(Array.from(iterable(request, value, what)).length),
};

/** The value if it is iterable and no string (python refuses a `str` for a list), else a `TypeError`. */
function iterable(request, value, what) {
  if (
    value === null ||
    value === undefined ||
    typeof value === "string" ||
    typeof value[Symbol.iterator] !== "function"
  ) {
    throw new TypeError(`${request.name}(): ${what} is an array, not ${typeof value === "string" ? "a string" : String(value)}`);
  }
  return value;
}

/**
 * The functions of `sbmlsim_core.stats`: the name of python, the required arguments of python
 * (`name:kind`) and the options, every optional or keyword-only argument of python by its name
 * in camelCase (a kind ending in `!` is required, the keyword-only `seed` and `lognormal`).
 */
const FUNCTIONS = {
  sse: [["residuals:array"]],
  mse: [["residuals:array"]],
  rmse_from_mse: [["mse:number"]],
  aic_from_mse: [["mse:number", "n:integer", "k:integer"]],
  bic_from_mse: [["mse:number", "n:integer", "k:integer"]],
  r_squared: [["y_observed:array", "y_predicted:array"]],
  fold_counts: [["y_observed:array", "y_predicted:array"], { fold: "number" }],
  fold_accepted: [["y_observed:array", "y_predicted:array"], { fold: "number" }],
  sd_accepted: [["y_observed:array", "y_predicted:array", "y_sd:array"], { k: "number" }],
  gmfe: [["y_observed:array", "y_predicted:array"]],
  afe: [["y_observed:array", "y_predicted:array"]],
  nrmse: [["nres:array"]],
  guest_limits: [["observed:array"]],
  within_guest: [["observed:array", "predicted:array"]],
  subset_agreement: [["y_observed:array", "y_predicted:array"]],
  gram: [["jacobian:array"]],
  fisher_statistics: [
    ["information:array", "cost:number", "n:integer"],
    { likelihood: "boolean", rankTolerance: "number" },
  ],
  fisher_decomposition: [["information:array"]],
  fisher_statistics_of_decomposition: [
    ["spectrum:array", "inverse:array", "cost:number", "n:integer"],
    { likelihood: "boolean", rankTolerance: "number" },
  ],
  fisher_intervals: [
    ["values:array", "standard_errors:array", "scales:names"],
    { level: "number", dof: "number" },
  ],
  fisher_relative_errors: [["values:array", "standard_errors:array", "scales:names"]],
  posterior_laplace: [["precision:array"], { pids: "pids", opid: "text", rankTolerance: "number" }],
  rank_correlation: [["r:array"]],
  normal_correlation: [["r:array"]],
  log_sd: [["scale:name", "sd:number"]],
  lognormal_cv: [["sigma:number"]],
  mean_se: [["x:array"]],
  sd_se: [["x:array"]],
  quantile: [["x:array", "q:number"]],
  quantiles: [["x:array", "qs:array"]],
  quantile_ci: [["x:array", "q:number"], { level: "number" }],
  first_order_given_data: [["x:array", "y:array"], { nBins: "integer" }],
  precise: [["values:array", "rel_tol:number"]],
  interval_parameters: [["low:number", "high:number"], { level: "number", lognormal: "boolean!" }],
  unit_points: [["d:integer", "n:integer"], { design: "name", seed: "seed!", start: "integer" }],
  copula: [["u:array", "r:array"]],
  normal_score_rho: [["m:number", "a:marginal", "b:marginal"], { scale: "text" }],
  nearest_correlation: [["matrix:array"], { maxIterations: "integer", tol: "number" }],
  is_psd: [["matrix:array"], { rtol: "number" }],
  validate_covariance: [["targets:strings", "matrix:array"], { kind: "text", units: "count" }],
  p_values: [["satisfied:booleans", "p0:number", "side:name"], { margin: "number" }],
  confidence_sequence: [["satisfied:booleans"], { level: "number" }],
  decide: [["p_values:array", "alpha:number"]],
  decision: [["p_greater:array", "p_less:array", "p_equivalent:array", "alpha:number"]],
  needed: [["estimate:number", "n:integer", "p0:number", "margin:number", "alpha:number"]],
  family_level: [["level:number", "m:integer"]],
  group_means: [
    ["values:array", "size:integer", "groups:integer"],
    { geometric: "boolean", seed: "seed!" },
  ],
  bands: [["distribution:array", "qs:array"]],
  norm_cdf: [["x:array"]],
  norm_ppf: [["q:array"]],
  t_cdf: [["x:array", "df:array"]],
  t_ppf: [["q:array", "df:array"]],
  chi2_cdf: [["x:array", "df:array"]],
  chi2_ppf: [["q:array", "df:array"]],
  erf: [["x:array"]],
  gamma: [["x:array"]],
};

/** Whether a value is an object of options: a plain object. */
function isOptions(value) {
  return value !== null && typeof value === "object" && Object.getPrototypeOf(value) === Object.prototype;
}

/** The calls of the statistics of one instance of `compiler.wasm`. */
class Interface {
  constructor(compiler) {
    const exports = compiler?.exports;
    const missing = STATS_EXPORTS.filter((name) => exports?.[name] === undefined);
    if (missing.length > 0) {
      throw new Error(
        `compiler.wasm does not export ${missing.join(", ")}: it is built without the feature stats`,
      );
    }
    this.exports = exports;
  }

  /** Runs `stats_call` of a request: the decoded answer, or the error thrown. */
  call(request, json) {
    const text = new TextEncoder().encode(JSON.stringify(json));
    const bytes = request.bytes();
    // every buffer of the call is freed whatever happens, the words read little-endian
    const { status, answer, doubles } = withBuffers(this.exports, "compiler.wasm", (buffers) => {
      const input = buffers.put(text);
      const values = buffers.put(bytes);
      const out = buffers.allocate(16);
      const status = this.exports.stats_call(...input, ...values, out);
      const [answer, doubles] = buffers.results(out, 2);
      return { status, answer, doubles };
    });
    const parsed = JSON.parse(new TextDecoder().decode(answer));
    if (status !== 1) {
      if (parsed.kind === "TypeError") throw new TypeError(parsed.message);
      throw new StatsError(parsed.kind, parsed.message);
    }
    return decode(parsed, toDoubles(doubles));
  }

  /** The function of python `name` as a function of JavaScript. */
  function(name, [positional, options = {}]) {
    const parameters = positional.map((parameter) => parameter.split(":"));
    const jsName = camel(name);
    const fn = (...args) => {
      const request = new Request(jsName);
      let given = {};
      if (args.length === parameters.length + 1 && (isOptions(args.at(-1)) || args.at(-1) === undefined)) {
        given = args.pop() ?? {};
      }
      if (args.length !== parameters.length) {
        throw new TypeError(
          `${jsName}() takes ${parameters.length} arguments and an object of options, not ${args.length} arguments`,
        );
      }
      const json = { function: name };
      parameters.forEach(([parameter, kind], k) => {
        json[parameter] = KINDS[kind](request, args[k], parameter);
      });
      const unknown = Object.keys(given).filter((key) => !Object.hasOwn(options, key));
      if (unknown.length > 0) throw new TypeError(`${jsName}(): unknown options ${unknown.join(", ")}`);
      for (const [option, spec] of Object.entries(options)) {
        const required = spec.endsWith("!");
        const kind = required ? spec.slice(0, -1) : spec;
        const value = given[option];
        if (value === undefined || value === null) {
          if (required) throw new TypeError(`${jsName}() needs the option ${option}`);
          continue;
        }
        json[snake(option)] = KINDS[kind](request, value, option);
      }
      return this.call(request, json);
    };
    Object.defineProperty(fn, "name", { value: jsName });
    return fn;
  }
}

/** The doubles of the little-endian bytes of an answer. */
function toDoubles(bytes) {
  if (LITTLE_ENDIAN) return new Float64Array(bytes.buffer, bytes.byteOffset, bytes.length / 8);
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.length);
  return Float64Array.from({ length: bytes.length / 8 }, (_, k) => view.getFloat64(8 * k, true));
}

/** Whether a value of an answer is an array of its doubles, `{at, shape}`. */
function isDoubles(value) {
  return (
    value !== null &&
    typeof value === "object" &&
    !Array.isArray(value) &&
    Object.keys(value).length === 2 &&
    typeof value.at === "number" &&
    Array.isArray(value.shape)
  );
}

/** An array of the doubles of an answer: a number, a `Float64Array` or nested rows. */
function shaped(values, at, shape) {
  if (shape.length === 0) return values[at];
  if (shape.length === 1) return values.slice(at, at + shape[0]);
  const [rows, ...rest] = shape;
  const size = rest.reduce((product, n) => product * n, 1);
  return Array.from({ length: rows }, (_, i) => shaped(values, at + i * size, rest));
}

/** The value of an answer of JavaScript: its arrays of doubles, its keys in camelCase. */
function decode(value, values) {
  if (Array.isArray(value)) return value.map((item) => decode(item, values));
  if (isDoubles(value)) return shaped(values, value.at, value.shape);
  if (value !== null && typeof value === "object") {
    return Object.fromEntries(Object.entries(value).map(([key, item]) => [camel(key), decode(item, values)]));
  }
  return value;
}

/**
 * The statistics of a compiler: every function of `sbmlsim_core.stats` of python by its name in
 * camelCase and its constants by their names (`EPS`, `MARGIN`, `MIXTURE`, ...).
 *
 * @param {{compiler: {exports: WebAssembly.Exports}}} options the compiler of `createCompiler`
 *   (`compile.mjs`) of a `compiler.wasm` built with the feature `stats`, the default
 * @returns {object} the functions and the constants, synchronous
 */
export function createStats({ compiler } = {}) {
  if (!compiler) throw new TypeError("createStats needs the compiler of createCompiler");
  const api = new Interface(compiler);
  const stats = {};
  for (const [name, spec] of Object.entries(FUNCTIONS)) stats[camel(name)] = api.function(name, spec);
  const constants = api.call(new Request("constants"), { function: "constants" });
  // `MIXTURE` is a tuple of python, which cannot change
  Object.assign(stats, constants, { MIXTURE: Object.freeze(constants.MIXTURE) });
  return Object.freeze(stats);
}
