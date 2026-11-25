// The renderer of the figures of sbmlsim in a JavaScript host: a `figure/1` spec becomes the SVG
// the python package draws, byte for byte, its geometry in pixels, or the data point under a
// pointer.
//
//   import { loadPlot } from "sbmlsim-core/plot";
//   const plot = await loadPlot(source);             // `plot.wasm`: a URL, a path, its bytes or its WebAssembly.Module
//   const svg = plot.render(spec);                  // synchronous
//   const { width, height, plots, legend_entries } = plot.geometry(spec);
//   const hit = plot.pick(spec, x, y);              // `null` when no point is near
//
//   // a figure which is asked many times (a pointer moving over it) is prepared once: its spec is
//   // parsed and its layout planned once, and the handle answers without either
//   const figure = plot.prepare(spec);
//   figure.pick(x, y); figure.geometry(); figure.svg(); figure.dispose();
//
// `plot.wasm` is the module of `crates/plot-wasm`, built for `wasm32-wasip1`, whose interface is
// the C ABI of `allocate`, `render`, `geometry`, `pick`, `deallocate` and the handles of
// `prepare`, `render_prepared`, `geometry_prepared`, `pick_prepared` and `release` (see the
// documentation of that crate). Every call writes the spec into the memory of the module, calls the function,
// reads the result and frees every buffer, so nothing outlives a call but the instance.
//
// The module imports five functions of WASI preview1. `random_get` is called once for the seeds
// of the hash maps of the renderer, which decide no byte of a result, and gets the random bytes
// of `crypto.getRandomValues`; `fd_write` collects what the module writes (the message of a
// panic, which the `Error` of a failed instance carries) and the others throw.
//
// A panic aborts the module (a trap, or the error a stub throws) and leaves its memory in an
// unknown state, so the instance is never used again: any exception but a `PlotError` out of a
// call marks the `Plot` broken, that call and every later one of it throw an `Error` saying so,
// and `loadPlot` gives a new one.

const WASI = "wasi_snapshot_preview1";

/** The exports of `plot.wasm` this host calls. */
const PLOT_EXPORTS = [
  "allocate",
  "deallocate",
  "render",
  "geometry",
  "pick",
  "prepare",
  "render_prepared",
  "geometry_prepared",
  "pick_prepared",
  "release",
  "memory",
];

/** The most bytes of what the module writes which a `Plot` keeps for a message. */
const MAX_DIAGNOSTICS = 4096;

/** A spec which the renderer refuses, with its message (`path: what is wrong`). */
export class PlotError extends Error {
  constructor(message) {
    super(message);
    this.name = "PlotError";
  }
}

/** Whether this host is Node, whose strings can be paths. */
function isNode() {
  return typeof process !== "undefined" && Boolean(process.versions?.node);
}

/**
 * The bytes of `plot.wasm` from where `source` says, or the module when it is compiled already.
 *
 * @param {URL|string|ArrayBuffer|Uint8Array|WebAssembly.Module} source a URL, a path (Node), the
 *   bytes or the module
 * @returns {Promise<BufferSource|WebAssembly.Module>} the bytes or the module
 */
async function bytesOf(source) {
  if (source instanceof WebAssembly.Module) return source;
  if (source instanceof ArrayBuffer || ArrayBuffer.isView(source)) return source;
  if (!(source instanceof URL) && typeof source !== "string") {
    throw new TypeError("loadPlot takes a URL, a path, an ArrayBuffer, a Uint8Array or a WebAssembly.Module");
  }
  const text = String(source);
  // a specifier which is no literal, so a bundle for a browser does not resolve it
  const nodeFs = "node:fs/promises";
  const nodeUrl = "node:url";
  if (/^file:/i.test(text)) {
    if (!isNode()) throw new Error(`plot.wasm: ${text} is a file URL, which only Node reads`);
    const { fileURLToPath } = await import(nodeUrl);
    return (await import(nodeFs)).readFile(fileURLToPath(text));
  }
  if (typeof source === "string" && isNode() && !/^[a-z][a-z0-9+.-]*:\/\//i.test(text)) {
    return (await import(nodeFs)).readFile(source);
  }
  if (typeof source === "string" && !isNode() && !/^[a-z][a-z0-9+.-]*:/i.test(text)) {
    // no file system in a browser: a string is a URL, relative to the page
    const base = globalThis.location?.href;
    source = base ? new URL(text, base) : text;
  }
  let response;
  try {
    response = await fetch(source);
  } catch (error) {
    throw new Error(`plot.wasm: ${text} is read as a URL (a browser has no paths): ${error.message}`, {
      cause: error,
    });
  }
  if (!response.ok) throw new Error(`plot.wasm: ${response.status} for ${text}`);
  return response.arrayBuffer();
}

const encoder = new TextEncoder();
const decoder = new TextDecoder("utf-8", { fatal: true });

/**
 * An instance of `plot.wasm`.
 *
 * @param {WebAssembly.Module} module the compiled module
 * @returns {{exports: object, diagnostics: () => string}} the exports and what the module wrote
 */
function instantiate(module) {
  let memory = null;
  let written = "";
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
  // what the standard library writes before it aborts, i.e. the message of a panic
  imports.fd_write = (fd, iovs, count, nwritten) => {
    const view = new DataView(memory.buffer);
    let total = 0;
    for (let i = 0; i < count; i += 1) {
      const pointer = view.getUint32(iovs + 8 * i, true);
      const length = view.getUint32(iovs + 8 * i + 4, true);
      if (written.length < MAX_DIAGNOSTICS) {
        written += new TextDecoder().decode(new Uint8Array(memory.buffer, pointer, length));
      }
      total += length;
    }
    view.setUint32(nwritten, total, true);
    return 0;
  };
  const instance = new WebAssembly.Instance(module, { [WASI]: imports });
  memory = instance.exports.memory;
  instance.exports._initialize?.();
  return { exports: instance.exports, diagnostics: () => written.trim().slice(0, MAX_DIAGNOSTICS) };
}

/** The text of a spec: its JSON, or the string it is. */
function textOf(spec) {
  const text = typeof spec === "string" ? spec : JSON.stringify(spec);
  if (typeof text !== "string") throw new TypeError("the spec is not JSON");
  return text;
}

/** The renderer: one instance of `plot.wasm`, whose calls are synchronous. */
export class Plot {
  #exports;
  #diagnostics;
  #failure = null;

  /**
   * @param {object} exports the exports of an instance of `plot.wasm`
   * @param {() => string} [diagnostics] what the instance wrote, for the message of a failure
   */
  constructor(exports, diagnostics = () => "") {
    this.#exports = exports;
    this.#diagnostics = diagnostics;
  }

  /** The exports of the instance, for the prepared figures of this plot. */
  get _exports() {
    return this.#exports;
  }

  /** Whether an earlier call failed and this instance is unusable. */
  get broken() {
    return this.#failure !== null;
  }

  /** Throws when an earlier call failed. */
  #check() {
    if (this.#failure) {
      throw new Error(`plot.wasm failed in an earlier call (${this.#failure}); the instance is unusable, load a new Plot`);
    }
  }

  /** Marks the instance broken by an exception which is no refusal of a spec, before any free. */
  #note(error) {
    if (!(error instanceof PlotError)) this.#failure ??= error?.message ?? String(error);
  }

  /** Frees buffers, a failing free marks the instance broken. */
  #free(buffers) {
    if (this.#failure) return;
    try {
      for (const [pointer, length] of buffers) this.#exports.deallocate(pointer, length);
    } catch (error) {
      this.#failure = error.message;
    }
  }

  /**
   * Runs `body` on the instance: any exception but a `PlotError` marks it broken.
   *
   * @template T
   * @param {() => T} body the calls into the instance
   * @returns {T} its result
   */
  #guard(body) {
    this.#check();
    try {
      return body();
    } catch (error) {
      if (error instanceof PlotError) throw error;
      this.#note(error);
      const wrote = this.#diagnostics();
      throw new Error(
        `plot.wasm failed (${this.#failure})${wrote ? `: ${wrote}` : ""}; the instance is unusable, load a new Plot`,
        { cause: error },
      );
    }
  }

  /** The text at the two words of `out`, freed; a `PlotError` of it when `status` is not 1. */
  #take(out, status) {
    const exports = this.#exports;
    // the two words of `out`, little endian, at whatever alignment `allocate` gave
    const words = new DataView(exports.memory.buffer, out, 8);
    const pointer = words.getUint32(0, true);
    const length = words.getUint32(4, true);
    let text;
    try {
      text = decoder.decode(new Uint8Array(exports.memory.buffer, pointer, length));
    } finally {
      this.#free([[pointer, length]]);
    }
    if (status !== 1) throw new PlotError(text);
    return text;
  }

  /** Calls `invoke(pointer, length, out)` on the text of a spec and gives the text it answers. */
  #call(spec, invoke) {
    const input = encoder.encode(textOf(spec));
    return this.#guard(() => {
      const exports = this.#exports;
      let pointer = 0;
      let out = 0;
      try {
        pointer = exports.allocate(input.length);
        new Uint8Array(exports.memory.buffer, pointer, input.length).set(input);
        out = exports.allocate(8);
        return this.#take(out, invoke(pointer, input.length, out));
      } catch (error) {
        this.#note(error);
        throw error;
      } finally {
        this.#free([
          [out, 8],
          [pointer, input.length],
        ]);
      }
    });
  }

  /** Calls `invoke(out)` of a prepared figure and gives the text it answers. */
  _callPrepared(invoke) {
    return this.#guard(() => {
      const exports = this.#exports;
      let out = 0;
      try {
        out = exports.allocate(8);
        return this.#take(out, invoke(out));
      } catch (error) {
        this.#note(error);
        throw error;
      } finally {
        this.#free([[out, 8]]);
      }
    });
  }

  /** Releases a handle; nothing for an instance which failed, whose memory is gone with it. */
  _release(handle) {
    if (this.#failure) return;
    this.#guard(() => this.#exports.release(handle));
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
   * The geometry of a figure in its pixels: the `width` and `height` it is drawn at (the size of
   * the spec grown by the legends outside the plots, the size of the SVG), its `plots`, their
   * scales and the `legend_entries`.
   *
   * @param {object|string} spec a `figure/1` spec
   * @returns {object} the geometry
   * @throws {PlotError} for a spec which is refused
   */
  geometry(spec) {
    return JSON.parse(this.#call(spec, (p, n, out) => this.#exports.geometry(p, n, out)));
  }

  /**
   * The data point nearest to the pixel `(x, y)` of the figure within `radius` pixels. It parses
   * the spec and plans the layout on every call; a figure which is asked often is `prepare`d.
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

  /**
   * Parses a spec and plans its layout once, for the many answers of a pointer moving over it.
   *
   * @param {object|string} spec a `figure/1` spec
   * @returns {PreparedFigure} the figure, which holds memory of the module until it is disposed
   * @throws {PlotError} for a spec which is refused
   */
  prepare(spec) {
    const input = encoder.encode(textOf(spec));
    const handle = this.#guard(() => {
      const exports = this.#exports;
      let pointer = 0;
      let out = 0;
      try {
        pointer = exports.allocate(input.length);
        new Uint8Array(exports.memory.buffer, pointer, input.length).set(input);
        out = exports.allocate(8);
        const handle = exports.prepare(pointer, input.length, out);
        if (handle === 0) this.#take(out, 0);
        return handle;
      } catch (error) {
        this.#note(error);
        throw error;
      } finally {
        this.#free([
          [out, 8],
          [pointer, input.length],
        ]);
      }
    });
    return new PreparedFigure(this, handle);
  }
}

/** Releases the handle of a prepared figure which was dropped without `dispose()`. */
const REGISTRY = new FinalizationRegistry(({ plot, handle }) => {
  try {
    plot._release(handle);
  } catch {
    // the instance failed meanwhile: its memory is gone with it
  }
});

/** A figure of a `Plot` whose spec is parsed and whose layout is planned, see `Plot.prepare`. */
export class PreparedFigure {
  #plot;
  #handle;
  #exports;

  /**
   * @param {Plot} plot the plot which prepared it
   * @param {number} handle the handle of the module
   */
  constructor(plot, handle) {
    this.#plot = plot;
    this.#handle = handle;
    this.#exports = plot._exports;
    // the garbage collector releases a figure which is dropped without `dispose()`
    REGISTRY.register(this, { plot, handle }, this);
  }

  /** The handle, or a throw for a disposed figure. */
  #live() {
    if (this.#handle === 0) throw new Error("the prepared figure is disposed");
    return this.#handle;
  }

  /**
   * The SVG of the figure.
   *
   * @returns {string} the SVG, the bytes of `Plot.render` of its spec
   */
  svg() {
    const handle = this.#live();
    return this.#plot._callPrepared((out) => this.#exports.render_prepared(handle, out));
  }

  /**
   * The geometry of the figure in its pixels.
   *
   * @returns {object} as `Plot.geometry`
   */
  geometry() {
    const handle = this.#live();
    return JSON.parse(this.#plot._callPrepared((out) => this.#exports.geometry_prepared(handle, out)));
  }

  /**
   * The data point nearest to the pixel `(x, y)` within `radius` pixels.
   *
   * @param {number} x the pixel from the left
   * @param {number} y the pixel from the top
   * @param {number} [radius=8] the reach in pixels
   * @returns {object|null} as `Plot.pick`
   */
  pick(x, y, radius = 8) {
    const handle = this.#live();
    return JSON.parse(this.#plot._callPrepared((out) => this.#exports.pick_prepared(handle, x, y, radius, out)));
  }

  /** Frees the figure in the module (a figure dropped without it is freed when it is garbage collected, at a time nobody decides); a disposed figure throws when it is used, and disposing it again does nothing. */
  dispose() {
    if (this.#handle === 0) return;
    const handle = this.#handle;
    this.#handle = 0;
    REGISTRY.unregister(this);
    this.#plot._release(handle);
  }
}

/**
 * Loads `plot.wasm` into a new `Plot`.
 *
 * @param {URL|string|ArrayBuffer|Uint8Array|WebAssembly.Module} source a URL, a path (Node; in a
 *   browser a string is a URL), the bytes of the module or the module compiled already (which a
 *   page that loads several renderers compiles once and is not compiled again)
 * @returns {Promise<Plot>} a renderer of its own instance
 */
export async function loadPlot(source) {
  const bytes = await bytesOf(source);
  const module = bytes instanceof WebAssembly.Module ? bytes : await WebAssembly.compile(bytes);
  const exports = new Set(WebAssembly.Module.exports(module).map((entry) => entry.name));
  const missing = PLOT_EXPORTS.filter((name) => !exports.has(name));
  if (missing.length > 0) throw new Error(`plot.wasm does not export ${missing.join(", ")}`);
  const { exports: instance, diagnostics } = instantiate(module);
  return new Plot(instance, diagnostics);
}
