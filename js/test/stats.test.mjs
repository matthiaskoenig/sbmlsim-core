// The statistics in the JavaScript host (`stats.mjs`): every function of `sbmlsim_core.stats`
// run by `compiler.wasm`.
//
// The decisive check is that a page gives what python gives, bit for bit, and the errors of
// python with their kind and message: `fixtures/stats.json` holds about a thousand calls of
// python built from the oracle fixtures of the crate (`crates/stats/oracles`) with their results,
// which python computed natively (`fixtures/make_stats.py`), and every call runs here through
// `compiler.wasm` with the arguments of JavaScript.

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import path from "node:path";
import { test } from "node:test";

import { createCompiler } from "../compile.mjs";
import { StatsError, createStats } from "../stats.mjs";
import { ROOT, compilerWasm } from "./helpers.mjs";

const FIXTURE = JSON.parse(
  readFileSync(path.join(ROOT, "js", "test", "fixtures", "stats.json"), "utf8"),
);

let statsPromise = null;

/** The statistics of the `compiler.wasm` of the runtime, created once. */
function statsOf() {
  statsPromise ??= createCompiler({ compiler: compilerWasm() }).then((compiler) =>
    createStats({ compiler }),
  );
  return statsPromise;
}

/** A value of the fixture with its tagged non-finite doubles as numbers and its large integers as BigInts. */
function fromFixture(value) {
  if (Array.isArray(value)) return value.map(fromFixture);
  if (value !== null && typeof value === "object") {
    if (Object.keys(value).length === 1 && typeof value.$ === "string") return Number(value.$.replace("inf", "Infinity"));
    if (Object.keys(value).length === 1 && typeof value.bigint === "string") return BigInt(value.bigint);
    return Object.fromEntries(Object.entries(value).map(([key, item]) => [key, fromFixture(item)]));
  }
  return value;
}

/** A name in camelCase of a name of python. */
function camel(name) {
  return name.replace(/_([a-z0-9])/g, (_, letter) => letter.toUpperCase());
}

/** A result as plain arrays, objects and numbers, the keys of python in camelCase. */
function canonical(value) {
  if (ArrayBuffer.isView(value) || Array.isArray(value)) return Array.from(value, canonical);
  if (value !== null && typeof value === "object") {
    return Object.fromEntries(Object.entries(value).map(([key, item]) => [camel(key), canonical(item)]));
  }
  return value;
}

/** The path of the first difference of two canonical values, the doubles by their bits. */
function difference(actual, expected, at = "") {
  if (typeof expected === "number") {
    return typeof actual === "number" && Object.is(actual, expected)
      ? null
      : `${at}: ${String(actual)} (${Object.is(actual, -0) ? "-0" : ""}) instead of ${String(expected)}`;
  }
  if (Array.isArray(expected)) {
    if (!Array.isArray(actual) || actual.length !== expected.length) {
      return `${at}: ${JSON.stringify(actual)?.slice(0, 80)} instead of an array of ${expected.length}`;
    }
    for (let k = 0; k < expected.length; k += 1) {
      const found = difference(actual[k], expected[k], `${at}[${k}]`);
      if (found) return found;
    }
    return null;
  }
  if (expected !== null && typeof expected === "object") {
    const keys = Object.keys(expected).sort();
    if (actual === null || typeof actual !== "object" || Object.keys(actual).sort().join() !== keys.join()) {
      return `${at}: the keys ${Object.keys(actual ?? {})} instead of ${keys}`;
    }
    for (const key of keys) {
      const found = difference(actual[key], expected[key], `${at}.${key}`);
      if (found) return found;
    }
    return null;
  }
  return actual === expected ? null : `${at}: ${JSON.stringify(actual)} instead of ${JSON.stringify(expected)}`;
}

/** Runs a call of the fixture in JavaScript: the result, or the error thrown. */
function run(stats, entry) {
  const args = fromFixture(entry.args);
  const options = Object.fromEntries(
    Object.entries(fromFixture(entry.kwargs)).map(([key, value]) => [camel(key), value]),
  );
  return stats[camel(entry.function)](...args, options);
}

test("every function of python is a function of the statistics", async () => {
  const stats = await statsOf();
  const functions = new Set(FIXTURE.calls.map((entry) => camel(entry.function)));
  assert.equal(functions.size, 46);
  for (const name of functions) assert.equal(typeof stats[name], "function", name);
  const own = Object.keys(stats).filter((key) => typeof stats[key] === "function");
  assert.deepEqual(own.sort(), [...functions].sort());
  assert.throws(() => {
    stats.sse = null;
  }, TypeError);
});

test("the constants are those of python", async () => {
  const stats = await statsOf();
  assert.deepEqual(
    {
      BISECTIONS: stats.BISECTIONS,
      DEFAULT_RANK_TOLERANCE: stats.DEFAULT_RANK_TOLERANCE,
      EPS: stats.EPS,
      MARGIN: stats.MARGIN,
      MAX_NEEDED: stats.MAX_NEEDED,
      MIN_POINTS_SUBSET_AGREEMENT: stats.MIN_POINTS_SUBSET_AGREEMENT,
      MIXTURE: stats.MIXTURE,
      PSD_RTOL: stats.PSD_RTOL,
    },
    {
      BISECTIONS: 52,
      DEFAULT_RANK_TOLERANCE: 1e-8,
      EPS: 1e-12,
      MARGIN: 0.05,
      MAX_NEEDED: 2 ** 26,
      MIN_POINTS_SUBSET_AGREEMENT: 10,
      MIXTURE: [0.5, 0.5],
      PSD_RTOL: 1e-10,
    },
  );
});

test("every call of the fixture gives the bits and the errors of python", async () => {
  const stats = await statsOf();
  const failures = [];
  let errors = 0;
  for (const entry of FIXTURE.calls) {
    const label = `${entry.name} ${entry.function}`;
    let result;
    try {
      result = run(stats, entry);
    } catch (error) {
      if (!entry.error) {
        failures.push(`${label}: threw ${error}`);
      } else if (!(error instanceof StatsError)) {
        failures.push(`${label}: threw ${error}, not a StatsError`);
      } else if (error.kind !== entry.error.kind || error.message !== entry.error.message) {
        failures.push(`${label}: ${error.kind}: ${error.message} instead of ${entry.error.kind}: ${entry.error.message}`);
      } else {
        errors += 1;
      }
      continue;
    }
    if (entry.error) {
      failures.push(`${label}: no ${entry.error.kind}`);
      continue;
    }
    const found = difference(canonical(result), canonical(fromFixture(entry.result)));
    if (found) failures.push(`${label}${found}`);
  }
  assert.deepEqual(failures, []);
  // every kind of the errors of python is among them
  assert.ok(errors > 100, `${errors} errors`);
  const kinds = new Set(FIXTURE.calls.filter((entry) => entry.error).map((entry) => entry.error.kind));
  assert.deepEqual([...kinds].sort(), ["IndexError", "OverflowError", "ValueError", "ZeroDivisionError"]);
});

test("the results are typed arrays, rows and tuples", async () => {
  const stats = await statsOf();
  const gram = stats.gram([
    [1, 0],
    [0, 2],
    [1, 1],
  ]);
  assert.equal(gram.length, 2);
  assert.ok(gram.every((row) => row instanceof Float64Array));
  assert.deepEqual(gram.map((row) => Array.from(row)), [
    [2, 1],
    [1, 5],
  ]);
  const fisher = stats.fisherStatistics(gram, 0.05, 20);
  assert.equal(fisher.rank, 2);
  assert.ok(fisher.standardErrors instanceof Float64Array);
  assert.equal(stats.subsetAgreement([1, 2], [1, 2]), null);
  const [mean, se, n] = stats.meanSe(new Float64Array([1, 2, NaN, 3]));
  assert.deepEqual([mean, n], [2, 3]);
  assert.ok(Math.abs(se - Math.sqrt(1 / 3)) < 1e-15, `${se}`);
  assert.deepEqual(stats.decision([1, 0.01], [1, 1], [1, 1], 0.05), ["accepted", 2]);
  assert.equal(stats.validateCovariance(["a"], [[1]]), null);
});

test("doubles cross as their bits, -0, NaN and the infinities included", async () => {
  const stats = await statsOf();
  assert.ok(Object.is(stats.rmseFromMse(-0), -0));
  assert.ok(Number.isNaN(stats.nrmse([NaN])));
  assert.equal(stats.sse([Infinity]), Infinity);
  const [low, high] = stats.guestLimits([-Infinity, -0]);
  assert.equal(low.length, 1 + 1);
  assert.equal(high.length, 2);
  // a number alone is an array of no dimension, its limits numbers
  const [scalarLow] = stats.guestLimits(2);
  assert.equal(typeof scalarLow, "number");
});

test("an array is any array of numbers, nested arrays its dimensions", async () => {
  const stats = await statsOf();
  const rows = [
    [0.1, 0.9],
    [0.4, 0.6],
    [0.7, 0.2],
  ];
  const r = [
    [1, 0.5],
    [0.5, 1],
  ];
  const expected = stats.copula(rows, r).map((row) => Array.from(row));
  assert.deepEqual(stats.copula(rows.map((row) => Float64Array.from(row)), r).map((row) => Array.from(row)), expected);
  assert.deepEqual(stats.copula(rows.map((row) => Float32Array.from(row, (x) => x)), r).length, 3);
  assert.equal(stats.sse(new Int32Array([1, 2, 3])), 14);
  assert.equal(stats.sse([[1, 2], [3, 4]]), 30);
  assert.equal(stats.sse([true, false, true]), 2);
  assert.throws(() => stats.sse([[1, 2], [3]]), { name: "TypeError", message: /not of one shape/ });
  assert.throws(() => stats.sse([[1, 2], 3]), { name: "TypeError", message: /not of one shape/ });
  assert.throws(() => stats.sse(["1"]), { name: "TypeError", message: /neither a number nor an array/ });
  assert.throws(() => stats.sse(new BigInt64Array(2)), TypeError);
});

test("booleans are true unless 0, as numpy converts them", async () => {
  const stats = await statsOf();
  const booleans = stats.pValues([true, false, true, true], 0.5, "greater");
  const numbers = stats.pValues([1, 0, NaN, -2], 0.5, "greater");
  assert.deepEqual(Array.from(numbers), Array.from(booleans));
});

test("the integers of 64 bits, a seed as a BigInt", async () => {
  const stats = await statsOf();
  const byNumber = stats.unitPoints(2, 4, { design: "random", seed: 7 });
  const byBigInt = stats.unitPoints(2, 4, { design: "random", seed: 7n });
  assert.deepEqual(byBigInt, byNumber);
  assert.equal(stats.unitPoints(2, 4, { design: "random", seed: 2n ** 64n - 1n }).length, 4);
  assert.throws(() => stats.unitPoints(2, 4, { seed: 2n ** 64n }), RangeError);
  assert.throws(() => stats.unitPoints(2, 4, { seed: 1.5 }), { name: "TypeError", message: /seed is an integer/ });
  assert.throws(() => stats.unitPoints(2.5, 4, { seed: 1 }), TypeError);
  assert.throws(() => stats.unitPoints(2, 4, { seed: -1 }), {
    name: "StatsError",
    kind: "ValueError",
    message: "expected non-negative integer",
  });
  assert.throws(() => stats.familyLevel(0.95, 2n ** 63n), RangeError);
});

test("arguments and options are checked as python checks them", async () => {
  const stats = await statsOf();
  // the options take the defaults of python, null as well
  assert.deepEqual(stats.foldCounts([1, 2], [1, 3]), stats.foldCounts([1, 2], [1, 3], { fold: 2 }));
  assert.deepEqual(stats.foldCounts([1, 2], [1, 3], { fold: null }), stats.foldCounts([1, 2], [1, 3]));
  assert.throws(() => stats.sse(), { name: "TypeError", message: /takes 1 arguments/ });
  assert.throws(() => stats.sse([1], [2]), TypeError);
  assert.throws(() => stats.foldCounts([1], [1], { folds: 3 }), { name: "TypeError", message: /unknown options folds/ });
  assert.throws(() => stats.foldCounts([1], [1], { toString: 3 }), { name: "TypeError", message: /unknown options/ });
  assert.throws(() => stats.unitPoints(2, 4), { name: "TypeError", message: /needs the option seed/ });
  assert.throws(() => stats.intervalParameters(1, 2), { name: "TypeError", message: /needs the option lognormal/ });
  assert.throws(() => stats.rmseFromMse("1"), { name: "TypeError", message: /mse is a number/ });
  assert.throws(() => stats.fisherStatistics([[1]], 1, 3, { likelihood: 1 }), TypeError);
  // a name python refuses is a StatsError with the message of python
  assert.throws(() => stats.unitPoints(2, 4, { design: 3, seed: 1 }), {
    name: "StatsError",
    kind: "ValueError",
    message: "The design 3 is none of 'sobol', 'lhs' and 'random'.",
  });
  const error = (() => {
    try {
      stats.mse([]);
    } catch (caught) {
      return caught;
    }
  })();
  assert.ok(error instanceof StatsError && error instanceof Error);
  assert.equal(error.kind, "ValueError");
  assert.equal(error.message, "MSE requires at least one residual.");
});

test("a compiler without the statistics is refused", () => {
  assert.throws(() => createStats({}), TypeError);
  assert.throws(
    () => createStats({ compiler: { exports: { allocate() {}, deallocate() {}, memory: {} } } }),
    /built without the feature stats/,
  );
});
