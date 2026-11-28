// The batched kernel in a JavaScript host: `compiler.compileKernel` of a page compiles what
// `sbmlsim-core compile --kernel` compiles, and `instantiateKernel` runs it in Node with
// mathlib and a memory of the caller.

import assert from "node:assert/strict";
import { readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { test } from "node:test";

import { CompileError, createCompiler } from "../compile.mjs";
import { KERNEL_CONTRACT, instantiateKernel } from "../kernel.mjs";
import {
  CELL_SBML,
  DECAY_SBML,
  compilerWasm,
  runCli,
  runtimeBytes,
  tempDir,
} from "./helpers.mjs";

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
  assert.equal(KERNEL_CONTRACT, "kernel/2");
  assert.equal(description.version, KERNEL_CONTRACT);
  assert.deepEqual(description.state_ids, ["y"]);
  assert.deepEqual(description.parameter_ids, ["k", "t_half"]);
  assert.deepEqual(description.assigned_ids, ["t_half"]);
  // the driven species and the reference time are those of the model, absent without
  assert.equal(description.model.driven, undefined);
  assert.equal(description.model.t_ref, undefined);
  assert.ok(!("driven" in description) && !("t_ref" in description));
});

test("a driven kernel compiles into the module and the description of the command line", async () => {
  const sbml = readFileSync(CELL_SBML, "utf8");
  const instance = await compiler();
  const { wasm, description } = instance.compileKernel(sbml, { driven: ["S_ext", "P_ext"] });
  const dir = tempDir("sbmlsim-core-kernel-driven-");
  runCli(["compile", CELL_SBML, "--kernel", "--driven", "S_ext,P_ext", "-o", dir]);
  assert.deepEqual(Buffer.from(wasm), readFileSync(path.join(dir, "kernel.wasm")));
  assert.deepEqual(description, JSON.parse(readFileSync(path.join(dir, "kernel.json"), "utf8")));
  assert.equal(description.version, KERNEL_CONTRACT);
  const { driven, t_ref } = description.model;
  assert.deepEqual(
    driven.map((d) => d.id),
    ["S_ext", "P_ext"],
  );
  assert.ok(!("driven" in description) && !("t_ref" in description));
  for (const d of driven) {
    assert.equal(description.parameter_ids[d.value], `${d.id}__value`);
    assert.equal(description.parameter_ids[d.slope], `${d.id}__slope`);
    assert.equal(description.state_ids[d.exchange], `${d.id}__exchange`);
  }
  assert.equal(description.parameter_ids[t_ref], "driven__t_ref");
  // the undriven kernel of the same model is another one
  assert.notDeepEqual(Buffer.from(instance.compileKernel(sbml).wasm), Buffer.from(wasm));
});

/**
 * `X -> C` of the rate `(k * blood) * X` (`crates/c/tests/common`, `uptake`): `X` in `blood` of
 * the size `Vb`, `C` in `cell` of size 1; with `X` driven along `value + slope (t - t_ref)` the
 * exchange over an interval is `-k Vb` times the integral of the path.
 */
const UPTAKE_K = 0.7;
const UPTAKE_VB = 0.15;
const MATH = `xmlns="http://www.w3.org/1998/Math/MathML"`;
const UPTAKE = `<?xml version="1.0" encoding="UTF-8"?>
<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" level="3" version="2"><model id="uptake">
<listOfCompartments><compartment id="blood" constant="true"/><compartment id="cell" size="1" constant="true"/></listOfCompartments>
<listOfSpecies><species id="X" compartment="blood" initialAmount="0.3" hasOnlySubstanceUnits="false" boundaryCondition="false" constant="false"/><species id="C" compartment="cell" initialConcentration="0" hasOnlySubstanceUnits="false" boundaryCondition="false" constant="false"/></listOfSpecies>
<listOfParameters><parameter id="k" value="${UPTAKE_K}" constant="true"/><parameter id="Vb" value="${UPTAKE_VB}" constant="true"/></listOfParameters>
<listOfInitialAssignments><initialAssignment symbol="blood"><math ${MATH}><ci>Vb</ci></math></initialAssignment></listOfInitialAssignments>
<listOfReactions><reaction id="R" reversible="false"><listOfReactants><speciesReference species="X" stoichiometry="1" constant="true"/></listOfReactants><listOfProducts><speciesReference species="C" stoichiometry="1" constant="true"/></listOfProducts><kineticLaw><math ${MATH}><apply><times/><apply><times/><ci>k</ci><ci>blood</ci></apply><ci>X</ci></apply></math></kineticLaw></reaction></listOfReactions>
</model></sbml>`;

/** The integral of the path `value + slope (t - tRef)` from `t0` to `t1`. */
function pathIntegral(value, slope, tRef, t0, t1) {
  const primitive = (t) => value * t + (slope * (t - tRef) ** 2) / 2;
  return primitive(t1) - primitive(t0);
}

test("a driven kernel exchanges the closed form", async () => {
  const { wasm, description } = (await compiler()).compileKernel(UPTAKE, { driven: ["X"] });
  assert.deepEqual(description.state_ids, ["C", "X__exchange"]);
  const memory = new WebAssembly.Memory({ initial: 1 });
  const kernel = await instantiateKernel({ wasm, mathlib: runtimeBytes().mathlib, memory });
  const ns = description.state_ids.length;
  const np = description.parameter_ids.length;
  const [x] = description.model.driven;
  const tRef = description.model.t_ref;
  const k = description.parameter_ids.indexOf("k");
  // three points, each array behind the one before
  const n = 3;
  const f64 = new Float64Array(memory.buffer);
  let offset = 64;
  const next = (values) => {
    const at = offset;
    offset += 8 * values;
    return at;
  };
  const [parameters, state, control, counts, context] = [n * np, n * ns, n * 4, n * 2, 5].map(next);
  const scratch = next(n * description.scratch_per_point);
  assert.ok(offset <= memory.buffer.byteLength);
  const p = (i, index) => parameters / 8 + i * np + index;
  f64.fill(NaN, parameters / 8, parameters / 8 + n * np);
  assert.equal(kernel.init(n, parameters, state), 0);
  // the initial value of the path is the initial concentration 0.3 / Vb
  for (let i = 0; i < n; i++) {
    assert.ok(Math.abs(f64[p(i, x.value)] - 0.3 / UPTAKE_VB) < 1e-14);
    assert.equal(f64[p(i, x.slope)], 0);
  }
  f64.fill(0, counts / 8, counts / 8 + n * 2);
  // two intervals, each point with its own path and its own k per interval
  const paths = [
    [[1, 0.5, 0.7], [3, -1, 0.2]],
    [[2, 3, 1.1], [0.5, 0.25, 0.9]],
    [[0, 4, 0.3], [4, -2, 1.7]],
  ];
  for (let interval = 0; interval < 2; interval++) {
    const [t0, t1] = [interval, interval + 1];
    for (let i = 0; i < n; i++) {
      const [value, slope, rate] = paths[i][interval];
      f64[p(i, x.value)] = value;
      f64[p(i, x.slope)] = slope;
      f64[p(i, tRef)] = t0;
      f64[p(i, k)] = rate;
      f64[state / 8 + i * ns + x.exchange] = 0;
      f64.set([t0, 0, 0, 0], control / 8 + 4 * i);
    }
    f64.set([t1, 1e-6, 1e-9, 1e-10, 1e6], context / 8);
    assert.equal(kernel.advance(n, state, parameters, control, counts, context, scratch), 0);
    for (let i = 0; i < n; i++) {
      const [value, slope, rate] = paths[i][interval];
      assert.deepEqual([f64[control / 8 + 4 * i], f64[control / 8 + 4 * i + 2]], [t1, 1]);
      const exact = -rate * UPTAKE_VB * pathIntegral(value, slope, t0, t0, t1);
      const exchange = f64[state / 8 + i * ns + x.exchange];
      assert.ok(
        Math.abs(exchange - exact) <= 1e-6 * Math.abs(exact),
        `point ${i} over [${t0}, ${t1}]: exchange ${exchange}, closed form ${exact}`,
      );
    }
  }
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

test("assign gives the rules of a point at its time and its state", async () => {
  // the decay model with the rule y_t = y * time, which reads the state and the time
  const time = `<csymbol encoding="text" definitionURL="http://www.sbml.org/sbml/symbols/time">t</csymbol>`;
  const sbml = readFileSync(DECAY_SBML, "utf8")
    .replace(
      "</listOfParameters>",
      `<parameter id="y_t" constant="false"/></listOfParameters>`,
    )
    .replace(
      "</listOfRules>",
      `<assignmentRule variable="y_t"><math xmlns="http://www.w3.org/1998/Math/MathML"><apply><times/><ci>y</ci>${time}</apply></math></assignmentRule></listOfRules>`,
    );
  const { wasm, description } = (await compiler()).compileKernel(sbml);
  assert.deepEqual(description.parameter_ids, ["k", "t_half", "y_t"]);
  assert.deepEqual(description.assigned_ids, ["t_half", "y_t"]);
  const memory = new WebAssembly.Memory({ initial: 1 });
  const kernel = await instantiateKernel({ wasm, mathlib: runtimeBytes().mathlib, memory });
  const [parameters, state, control, counts, context, scratch] = [64, 128, 192, 256, 320, 512];
  const f64 = new Float64Array(memory.buffer);
  const at = (offset) => offset / 8;
  f64.set([NaN, NaN, NaN], at(parameters));
  assert.equal(kernel.init(1, parameters, state), 0);
  // y_t at t = 0 is 0, and stays the value of init through an advance
  assert.equal(f64[at(parameters) + 2], 0);
  f64.set([0, 0, 0, 0], at(control));
  f64.set([0, 0], at(counts));
  f64.set([2, 1e-10, 1e-12, 1e-14, 1e6], at(context));
  assert.equal(kernel.advance(1, state, parameters, control, counts, context, scratch), 0);
  assert.equal(f64[at(parameters) + 2], 0);
  // assign: y_t = y(2) * 2, t_half = ln(2) / k, k and the state and the control as they were
  const before = Array.from(f64.subarray(at(state), at(control) + 4));
  assert.equal(kernel.assign(1, state, parameters, control), 0);
  const y = f64[at(state)];
  assert.equal(f64[at(parameters) + 2], y * 2);
  assert.ok(Math.abs(f64[at(parameters) + 1] - Math.LN2 / 0.5) < 1e-15);
  assert.equal(f64[at(parameters)], 0.5);
  assert.deepEqual(Array.from(f64.subarray(at(state), at(control) + 4)), before);
  // invalid arguments: -1, and nothing is written
  f64[at(parameters) + 2] = 7;
  assert.equal(kernel.assign(1, state, parameters + 4, control), -1);
  assert.equal(kernel.assign(-1, state, parameters, control), -1);
  assert.equal(kernel.assign(1, state, parameters, memory.buffer.byteLength - 8), -1);
  assert.equal(f64[at(parameters) + 2], 7);
});

test("an unknown driven species is refused with the message of the compiler", async () => {
  const instance = await compiler();
  let message;
  assert.throws(() => instance.compile(UPTAKE, { driven: ["Q"] }), (error) => {
    message = error.message;
    return error instanceof CompileError;
  });
  assert.match(message, /Q/);
  assert.throws(() => instance.compileKernel(UPTAKE, { driven: ["Q"] }), (error) => {
    assert.ok(error instanceof CompileError);
    assert.equal(error.message, message);
    return true;
  });
  assert.throws(() => instance.compileKernel(UPTAKE, { driven: "X" }), TypeError);
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
    /does not export sbmlsim_kernel_init, sbmlsim_kernel_advance, sbmlsim_kernel_assign/,
  );
  await assert.rejects(
    instantiateKernel({ wasm: model.wasm, mathlib: runtimeBytes().mathlib }),
    TypeError,
  );
});
