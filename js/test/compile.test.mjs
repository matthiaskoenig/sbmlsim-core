// The compiler of the JavaScript host: SBML text in a page becomes `model.wasm` and its
// description, which the loader instantiates and simulates.
//
// The decisive check is that the browser compiles what the command line compiles: the module and
// the description of `compile.mjs` are compared with the ones `sbmlsim-core compile` wrote, and
// a simulation of both gives the same bits.

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

import { CompileError, createCompiler } from "../compile.mjs";
import {
  DECAY_SBML,
  compileSbml,
  compilerWasm,
  loaderRuntime,
  readModel,
  request,
} from "./helpers.mjs";

/** The compiler of the tests, created once. */
async function compiler() {
  return createCompiler({ compiler: compilerWasm() });
}

test("a model compiles into the module and the description of the command line", async () => {
  const sbml = readFileSync(DECAY_SBML, "utf8");
  const { wasm, description } = (await compiler()).compile(sbml);
  const written = readModel(compileSbml(sbml, "decay"));
  assert.deepEqual(
    Buffer.from(wasm),
    written.wasm,
    "the module of the page is the module of the command line",
  );
  assert.deepEqual(
    description,
    JSON.parse(written.json),
    "the description of the page is the description of the command line",
  );
});

test("a compiled model simulates as the one of the command line does", async () => {
  const sbml = readFileSync(DECAY_SBML, "utf8");
  const { wasm, description } = (await compiler()).compile(sbml);
  const runtime = await loaderRuntime();
  const here = await runtime.instantiate(wasm, description);
  const written = readModel(compileSbml(sbml, "decay"));
  const there = await runtime.instantiate(written.wasm, written.json);
  const settings = request({ end: 20, output: { uniform: { steps: 20 } } });
  const mine = here.simulate(settings);
  const theirs = there.simulate(settings);
  assert.deepEqual(mine.columns, theirs.columns);
  assert.equal(mine.nRows, theirs.nRows);
  // the same module and the same request give the same bits
  assert.deepEqual(new Uint8Array(mine.values.buffer), new Uint8Array(theirs.values.buffer));
});

test("the optimizer of the compiler keeps the results", async () => {
  const sbml = readFileSync(DECAY_SBML, "utf8");
  const instance = await compiler();
  const optimized = instance.compile(sbml, { optimize: true });
  const plain = instance.compile(sbml, { optimize: false });
  const runtime = await loaderRuntime();
  const settings = request({ end: 20, output: { uniform: { steps: 20 } } });
  const left = (await runtime.instantiate(optimized.wasm, optimized.description)).simulate(
    settings,
  );
  const right = (await runtime.instantiate(plain.wasm, plain.description)).simulate(settings);
  assert.deepEqual(new Uint8Array(left.values.buffer), new Uint8Array(right.values.buffer));
});

test("a document which is no model is refused with the message of the compiler", async () => {
  const instance = await compiler();
  assert.throws(() => instance.compile("<not-sbml/>"), CompileError);
  assert.throws(() => instance.compile(""), CompileError);
  try {
    instance.compile("<not-sbml/>");
    assert.fail("the document was compiled");
  } catch (error) {
    assert.ok(error instanceof CompileError);
    assert.ok(error.message.length > 0, "the refusal has a message");
  }
});

test("the compiler takes a document of many bytes and compiles again after it", async () => {
  const instance = await compiler();
  const sbml = readFileSync(DECAY_SBML, "utf8");
  // a comment of a megabyte grows the memory of the module, which the next call reads again
  const large = sbml.replace("<model", `<!--${"x".repeat(1024 * 1024)}-->\n<model`);
  const first = instance.compile(large);
  const second = instance.compile(sbml);
  assert.deepEqual(Buffer.from(first.wasm), Buffer.from(second.wasm));
});

test("the arguments of a compilation are checked", async () => {
  const instance = await compiler();
  assert.throws(() => instance.compile(readFileSync(DECAY_SBML)), TypeError);
  assert.throws(() => instance.compile("<sbml/>", { jacobian: "none" }), TypeError);
  assert.throws(() => instance.compile("<sbml/>", { parameterJacobian: "false" }), TypeError);
});

test("parameter derivatives can be omitted without changing ordinary simulations", async () => {
  const sbml = readFileSync(DECAY_SBML, "utf8");
  const instance = await compiler();
  const complete = instance.compile(sbml);
  const ordinary = instance.compile(sbml, { parameterJacobian: false });
  assert.ok(complete.description.jacobian_p);
  assert.equal(ordinary.description.jacobian_p, undefined);
  assert.ok(ordinary.wasm.length < complete.wasm.length);
  const runtime = await loaderRuntime();
  const settings = request({ end: 20, output: { uniform: { steps: 20 } } });
  const left = (await runtime.instantiate(complete.wasm, complete.description)).simulate(settings);
  const right = (await runtime.instantiate(ordinary.wasm, ordinary.description)).simulate(settings);
  assert.deepEqual(new Uint8Array(left.values.buffer), new Uint8Array(right.values.buffer));
});

test("the compiler refuses a module which is not compiler.wasm", async () => {
  // `model.wasm` exports none of `allocate`, `compile` and `deallocate`
  const model = readModel(compileSbml(readFileSync(DECAY_SBML, "utf8"), "decay")).wasm;
  await assert.rejects(
    createCompiler({ compiler: model }),
    /does not export|imports from its host/,
  );
  await assert.rejects(createCompiler({}), TypeError);
});
