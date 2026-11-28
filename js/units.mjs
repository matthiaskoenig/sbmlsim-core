// The units of pint in a JavaScript host (since 0.45.0): unit expressions parsed, their factors
// to the root units and their dimensions, and the factors between two units.
//
// Plain ECMAScript without dependencies and without APIs of Node, so it runs in browsers as well:
//
//   import { createCompiler } from "sbmlsim-core/compile";
//   import { createUnits, UnitError } from "sbmlsim-core/units";
//   const units = await createUnits(await createCompiler({ compiler: compilerWasm }));
//   units.factor("mg/min", "g/hr"); // 0.06
//   units.parse("mg/l"); // { factor: 0.9999999999999999, dimensions: { "[length]": -3, "[mass]": 1 } }
//   units.define("IU_insulin", "0.0347 * mg");
//   units.convert([1, 2], "hr", "min"); // Float64Array [60, 120]
//   const model = units.withModelUnits({ unitIds: { mg_per_min: "mg/min" } });
//   model.factor("mg_per_min/kg", "mg/min/kg"); // 1
//   model.close();
//   units.close();
//
// The registry is the Rust of `sbmlsim-units` in `compiler.wasm` (the calls `units_open`,
// `units_define`, `units_parse`, `units_factor`, `units_with_model_units`, `units_unit_of` and
// `units_close` of `crates/wasm/src/units.rs`),
// the registry of python (`sbmlsim_core.units`) and of the scan planner, so a page converts with
// the numbers of pint as sbmlsim does.

/** The exports of `compiler.wasm` the units need. */
const UNITS_EXPORTS = [
  "allocate",
  "deallocate",
  "memory",
  "units_open",
  "units_define",
  "units_parse",
  "units_factor",
  "units_with_model_units",
  "units_unit_of",
  "units_close",
];

/**
 * A unit expression which is no unit, a unit with an offset (`degC`), two units of other
 * dimensions, whose message names both, or a unit defined again with another value.
 */
export class UnitError extends Error {
  constructor(message) {
    super(message);
    this.name = "UnitError";
  }
}

/** A number of an answer of the module: a number, or `"NaN"`, `"Infinity"`, `"-Infinity"`. */
function number(value) {
  return typeof value === "number" ? value : Number(value);
}

/** The exports of an instance of `compiler.wasm`: of a compiler, an instance, a module or bytes. */
async function exportsOf(compiler) {
  if (compiler?.exports !== undefined) return compiler.exports;
  if (compiler instanceof WebAssembly.Module) {
    return (await WebAssembly.instantiate(compiler, {})).exports;
  }
  if (compiler instanceof ArrayBuffer || ArrayBuffer.isView(compiler)) {
    return (await WebAssembly.instantiate(compiler, {})).instance.exports;
  }
  throw new TypeError(
    "createUnits needs compiler.wasm: a compiler of compile.mjs, its instance, its module or its bytes",
  );
}

/** A registry of units of one instance of `compiler.wasm`, see `createUnits`. */
class Units {
  constructor(exports, handle = exports.units_open()) {
    this.exports = exports;
    this.handle = handle;
    this.closed = false;
  }

  /** Calls a function of the registry with a text and gives its answer, or throws its message. */
  call(name, text) {
    if (this.closed) throw new Error("the unit registry is closed");
    const { allocate, deallocate, memory } = this.exports;
    const bytes = new TextEncoder().encode(text);
    const pointer = bytes.length === 0 ? 0 : allocate(bytes.length);
    if (pointer !== 0) new Uint8Array(memory.buffer, pointer, bytes.length).set(bytes);
    const out = allocate(8);
    try {
      const status = this.exports[name](this.handle, pointer, bytes.length, out);
      const [answer, length] = new Uint32Array(memory.buffer, out, 2);
      let message = "";
      if (answer !== 0 && length !== 0) {
        message = new TextDecoder().decode(new Uint8Array(memory.buffer, answer, length).slice());
        deallocate(answer, length);
      }
      if (status !== 1) throw new UnitError(message || `${name} failed`);
      return message;
    } finally {
      deallocate(out, 8);
      if (pointer !== 0) deallocate(pointer, bytes.length);
    }
  }

  /**
   * The factor of a unit expression to the root units of pint and its dimensions, by the names
   * of pint (`"[length]"`) with their exponents; a dimensionless unit has none.
   *
   * @param {string} expression a unit expression, `"mg/l"`, `"ml/min/(1.73*m^2)"`
   * @returns {{factor: number, dimensions: Record<string, number>}}
   * @throws {UnitError}
   */
  parse(expression) {
    const { factor, dimensions } = JSON.parse(this.call("units_parse", String(expression)));
    return { factor: number(factor), dimensions };
  }

  /**
   * The factor which converts a value in the unit `from` into the unit `to`, as pint converts:
   * exactly 1000 for `l` to `ml`.
   *
   * @param {string} from the unit of the value
   * @param {string} to the unit to convert it into
   * @returns {number}
   * @throws {UnitError} for two units of other dimensions, naming both
   */
  factor(from, to) {
    const request = JSON.stringify({ from: String(from), to: String(to) });
    return number(JSON.parse(this.call("units_factor", request)).factor);
  }

  /**
   * Values converted from the unit `from` into the unit `to`, each multiplied by the factor.
   *
   * @param {ArrayLike<number>} values
   * @param {string} from
   * @param {string} to
   * @returns {Float64Array}
   * @throws {UnitError}
   */
  convert(values, from, to) {
    const factor = this.factor(from, to);
    return Float64Array.from(values, (value) => value * factor);
  }

  /**
   * Defines a unit by an expression of the units of the registry,
   * `define("IU_insulin", "0.0347 * mg")`; the same value again changes nothing.
   *
   * @param {string} name
   * @param {string} expression
   * @throws {UnitError} for a name which is a unit of another value already
   */
  define(name, expression) {
    this.call("units_define", JSON.stringify({ name: String(name), expression: String(expression) }));
  }

  /**
   * A copy of the registry with the units of a model: the unit expressions of its entities by
   * id (`unitOf`) and its unit ids with their expressions, which name units in every
   * expression the copy parses (`mg_per_min/kg`), a unit id winning over a unit of the same
   * name. The copy is a registry of its own, which its caller closes.
   *
   * @param {{units?: Record<string, string>, unitIds?: Record<string, string>}} [model]
   * @returns {Units}
   */
  withModelUnits({ units = {}, unitIds = {} } = {}) {
    const request = JSON.stringify({ units, unit_ids: unitIds });
    return new Units(this.exports, Number(this.call("units_with_model_units", request)));
  }

  /**
   * The unit expression of an entity of the model of the registry (`withModelUnits`).
   *
   * @param {string} entity the id of the entity, `[S]` for the concentration of a species
   * @returns {string|null} its unit, `null` for an entity without one
   */
  unitOf(entity) {
    return JSON.parse(this.call("units_unit_of", String(entity))).unit;
  }

  /** Drops the registry in the module; further calls throw. */
  close() {
    if (!this.closed) {
      this.exports.units_close(this.handle);
      this.closed = true;
    }
  }
}

/**
 * A registry of units: the builtin units of pint which sbmlsim uses and the units it is given.
 *
 * @param {object|WebAssembly.Module|BufferSource} compiler `compiler.wasm` built with the
 *   feature `units` (the default): a compiler of `compile.mjs`, an instance, its module or its
 *   bytes
 * @returns {Promise<Units>} the registry: `parse`, `factor`, `convert`, `define`,
 *   `withModelUnits`, `unitOf`, `close`
 */
export async function createUnits(compiler) {
  const exports = await exportsOf(compiler);
  const missing = UNITS_EXPORTS.filter((name) => exports?.[name] === undefined);
  if (missing.length > 0) {
    throw new Error(
      `compiler.wasm does not export ${missing.join(", ")}: it is built without the feature units`,
    );
  }
  return new Units(exports);
}
