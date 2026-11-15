// The worker of the benchmark in Chrome (`bench/web/index.html`).
import { runBench } from "../../js/bench-core.mjs";

const bytes = async (url) => {
  const response = await fetch(url);
  if (!response.ok) throw new Error(`${url}: ${response.status}`);
  return new Uint8Array(await response.arrayBuffer());
};
const text = async (url) => new TextDecoder().decode(await bytes(url));

self.onmessage = async ({ data }) => {
  try {
    const manifest = JSON.parse(await text(`${data.exportDir}/bench.json`));
    const result = await runBench({
      host: "chrome",
      manifest,
      readBytes: (file) => bytes(`${data.exportDir}/${file}`),
      readText: (file) => text(`${data.exportDir}/${file}`),
      engine: await bytes("../../js/runtime/engine.wasm"),
      mathlib: await bytes("../../js/runtime/mathlib.wasm"),
      now: () => performance.now(),
      log: (line) => self.postMessage({ log: line }),
      only: data.only,
      rounds: data.rounds ?? manifest.rounds,
      warmups: data.warmups ?? manifest.warmups,
      reps: data.reps ?? manifest.reps,
    });
    self.postMessage({ result });
  } catch (error) {
    self.postMessage({ error: `${error}\n${error.stack}` });
  }
};
