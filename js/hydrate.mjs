// The page script of a static report: every `<figure data-figure="KEY">` holds the inline SVG of
// a figure and a child `<script type="application/json" data-figure-spec="KEY">` with its
// `figure/1` spec (`</` escaped as `<\/` in its strings, which JSON reads back). `hydrateFigures`
// attaches the interaction layer (`figure.mjs`) to each of them; a figure whose spec does not
// parse or render keeps its static SVG, and one `console.warn` names its key. An attached figure
// carries `data-hydrated`, so a second call skips it.
//
//   const handles = await hydrateFigures(document, { wasm: bytes });   // or `{ plot }`

import { attachFigure } from "./figure.mjs";
import { loadPlot } from "./plot.mjs";

/**
 * Attaches every figure of a report under `root`.
 *
 * @param {Document | Element} [root] where to look, the document by default
 * @param {object} [options]
 * @param {*} [options.wasm] the source of `plot.wasm` (see `loadPlot`)
 * @param {import("./plot.mjs").Plot} [options.plot] a renderer to reuse instead of `wasm`
 * @returns {Promise<import("./figure.mjs").FigureHandle[]>} the handles of the figures which attached
 */
export async function hydrateFigures(root = globalThis.document, options = {}) {
  const figures = [...root.querySelectorAll("figure[data-figure]:not([data-hydrated])")];
  const found = figures.map((figure) => ({
    figure,
    script: [...figure.children].find(
      (child) => child.localName === "script" && child.hasAttribute("data-figure-spec"),
    ),
  }));
  if (!found.some((item) => item.script)) return [];
  const plot = options.plot ?? (await loadPlot(options.wasm));
  const handles = [];
  for (const { figure, script } of found) {
    if (!script) continue;
    const key = figure.getAttribute("data-figure");
    try {
      handles.push(attachFigure(figure, plot, JSON.parse(script.textContent)));
      figure.setAttribute("data-hydrated", "");
    } catch (error) {
      console.warn(`figure ${key} stays static: ${error?.message ?? error}`);
    }
  }
  return handles;
}
