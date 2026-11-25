// The interaction layer of a figure in a DOM (linkedom): the legend toggles its groups, a click
// selects a group, the read-out shows the point under the pointer with its units, and an update
// or a dispose keeps or ends that state.

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import path from "node:path";
import { before, test } from "node:test";

import { parseHTML } from "linkedom";

import { attachFigure } from "../figure.mjs";
import { loadPlot } from "../plot.mjs";
import { ROOT, runtimeDir } from "./helpers.mjs";

const SPECS = path.join(ROOT, "crates", "plot", "tests", "specs");

/** Two curves of the group `a`, `b` right above the first of them and `c` far from both. */
const SPEC = {
  contract: "figure/1",
  width: 400,
  height: 300,
  grid: { rows: 1, columns: 1 },
  plots: [
    {
      id: "p",
      row: 0,
      column: 0,
      x: { label: "time", unit: "hr", min: 0, max: 10 },
      y: { label: "conc", unit: "mM", min: 0, max: 10 },
      elements: [
        { id: "a1", kind: "curve", label: "A", legend: "a", x: [0, 5, 10], y: [2, 2, 2], yerr: [0.5, 0.5, 0.5] },
        { id: "a2", kind: "curve", legend: "a", x: [0, 5, 10], y: [8, 8, 8] },
        { id: "b", kind: "curve", label: "B", x: [0, 5, 10], y: [2.1, 2.1, 2.1] },
        { id: "c", kind: "curve", label: "C", x: [0, 5, 10], y: [5, 5, 5] },
      ],
    },
  ],
};

let plot;
before(async () => {
  plot = await loadPlot(path.join(runtimeDir(), "plot.wasm"));
});

/** A document with a container, which holds a static figure and a caption. */
function page() {
  const { document, window } = parseHTML(
    '<!doctype html><html><head></head><body><div id="figure"><svg class="sbmlsim-figure"><text>static</text></svg><p>caption</p></div></body></html>',
  );
  return { document, window, container: document.getElementById("figure") };
}

/** Dispatches a bubbling event of `type` with the given properties on `target`. */
function fire(window, target, type, properties = {}) {
  const event = new window.Event(type, { bubbles: true, cancelable: true });
  for (const [key, value] of Object.entries(properties)) Object.defineProperty(event, key, { value });
  target.dispatchEvent(event);
  return event;
}

/** The pixel of the data point `(x, y)` in the panel of the test spec. */
function pixel(x, y) {
  const { rect, x: sx, y: sy } = plot.geometry(SPEC).plots[0];
  return {
    clientX: rect.x + ((x - sx.low) / (sx.high - sx.low)) * rect.width,
    clientY: rect.y + rect.height - ((y - sy.low) / (sy.high - sy.low)) * rect.height,
  };
}

const svgOf = (container) => container.querySelector("svg.sbmlsim-figure");
const entry = (container, group) => container.querySelector(`.sbmlsim-legend-entry[data-legend="${group}"]`);
const elements = (container, group) => [...container.querySelectorAll(`[data-element][data-legend="${group}"]`)];
const elementOf = (container, id) => container.querySelector(`[data-element="${id}"]`);
const tooltip = (container) => container.querySelector(".sbmlsim-tooltip");

test("attach replaces the static figure, keeps the rest and injects its stylesheet once", () => {
  const { document, container } = page();
  const figure = attachFigure(container, plot, SPEC);
  assert.equal(container.querySelectorAll("svg.sbmlsim-figure").length, 1);
  assert.equal(svgOf(container).querySelector("text").textContent === "static", false);
  assert.ok(container.querySelector("p"));
  const other = document.createElement("div");
  document.body.append(other);
  const second = attachFigure(other, plot, JSON.stringify(SPEC));
  assert.equal(document.querySelectorAll("#sbmlsim-figure-style").length, 1);
  assert.match(document.getElementById("sbmlsim-figure-style").textContent, /\.sbmlsim-hidden\{display:none\}/);
  assert.match(document.getElementById("sbmlsim-figure-style").textContent, /\.sbmlsim-dimmed\{opacity:\.25\}/);
  figure.dispose();
  second.dispose();
});

test("legend_click_hides_group", () => {
  const { window, container } = page();
  const figure = attachFigure(container, plot, SPEC);
  const a = entry(container, "a");
  assert.equal(a.getAttribute("aria-pressed"), "true");
  fire(window, a.querySelector("text"), "click");
  assert.deepEqual(figure.hidden(), ["a"]);
  assert.equal(elements(container, "a").length, 2);
  for (const element of elements(container, "a")) assert.ok(element.classList.contains("sbmlsim-hidden"));
  assert.ok(!elementOf(container, "b").classList.contains("sbmlsim-hidden"));
  assert.equal(a.getAttribute("aria-pressed"), "false");
  // the SVG keeps its static classes
  assert.ok(elementOf(container, "a1").classList.contains("sbmlsim-curve"));

  // the read-out skips the hidden group: `b` is the nearest visible point
  fire(window, svgOf(container), "pointermove", pixel(5, 2));
  assert.match(tooltip(container).textContent, /^B/);

  fire(window, a, "click");
  assert.deepEqual(figure.hidden(), []);
  for (const element of elements(container, "a")) assert.ok(!element.classList.contains("sbmlsim-hidden"));
  assert.equal(a.getAttribute("aria-pressed"), "true");
  fire(window, svgOf(container), "pointermove", pixel(5, 2));
  assert.match(tooltip(container).textContent, /^A/);
  figure.dispose();
});

test("keyboard_toggle", () => {
  const { window, container } = page();
  const figure = attachFigure(container, plot, SPEC);
  const b = entry(container, "b");
  assert.equal(b.getAttribute("tabindex"), "0");
  assert.equal(b.getAttribute("role"), "button");
  // the role `img` would make the buttons presentational
  assert.equal(svgOf(container).getAttribute("role"), "figure");
  const enter = fire(window, b, "keydown", { key: "Enter" });
  assert.ok(enter.defaultPrevented);
  assert.deepEqual(figure.hidden(), ["b"]);
  assert.equal(b.getAttribute("aria-pressed"), "false");
  fire(window, b, "keydown", { key: " " });
  assert.deepEqual(figure.hidden(), []);
  fire(window, b, "keydown", { key: "a" });
  assert.deepEqual(figure.hidden(), []);
  figure.dispose();
});

test("click_selects_and_dims", () => {
  const { window, container } = page();
  const figure = attachFigure(container, plot, SPEC);
  fire(window, svgOf(container), "click", pixel(5, 5));
  assert.equal(figure.selected(), "c");
  assert.ok(elementOf(container, "c").classList.contains("sbmlsim-selected"));
  for (const id of ["a1", "a2", "b"]) {
    assert.ok(elementOf(container, id).classList.contains("sbmlsim-dimmed"), id);
    assert.ok(!elementOf(container, id).classList.contains("sbmlsim-selected"), id);
  }
  // another group moves the selection
  fire(window, svgOf(container), "click", pixel(5, 8));
  assert.equal(figure.selected(), "a");
  assert.ok(elementOf(container, "a1").classList.contains("sbmlsim-selected"));
  assert.ok(elementOf(container, "a2").classList.contains("sbmlsim-selected"));
  assert.ok(elementOf(container, "c").classList.contains("sbmlsim-dimmed"));
  // the selected group again clears it
  fire(window, svgOf(container), "click", pixel(5, 8));
  assert.equal(figure.selected(), null);
  assert.equal(container.querySelectorAll(".sbmlsim-dimmed, .sbmlsim-selected").length, 0);
  // hiding the selected group clears the selection
  fire(window, svgOf(container), "click", pixel(5, 5));
  fire(window, entry(container, "c"), "click");
  assert.equal(figure.selected(), null);
  assert.equal(container.querySelectorAll(".sbmlsim-dimmed").length, 0);
  figure.dispose();
});

test("escape_clears_selection", () => {
  const { document, window, container } = page();
  const figure = attachFigure(container, plot, SPEC);
  fire(window, svgOf(container), "click", pixel(5, 5));
  assert.equal(figure.selected(), "c");
  fire(window, document.body, "keydown", { key: "Escape" });
  assert.equal(figure.selected(), null);
  assert.equal(container.querySelectorAll(".sbmlsim-dimmed, .sbmlsim-selected").length, 0);
  figure.dispose();
});

test("readout_shows_units", () => {
  const { window, container } = page();
  const figure = attachFigure(container, plot, SPEC);
  const svg = svgOf(container);
  // the SVG scaled to half its size by CSS: the pointer is converted to the pixels of the figure
  const { width, height } = plot.geometry(SPEC);
  svg.getBoundingClientRect = () => ({ left: 10, top: 20, width: width / 2, height: height / 2 });
  const at = (x, y) => {
    const { clientX, clientY } = pixel(x, y);
    return { clientX: 10 + clientX / 2, clientY: 20 + clientY / 2 };
  };
  fire(window, svg, "pointermove", at(5, 2));
  const tip = tooltip(container);
  assert.equal(tip.getAttribute("role"), "status");
  assert.equal(tip.getAttribute("aria-live"), "polite");
  assert.equal(tip.hidden, false);
  assert.equal(tip.textContent, "Atime: 5 hrconc: 2 ± 0.5 mM");
  assert.equal(tip.querySelector(".sbmlsim-tooltip-label").textContent, "A");
  // nothing near: the tooltip is hidden
  fire(window, svg, "pointermove", at(2, 3.5));
  assert.equal(tip.hidden, true);
  // the selected group is preferred over a nearer point of another group
  fire(window, svg, "click", at(5, 2.1));
  assert.equal(figure.selected(), "b");
  fire(window, svg, "pointermove", at(5, 2));
  assert.equal(tip.textContent, "Btime: 5 hrconc: 2.1 mM");
  fire(window, svg, "pointerleave");
  assert.equal(tip.hidden, true);
  figure.dispose();

  // a format of the caller
  const { window: w2, container: c2 } = page();
  const formatted = attachFigure(c2, plot, SPEC, { format: (value, unit) => `${value.toFixed(2)}[${unit ?? ""}]` });
  fire(w2, svgOf(c2), "pointermove", pixel(5, 5));
  assert.equal(tooltip(c2).textContent, "Ctime: 5.00[hr]conc: 5.00[mM]");
  formatted.dispose();
});

test("readout of bars, boxes and heatmap cells", () => {
  const kinds = JSON.parse(readFileSync(path.join(SPECS, "every_kind.json"), "utf8"));
  const { window, container } = page();
  const figure = attachFigure(container, plot, kinds);
  const geometry = plot.geometry(kinds);
  const seen = new Set();
  // sweep the panels: every kind with a read-out shows its label and at least one value line
  for (const { rect } of geometry.plots) {
    for (let i = 1; i < 12; i += 1) {
      for (let j = 1; j < 12; j += 1) {
        const clientX = rect.x + (rect.width * i) / 12;
        const clientY = rect.y + (rect.height * j) / 12;
        const hit = plot.pick(kinds, clientX, clientY, 8);
        fire(window, svgOf(container), "pointermove", { clientX, clientY });
        const tip = tooltip(container);
        if (!hit) {
          assert.equal(tip?.hidden ?? true, true);
          continue;
        }
        assert.equal(tip.hidden, false);
        assert.ok(tip.querySelectorAll(".sbmlsim-tooltip-row").length >= 1, JSON.stringify(hit));
        seen.add(kinds.plots.find((p) => p.id === hit.plot).elements.find((e) => e.id === hit.element).kind);
      }
    }
  }
  for (const kind of ["bar", "box", "heatmap"]) assert.ok(seen.has(kind), `${kind} in ${[...seen]}`);
  figure.dispose();
});

test("update_keeps_state", () => {
  const { window, container } = page();
  const figure = attachFigure(container, plot, SPEC);
  fire(window, entry(container, "a"), "click");
  fire(window, svgOf(container), "click", pixel(5, 5));
  assert.equal(figure.selected(), "c");
  const moved = structuredClone(SPEC);
  moved.plots[0].elements[3].y = [6, 6, 6];
  figure.update(moved);
  assert.equal(container.querySelectorAll("svg.sbmlsim-figure").length, 1);
  assert.deepEqual(figure.hidden(), ["a"]);
  assert.equal(figure.selected(), "c");
  for (const element of elements(container, "a")) assert.ok(element.classList.contains("sbmlsim-hidden"));
  assert.equal(entry(container, "a").getAttribute("aria-pressed"), "false");
  assert.equal(entry(container, "a").getAttribute("tabindex"), "0");
  assert.ok(elementOf(container, "c").classList.contains("sbmlsim-selected"));
  assert.ok(elementOf(container, "b").classList.contains("sbmlsim-dimmed"));
  // the listeners act on the new figure
  fire(window, entry(container, "b"), "click");
  assert.deepEqual(figure.hidden(), ["a", "b"]);
  // groups the new spec lacks are dropped
  const fewer = structuredClone(SPEC);
  fewer.plots[0].elements = fewer.plots[0].elements.filter((e) => e.id !== "c" && e.id !== "b");
  figure.update(fewer);
  assert.deepEqual(figure.hidden(), ["a"]);
  assert.equal(figure.selected(), null);
  figure.dispose();
});

test("dispose_removes_listeners", () => {
  const { document, window, container } = page();
  const figure = attachFigure(container, plot, SPEC);
  fire(window, svgOf(container), "pointermove", pixel(5, 5));
  fire(window, svgOf(container), "click", pixel(5, 5));
  figure.dispose();
  assert.equal(tooltip(container), null);
  assert.equal(container.querySelectorAll(".sbmlsim-dimmed, .sbmlsim-selected, .sbmlsim-hidden").length, 0);
  assert.equal(entry(container, "a").getAttribute("tabindex"), null);
  assert.equal(entry(container, "a").getAttribute("aria-pressed"), null);
  assert.equal(svgOf(container).getAttribute("role"), "img");
  fire(window, entry(container, "a"), "click");
  fire(window, svgOf(container), "click", pixel(5, 8));
  fire(window, svgOf(container), "pointermove", pixel(5, 5));
  fire(window, document.body, "keydown", { key: "Escape" });
  assert.equal(container.querySelectorAll(".sbmlsim-dimmed, .sbmlsim-selected, .sbmlsim-hidden").length, 0);
  assert.equal(tooltip(container), null);
  assert.deepEqual(figure.hidden(), []);
  assert.equal(figure.selected(), null);
  assert.throws(() => figure.update(SPEC), /disposed/);
  figure.dispose();
});

test("options switch the interactions off", () => {
  const { window, container } = page();
  const figure = attachFigure(container, plot, SPEC, { readout: false, select: false, toggle: false });
  assert.equal(entry(container, "a").getAttribute("tabindex"), null);
  fire(window, entry(container, "a"), "click");
  fire(window, svgOf(container), "click", pixel(5, 5));
  fire(window, svgOf(container), "pointermove", pixel(5, 5));
  assert.deepEqual(figure.hidden(), []);
  assert.equal(figure.selected(), null);
  assert.equal(tooltip(container), null);
  figure.dispose();
});
