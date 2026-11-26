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

/**
 * A document with a container, which holds a static figure and a caption, and the animation
 * frames and idle callbacks of its window, which a test runs when it wants.
 */
function page() {
  const { document, window } = parseHTML(
    '<!doctype html><html><head></head><body><div id="figure"><svg class="sbmlsim-figure"><text>static</text></svg><p>caption</p></div></body></html>',
  );
  const queue = () => {
    const callbacks = new Map();
    let next = 1;
    const request = (callback) => {
      callbacks.set(next, callback);
      return next++;
    };
    const cancel = (id) => callbacks.delete(id);
    const run = () => {
      while (callbacks.size > 0) {
        const [id, callback] = callbacks.entries().next().value;
        callbacks.delete(id);
        callback({ didTimeout: false, timeRemaining: () => 50 });
      }
    };
    return { callbacks, request, cancel, run };
  };
  const frames = queue();
  const idle = queue();
  window.requestAnimationFrame = frames.request;
  window.cancelAnimationFrame = frames.cancel;
  window.requestIdleCallback = idle.request;
  window.cancelIdleCallback = idle.cancel;
  window.runFrames = frames.run;
  return { document, window, container: document.getElementById("figure"), frames, idle };
}

/** Dispatches a bubbling event of `type` with the given properties on `target`. */
function dispatch(window, target, type, properties = {}) {
  const event = new window.Event(type, { bubbles: true, cancelable: true });
  for (const [key, value] of Object.entries(properties)) Object.defineProperty(event, key, { value });
  target.dispatchEvent(event);
  return event;
}

/** `dispatch`, and for a pointer move the animation frame of its read-out. */
function fire(window, target, type, properties = {}) {
  const event = dispatch(window, target, type, properties);
  if (type === "pointermove") window.runFrames();
  return event;
}

/** A renderer which counts the figures it prepares and the picks of their figures. */
function counting(inner) {
  const counts = { prepare: 0, pick: 0 };
  const proxy = {
    prepare(spec) {
      counts.prepare += 1;
      const figure = inner.prepare(spec);
      const pick = figure.pick.bind(figure);
      figure.pick = (...args) => {
        counts.pick += 1;
        return pick(...args);
      };
      return figure;
    },
  };
  return { plot: proxy, counts };
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
  // a hover is not read aloud
  assert.equal(tip.getAttribute("aria-live"), "off");
  assert.equal(tip.hidden, false);
  assert.equal(tip.textContent, "Atime: 5 hrconc: 2 ± 0.5 mM");
  assert.equal(tip.querySelector(".sbmlsim-tooltip-label").textContent, "A");
  // nothing near: the tooltip is hidden
  fire(window, svg, "pointermove", at(2, 3.5));
  assert.equal(tip.hidden, true);
  // the selected group is preferred over a nearer point of another group
  fire(window, svg, "click", at(5, 2.1));
  assert.equal(figure.selected(), "b");
  // a click announces its read-out
  assert.equal(tip.getAttribute("aria-live"), "polite");
  assert.match(tip.textContent, /^B/);
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
  const prepared = plot.prepare(kinds);
  const geometry = prepared.geometry();
  const seen = new Set();
  // sweep the panels: every kind with a read-out shows its label and at least one value line
  for (const { rect } of geometry.plots) {
    for (let i = 1; i < 12; i += 1) {
      for (let j = 1; j < 12; j += 1) {
        const clientX = rect.x + (rect.width * i) / 12;
        const clientY = rect.y + (rect.height * j) / 12;
        const hit = prepared.pick(clientX, clientY, 8);
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
  prepared.dispose();
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

/** The client pixel of the data point `(x, y)` of the plot `id` of a spec, on any scale. */
function pixelOf(spec, id, x, y) {
  const { rect, x: sx, y: sy } = plot.geometry(spec).plots.find((p) => p.id === id);
  const share = (s, v) => {
    const t = (u) => (s.kind === "log" ? Math.log10(u) : u);
    const f = (t(v) - t(s.low)) / (t(s.high) - t(s.low));
    return s.reverse ? 1 - f : f;
  };
  return { clientX: rect.x + share(sx, x) * rect.width, clientY: rect.y + rect.height - share(sy, y) * rect.height };
}

test("pointer_moves_are_throttled", () => {
  const { window, container, frames } = page();
  const { plot: counted, counts } = counting(plot);
  const figure = attachFigure(container, counted, SPEC);
  const svg = svgOf(container);
  const before = counts.pick;
  // five moves in one frame: one pick, at the last of them
  for (const y of [2, 4.9, 2.1, 8, 5]) dispatch(window, svg, "pointermove", pixel(5, y));
  assert.equal(counts.pick, before);
  assert.equal(frames.callbacks.size, 1);
  frames.run();
  assert.equal(counts.pick, before + 1);
  assert.match(tooltip(container).textContent, /^C/);
  // the pointer leaves before the frame: it is cancelled and nothing is read out
  dispatch(window, svg, "pointermove", pixel(5, 2));
  dispatch(window, container, "pointerleave");
  assert.equal(frames.callbacks.size, 0);
  frames.run();
  assert.equal(counts.pick, before + 1);
  assert.equal(tooltip(container).hidden, true);
  // dispose cancels a pending frame
  dispatch(window, svg, "pointermove", pixel(5, 2));
  assert.equal(frames.callbacks.size, 1);
  figure.dispose();
  assert.equal(frames.callbacks.size, 0);
});

test("click_between_points_selects", () => {
  const { window, container } = page();
  const figure = attachFigure(container, plot, SPEC);
  // on the line of `c` half way between two of its points, which are far more than 8 px apart
  const { clientX: x0 } = pixel(0, 5);
  const { clientX: x1 } = pixel(5, 5);
  assert.ok(x1 - x0 > 100, `${x1 - x0}`);
  fire(window, svgOf(container), "click", pixel(2.5, 5));
  assert.equal(figure.selected(), "c");
  fire(window, svgOf(container), "click", pixel(2.5, 5));
  assert.equal(figure.selected(), null);
  // a pick which misses falls back to the element under the pointer
  fire(window, elementOf(container, "a2").querySelector("path"), "click", { clientX: 0, clientY: 0 });
  assert.equal(figure.selected(), "a");
  // and neither: nothing changes
  fire(window, svgOf(container), "click", { clientX: 0, clientY: 0 });
  assert.equal(figure.selected(), "a");
  figure.dispose();
});

test("escape_clears_only_the_engaged_figure", () => {
  const { document, window, container } = page();
  const other = document.createElement("div");
  document.body.append(other);
  const first = attachFigure(container, plot, SPEC);
  const second = attachFigure(other, plot, SPEC);
  fire(window, svgOf(container), "click", pixel(5, 5));
  dispatch(window, container, "pointerleave");
  fire(window, svgOf(other), "click", pixel(5, 8));
  assert.deepEqual([first.selected(), second.selected()], ["c", "a"]);
  // the pointer was last in the second figure
  fire(window, document.body, "keydown", { key: "Escape" });
  assert.deepEqual([first.selected(), second.selected()], ["c", null]);
  // the focus is in the first figure
  fire(window, entry(container, "b"), "keydown", { key: "Escape" });
  assert.equal(first.selected(), null);
  first.dispose();
  second.dispose();
});

test("filtered_figures_prepare_in_idle_time", () => {
  const { window, container, idle } = page();
  const { plot: counted, counts } = counting(plot);
  const figure = attachFigure(container, counted, SPEC);
  assert.equal(counts.prepare, 1);
  assert.equal(idle.callbacks.size, 0);
  fire(window, entry(container, "a"), "click");
  assert.equal(counts.prepare, 1);
  assert.equal(idle.callbacks.size, 1);
  idle.run();
  assert.equal(counts.prepare, 2);
  // the first move after the toggle prepares nothing
  fire(window, svgOf(container), "pointermove", pixel(5, 2));
  assert.equal(counts.prepare, 2);
  assert.match(tooltip(container).textContent, /^B/);
  // a change of the state cancels the work of the earlier one
  fire(window, entry(container, "b"), "click");
  fire(window, entry(container, "c"), "click");
  assert.equal(idle.callbacks.size, 1);
  figure.dispose();
  assert.equal(idle.callbacks.size, 0);
});

test("category_axes_are_pinned_in_the_filtered_figure", () => {
  // explicit categories: the bars of `b1` hidden, the read-out of `b2` just above its top,
  // where the hidden bar of `b1` is nearer
  const bars = {
    contract: "figure/1",
    width: 400,
    height: 300,
    grid: { rows: 1, columns: 1 },
    plots: [
      {
        id: "p",
        row: 0,
        column: 0,
        x: { label: "arm", scale: "category", categories: ["x0", "x1", "x2"] },
        y: { label: "v", unit: "mM" },
        elements: [
          { id: "b1", kind: "bar", label: "B1", x: [0, 1, 2], height: [1, 2, 3] },
          { id: "b2", kind: "bar", label: "B2", x: [0, 1, 2], height: [0.5, 1, 1.5] },
        ],
      },
    ],
  };
  const { window, container, idle } = page();
  const { plot: counted, counts } = counting(plot);
  const figure = attachFigure(container, counted, bars);
  fire(window, entry(container, "b1"), "click");
  const prepared = counts.prepare;
  fire(window, svgOf(container), "pointermove", pixelOf(bars, "p", 1, 1.03));
  assert.equal(tooltip(container).textContent, "B2arm: x1v: 1 mM");
  assert.equal(counts.prepare, prepared + 1);
  idle.run();
  figure.dispose();

  // implicit categories of a heatmap: the heatmap hidden, a curve over it read out
  const heat = {
    contract: "figure/1",
    width: 400,
    height: 300,
    grid: { rows: 1, columns: 1 },
    plots: [
      {
        id: "h",
        row: 0,
        column: 0,
        x: {},
        y: {},
        elements: [
          {
            id: "map",
            kind: "heatmap",
            label: "H",
            rows: ["r0", "r1"],
            columns: ["c0", "c1", "c2"],
            values: [
              [1, 2, 3],
              [4, 5, 6],
            ],
            scale: { name: "viridis", low: 0, high: 6 },
          },
          { id: "k", kind: "curve", label: "K", x: [0, 1, 2], y: [0, 1, 0] },
        ],
      },
    ],
  };
  const second = page();
  const shown = attachFigure(second.container, plot, heat);
  const at = { ...pixelOf(heat, "h", 1, 1) };
  at.clientY += 3;
  fire(second.window, svgOf(second.container), "pointermove", at);
  // a heatmap cell is read out by its row and column
  assert.match(tooltip(second.container).textContent, /^r1 \/ c1|^K/);
  fire(second.window, entry(second.container, "map"), "click");
  fire(second.window, svgOf(second.container), "pointermove", at);
  assert.match(tooltip(second.container).textContent, /^K/);
  shown.dispose();
});

test("readout of a heatmap over the edges of its cells", () => {
  // a space-time heatmap: its cells over a numeric time axis, the cell read out by its spans
  const spec = {
    contract: "figure/1",
    width: 400,
    height: 300,
    grid: { rows: 1, columns: 1 },
    plots: [
      {
        id: "h",
        row: 0,
        column: 0,
        x: { label: "time", unit: "h" },
        y: { label: "position" },
        colorbar: { scale: { name: "purd", low: 0, high: 6 }, label: "signal" },
        elements: [
          {
            id: "map",
            kind: "heatmap",
            label: "cells",
            x_edges: [0, 1, 3, 10],
            y_edges: [0, 0.5, 1],
            values: [
              [1, 2, 3],
              [4, 5, 6],
            ],
            scale: { name: "purd", low: 0, high: 6 },
          },
        ],
      },
    ],
  };
  const { window, container } = page();
  const figure = attachFigure(container, plot, spec);
  fire(window, svgOf(container), "pointermove", pixelOf(spec, "h", 5, 0.75));
  assert.equal(tooltip(container).textContent, "cellstime: 3 to 10 hposition: 0.5 to 1signal: 6");
  // the axis of the edges is no category axis of the filtered figure of a selection
  fire(window, svgOf(container), "click", pixelOf(spec, "h", 5, 0.75));
  fire(window, svgOf(container), "pointermove", pixelOf(spec, "h", 0.5, 0.25));
  assert.equal(tooltip(container).textContent, "cellstime: 0 to 1 hposition: 0 to 0.5signal: 1");
  figure.dispose();
});

test("a_hit_of_a_panel_moved_by_a_legend_is_kept", () => {
  // legends outside the plots move the right panel; the filtered figure has no legends, so its
  // panels lie elsewhere, and a hit is searched in the panel under the pointer
  const marker = { marker: { type: "circle" }, line: { type: "none" } };
  const moved = {
    contract: "figure/1",
    width: 500,
    height: 300,
    grid: { rows: 1, columns: 2 },
    legend: { position: "outside" },
    plots: [
      {
        id: "left",
        row: 0,
        column: 0,
        x: { min: 0, max: 10 },
        y: { min: 0, max: 10 },
        elements: [
          { id: "a", kind: "curve", label: "A long label", x: [9], y: [5], style: marker },
          { id: "z", kind: "curve", label: "Z", x: [9], y: [5.1], style: marker },
        ],
      },
      {
        id: "right",
        row: 0,
        column: 1,
        x: { min: 0, max: 10 },
        y: { min: 0, max: 10 },
        elements: [{ id: "b", kind: "curve", label: "B", x: [1], y: [5], style: marker }],
      },
    ],
  };
  const { window, container } = page();
  const figure = attachFigure(container, plot, moved);
  fire(window, entry(container, "z"), "click");
  // the hidden `z` is nearer than `a` at its own point
  fire(window, svgOf(container), "pointermove", pixelOf(moved, "left", 9, 5.1));
  assert.match(tooltip(container).textContent, /^A long label/);
  fire(window, svgOf(container), "pointermove", pixelOf(moved, "right", 1, 5));
  assert.match(tooltip(container).textContent, /^B/);
  fire(window, svgOf(container), "pointermove", pixelOf(moved, "right", 5, 5));
  assert.equal(tooltip(container).hidden, true);
  figure.dispose();
});

test("shift_enter_selects_a_group", () => {
  const { window, container } = page();
  const figure = attachFigure(container, plot, SPEC);
  const shiftEnter = { key: "Enter", shiftKey: true };
  const event = fire(window, entry(container, "c"), "keydown", shiftEnter);
  assert.ok(event.defaultPrevented);
  assert.equal(figure.selected(), "c");
  assert.deepEqual(figure.hidden(), []);
  fire(window, entry(container, "c"), "keydown", shiftEnter);
  assert.equal(figure.selected(), null);
  // a hidden group is shown when it is selected
  fire(window, entry(container, "b"), "click");
  fire(window, entry(container, "b"), "keydown", shiftEnter);
  assert.equal(figure.selected(), "b");
  assert.deepEqual(figure.hidden(), []);
  assert.ok(elementOf(container, "b").classList.contains("sbmlsim-selected"));
  figure.dispose();
});
