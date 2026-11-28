// The compiler of sbmlsim-core in a JavaScript host: compiles SBML text into `model.wasm` and
// its description, which `loader.mjs` instantiates and simulates.
//
// Plain ECMAScript without dependencies and without APIs of Node (no `fs`, no `process`), so it
// runs in browsers as well:
//
//   import { createCompiler } from "sbmlsim-core/compile";
//   import { createRuntime } from "sbmlsim-core";
//   const compiler = await createCompiler({ compiler: compilerWasm }); // a Module or bytes
//   const { wasm, description } = compiler.compile(sbmlText);
//   const runtime = await createRuntime({ engine, mathlib });
//   const instance = await runtime.instantiate(wasm, description);
//
// `compiler.wasm` is the module of `crates/wasm`, built for `wasm32-unknown-unknown`: it is the
// compiler of the command line and of the python package (`sbmlsim-core compile`), the same
// reader, the same model representation and the same code generation, so a model compiled in a
// page and the same model compiled by the command line give the same `model.wasm`. The module
// reads no file and imports nothing of the host, so a page compiles without a server.
//
// The interface of the module is the C ABI of `allocate`, `compile` and `deallocate`
// (`crates/wasm/src/lib.rs`): the document goes into the memory of the module as the bytes of
// its UTF-8, the result comes back as the description (the JSON of `model.json`) and the bytes
// of `model.wasm`, and every buffer is freed again. `compile` is synchronous and takes a few
// milliseconds for a model of the size of the repressilator. `compileKernel` (since 0.41.0)
// compiles the batched kernel of a model instead (`compile_kernel`, `kernel.wasm` and
// `kernel.json`), which `kernel.mjs` instantiates.

// The buffers of a call into `compiler.wasm` (`Buffers`, `withBuffers`), which this module,
// `petab.mjs`, `stats.mjs` and every other host of an export of the module use: the arguments
// copied into the memory of the module, the words of the result read out of it, and every buffer
// freed again, whatever the call throws.
//
// - Every buffer of a call (the arguments, the words of the result and the buffers the words
//   point at) is freed when the call ends, also when the module traps or a result does not
//   decode, so a page which calls the module many times does not lose its memory.
// - `allocate` gives 0 for a buffer it cannot allocate (the memory of the module cannot grow): a
//   host which wrote there would overwrite the memory at the address 0 of the module, so a failed
//   allocation throws instead.
// - A buffer of `allocate` has the alignment of bytes and every word of WebAssembly is
//   little-endian, so a word is read with a `DataView`, little-endian at any address; a
//   `Uint32Array` has the byte order of the host and refuses an address which is not a multiple
//   of 4.

/** The bytes of a word of `wasm32`, a `usize` of the module. */
const WORD = 4;

/** The buffers of one call into a module, freed together by `free`. */
export class Buffers {
  /** The exports of the module: `memory`, `allocate` and `deallocate`. */
  #exports;
  /** The name of the module in a message. */
  #name;
  /** The buffers to free, `[pointer, length]`. */
  #owned = [];

  /**
   * @param {{memory: WebAssembly.Memory, allocate: (length: number) => number,
   *   deallocate: (pointer: number, length: number) => void}} exports the exports of the module
   * @param {string} name the name of the module, e.g. `compiler.wasm`
   */
  constructor(exports, name) {
    this.#exports = exports;
    this.#name = name;
  }

  /** The bytes of the memory of the module, read again for every view: the memory grows. */
  #memory() {
    return this.#exports.memory.buffer;
  }

  /**
   * A new buffer of `length` bytes, freed with the others; 0 for a length of 0.
   *
   * @param {number} length the length in bytes
   * @returns {number} the address of the buffer
   * @throws {Error} when the module cannot allocate the buffer
   */
  allocate(length) {
    if (length === 0) return 0;
    // an address above 2 GiB is a negative i32 in JavaScript
    const pointer = this.#exports.allocate(length) >>> 0;
    if (pointer === 0) {
      throw new Error(`${this.#name} could not allocate ${length} bytes`);
    }
    this.#owned.push([pointer, length]);
    return pointer;
  }

  /**
   * Copies bytes into a new buffer, freed with the others.
   *
   * @param {ArrayBufferView|ArrayBuffer} bytes the bytes: of a typed array its bytes, not its
   *   elements (a `Float64Array` of 2 values is 16 bytes)
   * @returns {[number, number]} the address and the length of the buffer, `[0, 0]` for no bytes
   */
  put(bytes) {
    const source = ArrayBuffer.isView(bytes)
      ? new Uint8Array(bytes.buffer, bytes.byteOffset, bytes.byteLength)
      : new Uint8Array(bytes);
    const pointer = this.allocate(source.length);
    if (source.length > 0) new Uint8Array(this.#memory(), pointer, source.length).set(source);
    return [pointer, source.length];
  }

  /**
   * Takes a buffer the module gave to the caller, so it is freed with the others; the null
   * pointer and a length of 0 are no buffer.
   *
   * @param {number} pointer the address of the buffer
   * @param {number} length its length in bytes
   */
  adopt(pointer, length) {
    if (pointer !== 0 && length !== 0) this.#owned.push([pointer >>> 0, length]);
  }

  /**
   * The buffers of a result, which a call wrote as pairs of words (the address and the length of
   * each) to `out`: the buffers are freed with the others, their bytes are copied.
   *
   * @param {number} out the address of the words, a buffer of `allocate` of `2 * pairs` words
   * @param {number} pairs the number of the buffers
   * @returns {Uint8Array[]} the bytes of each buffer, empty for a buffer the call left empty
   */
  results(out, pairs) {
    const words = this.words(out, 2 * pairs);
    const buffers = [];
    for (let k = 0; k < words.length; k += 2) buffers.push([words[k], words[k + 1]]);
    // every buffer is taken before any is read, so a read which throws frees them all
    for (const [pointer, length] of buffers) this.adopt(pointer, length);
    return buffers.map(([pointer, length]) =>
      pointer === 0 || length === 0
        ? new Uint8Array(0)
        : new Uint8Array(this.#memory(), pointer, length).slice(),
    );
  }

  /**
   * The words of `wasm32` at an address, little-endian at any alignment.
   *
   * @param {number} pointer the address of the first word
   * @param {number} count the number of the words
   * @returns {number[]} the words, unsigned
   */
  words(pointer, count) {
    const view = new DataView(this.#memory(), pointer, WORD * count);
    return Array.from({ length: count }, (_, k) => view.getUint32(WORD * k, true));
  }

  /**
   * Frees every buffer, the last first; a buffer is freed once, whatever a free throws.
   *
   * @returns {unknown} the first error a free threw, `undefined` when none did
   */
  free() {
    let failure;
    for (const [pointer, length] of this.#owned.splice(0).reverse()) {
      try {
        this.#exports.deallocate(pointer, length);
      } catch (error) {
        failure ??= error;
      }
    }
    return failure;
  }
}

/**
 * Runs `body` with the buffers of a call and frees them when it ends. An error of `body` is
 * thrown as it is; an error of a free is thrown when `body` threw none.
 *
 * @template T
 * @param {object} exports the exports of the module: `memory`, `allocate` and `deallocate`
 * @param {string} name the name of the module in a message
 * @param {(buffers: Buffers) => T} body the call
 * @returns {T} the result of `body`
 */
export function withBuffers(exports, name, body) {
  const buffers = new Buffers(exports, name);
  let result;
  try {
    result = body(buffers);
  } catch (error) {
    buffers.free();
    throw error;
  }
  const failure = buffers.free();
  if (failure !== undefined) throw failure;
  return result;
}

/** The exports of `compiler.wasm` this host calls. */
const COMPILER_EXPORTS = ["allocate", "compile", "deallocate", "memory"];

/** The source of the Jacobian of a model, the numbers of the interface. */
const JACOBIAN = { auto: 0, "difference-quotient": 1, analytic: 2 };

/** A model which the compiler refuses, with the message of the compiler. */
export class CompileError extends Error {
  constructor(message) {
    super(message);
    this.name = "CompileError";
  }
}

/**
 * Compiles `compiler.wasm` when it is given as bytes, and checks its exports.
 *
 * @param {WebAssembly.Module|BufferSource} source the module or its bytes
 * @returns {Promise<WebAssembly.Module>} the compiled module
 */
async function moduleOf(source) {
  const module =
    source instanceof WebAssembly.Module ? source : await WebAssembly.compile(source);
  const exports = new Set(WebAssembly.Module.exports(module).map((entry) => entry.name));
  const missing = COMPILER_EXPORTS.filter((name) => !exports.has(name));
  if (missing.length > 0) {
    throw new Error(`compiler.wasm does not export ${missing.join(", ")}`);
  }
  if (WebAssembly.Module.imports(module).length > 0) {
    throw new Error("compiler.wasm imports from its host, which this host does not provide");
  }
  return module;
}

/** The compiler of a host: one instance of `compiler.wasm`, which compiles one model at a time. */
class Compiler {
  /**
   * @param {WebAssembly.Module} module the compiled `compiler.wasm`
   * @param {WebAssembly.Instance} instance its instance
   */
  constructor(module, instance) {
    /** The compiled module, which a worker gets to create a compiler of its own. */
    this.module = module;
    this.exports = instance.exports;
  }

  /**
   * Compiles an SBML Level 3 Version 2 document.
   *
   * @param {string} sbml the text of the document
   * @param {{optimize?: boolean, jacobian?: "auto"|"difference-quotient"|"analytic", parameterJacobian?: boolean}} options
   *   `optimize` runs the optimizer of the compiler (the default, the results are the same
   *   without it), `jacobian` is the source of the state Jacobian, and
   *   `parameterJacobian: false` omits analytic parameter derivatives and their constants
   * @returns {{wasm: Uint8Array, description: object}} `model.wasm` and `model.json`
   * @throws {CompileError} for a document the compiler refuses
   */
  compile(sbml, { optimize = true, jacobian = "auto", parameterJacobian = true } = {}) {
    if (typeof sbml !== "string") {
      throw new TypeError("the document of a compilation is text");
    }
    const choice = JACOBIAN[jacobian];
    if (choice === undefined) {
      throw new TypeError(`the Jacobian ${jacobian} is none of ${Object.keys(JACOBIAN)}`);
    }
    if (typeof parameterJacobian !== "boolean") {
      throw new TypeError("parameterJacobian must be a boolean");
    }
    if (!parameterJacobian && typeof this.exports.compile_with_options !== "function") {
      throw new Error("compiler.wasm does not support parameterJacobian: false");
    }
    const { compile, compile_with_options } = this.exports;
    const flag = optimize ? 1 : 0;
    return this.run(sbml, (input, length, words) =>
      parameterJacobian
        ? compile(input, length, flag, choice, words)
        : compile_with_options(input, length, flag, choice, 0, words),
    );
  }

  /**
   * Compiles an SBML Level 3 Version 2 document into a batched kernel (`kernel/1`, since
   * 0.41.0): the model with its RODAS4 integrator in one module, which `kernel.mjs` instantiates
   * and which advances many points (the cells of a tissue) per call. The bytes and the description are those of
   * `sbmlsim-core compile --kernel`.
   *
   * @param {string} sbml the text of the document
   * @returns {{wasm: Uint8Array, description: object}} `kernel.wasm` and `kernel.json`
   * @throws {CompileError} for a document the compiler refuses, e.g. a model with events
   */
  compileKernel(sbml) {
    if (typeof sbml !== "string") {
      throw new TypeError("the document of a compilation is text");
    }
    const { compile_kernel } = this.exports;
    if (typeof compile_kernel !== "function") {
      throw new Error("compiler.wasm does not compile kernels");
    }
    return this.run(sbml, (input, length, words) => compile_kernel(input, length, words));
  }

  /**
   * Runs a compilation of the module: the document into a buffer, `call(input, length, words)`
   * of a function which writes its result to four words (the description or the message, and
   * the module), the result out of the memory and every buffer freed.
   *
   * @param {string} sbml the text of the document
   * @param {(input: number, length: number, words: number) => number} call the compilation,
   *   1 for a document it compiled
   * @returns {{wasm: Uint8Array, description: object}} the module and the description
   * @throws {CompileError} for a document the compiler refuses
   */
  run(sbml, call) {
    const document_ = new TextEncoder().encode(sbml);
    return withBuffers(this.exports, "compiler.wasm", (buffers) => {
      const [input, length] = buffers.put(document_);
      const words = buffers.allocate(4 * 4);
      const status = call(input, length, words);
      const [text, wasm] = buffers.results(words, 2);
      if (status !== 1) {
        throw new CompileError(
          new TextDecoder().decode(text) || "the compiler refused the document",
        );
      }
      return { wasm, description: JSON.parse(new TextDecoder().decode(text)) };
    });
  }

  /**
   * Generate standalone ODE code or a document, without a server or simulation runtime.
   * @param {string|BufferSource} sbml source text or bytes
   * @param {"python"|"diffrax"|"julia"|"r"|"markdown"|"latex"|"typst"} format
   * @param {{simulator?:boolean,standalone?:boolean,symbols?:"id"|"name",source?:string,
   * documents?:Record<string,string>,location?:string,maxExpandedElements?:number}} options
   * @returns {{filename:string,content:string}}
   */
  exportOde(sbml, format, {
    simulator = true, standalone = true, symbols = "id", source,
    documents = {}, location = "./model.xml", maxExpandedElements = 10000,
  } = {}) {
    if (typeof this.exports.export_ode !== "function") {
      throw new Error("compiler.wasm does not support ODE exports");
    }
    if (typeof simulator !== "boolean" || typeof standalone !== "boolean") {
      throw new TypeError("simulator and standalone must be booleans");
    }
    if (!Number.isSafeInteger(maxExpandedElements) || maxExpandedElements < 0) {
      throw new TypeError("maxExpandedElements must be a non-negative integer");
    }
    if (typeof sbml !== "string" && !ArrayBuffer.isView(sbml) && !(sbml instanceof ArrayBuffer)) {
      throw new TypeError("SBML source must be text or a BufferSource");
    }
    if (typeof format !== "string" || typeof location !== "string" || (source !== undefined && typeof source !== "string")) {
      throw new TypeError("format, location and source must be strings");
    }
    if (!documents || typeof documents !== "object" || Array.isArray(documents)
        || Object.values(documents).some(value => typeof value !== "string")) {
      throw new TypeError("documents must map source locations to XML strings");
    }
    const encoder = new TextEncoder();
    const bytes = typeof sbml === "string" ? encoder.encode(sbml)
      : ArrayBuffer.isView(sbml) ? new Uint8Array(sbml.buffer, sbml.byteOffset, sbml.byteLength)
      : new Uint8Array(sbml);
    const config = encoder.encode(JSON.stringify({
      format, location, documents, max_expanded_elements: maxExpandedElements,
      render: { simulator, standalone, symbols, source },
    }));
    if (bytes.byteLength > 100 * 1024 * 1024 || config.byteLength > 100 * 1024 * 1024) {
      throw new RangeError("The source or options exceed 100 MB");
    }
    const { export_ode } = this.exports;
    return withBuffers(this.exports, "compiler.wasm", (buffers) => {
      const input = buffers.put(bytes);
      const options = buffers.put(config);
      const words = buffers.allocate(4 * 4);
      const status = export_ode(...input, ...options, words);
      const [answer] = buffers.results(words, 2);
      const text = new TextDecoder().decode(answer);
      if (status !== 1) throw new CompileError(text || "The source export failed");
      return JSON.parse(text);
    });
  }
}

/**
 * Creates the compiler of a host.
 *
 * @param {{compiler: WebAssembly.Module|BufferSource}} options `compiler.wasm` of this version
 *   of sbmlsim-core, as a compiled module or as its bytes
 * @returns {Promise<Compiler>} the compiler, whose `compile` is synchronous
 */
export async function createCompiler({ compiler }) {
  if (!compiler) {
    throw new TypeError("createCompiler needs compiler.wasm");
  }
  const module = await moduleOf(compiler);
  const instance = await WebAssembly.instantiate(module, {});
  return new Compiler(module, instance);
}
