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
// The interface of the module is the C ABI of `allocate`, `compile`, `compile_kernel` and
// `deallocate` (`crates/wasm/src/lib.rs`): the document and the options (their JSON) go into the
// memory of the module as the bytes of their UTF-8, the result comes back as the description
// (the JSON of `model.json`) and the bytes of `model.wasm`, and every buffer is freed again.
// `compile` is synchronous and takes a few milliseconds for a model of the size of the
// repressilator. `compileKernel` (since 0.41.0) compiles the batched kernel of a model instead
// (`compile_kernel`, `kernel.wasm` and `kernel.json` of `kernel/2`), which `kernel.mjs`
// instantiates. Both drive the species of `driven` (`sbmlsim_ir::driven`).

/** The exports of `compiler.wasm` this host calls. */
const COMPILER_EXPORTS = ["allocate", "compile", "compile_kernel", "deallocate", "memory"];

/** The source of the Jacobian of a model, the names of the options of the interface. */
const JACOBIAN = {
  auto: "auto",
  "difference-quotient": "difference_quotient",
  analytic: "analytic",
};

/**
 * Checks the driven species of a compilation: SBML ids, an array of strings.
 *
 * @param {unknown} driven the option
 * @returns {string[]} the ids
 */
function drivenIds(driven) {
  if (!Array.isArray(driven) || driven.some((id) => typeof id !== "string")) {
    throw new TypeError("driven must be an array of the SBML ids of species");
  }
  return driven;
}

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

  /** The memory of the module, read again after every call: it grows while a model compiles. */
  bytes() {
    return new Uint8Array(this.exports.memory.buffer);
  }

  /**
   * Compiles an SBML Level 3 Version 2 document.
   *
   * @param {string} sbml the text of the document
   * @param {{optimize?: boolean, jacobian?: "auto"|"difference-quotient"|"analytic",
   *   parameterJacobian?: boolean, driven?: string[]}} options
   *   `optimize` runs the optimizer of the compiler (the default, the results are the same
   *   without it), `jacobian` is the source of the state Jacobian,
   *   `parameterJacobian: false` omits analytic parameter derivatives and their constants, and
   *   `driven` are the ids of the species to drive (each follows the path
   *   `X__value + X__slope * (time - driven__t_ref)` of three parameters, with the accumulator
   *   state `X__exchange`; `description.driven` lists them)
   * @returns {{wasm: Uint8Array, description: object}} `model.wasm` and `model.json`
   * @throws {CompileError} for a document the compiler refuses
   */
  compile(
    sbml,
    { optimize = true, jacobian = "auto", parameterJacobian = true, driven = [] } = {},
  ) {
    if (typeof sbml !== "string") {
      throw new TypeError("the document of a compilation is text");
    }
    const choice = JACOBIAN[jacobian];
    if (choice === undefined) {
      throw new TypeError(`the Jacobian ${jacobian} is none of ${Object.keys(JACOBIAN)}`);
    }
    if (typeof optimize !== "boolean") {
      throw new TypeError("optimize must be a boolean");
    }
    if (typeof parameterJacobian !== "boolean") {
      throw new TypeError("parameterJacobian must be a boolean");
    }
    const options = {
      optimize,
      jacobian: choice,
      parameter_jacobian: parameterJacobian,
      driven: drivenIds(driven),
    };
    return this.run(sbml, options, this.exports.compile);
  }

  /**
   * Compiles an SBML Level 3 Version 2 document into a batched kernel (`kernel/2`): the model
   * with its RODAS4 integrator in one module, which `kernel.mjs` instantiates and which advances
   * many points (the cells of a tissue) per call. The bytes and the description are those of
   * `sbmlsim-core compile --kernel` (with `--driven`).
   *
   * @param {string} sbml the text of the document
   * @param {{driven?: string[]}} options `driven` are the ids of the species to drive, whose
   *   paths the caller writes into the parameters of every point before an advance
   *   (`description.model.driven` and `description.model.t_ref` give their indices)
   * @returns {{wasm: Uint8Array, description: object}} `kernel.wasm` and `kernel.json`
   * @throws {CompileError} for a document the compiler refuses, e.g. a model with events
   */
  compileKernel(sbml, { driven = [] } = {}) {
    if (typeof sbml !== "string") {
      throw new TypeError("the document of a compilation is text");
    }
    return this.run(sbml, { driven: drivenIds(driven) }, this.exports.compile_kernel);
  }

  /**
   * Runs a compilation of the module: the document and the JSON of the options into buffers,
   * `call(input, length, options, optionsLength, words)` of a function which writes its result
   * to four words (the description or the message, and the module), the result out of the
   * memory and every buffer freed.
   *
   * @param {string} sbml the text of the document
   * @param {object} options the options of the compilation, as the interface reads their JSON
   * @param {(input: number, length: number, options: number, optionsLength: number,
   *   words: number) => number} call the compilation, 1 for a document it compiled
   * @returns {{wasm: Uint8Array, description: object}} the module and the description
   * @throws {CompileError} for a document the compiler refuses
   */
  run(sbml, options, call) {
    const { allocate, deallocate } = this.exports;
    const encoder = new TextEncoder();
    const document_ = encoder.encode(sbml);
    const config = encoder.encode(JSON.stringify(options));
    const input = allocate(document_.length);
    const settings = allocate(config.length);
    const words = allocate(4 * 4);
    try {
      if (document_.length > 0) {
        this.bytes().set(document_, input);
      }
      this.bytes().set(config, settings);
      const status = call(input, document_.length, settings, config.length, words);
      const header = new Uint32Array(this.exports.memory.buffer, words, 4);
      const [textPointer, textLength, wasmPointer, wasmLength] = header;
      try {
        const text = this.read(textPointer, textLength);
        if (status !== 1) {
          throw new CompileError(
            new TextDecoder().decode(text) || "the compiler refused the document",
          );
        }
        return {
          wasm: this.read(wasmPointer, wasmLength).slice(),
          description: JSON.parse(new TextDecoder().decode(text)),
        };
      } finally {
        deallocate(textPointer, textLength);
        deallocate(wasmPointer, wasmLength);
      }
    } finally {
      deallocate(words, 4 * 4);
      deallocate(settings, config.length);
      deallocate(input, document_.length);
    }
  }

  /**
   * The bytes of a buffer of the module, as a view on its memory.
   *
   * @param {number} pointer the address of the buffer
   * @param {number} length its length in bytes
   * @returns {Uint8Array} the view, empty when the buffer is
   */
  read(pointer, length) {
    if (pointer === 0 || length === 0) {
      return new Uint8Array(0);
    }
    return new Uint8Array(this.exports.memory.buffer, pointer, length);
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
    const { allocate, deallocate, export_ode } = this.exports;
    const input = allocate(bytes.length);
    const options = allocate(config.length);
    const words = allocate(16);
    try {
      this.bytes().set(bytes, input);
      this.bytes().set(config, options);
      const status = export_ode(input, bytes.length, options, config.length, words);
      const [pointer, length] = new Uint32Array(this.exports.memory.buffer, words, 4);
      try {
        const text = new TextDecoder().decode(this.read(pointer, length));
        if (status !== 1) throw new CompileError(text || "The source export failed");
        return JSON.parse(text);
      } finally {
        deallocate(pointer, length);
      }
    } finally {
      deallocate(words, 16);
      deallocate(options, config.length);
      deallocate(input, bytes.length);
    }
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
