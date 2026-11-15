// A worker of `module_shared_by_workers`: gets the compiled modules of engine, mathlib and a
// model, instantiates them and simulates the request; posts back the values.
import { parentPort, workerData } from "node:worker_threads";

import { createRuntime } from "../../loader.mjs";

const { engine, mathlib, model, description, request } = workerData;
const runtime = await createRuntime({ engine, mathlib });
const instance = await runtime.instantiate(model, description);
const { columns, nRows, values } = instance.simulate(request);
parentPort.postMessage({ columns, nRows, values, wasiCalls: instance.wasiCalls() });
