// The inline build (`inline.mjs`, built by `build.mjs` into the python package): the committed
// build is the one of the sources, it is one classic script, and its endpoint of a bundle held in
// memory, on workers started from the embedded source of the worker, answers the routes with the
// bits of the endpoint of the files.

import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import path from "node:path";
import { pathToFileURL } from "node:url";
import vm from "node:vm";
import { Worker } from "node:worker_threads";
import { after, before, test } from "node:test";

import { createPanelEndpoint } from "../panels.mjs";
import {
  PANEL_MODEL,
  ROOT,
  compilerWasm,
  fileFetch,
  nodeWorker,
  panelBundle,
  runtimeBytes,
  runtimeUrls,
} from "./helpers.mjs";

const SCRIPT = path.join(ROOT, "python", "sbmlsim_core", "web", "sbmlsim-core.js");
const BASE = "local:toy";

let core;
let files;
let fromFiles;
let inline;

/** A simulation of a panel of an endpoint: its status and its body. */
async function simulate(fetcher, panel, body) {
  const response = await fetcher(`${BASE}/api/models/toy/panels/${panel}/simulate`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
  });
  return { status: response.status, body: await response.json() };
}

before(() => {
  // the script defines its global as a classic script does, in the context of this test
  vm.runInThisContext(readFileSync(SCRIPT, "utf8"), { filename: SCRIPT });
  core = globalThis.sbmlsimCore;
  const bundleFile = panelBundle();
  const dir = path.join(path.dirname(bundleFile), "models", PANEL_MODEL);
  files = {
    bundle: JSON.parse(readFileSync(bundleFile, "utf8")),
    models: {
      [`models/${PANEL_MODEL}/model.wasm`]: readFileSync(path.join(dir, "model.wasm")),
      [`models/${PANEL_MODEL}/model.json`]: readFileSync(path.join(dir, "model.json")),
    },
  };
  fromFiles = createPanelEndpoint({
    bundle: pathToFileURL(bundleFile),
    runtime: runtimeUrls(),
    workers: 2,
    createWorker: nodeWorker,
    fetch: fileFetch,
  });
  inline = core.createInlineEndpoint({
    bundle: files.bundle,
    files: files.models,
    ...runtimeBytes(),
    workers: 2,
    // the embedded worker as the classic script a browser starts from its blob: URL
    createWorker: () => new Worker(core.WORKER_SOURCE, { eval: true }),
  });
});

after(() => {
  fromFiles?.close();
  inline?.close();
});

test("the committed inline build is the one of the sources", () => {
  execFileSync(process.execPath, [path.join(ROOT, "js", "build.mjs"), "--check"], { stdio: "pipe" });
});

test("the inline build is one classic script of the version of the package", () => {
  const { version } = JSON.parse(readFileSync(path.join(ROOT, "js", "package.json"), "utf8"));
  assert.equal(core.CORE_VERSION, version);
  assert.equal(core.BUNDLE_CONTRACT, "panelbundle/1");
  const text = readFileSync(SCRIPT, "utf8");
  for (const syntax of ["import.meta", "\nexport ", "\nimport "]) assert.ok(!text.includes(syntax), syntax);
  assert.equal(typeof core.WORKER_SOURCE, "string");
  assert.ok(!core.WORKER_SOURCE.includes("import.meta"));
});

test("the inline build has the statistics (since 0.43.0)", async () => {
  const stats = core.createStats({ compiler: await core.createCompiler({ compiler: compilerWasm() }) });
  assert.equal(stats.sse([3, 4]), 25);
  assert.throws(() => stats.mse([]), { name: "StatsError", kind: "ValueError" });
  assert.equal(typeof core.StatsError, "function");
});

test("the inline endpoint answers the routes of the bundle held in memory", async () => {
  for (const panel of ["amount", "conc"]) {
    const response = await inline(`${BASE}/api/models/toy/panels/${panel}`);
    assert.equal(response.status, 200);
    assert.equal((await response.json()).panel.id, panel);
  }
});

test("a simulation of the inline endpoint has the bits of the endpoint of the files", async () => {
  for (const body of [{}, { values: { dose: 0.02 }, grid: { speed: 3 } }]) {
    for (const panel of ["amount", "conc"]) {
      const expected = await simulate(fromFiles, panel, body);
      const answered = await simulate(inline, panel, body);
      assert.equal(answered.status, 200);
      assert.deepEqual(answered.body.curves, expected.body.curves);
      assert.deepEqual(answered.body.x, expected.body.x);
      assert.equal(answered.body.provenance.spec_hash, expected.body.provenance.spec_hash);
    }
  }
});

test("a file the bundle does not have is a 404 of the inline endpoint, and nothing is fetched", async () => {
  const fetched = [];
  const original = globalThis.fetch;
  globalThis.fetch = (url) => {
    fetched.push(String(url));
    return original(url);
  };
  try {
    const endpoint = core.createInlineEndpoint({
      bundle: { ...files.bundle },
      files: {},
      ...runtimeBytes(),
      workers: 1,
      createWorker: () => new Worker(core.WORKER_SOURCE, { eval: true }),
    });
    const { status } = await simulate(endpoint, "amount", {});
    endpoint.close();
    assert.equal(status, 500);
  } finally {
    globalThis.fetch = original;
  }
  assert.deepEqual(fetched, []);
});
