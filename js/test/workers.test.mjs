// `module_shared_by_workers`: the compiled `WebAssembly.Module`s of engine, mathlib and a model,
// posted to two worker threads, instantiate and simulate in both, with the bits of the main
// thread.

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import path from "node:path";
import { test } from "node:test";
import { Worker } from "node:worker_threads";

import { createRuntime, toCsv } from "../loader.mjs";
import { compileSbml, readModel, request, runtimeBytes } from "./helpers.mjs";

const MATH = 'xmlns="http://www.w3.org/1998/Math/MathML"';

/** A model with a rule, an event and mathlib calls. */
const SBML = `<?xml version="1.0" encoding="UTF-8"?>
<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" level="3" version="2"><model id="shared">
<listOfCompartments><compartment id="c" size="1" constant="true"/></listOfCompartments>
<listOfSpecies><species id="A" compartment="c" initialConcentration="10" hasOnlySubstanceUnits="false" boundaryCondition="false" constant="false"/>
<species id="B" compartment="c" initialConcentration="0" hasOnlySubstanceUnits="false" boundaryCondition="false" constant="false"/></listOfSpecies>
<listOfParameters><parameter id="k" value="0.7" constant="false"/><parameter id="r" constant="false"/></listOfParameters>
<listOfRules><assignmentRule variable="r"><math ${MATH}><apply><times/><ci>k</ci><apply><exp/><apply><minus/><ci>B</ci></apply></apply></apply></math></assignmentRule></listOfRules>
<listOfReactions><reaction id="R" reversible="false"><listOfReactants><speciesReference species="A" stoichiometry="1" constant="true"/></listOfReactants><listOfProducts><speciesReference species="B" stoichiometry="1" constant="true"/></listOfProducts>
<kineticLaw><math ${MATH}><apply><times/><ci>c</ci><ci>r</ci><ci>A</ci></apply></math></kineticLaw></reaction></listOfReactions>
<listOfEvents><event id="E" useValuesFromTriggerTime="true"><trigger initialValue="false" persistent="true"><math ${MATH}><apply><gt/><ci>B</ci><cn>2</cn></apply></math></trigger>
<listOfEventAssignments><eventAssignment variable="k"><math ${MATH}><apply><divide/><ci>k</ci><cn>3</cn></apply></math></eventAssignment></listOfEventAssignments></event></listOfEvents>
</model></sbml>`;

test("module_shared_by_workers", async () => {
  const dir = compileSbml(SBML, "shared");
  const { wasm, json } = readModel(dir);
  const { engine, mathlib } = runtimeBytes();
  const modules = {
    engine: await WebAssembly.compile(engine),
    mathlib: await WebAssembly.compile(mathlib),
    model: await WebAssembly.compile(wasm),
  };
  const req = request({ end: 20, output: { uniform: { steps: 200 } }, selections: ["time", "A", "B", "k", "r"] });
  const worker = () =>
    new Promise((resolve, reject) => {
      const w = new Worker(new URL("./fixtures/worker.mjs", import.meta.url), {
        workerData: { ...modules, description: json, request: req },
      });
      w.once("message", (m) => {
        resolve(m);
        w.terminate();
      });
      w.once("error", reject);
    });
  const [first, second] = await Promise.all([worker(), worker()]);
  const runtime = await createRuntime(modules);
  const main = (await runtime.instantiate(modules.model, json)).simulate(req);
  assert.equal(first.nRows, 201);
  for (const result of [first, second]) {
    assert.equal(result.wasiCalls, 0);
    assert.deepEqual(result.columns, main.columns);
    assert.equal(toCsv(result), toCsv(main));
    assert.deepEqual(new BigUint64Array(result.values.buffer), new BigUint64Array(main.values.buffer));
  }
  // the event fired: k was divided
  assert.ok(main.values[main.values.length - 2] < 0.7);
  assert.ok(readFileSync(path.join(dir, "model.wasm")).length > 0);
});
