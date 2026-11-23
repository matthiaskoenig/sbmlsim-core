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
// milliseconds for a model of the size of the repressilator.

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

  /** The memory of the module, read again after every call: it grows while a model compiles. */
  bytes() {
    return new Uint8Array(this.exports.memory.buffer);
  }

  /**
   * Compiles an SBML Level 3 Version 2 document.
   *
   * @param {string} sbml the text of the document
   * @param {{optimize?: boolean, jacobian?: "auto"|"difference-quotient"|"analytic"}} options
   *   `optimize` runs the optimizer of the compiler (the default, the results are the same
   *   without it) and `jacobian` is the source of the Jacobian of the model
   * @returns {{wasm: Uint8Array, description: object}} `model.wasm` and `model.json`
   * @throws {CompileError} for a document the compiler refuses
   */
  compile(sbml, { optimize = true, jacobian = "auto" } = {}) {
    if (typeof sbml !== "string") {
      throw new TypeError("the document of a compilation is text");
    }
    const choice = JACOBIAN[jacobian];
    if (choice === undefined) {
      throw new TypeError(`the Jacobian ${jacobian} is none of ${Object.keys(JACOBIAN)}`);
    }
    const { allocate, compile, deallocate } = this.exports;
    const document_ = new TextEncoder().encode(sbml);
    const input = allocate(document_.length);
    const words = allocate(4 * 4);
    try {
      if (document_.length > 0) {
        this.bytes().set(document_, input);
      }
      const status = compile(input, document_.length, optimize ? 1 : 0, choice, words);
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
