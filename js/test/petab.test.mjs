// PEtab v2 in the JavaScript host (`petab.mjs`): a problem read, planned and evaluated by
// `compiler.wasm`, its simulations run by the loader or the pool.
//
// The decisive checks are that a page evaluates a problem as the command line does, the
// simulation table and the log-likelihood bit for bit (both run `engine.wasm`, and the
// likelihood is the Rust of `sbmlsim-petab` in either), and that the cases of the PEtab test
// suite pass in the page as they pass in `crates/petab/tests/testsuite.rs`. The cases are the
// ones of `python scripts/petab_testsuite.py download` (`~/.cache/sbmlsim/petab-test-suite/
// <commit>`, or `SBMLSIM_PETAB_SUITE_PATH`); without them those tests skip, unless
// `SBMLSIM_CORE_REQUIRE_CASES` is set, as CI sets it. The comparison with the command line
// needs one built with the feature `native` (its command `petab`), which CI builds.

import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { existsSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import path from "node:path";
import { test } from "node:test";
import vm from "node:vm";

import { createCompiler } from "../compile.mjs";
import { PetabError, openPetab } from "../petab.mjs";
import { createPool } from "../pool.mjs";
import {
  ROOT,
  cli,
  compilerWasm,
  loaderRuntime,
  nodeWorker,
  runtimeBytes,
  tempDir,
} from "./helpers.mjs";

/** The commit of the PEtab test suite, `COMMIT` of `crates/petab/tests/testsuite.rs`. */
const SUITE_COMMIT = "d09985e7bb03280b461599a24337137533d931e2";

/** The model of `crates/wasm/tests/petab.rs`: A and B converted into each other. */
const MODEL = `<?xml version="1.0" encoding="UTF-8"?>
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

/** The files of the problem of `crates/wasm/tests/petab.rs`: a dose of A at 5. */
function files() {
  return {
    "problem.yaml":
      "format_version: 2.0.0\nparameter_files: [parameters.tsv]\nmodel_files:\n  m: {location: model.xml, language: sbml}\ncondition_files: [conditions.tsv]\nexperiment_files: [experiments.tsv]\nobservable_files: [observables.tsv]\nmeasurement_files: [measurements.tsv]\n",
    "model.xml": MODEL,
    "conditions.tsv": "conditionId\ttargetId\ttargetValue\nc0\tA\ta0\ndose\tA\tA + 1\n",
    "experiments.tsv": "experimentId\ttime\tconditionId\ne1\t0\tc0\ne1\t5\tdose\n",
    "observables.tsv":
      "observableId\tobservableFormula\tnoiseFormula\tnoiseDistribution\nobs_a\tA\t0.5\tnormal\nobs_b\tscale * B\t0.2\tlog-normal\n",
    "measurements.tsv":
      "observableId\texperimentId\ttime\tmeasurement\nobs_a\te1\t1\t0.7\nobs_a\te1\t5\t1.4\nobs_b\te1\t10\t1.1\n",
    "parameters.tsv":
      "parameterId\tlowerBound\tupperBound\tnominalValue\testimate\tpriorDistribution\tpriorParameters\nk1\t0.01\t10\t0.8\ttrue\t\t\nk2\t0.01\t10\t0.6\ttrue\tlog-normal\t-0.5;0.5\nscale\t0.1\t10\t2\ttrue\t\t\na0\t\t\t1.5\tfalse\t\t\n",
  };
}

/** The tolerances of the integrator of the analytic comparison. */
const TIGHT = { relativeTolerance: 1e-12, absoluteTolerance: 1e-12 };

/** A and B of the problem at a point: the analytic solution, with the dose of 1 at 5. */
function analytic([k1, k2, scale]) {
  const a0 = 1.5;
  const k = k1 + k2;
  // A + B is conserved; A relaxes to k2 / k of it
  const relax = (a, b, t) => {
    const total = a + b;
    const a1 = (total * k2) / k + (a - (total * k2) / k) * Math.exp(-k * t);
    return [a1, total - a1];
  };
  const at1 = relax(a0, 0, 1);
  const [a5, b5] = relax(a0, 0, 5);
  const at10 = relax(a5 + 1, b5, 5);
  return [at1[0], a5 + 1, scale * at10[1]];
}

/** The log density of a normal and of a log-normal noise, PEtab's. */
const normal = (m, y, s) => -0.5 * Math.log(2 * Math.PI * s * s) - 0.5 * ((m - y) / s) ** 2;
const logNormal = (m, y, s) =>
  -0.5 * Math.log(2 * Math.PI * s * s * m * m) - 0.5 * ((Math.log(m) - Math.log(y)) / s) ** 2;

let compilerPromise = null;
/** The compiler of the tests, created once. */
function compiler() {
  compilerPromise ??= createCompiler({ compiler: compilerWasm() });
  return compilerPromise;
}

test("a problem opens with its parameters, and evaluates as its analytic solution", async () => {
  const problem = await openPetab({
    compiler: await compiler(),
    runtime: await loaderRuntime(),
    files: files(),
    options: TIGHT,
  });
  try {
    assert.deepEqual(problem.models, ["m"]);
    assert.deepEqual(problem.parameters, ["k1", "k2", "scale"]);
    assert.deepEqual(Array.from(problem.nominal), [0.8, 0.6, 2]);
    assert.deepEqual(Array.from(problem.lower), [0.01, 0.01, 0.1]);
    assert.deepEqual(Array.from(problem.upper), [10, 10, 10]);
    assert.equal(problem.nSimulations, 1);
    assert.equal(problem.nMeasurements, 3);

    const x = [0.5, 0.4, 3];
    const evaluation = await problem.evaluate(x);
    const expected = analytic(x);
    for (const [k, y] of expected.entries()) {
      assert.ok(Math.abs(evaluation.simulated[k] - y) < 1e-9, `measurement ${k}: ${evaluation.simulated[k]} != ${y}`);
    }
    assert.deepEqual(Array.from(evaluation.sigma), [0.5, 0.5, 0.2]);
    const llh = normal(0.7, expected[0], 0.5) + normal(1.4, expected[1], 0.5) + logNormal(1.1, expected[2], 0.2);
    assert.ok(Math.abs(evaluation.llh - llh) < 1e-8, `${evaluation.llh} != ${llh}`);
    // the prior of k2 is log-normal, truncated at its bounds; the others are uniform
    assert.deepEqual(Object.keys(evaluation.logPriors), ["k1", "k2", "scale"]);
    assert.equal(evaluation.logPriors.k1, -Math.log(10 - 0.01));
    const sum = Object.values(evaluation.logPriors).reduce((a, b) => a + b, 0);
    assert.equal(evaluation.logPrior, sum);
    assert.equal(evaluation.unnormLogPosterior, evaluation.llh + evaluation.logPrior);
    assert.deepEqual(evaluation.failures, []);
    const table = evaluation.simulationTable.trim().split("\n");
    assert.equal(table[0], "observableId\texperimentId\ttime\tsimulation");
    assert.equal(table.length, 4);
    assert.equal(Number(table[3].split("\t")[3]), evaluation.simulated[2]);

    // the nominal values without a point
    const nominal = await problem.evaluate();
    const again = await problem.evaluate(problem.nominal);
    assert.equal(nominal.llh, again.llh);
    assert.notEqual(nominal.llh, evaluation.llh);
  } finally {
    problem.close();
  }
});

test("the simulations of a problem are the results of the loader, and evaluate as they are", async () => {
  const problem = await openPetab({
    compiler: await compiler(),
    runtime: await loaderRuntime(),
    files: files(),
  });
  try {
    const x = [0.5, 0.4, 3];
    const simulations = await problem.simulations(x);
    assert.equal(simulations.length, 1);
    const [{ model, request, result }] = simulations;
    assert.equal(model, 0);
    assert.equal(request.contract, "simulation/1");
    assert.deepEqual(request.preinit.find(([id]) => id === "k1"), ["k1", 0.5]);
    assert.deepEqual(result.columns, request.selections);
    assert.ok(result.values instanceof Float64Array);
    assert.equal(result.values.length, result.nRows * result.columns.length);
    const evaluation = await problem.evaluate(x);
    const reused = await problem.evaluate(x, { simulations });
    assert.equal(reused.llh, evaluation.llh);
    assert.deepEqual(reused.simulated, evaluation.simulated);

    // a simulation which failed gives NaN for its measurements and a log-likelihood of NaN
    const failed = problem.evaluateSimulations(x, [{ result: { error: "the integration failed" } }]);
    assert.ok(Number.isNaN(failed.llh));
    assert.ok(failed.simulated.every(Number.isNaN));
    assert.deepEqual(failed.failures, [{ simulation: 0, message: "the integration failed" }]);
    // a value which is no number crosses as one
    const values = result.values.slice();
    values.fill(Number.NaN, 1, result.columns.length);
    const nan = problem.evaluateSimulations(x, [{ result: { ...result, values } }]);
    assert.ok(Number.isNaN(nan.simulated[0]));
    assert.ok(Number.isNaN(nan.llh));

    // a point outside of the support of a prior has a log prior of -Infinity
    const outside = await problem.evaluate([0.5, 20, 3]);
    assert.equal(outside.logPriors.k2, -Infinity);
    assert.equal(outside.unnormLogPosterior, -Infinity);

    assert.throws(() => problem.requests([1, 2]), /estimates 3 parameters/);
    assert.throws(() => problem.requests([1, 2, Number.NaN]), /finite/);
  } finally {
    problem.close();
  }
  await assert.rejects(problem.evaluate(), /closed/);
  problem.close();
});

test("a pool evaluates a problem with the bits of the loader", async () => {
  const runtime = await loaderRuntime();
  const pool = await createPool({ ...runtime.modules, workers: 2, createWorker: nodeWorker });
  try {
    const here = await openPetab({ compiler: await compiler(), runtime, files: files() });
    const there = await openPetab({ compiler: await compiler(), pool, files: files() });
    for (const x of [undefined, [0.5, 0.4, 3], [2, 0.1, 0.5]]) {
      const a = await here.evaluate(x);
      const b = await there.evaluate(x);
      assert.equal(b.llh, a.llh);
      assert.deepEqual(b.simulated, a.simulated);
      assert.equal(b.simulationTable, a.simulationTable);
    }
    here.close();
    there.close();
  } finally {
    pool.close();
  }
});

test("a problem which is not read or planned is refused with the reason", async () => {
  const options = { compiler: await compiler(), runtime: await loaderRuntime() };
  const missing = { ...files(), "observables.tsv": "observableId\tobservableFormula\nobs_a\tA\n" };
  await assert.rejects(openPetab({ ...options, files: missing }), (error) => {
    assert.ok(error instanceof PetabError);
    assert.match(error.message, /noiseFormula/);
    return true;
  });
  const bytes = Object.fromEntries(
    Object.entries(files()).map(([name, text]) => [name, new TextEncoder().encode(text)]),
  );
  const fromBytes = await openPetab({ ...options, files: new Map(Object.entries(bytes)) });
  assert.equal(fromBytes.nMeasurements, 3);
  fromBytes.close();
  await assert.rejects(openPetab({ ...options, files: { ...files(), "other.yaml": "" } }), /name the YAML file/);
  await assert.rejects(openPetab({ ...options, files: files(), yaml: "nope.yaml" }), /no nope.yaml/);
  await assert.rejects(openPetab({ ...options, files: files(), options: { dataAtChange: "never" } }), /data_at_change/);
  await assert.rejects(openPetab({ ...options, files: files(), options: { tolerance: 1 } }), /unknown options/);
  await assert.rejects(openPetab({ compiler: await compiler(), files: files() }), /a runtime or a pool/);
  await assert.rejects(
    openPetab({ compiler: { exports: { memory: null } }, runtime: options.runtime, files: files() }),
    /without the feature petab/,
  );
});

test("the inline build of the python package evaluates a problem with the bits of the module", async () => {
  const script = path.join(ROOT, "python", "sbmlsim_core", "web", "sbmlsim-core.js");
  // the script defines its global as a classic script does, in the context of this test
  vm.runInThisContext(readFileSync(script, "utf8"), { filename: script });
  const core = globalThis.sbmlsimCore;
  const { engine, mathlib } = runtimeBytes();
  const inline = await core.openPetab({
    compiler: await core.createCompiler({ compiler: compilerWasm() }),
    runtime: await core.createRuntime({ engine, mathlib }),
    files: files(),
  });
  const module = await openPetab({ compiler: await compiler(), runtime: await loaderRuntime(), files: files() });
  try {
    const x = [0.5, 0.4, 3];
    const [a, b] = [await inline.evaluate(x), await module.evaluate(x)];
    assert.equal(a.llh, b.llh);
    assert.equal(a.simulationTable, b.simulationTable);
  } finally {
    inline.close();
    module.close();
  }
});

/** Whether the command line has the command `petab` (a build with the feature `native`). */
function cliHasPetab() {
  try {
    execFileSync(cli(), ["petab", "--help"], { stdio: "ignore" });
    return true;
  } catch {
    return false;
  }
}

test("a page evaluates a problem with the bits of the command line", async (t) => {
  if (!cliHasPetab()) {
    if (process.env.SBMLSIM_CORE_REQUIRE_CASES) {
      throw new Error("the command line has no command petab: build it with --features native");
    }
    t.skip("the command line has no command petab (build it with --features native)");
    return;
  }
  const dir = tempDir("sbmlsim-core-petab-");
  for (const [name, text] of Object.entries(files())) writeFileSync(path.join(dir, name), text);
  const table = path.join(dir, "simulations.tsv");
  const output = execFileSync(cli(), ["petab", path.join(dir, "problem.yaml"), "--simulations", table], {
    encoding: "utf8",
  });
  const field = (name) => Number(new RegExp(`${name} (\\S+?),? `).exec(output)[1]);
  const problem = await openPetab({ compiler: await compiler(), runtime: await loaderRuntime(), files: files() });
  try {
    const evaluation = await problem.evaluate();
    assert.equal(evaluation.simulationTable, readFileSync(table, "utf8"));
    assert.equal(evaluation.llh, field("llh"));
    assert.equal(evaluation.chi2, field("chi2"));
    assert.equal(evaluation.logPrior, field("log prior"));
  } finally {
    problem.close();
  }
});

/** The directory of the cases of the PEtab test suite, or null. */
function suite() {
  if (process.env.SBMLSIM_PETAB_SUITE_PATH) return process.env.SBMLSIM_PETAB_SUITE_PATH;
  const cache = process.env.XDG_CACHE_HOME ?? path.join(homedir(), ".cache");
  const dir = path.join(cache, "sbmlsim", "petab-test-suite", SUITE_COMMIT);
  return existsSync(path.join(dir, "v2.0.0")) ? dir : null;
}

/** A number of a solution or of a table, `inf` and `-inf` included. */
function textNumber(text) {
  const value = String(text).trim().toLowerCase();
  if (value === "inf" || value === "+inf") return Infinity;
  if (value === "-inf") return -Infinity;
  return value === "" ? Number.NaN : Number(value);
}

/**
 * The solution of a case, `_<id>_solution.yaml`: its scalars, the mapping `log_prior` and the
 * list `simulation_files`, the YAML the suite writes (block style, one level deep).
 */
function solution(text) {
  const result = {};
  let open = null;
  for (const line of text.split("\n")) {
    if (line.trim() === "") continue;
    const item = /^\s*-\s+(.*)$/.exec(line);
    if (item && Array.isArray(result[open])) {
      result[open].push(item[1].trim());
      continue;
    }
    const nested = /^\s+([^:]+):\s*(.*)$/.exec(line);
    if (nested && open !== null) {
      result[open] ??= {};
      result[open][nested[1].trim()] = textNumber(nested[2]);
      continue;
    }
    const [, key, value] = /^([^:\s][^:]*):\s*(.*)$/.exec(line);
    if (value.trim() === "") {
      open = key;
      result[key] = key === "simulation_files" ? [] : {};
    } else {
      open = null;
      result[key] = textNumber(value);
    }
  }
  return result;
}

/** The rows of a table of simulations: observable, experiment, time and simulation. */
function simulationRows(text) {
  const lines = text.split("\n").filter((line) => line.trim() !== "");
  const header = lines[0].split("\t").map((cell) => cell.trim());
  const column = (name) => header.indexOf(name);
  return lines.slice(1).map((line) => {
    const cells = line.split("\t").map((cell) => cell.trim());
    return {
      observable: cells[column("observableId")],
      experiment: column("experimentId") >= 0 ? cells[column("experimentId")] || "" : "",
      time: textNumber(cells[column("time")]),
      simulation: textNumber(cells[column("simulation")]),
    };
  });
}

/** Runs a case in the page: the reasons it fails, empty when it passes. */
async function runCase(directory, options) {
  const id = path.basename(directory);
  if (existsSync(path.join(directory, "conversion.error"))) return ["not converted"];
  const files = {};
  for (const name of readdirSync(directory)) {
    if (!name.endsWith(".py") && name !== "README.md") files[name] = readFileSync(path.join(directory, name));
  }
  const expected = solution(readFileSync(path.join(directory, `_${id}_solution.yaml`), "utf8"));
  const problem = await openPetab({ ...options, files, yaml: `_${id}.yaml`, options: TIGHT });
  try {
    const evaluation = await problem.evaluate();
    const failed = evaluation.failures.map(({ simulation, message }) => `simulation ${simulation}: ${message}`);
    const compare = (name, value, reference, tolerance = 0) => {
      const delta = value === reference ? 0 : Math.abs(value - reference);
      if (!(delta <= tolerance)) failed.push(`${name}: ${value} != ${reference}`);
    };
    compare("llh", evaluation.llh, expected.llh, expected.tol_llh);
    compare("chi2", evaluation.chi2, expected.chi2, expected.tol_chi2);
    for (const [parameter, value] of Object.entries(expected.log_prior ?? {})) {
      compare(`log_prior ${parameter}`, evaluation.logPriors[parameter], value, expected.tol_log_prior);
    }
    if (expected.unnorm_log_posterior !== undefined) {
      compare(
        "unnorm_log_posterior",
        evaluation.unnormLogPosterior,
        expected.unnorm_log_posterior,
        expected.tol_unnorm_log_posterior,
      );
    }
    // every simulation of the solution, matched with an unmatched row of the page's table
    const mine = simulationRows(evaluation.simulationTable);
    const used = new Array(mine.length).fill(false);
    let rows = 0;
    for (const name of expected.simulation_files ?? []) {
      for (const row of simulationRows(readFileSync(path.join(directory, name), "utf8"))) {
        rows += 1;
        const k = mine.findIndex(
          (m, j) => !used[j] && m.observable === row.observable && m.experiment === row.experiment && m.time === row.time,
        );
        if (k < 0) {
          failed.push(`the simulation of ${row.observable} at ${row.time} is of no measurement`);
          continue;
        }
        used[k] = true;
        compare(`simulation ${row.observable} at ${row.time}`, mine[k].simulation, row.simulation, expected.tol_simulations);
      }
    }
    if (rows !== problem.nMeasurements) failed.push(`${rows} simulations of ${problem.nMeasurements} measurements`);
    return failed;
  } finally {
    problem.close();
  }
}

test("the cases of the PEtab test suite pass in a page", async (t) => {
  const dir = suite();
  if (dir === null) {
    if (process.env.SBMLSIM_CORE_REQUIRE_CASES) {
      throw new Error("the cases of the PEtab test suite are absent: python3 scripts/petab_testsuite.py download");
    }
    t.skip("the cases of the PEtab test suite are absent (python3 scripts/petab_testsuite.py download)");
    return;
  }
  const baseline = JSON.parse(
    readFileSync(path.join(ROOT, "crates", "petab", "tests", "data", "petab_baseline.json"), "utf8"),
  );
  const options = { compiler: await compiler(), runtime: await loaderRuntime() };
  let passed = 0;
  // the cases of PEtab v2, and those of v1 which sbmlsim converts where they exist
  for (const [group, sub] of [["sbml", "v2.0.0/sbml"], ["v1/sbml", "v1.0.0-converted/sbml"]]) {
    const groupDir = path.join(dir, sub);
    if (!existsSync(groupDir)) continue;
    for (const id of readdirSync(groupDir).filter((name) => /^\d+$/.test(name)).sort()) {
      const key = `${group}/${id}`;
      let failed;
      try {
        failed = await runCase(path.join(groupDir, id), options);
      } catch (error) {
        failed = [error.message];
      }
      if (key in baseline.expected_failures) {
        assert.notDeepEqual(failed, [], `${key} passes, but the baseline lists it`);
      } else {
        assert.deepEqual(failed, [], `${key}: ${failed.join("; ")}`);
        passed += 1;
      }
    }
  }
  t.diagnostic(`${passed} cases pass`);
  assert.ok(passed >= 31, `${passed} cases passed`);
});
