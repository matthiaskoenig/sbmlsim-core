// The benchmark of the JavaScript hosts, shared by Node (`js/bench.mjs`) and the browser
// (`bench/web/worker.mjs`); plain ECMAScript without APIs of Node.
//
// It runs the models and requests `sbmlsim-core bench --export DIR` wrote (`DIR/bench.json`) as
// the command line runs them in wasmtime (`crates/cli/src/bench.rs`): per model the median of
// the instantiations (`runtime.instantiate`: the compilation of `model.wasm` by the host, the
// instances and the load of the description), then the configurations take turns over the
// rounds, a measurement of a configuration is the median of the timed simulations after the
// untimed ones (`instance.simulate`: the request to JSON, the simulation and the copy of the
// result), and its time is the median of its medians over the rounds. The counters are the ones
// of the last simulation, and every timed simulation must give the bits of the first.

import { createRuntime } from "./loader.mjs";

/** The median of a list of numbers. */
export function median(values) {
  const sorted = [...values].sort((a, b) => a - b);
  const n = sorted.length;
  if (n === 0) return NaN;
  return n % 2 === 1 ? sorted[(n - 1) / 2] : 0.5 * (sorted[n / 2 - 1] + sorted[n / 2]);
}

/** Whether two `Float64Array`s have the same bits. */
function sameBits(a, b) {
  if (a.length !== b.length) return false;
  const x = new Uint8Array(a.buffer, a.byteOffset, a.byteLength);
  const y = new Uint8Array(b.buffer, b.byteOffset, b.byteLength);
  for (let i = 0; i < x.length; i += 1) if (x[i] !== y[i]) return false;
  return true;
}

/**
 * Runs the benchmark of a manifest.
 *
 * @param {object} options
 * @param {string} options.host the name of the host in the results
 * @param {object} options.manifest `bench.json`
 * @param {(path: string) => Promise<Uint8Array>} options.readBytes reads a file of the export
 * @param {(path: string) => Promise<string>} options.readText reads a file of the export as text
 * @param {Uint8Array} options.engine `engine.wasm`
 * @param {Uint8Array} options.mathlib `mathlib.wasm`
 * @param {() => number} options.now the time in milliseconds
 * @param {(line: string) => void} [options.log] progress
 * @param {string[]} [options.only] only these models
 * @param {number} [options.rounds] overrides of the manifest
 * @param {number} [options.warmups]
 * @param {number} [options.reps]
 * @param {number} [options.instantiateReps] the instantiations of a model (5)
 */
export async function runBench({
  host,
  manifest,
  readBytes,
  readText,
  engine,
  mathlib,
  now,
  log = () => {},
  only = null,
  rounds = manifest.rounds,
  warmups = manifest.warmups,
  reps = manifest.reps,
  instantiateReps = 5,
}) {
  const runtime = await createRuntime({ engine, mathlib });
  const models = [];
  for (const entry of manifest.models) {
    if (only && !only.includes(entry.name)) continue;
    const wasm = await readBytes(`${entry.dir}/model.wasm`);
    const description = await readText(`${entry.dir}/model.json`);
    const module = await WebAssembly.compile(wasm);
    const instantiations = [];
    for (let i = 0; i < instantiateReps; i += 1) {
      const start = now();
      await runtime.instantiate(wasm, description);
      instantiations.push((now() - start) / 1e3);
    }
    const instance = await runtime.instantiate(module, description);
    const requests = [];
    for (const config of entry.configs) {
      requests.push(await readText(`${entry.dir}/${config.request}`));
    }
    const roundMedians = entry.configs.map(() => []);
    const last = entry.configs.map(() => ({ nRows: 0, bitwise: true, stats: null }));
    for (let round = 0; round < Math.max(rounds, 1); round += 1) {
      for (let k = 0; k < requests.length; k += 1) {
        for (let i = 0; i < warmups; i += 1) instance.simulate(requests[k]);
        const times = [];
        let first = null;
        let bitwise = true;
        let nRows = 0;
        for (let i = 0; i < Math.max(reps, 1); i += 1) {
          const start = now();
          const result = instance.simulate(requests[k]);
          times.push((now() - start) / 1e3);
          nRows = result.nRows;
          if (first === null) first = new Float64Array(result.values);
          else bitwise &&= sameBits(first, result.values);
        }
        roundMedians[k].push(median(times));
        last[k] = { nRows, bitwise: last[k].bitwise && bitwise, stats: instance.stats() };
      }
      log(
        `${entry.name}: round ${round + 1}/${Math.max(rounds, 1)}: ` +
          entry.configs
            .map((c, k) => `${c.solver}_${c.jacobian}_${c.output} ${(roundMedians[k][round] * 1e3).toFixed(4)} ms`)
            .join(", "),
      );
    }
    const json = JSON.parse(description);
    models.push({
      name: entry.name,
      states: json.states.length,
      instantiate_s: median(instantiations),
      configs: entry.configs.map((c, k) => ({
        solver: c.solver,
        jacobian: c.jacobian,
        output: c.output,
        median_s: median(roundMedians[k]),
        round_medians_s: roundMedians[k],
        n_rows: last[k].nRows,
        repeat_bitwise: last[k].bitwise,
        stats: last[k].stats,
      })),
    });
  }
  return { host, rounds, warmups, reps, models };
}
