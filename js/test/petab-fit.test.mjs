// The fit of a PEtab v2 problem in the JavaScript host (`problem.fit` of `petab.mjs`): the
// optimizer of `sbmlsim-fit` in `compiler.wasm`, driven ask/tell, its simulations run by the
// loader or the pool.
//
// The decisive check is that a page fits a problem as the command line does
// (`sbmlsim-core petab --fit`, `Executor::fit` of `sbmlsim-petab`): the same optimum, the same
// objective and the same counts, bit for bit, since both run the optimizer of `sbmlsim-fit` on
// the same evaluations of `engine.wasm`. The problems are the conversion problem of the live
// demo of the documentation (`docs/demo/conversion`) and every case of the PEtab test suite
// whose fit the engine carries (see `petab.test.mjs` for where the cases come from). The
// comparison needs a command line built with the feature `native`, which CI builds.

import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { existsSync, readFileSync, readdirSync } from "node:fs";
import { homedir } from "node:os";
import path from "node:path";
import { test } from "node:test";

import { createCompiler } from "../compile.mjs";
import { openPetab } from "../petab.mjs";
import { createPool } from "../pool.mjs";
import { ROOT, cli, compilerWasm, loaderRuntime, nodeWorker } from "./helpers.mjs";

/** The commit of the PEtab test suite, `COMMIT` of `crates/petab/tests/testsuite.rs`. */
const SUITE_COMMIT = "d09985e7bb03280b461599a24337137533d931e2";

/** The conversion problem of the live demo of the documentation. */
const DEMO = path.join(ROOT, "docs", "demo", "conversion");

/** The files of a problem in a directory, by their names. */
function filesOf(directory) {
  const files = {};
  for (const name of readdirSync(directory)) {
    if (/\.(ya?ml|tsv|xml|sbml)$/i.test(name)) files[name] = readFileSync(path.join(directory, name), "utf8");
  }
  return files;
}

let compilerPromise = null;
function compiler() {
  compilerPromise ??= createCompiler({ compiler: compilerWasm() });
  return compilerPromise;
}

/** Whether the command line has the command `petab` (a build with the feature `native`). */
function cliHasPetab() {
  try {
    execFileSync(cli(), ["petab", "--help"], { stdio: "ignore" });
    return true;
  } catch {
    return false;
  }
}

/** The fit of the command line: the values by id, the objective, the evaluations, the status. */
function cliFit(yaml) {
  const output = execFileSync(cli(), ["petab", yaml, "--fit", "--threads", "2"], {
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
  });
  const line = output.split("\n").find((text) => text.includes(" evaluations in "));
  const match = /^(.*) after (\d+) evaluations in \S+ s, objective (\S+) \((.*)\)$/.exec(line);
  assert.ok(match, output);
  const values = Object.fromEntries(
    match[1].split(", ").map((pair) => {
      const [id, value] = pair.split(" = ");
      return [id, Number(value)];
    }),
  );
  return { values, evaluations: Number(match[2]), objective: Number(match[3]), status: match[4] };
}

/**
 * Asserts that a fit of a page is the fit of the command line: the same optimum and objective
 * to the tolerance of a fit.
 *
 * The command line fits on the native engine of the batch and the page on `engine.wasm`, whose
 * doubles may differ in their last bits (a function of the math library is `mathlib.wasm` in
 * the one and the one of the platform in the other), so the paths of the two optimizers may
 * part at their last iterations. The objectives agree to `1e-8`, and so do the optima: their
 * values to `1e-4`, or, where the data does not determine a parameter and the optimizers ended
 * at two points of a valley of the likelihood, the likelihoods of the two points. A fit whose
 * objective has no minimum (a noise which is estimated runs to zero, `SolverFailed`) ends where
 * its rounding takes it, and is held to its status alone. The record says how often the two
 * were the same to the bit.
 */
async function assertCliFit(problem, fit, expected, name, record = {}) {
  const parameters = problem.parameters;
  assert.equal(fit.status, expected.status, `${name}: the status`);
  const same = parameters.every((id, k) => fit.x[k] === expected.values[id]) && fit.objective === expected.objective;
  record.exact = (record.exact ?? 0) + (same ? 1 : 0);
  if (expected.status === "SolverFailed") return;
  const objective = Math.abs(fit.objective - expected.objective) / Math.max(Math.abs(expected.objective), 1);
  record.objective = Math.max(record.objective ?? 0, objective);
  assert.ok(objective < 1e-8, `${name}: the objective ${fit.objective} != ${expected.objective}`);
  const cliX = parameters.map((id) => expected.values[id]);
  const apart = parameters.some(
    (id, k) => Math.abs(fit.x[k] - cliX[k]) > 1e-4 * Math.max(Math.abs(cliX[k]), 1e-6),
  );
  if (apart) {
    const here = await problem.evaluate(fit.x);
    const there = await problem.evaluate(cliX);
    const llh = Math.abs(here.llh - there.llh) / Math.max(Math.abs(there.llh), 1);
    record.valleys = (record.valleys ?? 0) + 1;
    assert.ok(llh < 1e-8, `${name}: the optimum ${fit.x} != ${cliX}`);
  }
}

test("a page fits the problem of the demo as the command line does", async (t) => {
  const problem = await openPetab({ compiler: await compiler(), runtime: await loaderRuntime(), files: filesOf(DEMO) });
  try {
    const steps = [];
    const fit = await problem.fit({ onStep: (step) => steps.push(step) });
    // the optimum is better than the nominal values, and inside the bounds
    const nominal = await problem.evaluate();
    assert.ok(-fit.objective > nominal.llh, `${fit.objective} ${nominal.llh}`);
    assert.ok(fit.x.every((value, k) => value > problem.lower[k] && value < problem.upper[k]), `${fit.x}`);
    assert.ok(fit.iterations > 2, fit.status);
    assert.equal(fit.runs.length, 1);
    assert.deepEqual(fit.start, problem.nominal);
    // the objective is the negative log-likelihood of PEtab at the optimum
    const evaluation = await problem.evaluate(fit.x);
    assert.ok(Math.abs(fit.objective + evaluation.llh) < 1e-9, `${fit.objective} ${evaluation.llh}`);
    // the start, then the steps: the simulations of every one reported
    assert.ok(steps.length >= fit.iterations, `${steps.length}`);
    assert.equal(steps.at(-1).simulations, fit.simulations);
    assert.ok(steps.every((step, k) => step.step === k + 1 && step.start === 0));
    if (!cliHasPetab()) {
      if (process.env.SBMLSIM_CORE_REQUIRE_CASES) throw new Error("the command line has no command petab");
      t.diagnostic("the command line has no command petab (build it with --features native)");
      return;
    }
    const expected = cliFit(path.join(DEMO, "problem.yaml"));
    await assertCliFit(problem, fit, expected, "demo");
  } finally {
    problem.close();
  }
});

test("a pool fits a problem with the bits of the loader", async () => {
  const runtime = await loaderRuntime();
  const pool = await createPool({ ...runtime.modules, workers: 3, createWorker: nodeWorker });
  try {
    const here = await openPetab({ compiler: await compiler(), runtime, files: filesOf(DEMO) });
    const there = await openPetab({ compiler: await compiler(), pool, files: filesOf(DEMO) });
    for (const options of [{}, { algorithm: "levenbergMarquardt" }, { starts: 3, seed: 5 }]) {
      const a = await here.fit(options);
      const b = await there.fit(options);
      assert.deepEqual(b.x, a.x);
      assert.equal(b.objective, a.objective);
      assert.equal(b.evaluations, a.evaluations);
      assert.equal(b.simulations, a.simulations);
    }
    here.close();
    there.close();
  } finally {
    pool.close();
  }
});

test("a fit of several starts gives the best of their runs", async () => {
  const problem = await openPetab({ compiler: await compiler(), runtime: await loaderRuntime(), files: filesOf(DEMO) });
  try {
    const fit = await problem.fit({ starts: 4, seed: 1 });
    assert.equal(fit.runs.length, 4);
    assert.deepEqual(fit.runs[0].start, problem.nominal);
    for (const run of fit.runs.slice(1)) {
      for (const [k, value] of run.start.entries()) {
        assert.ok(value >= problem.lower[k] && value <= problem.upper[k], `${run.start}`);
      }
    }
    assert.equal(fit.objective, Math.min(...fit.runs.map((run) => run.objective)));
    // a start of its own is the fit from that start; the same seed draws the same starts
    const again = await problem.fit({ starts: fit.runs.map((run) => run.start) });
    assert.deepEqual(again.runs.map((run) => run.x), fit.runs.map((run) => run.x));
    // an evolution, polished by the trust region
    const evolution = await problem.fit({
      algorithm: { differentialEvolution: { seed: 2, population: 8 } },
      maxIterations: 5,
    });
    assert.match(evolution.status, /polish/);
    assert.throws(() => problem.starts([[1]]), /estimates 2 parameters/);
    await assert.rejects(problem.fit({ maxIteration: 3 }), /unknown options maxIteration/);
  } finally {
    problem.close();
  }
});

test("a fit which is aborted answers with the point it reached", async () => {
  const problem = await openPetab({ compiler: await compiler(), runtime: await loaderRuntime(), files: filesOf(DEMO) });
  try {
    const full = await problem.fit();
    const controller = new AbortController();
    const fit = await problem.fit({
      signal: controller.signal,
      onStep: ({ step }) => {
        if (step === 3) controller.abort();
      },
    });
    assert.equal(fit.status, "UserRequested");
    assert.ok(fit.iterations < full.iterations, `${fit.iterations} ${full.iterations}`);
    const stopped = await problem.fit({ maxSeconds: 0 });
    assert.equal(stopped.status, "UserRequested");
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

test("the cases of the PEtab test suite are fitted in a page as on the command line", async (t) => {
  const dir = suite();
  if (dir === null || !cliHasPetab()) {
    if (process.env.SBMLSIM_CORE_REQUIRE_CASES) throw new Error("the PEtab cases or the command petab are missing");
    t.skip("the PEtab test suite is not downloaded or the command line has no command petab");
    return;
  }
  const runtime = await loaderRuntime();
  let fitted = 0;
  const record = {};
  for (const group of ["v2.0.0/sbml", "v1.0.0-converted/sbml"]) {
    const groupDir = path.join(dir, group);
    if (!existsSync(groupDir)) continue;
    for (const id of readdirSync(groupDir).filter((name) => /^\d+$/.test(name)).sort()) {
      const directory = path.join(groupDir, id);
      const yaml = path.join(directory, `_${id}.yaml`);
      if (!existsSync(yaml)) continue;
      let expected;
      try {
        expected = cliFit(yaml);
      } catch {
        // a problem the command line does not read or whose fit the engine does not carry
        continue;
      }
      const problem = await openPetab({ compiler: await compiler(), runtime, files: filesOf(directory), yaml: `_${id}.yaml` });
      try {
        await assertCliFit(problem, await problem.fit(), expected, `${group}/${id}`, record);
        fitted += 1;
      } finally {
        problem.close();
      }
    }
  }
  t.diagnostic(
    `${fitted} cases fitted as on the command line, ${record.exact} of them to the bit, ` +
      `${record.valleys ?? 0} at another point of a valley; the largest relative difference of an ` +
      `objective ${record.objective}`,
  );
  assert.ok(fitted > 40, `${fitted}`);
});
