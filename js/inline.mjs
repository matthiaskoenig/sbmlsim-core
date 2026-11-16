// The entry of the inline build of the JavaScript host (`node build.mjs`): one classic script, an
// IIFE with the global `sbmlsimCore`, for a page which embeds everything it runs, e.g. the app of
// a model which sbmlsim writes as one HTML file, which opens from `file://` as well. Such a page
// has no module to import, no URL of a worker script and nothing to fetch, so:
//
// - the workers of the pool run the worker of this package from a `blob:` URL of its source,
//   which the build embeds (`WORKER_SOURCE`, `worker.mjs` with `loader.mjs` as one classic
//   script), as classic workers, which a page of an opaque origin starts as well;
// - `createInlineEndpoint` serves the files of a bundle from memory: the bundle as an object and
//   its compiled models as bytes keyed by their path in the bundle, `models/<key>/model.wasm`
//   and `models/<key>/model.json`, and takes the runtime as bytes.
//
// The python package `sbmlsim-core` ships the build as `sbmlsim_core.web.script()` together with
// the runtime (`sbmlsim_core.runtime_wasm()`), so a page and the models compiled in python are of
// one release.

import { BUNDLE_CONTRACT, CORE_VERSION, ENGINE, createPanelEndpoint } from "./panels.mjs";

export { BUNDLE_CONTRACT, CORE_VERSION, ENGINE };

/** The source of the worker of the pool as one classic script, embedded by the build. */
// eslint-disable-next-line no-undef
export const WORKER_SOURCE = __SBMLSIM_CORE_WORKER__;

/** The origin of the files of an inline endpoint, which no request leaves the page for. */
const ORIGIN = "https://sbmlsim-core.invalid";

/** The URL of the bundle of an inline endpoint; its files are relative to it. */
const BUNDLE_URL = `${ORIGIN}/bundle/bundle.json`;

/**
 * A factory of the workers of a pool which starts every worker from one `blob:` URL of `source`,
 * as a classic Web Worker.
 */
export function blobWorkers(source = WORKER_SOURCE) {
  let url = null;
  return () => {
    if (typeof globalThis.Worker !== "function") throw new Error("this host has no Web Workers");
    url ??= URL.createObjectURL(new Blob([source], { type: "text/javascript" }));
    return new globalThis.Worker(url);
  };
}

/** A response of bytes or of a text. */
function respond(body, status = 200) {
  return new Response(body, { status });
}

/**
 * Creates the panel endpoint of a bundle held in memory, see `createPanelEndpoint` of
 * `panels.mjs`. Options:
 *
 * - `bundle`: the content of `bundle.json`, an object;
 * - `files`: the files of the bundle by their path in it (`models/<key>/model.wasm` as bytes,
 *   `models/<key>/model.json` as bytes or text);
 * - `engine`, `mathlib`: the bytes (or `WebAssembly.Module`s) of the runtime;
 * - `workers`: the most workers of the pool (`defaultWorkers()` of `pool.mjs` without);
 * - `createWorker`: creates a worker (`blobWorkers()` without).
 */
export function createInlineEndpoint({ bundle, files = {}, engine, mathlib, workers, createWorker = blobWorkers() }) {
  if (bundle === null || typeof bundle !== "object") throw new Error("createInlineEndpoint needs the bundle as an object");
  if (!engine || !mathlib) throw new Error("createInlineEndpoint needs the engine and the math library");
  const text = JSON.stringify(bundle);
  const base = new URL(BUNDLE_URL);
  const load = async (url) => {
    const target = new URL(String(url), base);
    if (target.href === base.href) return respond(text);
    const prefix = new URL("./", base).href;
    if (!target.href.startsWith(prefix)) return respond(`${target.href} is no file of the bundle`, 404);
    const file = files[decodeURIComponent(target.href.slice(prefix.length))];
    if (file === undefined) return respond(`${target.href} is no file of the bundle`, 404);
    return respond(file);
  };
  return createPanelEndpoint({
    bundle: BUNDLE_URL,
    runtime: { engine, mathlib },
    ...(workers === undefined ? {} : { workers }),
    createWorker,
    fetch: load,
  });
}
