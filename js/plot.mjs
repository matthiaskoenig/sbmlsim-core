// The renderer of the figures of sbmlsim in a JavaScript host: a `figure/1` spec becomes the SVG
// the python package draws, byte for byte, its geometry in pixels, or the data point under a
// pointer.
//
//   import { loadPlot } from "sbmlsim-core/plot";
//   const plot = await loadPlot(bytesOrUrlOrPath); // `plot.wasm`
//   const svg = plot.render(spec);                  // synchronous
//   const { panels } = plot.geometry(spec);
//   const hit = plot.pick(spec, x, y);              // `null` when no point is near
//
// `plot.wasm` is the module of `crates/plot-wasm`, built for `wasm32-wasip1`, whose interface is
// the C ABI of `allocate`, `render`, `geometry`, `pick` and `deallocate` (see the documentation
// of that crate). Every call writes the spec into the memory of the module, calls the function,
// reads the result and frees every buffer, so nothing outlives a call but the instance.
//
// The module imports five functions of WASI preview1. `random_get` is called once for the seeds
// of the hash maps of the renderer, which decide no byte of a result, and gets the random bytes
// of `crypto.getRandomValues`; the others are called at most on a panic, and throw.
//
// A panic aborts the module (a trap) and leaves its memory in an unknown state, so the instance
// is never used again: the call which trapped and every later one of that `Plot` throw an
// `Error` saying so, and `loadPlot` gives a new one.

const WASI = "wasi_snapshot_preview1";

/** The exports of `plot.wasm` this host calls. */
const PLOT_EXPORTS = ["allocate", "deallocate", "render", "geometry", "pick", "memory"];

/** A spec which the renderer refuses, with its message (`path: what is wrong`). */
export class PlotError extends Error {
  constructor(message) {
    super(message);
    this.name = "PlotError";
  }
}

/**
 * The bytes of `plot.wasm` from where `source` says.
 *
 * @param {URL|string|ArrayBuffer|Uint8Array} source a URL, a path (Node), or the bytes
 * @returns {Promise<BufferSource>} the bytes
 */
async function bytesOf(source) {
  if (source instanceof ArrayBuffer || ArrayBuffer.isView(source)) return source;
  const isUrl = source instanceof URL || (typeof source === "string" && /^[a-z][a-z0-9+.-]+:\/\//i.test(source));
  if (isUrl && !String(source).startsWith("file:")) {
    const response = await fetch(source);
    if (!response.ok) throw new Error(`plot.wasm: ${response.status} for ${source}`);
    return response.arrayBuffer();
  }
  if (typeof source === "string" || source instanceof URL) {
    // a specifier which is no literal, so a bundle for a browser does not resolve it
    const fs = "node:fs/promises";
    const { readFile } = await import(fs);
    return readFile(source);
  }
  throw new TypeError("loadPlot takes a URL, a path, an ArrayBuffer or a Uint8Array");
}

/**
 * An instance of `plot.wasm` and the calls of its interface.
 *
 * @param {WebAssembly.Module} module the compiled module
 */
function instantiate(module) {
  let memory = null;
  const imports = {};
  for (const { module: from, name, kind } of WebAssembly.Module.imports(module)) {
    if (from !== WASI || kind !== "function") {
      throw new Error(`plot.wasm imports ${from}.${name} (${kind}), which is no function of WASI`);
    }
    imports[name] = () => {
      throw new Error(`plot.wasm called WASI ${name}`);
    };
  }
  imports.random_get = (pointer, length) => {
    // `crypto.getRandomValues` takes at most 65536 bytes per call
    const bytes = new Uint8Array(memory.buffer, pointer, length);
    for (let at = 0; at < length; at += 65536) {
      globalThis.crypto.getRandomValues(bytes.subarray(at, Math.min(at + 65536, length)));
    }
    return 0;
  };
  const instance = new WebAssembly.Instance(module, { [WASI]: imports });
  memory = instance.exports.memory;
  instance.exports._initialize?.();
  return instance.exports;
}

const encoder = new TextEncoder();
const decoder = new TextDecoder("utf-8", { fatal: true });

/** The renderer: one instance of `plot.wasm`, whose calls are synchronous. */
export class Plot {
  #exports;
  #broken = false;

  /** @param {object} exports the exports of an instance of `plot.wasm` */
  constructor(exports) {
    this.#exports = exports;
  }

  /** Calls `invoke(pointer, length, out)` on the text of a spec and gives the text it answers. */
  #call(spec, invoke) {
    if (this.#broken) {
      throw new Error("plot.wasm trapped in an earlier call and its instance is unusable; load a new Plot");
    }
    const text = typeof spec === "string" ? spec : JSON.stringify(spec);
    if (typeof text !== "string") throw new TypeError("the spec is not JSON");
    const exports = this.#exports;
    const input = encoder.encode(text);
    let pointer = 0;
    let out = 0;
    try {
      pointer = exports.allocate(input.length);
      new Uint8Array(exports.memory.buffer, pointer, input.length).set(input);
      out = exports.allocate(8);
      const status = invoke(pointer, input.length, out);
      // the two words of `out`, little endian, at whatever alignment `allocate` gave
      const words = new DataView(exports.memory.buffer, out, 8);
      const resultPointer = words.getUint32(0, true);
      const resultLength = words.getUint32(4, true);
      const result = decoder.decode(new Uint8Array(exports.memory.buffer, resultPointer, resultLength));
      exports.deallocate(resultPointer, resultLength);
      exports.deallocate(out, 8);
      exports.deallocate(pointer, input.length);
      if (status !== 1) throw new PlotError(result);
      return result;
    } catch (error) {
      if (error instanceof PlotError) throw error;
      if (error instanceof WebAssembly.RuntimeError) {
        this.#broken = true;
        throw new Error(`plot.wasm trapped (${error.message}); the instance is unusable, load a new Plot`, {
          cause: error,
        });
      }
      throw error;
    }
  }

  /**
   * The SVG of a figure, the bytes of the renderer of the python package.
   *
   * @param {object|string} spec a `figure/1` spec, an object or its JSON text
   * @returns {string} the SVG
   * @throws {PlotError} with `path: what is wrong` for a spec which is refused
   */
  render(spec) {
    return this.#call(spec, (p, n, out) => this.#exports.render(p, n, out));
  }

  /**
   * The geometry of a figure in its pixels: its panels, their scales and the entries of its
   * legends.
   *
   * @param {object|string} spec a `figure/1` spec
   * @returns {object} the geometry
   * @throws {PlotError} for a spec which is refused
   */
  geometry(spec) {
    return JSON.parse(this.#call(spec, (p, n, out) => this.#exports.geometry(p, n, out)));
  }

  /**
   * The data point nearest to the pixel `(x, y)` of the figure within `radius` pixels.
   *
   * @param {object|string} spec a `figure/1` spec
   * @param {number} x the pixel from the left
   * @param {number} y the pixel from the top
   * @param {number} [radius=8] the reach in pixels
   * @returns {object|null} the point in data units, or `null` when there is none
   * @throws {PlotError} for a spec which is refused
   */
  pick(spec, x, y, radius = 8) {
    return JSON.parse(this.#call(spec, (p, n, out) => this.#exports.pick(p, n, x, y, radius, out)));
  }
}

/**
 * Loads `plot.wasm` into a new `Plot`.
 *
 * @param {URL|string|ArrayBuffer|Uint8Array} source a URL, a path (Node) or the bytes of the module
 * @returns {Promise<Plot>} a renderer of its own instance
 */
export async function loadPlot(source) {
  const bytes = await bytesOf(source);
  const module = bytes instanceof WebAssembly.Module ? bytes : await WebAssembly.compile(bytes);
  const exports = new Set(WebAssembly.Module.exports(module).map((entry) => entry.name));
  const missing = PLOT_EXPORTS.filter((name) => !exports.has(name));
  if (missing.length > 0) throw new Error(`plot.wasm does not export ${missing.join(", ")}`);
  return new Plot(instantiate(module));
}
