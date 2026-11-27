// The batched kernel of sbmlsim-core in a JavaScript host (since 0.41.0): instantiates `kernel.wasm` (the
// contract `kernel/1`, a model with its RODAS4 integrator in one module, which
// `compiler.compileKernel` of `compile.mjs` and `sbmlsim-core compile --kernel` write) with
// mathlib and a memory of the caller, which holds every array of the points.
//
// Plain ECMAScript without dependencies and without APIs of Node, so it runs in browsers as well:
//
//   import { createCompiler } from "sbmlsim-core/compile";
//   import { instantiateKernel } from "sbmlsim-core/kernel";
//   const compiler = await createCompiler({ compiler: compilerWasm }); // a Module or bytes
//   const { wasm, description } = compiler.compileKernel(sbmlText);
//   const memory = new WebAssembly.Memory({ initial: 16 });
//   const kernel = await instantiateKernel({ wasm, mathlib, memory });
//   kernel.init(n, parameters, state);
//   const failed = kernel.advance(n, state, parameters, control, counts, context, scratch);
//   kernel.assign(n, state, parameters, control);
//
// The arguments are byte offsets into `memory`, each a multiple of 8, of arrays of `f64` with the
// values of the points one point after the other: the parameters (`description.parameter_ids` per
// point; `init` takes a NaN as a parameter which is not given, which then gets its SBML value and
// initial assignment, and writes the parameters and the states), the states
// (`description.state_ids`), the control (`t`, `h`, `done`, `failed` per point), the counts (the
// accepted steps and the rejected step attempts per point), the context (`t1`, `rtol`, `atol`,
// `h_min`, `attempts`) and the scratch (`description.scratch_per_point` per point). `advance` gives
// the number of failed points. `assign` evaluates the assignment rules of every point at its time
// and its states and writes their targets (`description.assigned_ids`) into its parameters, the
// values of outputs which are targets of rules after an advance. All three give -1 for invalid
// arguments (`crates/abi/src/lib.rs` documents the contract). The kernel never grows the memory:
// the caller sizes it for its points.

/** The contract of a kernel and its description. */
export const KERNEL_CONTRACT = "kernel/1";

/** The functions a kernel exports. */
const KERNEL_EXPORTS = ["sbmlsim_kernel_init", "sbmlsim_kernel_advance", "sbmlsim_kernel_assign"];

/** A module, compiled when it is given as bytes. */
async function toModule(source, what) {
  if (source instanceof WebAssembly.Module) return source;
  try {
    return await WebAssembly.compile(source);
  } catch (error) {
    throw new Error(`${what} does not compile: ${error.message}`);
  }
}

/** Checks that a kernel imports `env.memory` and otherwise only functions of mathlib. */
function checkKernel(kernelModule, mathlibModule) {
  const math = new Set(
    WebAssembly.Module.exports(mathlibModule)
      .filter((entry) => entry.kind === "function")
      .map((entry) => entry.name),
  );
  let memory = false;
  for (const { module, name, kind } of WebAssembly.Module.imports(kernelModule)) {
    if (module === "env" && name === "memory" && kind === "memory") memory = true;
    else if (module === "math" && kind === "function" && math.has(name)) continue;
    else {
      throw new Error(
        `kernel.wasm imports ${module}.${name}, which is neither env.memory nor a function of mathlib`,
      );
    }
  }
  if (!memory) throw new Error("kernel.wasm does not import env.memory");
  const exports = new Set(WebAssembly.Module.exports(kernelModule).map((entry) => entry.name));
  const missing = KERNEL_EXPORTS.filter((name) => !exports.has(name));
  if (missing.length > 0) {
    throw new Error(`kernel.wasm does not export ${missing.join(", ")}`);
  }
}

/**
 * Instantiates a batched kernel with mathlib and the memory of its caller.
 *
 * @param {{wasm: WebAssembly.Module|BufferSource, mathlib: WebAssembly.Module|BufferSource,
 *   memory: WebAssembly.Memory}} options `kernel.wasm`, `mathlib.wasm` of the same version of
 *   sbmlsim-core (modules or their bytes) and the memory which holds the arrays of the points
 * @returns {Promise<{module: WebAssembly.Module, memory: WebAssembly.Memory,
 *   init: (n: number, parameters: number, state: number) => number,
 *   advance: (n: number, state: number, parameters: number, control: number, counts: number,
 *     context: number, scratch: number) => number,
 *   assign: (n: number, state: number, parameters: number, control: number) => number}>} the
 *   kernel: its compiled module (which a worker can instantiate again), the memory and its three
 *   functions
 */
export async function instantiateKernel({ wasm, mathlib, memory }) {
  if (!wasm || !mathlib) {
    throw new TypeError("instantiateKernel needs kernel.wasm and mathlib.wasm");
  }
  if (!(memory instanceof WebAssembly.Memory)) {
    throw new TypeError("instantiateKernel needs the WebAssembly.Memory of its caller");
  }
  const kernelModule = await toModule(wasm, "kernel.wasm");
  const mathlibModule = await toModule(mathlib, "mathlib.wasm");
  checkKernel(kernelModule, mathlibModule);
  const math = await WebAssembly.instantiate(mathlibModule, {});
  const instance = await WebAssembly.instantiate(kernelModule, {
    env: { memory },
    math: math.exports,
  });
  const {
    sbmlsim_kernel_init: init,
    sbmlsim_kernel_advance: advance,
    sbmlsim_kernel_assign: assign,
  } = instance.exports;
  return { module: kernelModule, memory, init, advance, assign };
}
