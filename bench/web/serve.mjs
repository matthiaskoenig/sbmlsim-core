#!/usr/bin/env node
// `node bench/web/serve.mjs <result.json> [--port 8765]`: serves the repository for the benchmark
// in Chrome (`bench/web/index.html`) with cross-origin isolation (COOP and COEP, so
// `performance.now()` in a worker has a fine resolution), and writes the result the page posts to
// `/result` into `<result.json>`, then exits.

import { createServer } from "node:http";
import { readFile, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const [out, ...rest] = process.argv.slice(2);
if (!out) {
  console.error("usage: node serve.mjs <result.json> [--port 8765]");
  process.exit(2);
}
const port = rest[0] === "--port" ? Number(rest[1]) : 8765;
const types = { ".html": "text/html", ".mjs": "text/javascript", ".js": "text/javascript", ".json": "application/json", ".wasm": "application/wasm" };

const server = createServer(async (req, res) => {
  const headers = {
    "Cross-Origin-Opener-Policy": "same-origin",
    "Cross-Origin-Embedder-Policy": "require-corp",
    "Cache-Control": "no-store",
  };
  const url = new URL(req.url, `http://localhost:${port}`);
  if (req.method === "POST" && url.pathname === "/result") {
    const chunks = [];
    for await (const chunk of req) chunks.push(chunk);
    await writeFile(out, Buffer.concat(chunks));
    res.writeHead(200, headers).end("ok");
    console.error(`wrote ${out}`);
    server.close();
    return;
  }
  if (req.method === "POST" && url.pathname === "/log") {
    const chunks = [];
    for await (const chunk of req) chunks.push(chunk);
    console.error(Buffer.concat(chunks).toString());
    res.writeHead(200, headers).end("ok");
    return;
  }
  const file = path.resolve(root, `.${decodeURIComponent(url.pathname)}`);
  if (!file.startsWith(root + path.sep)) {
    res.writeHead(403, headers).end();
    return;
  }
  try {
    const body = await readFile(file);
    res.writeHead(200, { ...headers, "Content-Type": types[path.extname(file)] ?? "application/octet-stream" });
    res.end(body);
  } catch {
    res.writeHead(404, headers).end();
  }
});
server.listen(port, "127.0.0.1", () => console.error(`http://127.0.0.1:${port}/bench/web/index.html`));
