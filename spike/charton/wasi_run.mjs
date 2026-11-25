// Runs a wasm32-wasip1 binary in Node's WASI with the given directory as "."
import { WASI } from 'node:wasi'
import fs from 'node:fs'
const [wasmPath, dir, reps] = process.argv.slice(2)
const wasi = new WASI({ version: 'preview1', args: ['charton-spike'], env: { REPS: reps ?? '200' }, preopens: { '.': dir } })
const module = await WebAssembly.compile(fs.readFileSync(wasmPath))
const instance = await WebAssembly.instantiate(module, wasi.getImportObject())
wasi.start(instance)
