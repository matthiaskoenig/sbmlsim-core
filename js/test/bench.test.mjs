// The benchmark: `sbmlsim-core bench --export` in wasmtime and `js/bench-core.mjs` in Node run
// the same requests with the same counters of the integrator.

import assert from "node:assert/strict";
import { readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { test } from "node:test";

import { median, runBench } from "../bench-core.mjs";
import { STATS_NAMES } from "../loader.mjs";
import { ROOT, runCli, runtimeBytes, tempDir } from "./helpers.mjs";

test("the median of an even count is the mean of the middle", () => {
  assert.equal(median([3, 1, 2]), 2);
  assert.equal(median([4, 1, 3, 2]), 2.5);
});

test("Node runs the export of the wasmtime benchmark with the same counters", async () => {
  const dir = tempDir("sbmlsim-core-bench-");
  const sbml = path.join(ROOT, "bench", "models", "repeated_dose.xml");
  const toml = path.join(dir, "models.toml");
  writeFileSync(
    toml,
    `[models.repeated_dose]\nsource = "test"\nsbml = ${JSON.stringify(sbml)}\nend = 48.0\npoints = 51\n` +
      "relative_tolerance = 1e-6\nabsolute_tolerance = 1e-12\nsteps = true\n",
  );
  const exportDir = path.join(dir, "export");
  const json = path.join(dir, "wasmtime.json");
  const quick = ["--rounds", "1", "--reps", "2", "--warmups", "0", "--compile-reps", "1"];
  runCli(["bench", toml, ...quick, "--export", exportDir, "--json", json]);
  const wasmtime = JSON.parse(readFileSync(json, "utf8"));
  const manifest = JSON.parse(readFileSync(path.join(exportDir, "bench.json"), "utf8"));
  const node = await runBench({
    host: "node",
    manifest,
    readBytes: async (file) => readFileSync(path.join(exportDir, file)),
    readText: async (file) => readFileSync(path.join(exportDir, file), "utf8"),
    ...runtimeBytes(),
    now: () => performance.now(),
    instantiateReps: 1,
  });
  const [w] = wasmtime.models;
  const [n] = node.models;
  assert.equal(n.states, w.states);
  assert.equal(n.configs.length, 8);
  for (const [k, config] of n.configs.entries()) {
    const other = w.configs[k];
    assert.deepEqual([config.solver, config.jacobian, config.output], [other.solver, other.jacobian, other.output]);
    assert.deepEqual(Object.keys(config.stats), STATS_NAMES);
    assert.deepEqual(config.stats, other.stats);
    assert.equal(config.n_rows, other.n_rows);
    assert.ok(config.repeat_bitwise && other.repeat_bitwise);
    assert.ok(config.stats.steps > 0 && config.stats.probe_rhs_evals > 0, JSON.stringify(config.stats));
    // the difference quotient calls the right hand side per color of the pattern
    assert.equal(config.stats.jacobian_rhs_evals > 0, config.jacobian === "difference");
  }
});
