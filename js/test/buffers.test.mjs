// The buffers of the calls into `compiler.wasm` (`withBuffers` of `compile.mjs`, `petab.mjs`):
// every buffer a call allocates or gets as a result is freed, also when the call throws; a
// failed allocation (0) throws before anything is written; and the words of a result are read
// little-endian at any address, since a buffer of `allocate` has the alignment of bytes.
//
// The checks count the buffers on the exports of a real `compiler.wasm` (`countedExports` of
// `helpers.mjs`), so they hold for the module the package ships.

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

import { Buffers, CompileError, createCompiler, withBuffers } from "../compile.mjs";
import { PetabError, openPetab } from "../petab.mjs";
import { DECAY_SBML, compilerWasm, countedExports, loaderRuntime } from "./helpers.mjs";

/** A compiler of its own, whose exports count the buffers. */
async function countedCompiler(options) {
  const compiler = await createCompiler({ compiler: compilerWasm() });
  const counted = countedExports(compiler.exports, options);
  compiler.exports = counted.exports;
  return { compiler, counted };
}

/** Asserts that the host holds no buffer and freed none it did not hold. */
function assertBalanced(counted, what) {
  assert.ok(counted.results > 0, `${what}: the host was given buffers of results`);
  assert.deepEqual([...counted.live.entries()], [], `${what}: every buffer is freed`);
  assert.deepEqual(counted.misfreed, [], `${what}: every free is of a buffer the host held`);
}

/** The first bytes of the memory of the module, which a write to the address 0 would change. */
function low(exports) {
  return new Uint8Array(exports.memory.buffer, 0, 64).slice();
}

const SBML = readFileSync(DECAY_SBML, "utf8");

/** The problem of `petab.test.mjs`: a dose of A at 5, its parameters estimated. */
const PETAB_MODEL = `<?xml version="1.0" encoding="UTF-8"?>
<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" level="3" version="2">
  <model id="conversion">
    <listOfCompartments><compartment id="c" size="1" constant="true"/></listOfCompartments>
    <listOfSpecies>
      <species id="A" compartment="c" initialConcentration="1" hasOnlySubstanceUnits="false" boundaryCondition="false" constant="false"/>
      <species id="B" compartment="c" initialConcentration="0" hasOnlySubstanceUnits="false" boundaryCondition="false" constant="false"/>
    </listOfSpecies>
    <listOfParameters>
      <parameter id="k1" value="0.8" constant="true"/>
      <parameter id="k2" value="0.6" constant="true"/>
    </listOfParameters>
    <listOfReactions>
      <reaction id="forward" reversible="false">
        <listOfReactants><speciesReference species="A" stoichiometry="1" constant="true"/></listOfReactants>
        <listOfProducts><speciesReference species="B" stoichiometry="1" constant="true"/></listOfProducts>
        <kineticLaw><math xmlns="http://www.w3.org/1998/Math/MathML"><apply><times/><ci>k1</ci><ci>A</ci></apply></math></kineticLaw>
      </reaction>
      <reaction id="backward" reversible="false">
        <listOfReactants><speciesReference species="B" stoichiometry="1" constant="true"/></listOfReactants>
        <listOfProducts><speciesReference species="A" stoichiometry="1" constant="true"/></listOfProducts>
        <kineticLaw><math xmlns="http://www.w3.org/1998/Math/MathML"><apply><times/><ci>k2</ci><ci>B</ci></apply></math></kineticLaw>
      </reaction>
    </listOfReactions>
  </model>
</sbml>`;

/** The files of the problem. */
function petabFiles() {
  return {
    "problem.yaml":
      "format_version: 2.0.0\nparameter_files: [parameters.tsv]\nmodel_files:\n  m: {location: model.xml, language: sbml}\ncondition_files: [conditions.tsv]\nexperiment_files: [experiments.tsv]\nobservable_files: [observables.tsv]\nmeasurement_files: [measurements.tsv]\n",
    "model.xml": PETAB_MODEL,
    "conditions.tsv": "conditionId\ttargetId\ttargetValue\nc0\tA\ta0\ndose\tA\tA + 1\n",
    "experiments.tsv": "experimentId\ttime\tconditionId\ne1\t0\tc0\ne1\t5\tdose\n",
    "observables.tsv":
      "observableId\tobservableFormula\tnoiseFormula\tnoiseDistribution\nobs_a\tA\t0.5\tnormal\nobs_b\tscale * B\t0.2\tlog-normal\n",
    "measurements.tsv":
      "observableId\texperimentId\ttime\tmeasurement\nobs_a\te1\t1\t0.7\nobs_a\te1\t5\t1.4\nobs_b\te1\t10\t1.1\n",
    "parameters.tsv":
      "parameterId\tlowerBound\tupperBound\tnominalValue\testimate\tpriorDistribution\tpriorParameters\nk1\t0.01\t10\t0.8\ttrue\t\t\nk2\t0.01\t10\t0.6\ttrue\t\t\nscale\t0.1\t10\t2\ttrue\t\t\na0\t\t\t1.5\tfalse\t\t\n",
  };
}

/** Exports of no module: a memory, a bump allocator and a `deallocate` which records. */
function fakeExports() {
  const freed = [];
  let next = 1;
  return {
    freed,
    exports: {
      memory: new WebAssembly.Memory({ initial: 1 }),
      allocate(length) {
        const pointer = next;
        next += length;
        return pointer;
      },
      deallocate(pointer, length) {
        freed.push([pointer, length]);
      },
    },
  };
}

test("the buffers of a call are freed, the last first, whatever the call throws", () => {
  const { exports, freed } = fakeExports();
  assert.throws(
    () =>
      withBuffers(exports, "test.wasm", (buffers) => {
        buffers.put(Uint8Array.of(1, 2, 3));
        buffers.allocate(16);
        throw new Error("the call failed");
      }),
    /the call failed/,
  );
  assert.deepEqual(freed, [
    [4, 16],
    [1, 3],
  ]);
});

test("a typed array is copied as its bytes", () => {
  const { exports } = fakeExports();
  withBuffers(exports, "test.wasm", (buffers) => {
    const values = Float64Array.of(1.5, -0);
    const [pointer, length] = buffers.put(values.subarray(1));
    assert.equal(length, 8);
    const view = new DataView(exports.memory.buffer, pointer, length);
    assert.ok(Object.is(view.getFloat64(0, true), -0));
  });
});

test("a buffer of no bytes is no allocation", () => {
  const { exports, freed } = fakeExports();
  const buffers = new Buffers(exports, "test.wasm");
  assert.deepEqual(buffers.put(new Uint8Array(0)), [0, 0]);
  assert.equal(buffers.allocate(0), 0);
  assert.equal(buffers.free(), undefined);
  assert.deepEqual(freed, []);
});

test("a failed allocation throws, and the buffers before it are freed", () => {
  const { exports, freed } = fakeExports();
  const failing = { ...exports, allocate: (length) => (length === 16 ? 0 : exports.allocate(length)) };
  assert.throws(
    () =>
      withBuffers(failing, "test.wasm", (buffers) => {
        buffers.put(Uint8Array.of(1, 2, 3));
        buffers.allocate(16);
      }),
    /test\.wasm could not allocate 16 bytes/,
  );
  assert.deepEqual(freed, [[1, 3]]);
});

test("the words of a result are little-endian at any address, their buffers freed", () => {
  const { exports, freed } = fakeExports();
  const memory = new DataView(exports.memory.buffer);
  const result = withBuffers(exports, "test.wasm", (buffers) => {
    const out = buffers.allocate(16);
    assert.equal(out % 4, 1, "the words are not aligned");
    // the module writes two buffers: three bytes at 100 and none
    new Uint8Array(exports.memory.buffer, 100, 3).set([7, 8, 9]);
    memory.setUint32(out, 100, true);
    memory.setUint32(out + 4, 3, true);
    memory.setUint32(out + 8, 0, true);
    memory.setUint32(out + 12, 0, true);
    assert.deepEqual(buffers.words(out, 2), [100, 3]);
    return buffers.results(out, 2);
  });
  assert.deepEqual(result, [Uint8Array.of(7, 8, 9), new Uint8Array(0)]);
  assert.deepEqual(freed, [
    [100, 3],
    [1, 16],
  ]);
});

test("a failing free is thrown when the call threw nothing, and the call's error otherwise", () => {
  const { exports } = fakeExports();
  const failing = {
    ...exports,
    deallocate() {
      throw new WebAssembly.RuntimeError("unreachable");
    },
  };
  assert.throws(
    () => withBuffers(failing, "test.wasm", (buffers) => buffers.allocate(4)),
    WebAssembly.RuntimeError,
  );
  assert.throws(
    () =>
      withBuffers(failing, "test.wasm", (buffers) => {
        buffers.allocate(4);
        throw new Error("the call failed");
      }),
    /the call failed/,
  );
});

test("a compilation frees every buffer: of a model, of a refusal and of a call which throws", async () => {
  const { compiler, counted } = await countedCompiler();
  assert.ok(compiler.compile(SBML).wasm.length > 0);
  assertBalanced(counted, "a model");
  assert.ok(compiler.compileKernel(SBML).wasm.length > 0);
  assertBalanced(counted, "a kernel");
  assert.throws(() => compiler.compile("<not-sbml/>"), CompileError);
  assertBalanced(counted, "a refusal");
  assert.throws(() => compiler.compile(""), CompileError);
  assertBalanced(counted, "an empty document");

  compiler.exports = { ...counted.exports, compile() { throw new WebAssembly.RuntimeError("unreachable"); } };
  assert.throws(() => compiler.compile(SBML), WebAssembly.RuntimeError);
  assertBalanced(counted, "a trap");
});

test("a failed allocation of a compilation throws and writes nothing to the address 0", async () => {
  const { compiler, counted } = await countedCompiler();
  assert.ok(compiler.compile(SBML).wasm.length > 0);
  const before = low(counted.exports);
  for (const k of [1, 2]) {
    counted.failAt(k);
    assert.throws(() => compiler.compile(SBML), /compiler\.wasm could not allocate/);
    assertBalanced(counted, `the allocation ${k} fails`);
    assert.deepEqual(low(counted.exports), before, "nothing is written at the address 0");
  }
  assert.ok(compiler.compile(SBML).wasm.length > 0, "the compiler is usable afterwards");
});

test("the words of a compilation at an odd address give the module of aligned ones", async () => {
  const { compiler: plain } = await countedCompiler();
  const { compiler, counted } = await countedCompiler({ oddWords: true });
  const expected = plain.compile(SBML);
  const result = compiler.compile(SBML);
  assert.deepEqual(result.wasm, expected.wasm);
  assert.deepEqual(result.description, expected.description);
  assert.throws(() => compiler.compile("<not-sbml/>"), CompileError);
  assert.equal(
    compiler.exportOde(SBML, "markdown").content,
    plain.exportOde(SBML, "markdown").content,
  );
  assertBalanced(counted, "odd words");
});

test("an export of the ODEs frees every buffer, also of a call which throws", async () => {
  const { compiler, counted } = await countedCompiler();
  assert.ok(compiler.exportOde(SBML, "python").content.length > 0);
  assertBalanced(counted, "an export");
  assert.throws(() => compiler.exportOde(SBML, "no-format"), CompileError);
  assertBalanced(counted, "a refusal");
  const before = low(counted.exports);
  for (const k of [1, 2, 3]) {
    counted.failAt(k);
    assert.throws(() => compiler.exportOde(SBML, "python"), /compiler\.wasm could not allocate/);
    assertBalanced(counted, `the allocation ${k} fails`);
    assert.deepEqual(low(counted.exports), before, "nothing is written at the address 0");
  }
  compiler.exports = { ...counted.exports, export_ode() { throw new WebAssembly.RuntimeError("unreachable"); } };
  assert.throws(() => compiler.exportOde(SBML, "python"), WebAssembly.RuntimeError);
  assertBalanced(counted, "a trap");
});

test("a problem of PEtab frees every buffer of its calls, also of calls which throw", async () => {
  const runtime = await loaderRuntime();
  const { compiler: plain } = await countedCompiler();
  const reference = await openPetab({ compiler: plain, runtime, files: petabFiles() });
  const expected = await reference.evaluate();
  reference.close();

  for (const oddWords of [false, true]) {
    const { counted } = await countedCompiler({ oddWords });
    const problem = await openPetab({ compiler: counted, runtime, files: petabFiles() });
    const evaluation = await problem.evaluate();
    assert.equal(evaluation.llh, expected.llh, "the words at any address give the evaluation");
    problem.close();
    assertBalanced(counted, `an evaluation, odd words ${oddWords}`);
  }

  const { counted } = await countedCompiler();
  await assert.rejects(openPetab({ compiler: counted, runtime, files: { "problem.yaml": "no: problem" } }), PetabError);
  assertBalanced(counted, "a refusal");
  for (const k of [1, 2, 3]) {
    counted.failAt(k);
    await assert.rejects(
      openPetab({ compiler: counted, runtime, files: petabFiles() }),
      /compiler\.wasm could not allocate/,
    );
    assertBalanced(counted, `the allocation ${k} fails`);
  }
  const trapping = {
    exports: { ...counted.exports, petab_model() { throw new WebAssembly.RuntimeError("unreachable"); } },
  };
  await assert.rejects(openPetab({ compiler: trapping, runtime, files: petabFiles() }), WebAssembly.RuntimeError);
  assertBalanced(counted, "a trap of petab_model");
});
