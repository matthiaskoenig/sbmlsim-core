// The batched kernel in a JavaScript host: `compiler.compileKernel` of a page compiles what
// `sbmlsim-core compile --kernel` compiles, and `instantiateKernel` runs it in Node with
// mathlib and a memory of the caller.

import assert from "node:assert/strict";
import { readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { test } from "node:test";

import { CompileError, createCompiler } from "../compile.mjs";
import { KERNEL_CONTRACT, instantiateKernel } from "../kernel.mjs";
import { DECAY_SBML, compilerWasm, runCli, runtimeBytes, tempDir } from "./helpers.mjs";

/** The compiler of the tests. */
async function compiler() {
  return createCompiler({ compiler: compilerWasm() });
}

test("a kernel compiles into the module and the description of the command line", async () => {
  const sbml = readFileSync(DECAY_SBML, "utf8");
  const { wasm, description } = (await compiler()).compileKernel(sbml);
  const dir = tempDir("sbmlsim-core-kernel-");
  const file = path.join(dir, "model.xml");
  writeFileSync(file, sbml);
  runCli(["compile", file, "--kernel", "-o", dir]);
  assert.deepEqual(Buffer.from(wasm), readFileSync(path.join(dir, "kernel.wasm")));
  assert.deepEqual(description, JSON.parse(readFileSync(path.join(dir, "kernel.json"), "utf8")));
  assert.equal(description.version, KERNEL_CONTRACT);
  assert.deepEqual(description.state_ids, ["y"]);
  assert.deepEqual(description.parameter_ids, ["k", "t_half"]);
});

test("the decay model advances to t = 1 as exp(-k)", async () => {
  const { wasm, description } = (await compiler()).compileKernel(
    readFileSync(DECAY_SBML, "utf8"),
  );
  const memory = new WebAssembly.Memory({ initial: 1 });
  const kernel = await instantiateKernel({ wasm, mathlib: runtimeBytes().mathlib, memory });
  // one point: the parameters, the state, the control, the counts, the context, the scratch
  const [parameters, state, control, counts, context, scratch] = [64, 128, 192, 256, 320, 512];
  const f64 = new Float64Array(memory.buffer);
  const at = (offset) => offset / 8;
  // every parameter not given: k = 0.5 and t_half = ln(2) / k of the model
  f64.set([NaN, NaN], at(parameters));
  assert.equal(kernel.init(1, parameters, state), 0);
  assert.equal(f64[at(parameters)], 0.5);
  assert.ok(Math.abs(f64[at(parameters) + 1] - Math.LN2 / 0.5) < 1e-15);
  assert.equal(f64[at(state)], 1);
  f64.set([0, 0, 0, 0], at(control));
  f64.set([0, 0], at(counts));
  f64.set([1, 1e-10, 1e-12, 1e-14, 1e6], at(context));
  assert.ok(512 + 8 * description.scratch_per_point <= memory.buffer.byteLength);
  const failed = kernel.advance(1, state, parameters, control, counts, context, scratch);
  assert.equal(failed, 0);
  assert.deepEqual([f64[at(control)], f64[at(control) + 2], f64[at(control) + 3]], [1, 1, 0]);
  assert.ok(f64[at(counts)] > 1, "accepted steps");
  const y = f64[at(state)];
  assert.ok(Math.abs(y - Math.exp(-0.5)) < 1e-6, `y(1) = ${y}, exp(-k) = ${Math.exp(-0.5)}`);
  // an argument which is not a multiple of 8 is refused
  assert.equal(kernel.advance(1, state + 4, parameters, control, counts, context, scratch), -1);
});

test("a model with events is refused, and a model.wasm is no kernel", async () => {
  const instance = await compiler();
  const events = readFileSync(DECAY_SBML, "utf8").replace(
    "</listOfRules>",
    `</listOfRules><listOfEvents><event useValuesFromTriggerTime="true"><trigger initialValue="false" persistent="true"><math xmlns="http://www.w3.org/1998/Math/MathML"><true/></math></trigger></event></listOfEvents>`,
  );
  assert.throws(() => instance.compileKernel(events), (error) => {
    assert.ok(error instanceof CompileError);
    assert.match(error.message, /events in a kernel/);
    return true;
  });
  const model = instance.compile(readFileSync(DECAY_SBML, "utf8"));
  const memory = new WebAssembly.Memory({ initial: 1 });
  await assert.rejects(
    instantiateKernel({ wasm: model.wasm, mathlib: runtimeBytes().mathlib, memory }),
    /does not export sbmlsim_kernel_init, sbmlsim_kernel_advance/,
  );
  await assert.rejects(
    instantiateKernel({ wasm: model.wasm, mathlib: runtimeBytes().mathlib }),
    TypeError,
  );
});
