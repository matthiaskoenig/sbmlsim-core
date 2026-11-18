import assert from "node:assert/strict";
import { writeFileSync } from "node:fs";
import path from "node:path";
import { test } from "node:test";

import { toCsv } from "../loader.mjs";
import { compileSbml, loaderRuntime, readModel, request, runCli } from "./helpers.mjs";

test("delay feedback and prepared replay match Wasmtime bitwise", async () => {
  const dir = compileSbml(`<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" level="3" version="2">
<model id="feedback"><listOfParameters><parameter id="x" value="1" constant="false"/></listOfParameters>
<listOfRules><rateRule variable="x"><math xmlns="http://www.w3.org/1998/Math/MathML">
<apply><csymbol encoding="text" definitionURL="http://www.sbml.org/sbml/symbols/delay">delay</csymbol><ci>x</ci><cn>1</cn></apply>
</math></rateRule></listOfRules></model></sbml>`, "dde");
  const req = request({end: 3, output: {uniform: {steps: 6}}, selections: ["time", "x"]});
  const file = path.join(dir, "request.json");
  writeFileSync(file, JSON.stringify(req));
  const expected = runCli(["simulate", dir, file]).toString();
  const { wasm, json } = readModel(dir);
  const instance = await (await loaderRuntime()).instantiate(wasm, json);
  const plan = instance.prepare(req);
  try {
    const result = instance.simulate(req);
    assert.ok(Math.abs(result.values.at(-1) - (6 + 1 / 6)) < 1e-4);
    assert.equal(toCsv(result), expected);
    assert.equal(toCsv(instance.simulatePrepared(plan)), expected);
    assert.equal(toCsv(instance.simulate(req)), expected);
  } finally {
    plan.close();
  }
});
