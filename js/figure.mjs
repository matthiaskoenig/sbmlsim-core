// The interaction layer of a figure of sbmlsim in a page: a `figure/1` spec rendered by a `Plot`
// into a container, whose legend toggles the elements of its groups, whose curves select their
// group by a click, and which reads out the data point under the pointer.
//
//   import { loadPlot } from "sbmlsim-core/plot";
//   import { attachFigure } from "sbmlsim-core/figure";
//   const plot = await loadPlot(source);
//   const figure = attachFigure(container, plot, spec);   // replaces a static SVG already there
//   figure.hidden(); figure.selected(); figure.update(next); figure.dispose();
//
// It uses the standard DOM only, no framework, and works from `file://`. The SVG keeps its static
// classes and attributes (`docs/plot.md`, the semantic classes): the state is the classes
// `sbmlsim-hidden`, `sbmlsim-selected` and `sbmlsim-dimmed` on the `[data-element]` groups, which
// a stylesheet injected once per document (`#sbmlsim-figure-style`) styles, so it works on the
// inline SVG of a static report as well.
//
// - A legend entry (`.sbmlsim-legend-entry[data-legend]`) is a toggle button: a click, Enter or
//   Space hides or shows every element of its group (`aria-pressed` is whether they are shown),
//   Shift+Enter selects the group.
// - A click on a data point (or on the line of a curve between its points) selects its legend
//   group, which is drawn as it is while every other element is dimmed; a click on it again or
//   Escape (with the focus or the pointer last in the figure) clears the selection.
// - The read-out is a tooltip (`role="status"`) in the container with the point the pointer is
//   near: its label, its values with the units of the axes. A hidden group is never read out
//   and the selected group is preferred over a nearer point of another one.
//
// The renderer picks the nearest point of every element (`PreparedFigure.pick`); the filters of
// the read-out pick, per plot, on a figure of the elements they keep in that plot, whose axes are
// pinned to the ends and categories the whole figure has, so its data lies at the pixels of the
// drawn figure. Such figures are prepared in idle time after a toggle or a selection. Where one
// is laid out otherwise than the drawn plot (which `geometry` tells), the nearest point of the
// whole figure is taken and dropped when the filter does not keep it. The read-out runs once per
// animation frame, on the last pointer move; it is a live region only when a click shows it.

/** The id of the stylesheet the module injects, once per document. */
export const STYLE_ID = "sbmlsim-figure-style";

/** The classes of the state of a figure. */
const HIDDEN = "sbmlsim-hidden";
const SELECTED = "sbmlsim-selected";
const DIMMED = "sbmlsim-dimmed";
const TOOLTIP = "sbmlsim-tooltip";

/** The stylesheet of the state, the legend buttons and the read-out. */
const STYLE = [
  `.${HIDDEN}{display:none}`,
  `.${DIMMED}{opacity:.25}`,
  ".sbmlsim-legend-entry[role=button]{cursor:pointer}",
  '.sbmlsim-legend-entry[aria-pressed="false"]{opacity:.4}',
  ".sbmlsim-legend-entry[role=button]:focus{outline:none}",
  ".sbmlsim-legend-entry[role=button]:focus-visible{outline:2px solid #0969da;outline-offset:1px}",
  `.${TOOLTIP}{position:absolute;z-index:10;pointer-events:none;max-width:24em;padding:4px 8px;` +
    "background:#ffffff;color:#1b1f24;border:1px solid #d0d7de;border-radius:4px;" +
    "box-shadow:0 1px 4px rgba(27,31,36,.15);" +
    "font:12px/1.4 Inter,-apple-system,BlinkMacSystemFont,'Segoe UI',Roboto,Helvetica,Arial,sans-serif;" +
    "font-variant-numeric:tabular-nums;white-space:nowrap}",
  `.${TOOLTIP}[hidden]{display:none}`,
  `.${TOOLTIP}-label{font-weight:600}`,
  `.${TOOLTIP}-name{color:#57606a}`,
].join("\n");

/** The pixels between the pointer and the tooltip. */
const OFFSET = 12;

/** The reach of the read-out and of a click in pixels of the figure. */
const RADIUS = 8;

/**
 * A number with up to 4 significant digits and its unit.
 *
 * @param {number} value the number
 * @param {string} [unit] the unit, appended after a space
 * @returns {string} the text
 */
export function formatValue(value, unit) {
  let text;
  if (!Number.isFinite(value) || value === 0) {
    text = String(value);
  } else {
    const magnitude = Math.abs(value);
    text =
      magnitude >= 1e-3 && magnitude < 1e5
        ? String(Number(value.toPrecision(4)))
        : value.toExponential(3).replace(/\.?0+e/, "e");
  }
  return unit ? `${text} ${unit}` : text;
}

/** The legend group of an element of a spec, `null` for one in no legend (`docs/plot.md`). */
function groupOf(element) {
  if (typeof element.legend === "string") return element.legend;
  return typeof element.label === "string" ? element.id : null;
}

/** The text of an axis label on one line. */
function oneLine(text) {
  return String(text).replace(/\s*\n\s*/g, " ");
}

/** Injects the stylesheet into `document` unless it has it. */
function injectStyle(document) {
  if (document.getElementById(STYLE_ID)) return;
  const style = document.createElement("style");
  style.id = STYLE_ID;
  style.textContent = STYLE;
  (document.head ?? document.documentElement).append(style);
}

/** The first child of `container` which is an SVG, or `null`. */
function svgChild(container) {
  for (const child of container.children) {
    if (child.localName?.toLowerCase() === "svg" || child.tagName?.toLowerCase() === "svg") return child;
  }
  return null;
}

/** The SVG element of the text of an SVG, parsed in `document`. */
function parseSvg(document, text) {
  const holder = document.createElement("div");
  holder.innerHTML = text;
  const svg = holder.querySelector("svg");
  if (!svg) throw new Error("the renderer gave no SVG");
  return svg;
}

/** Whether the scales `a` and `b` of an axis are the same, `null` and `null` too. */
function sameScale(a, b) {
  if (!a || !b) return !a && !b;
  const close = (u, v) => Math.abs(u - v) <= 1e-9 * Math.max(1, Math.abs(u), Math.abs(v));
  return a.kind === b.kind && a.reverse === b.reverse && close(a.low, b.low) && close(a.high, b.high);
}

/**
 * The categories of an axis the drawn figure has as categories without the spec naming them,
 * i.e. the columns (x) or the rows (y) of the first heatmap of the plot, or `null`.
 */
function implicitCategories(plot, name) {
  const heatmap = plot.elements.find((element) => element.kind === "heatmap");
  if (!heatmap) return null;
  return name === "x" ? heatmap.columns : name === "y" ? heatmap.rows : null;
}

/**
 * The spec of the elements `keep` keeps in the plot `only` (every other plot without elements,
 * so a pick can only hit `only`): no legends (which may move a panel but never change its size)
 * and the axes pinned to what the drawn figure has (`geometry`), the ends of a linear or log
 * axis and the categories of a category axis, so the kept data keeps its pixels within the panel.
 */
function filteredSpec(spec, geometry, keep, only) {
  const scales = new Map(geometry.plots.map((plot) => [plot.id, plot]));
  return {
    ...spec,
    plots: spec.plots.map((plot) => {
      const drawn = scales.get(plot.id);
      const elements = plot.id === only ? plot.elements.filter(keep) : [];
      const out = { ...plot, legend: false, elements };
      delete out.legend_swatches;
      delete out.legend_order;
      delete out.declared_legends;
      for (const name of ["x", "y", "y2"]) {
        const scale = drawn?.[name];
        if (!plot[name] || !scale) continue;
        if (scale.kind === "category") {
          if (plot[name].scale === "category") continue;
          const categories = implicitCategories(plot, name);
          if (categories) out[name] = { ...plot[name], scale: "category", categories, reverse: scale.reverse };
          continue;
        }
        const axis = { ...plot[name], min: scale.low, max: scale.high };
        delete axis.frame;
        out[name] = axis;
      }
      return out;
    }),
  };
}

/** Whether the plot `id` has in `filtered` the panel size and the scales it has in `drawn`. */
function sameLayout(drawn, filtered, id) {
  const plot = drawn.plots.find((candidate) => candidate.id === id);
  const other = filtered.plots.find((candidate) => candidate.id === id);
  return (
    plot !== undefined &&
    other !== undefined &&
    Math.abs(other.rect.width - plot.rect.width) < 1e-6 &&
    Math.abs(other.rect.height - plot.rect.height) < 1e-6 &&
    sameScale(plot.x, other.x) &&
    sameScale(plot.y, other.y) &&
    sameScale(plot.y2, other.y2)
  );
}

/** The plots of `geometry` whose panel, grown by `radius`, holds `(x, y)`, those holding it first. */
function plotsAt(geometry, x, y, radius) {
  const inside = (rect, grow) =>
    x >= rect.x - grow && x <= rect.x + rect.width + grow && y >= rect.y - grow && y <= rect.y + rect.height + grow;
  const holding = geometry.plots.filter((plot) => inside(plot.rect, 0));
  const near = geometry.plots.filter((plot) => !inside(plot.rect, 0) && inside(plot.rect, radius));
  return [...holding, ...near];
}

/**
 * The lines of the read-out of a hit: `[name, value]` pairs with the units of the axes.
 *
 * @param {object} hit a pick
 * @param {object} plot the plot of the spec it is in
 * @param {object} element the element of the spec it is of
 * @param {(value: number, unit?: string) => string} format the format of a value
 * @returns {Array<[string, string]>} the lines
 */
function readoutLines(hit, plot, element, format) {
  const values = hit.values ?? {};
  const xAxis = plot.x ?? {};
  const yAxis = (element.axis === "y2" ? plot.y2 : plot.y) ?? {};
  const name = (axis, fallback) => (axis.label ? oneLine(axis.label) : fallback);
  const at = (axis, value) => {
    if (axis.scale === "category" && Number.isInteger(value) && axis.categories?.[value] !== undefined) {
      return axis.categories[value];
    }
    return format(value, axis.unit);
  };
  const plusMinus = (axis, value, error) =>
    error === undefined || error === null ? at(axis, value) : `${format(value)} ± ${format(error, axis.unit)}`;
  const has = (key) => typeof values[key] === "number";
  const lines = [];
  switch (element.kind) {
    case "curve":
      lines.push([name(xAxis, "x"), plusMinus(xAxis, values.x, values.xerr)]);
      lines.push([name(yAxis, "y"), plusMinus(yAxis, values.y, values.yerr)]);
      break;
    case "text":
      lines.push([name(xAxis, "x"), at(xAxis, values.x)], [name(yAxis, "y"), at(yAxis, values.y)]);
      break;
    case "band":
      lines.push([name(xAxis, "x"), at(xAxis, values.x)]);
      if (has("median")) lines.push(["median", format(values.median, yAxis.unit)]);
      lines.push(["low", format(values.low, yAxis.unit)], ["high", format(values.high, yAxis.unit)]);
      break;
    case "area":
      lines.push([name(xAxis, "x"), at(xAxis, values.x)]);
      lines.push(["from", format(values.from, yAxis.unit)], ["to", format(values.to, yAxis.unit)]);
      break;
    case "bar": {
      const [position, length] = element.horizontal ? [yAxis, xAxis] : [xAxis, yAxis];
      lines.push([name(position, "position"), at(position, values.x)]);
      lines.push([name(length, "value"), plusMinus(length, values.height, values.err)]);
      break;
    }
    case "box":
      lines.push([name(xAxis, "x"), at(xAxis, values.x)]);
      for (const [key, label] of [
        ["high_whisker", "high whisker"],
        ["q3", "q3"],
        ["median", "median"],
        ["q1", "q1"],
        ["low_whisker", "low whisker"],
      ]) {
        if (has(key)) lines.push([label, format(values[key], yAxis.unit)]);
      }
      break;
    case "violin":
      lines.push([name(xAxis, "x"), at(xAxis, values.x)]);
      break;
    case "histogram":
      lines.push([name(xAxis, "bin"), `${format(values.low)} to ${format(values.high, xAxis.unit)}`]);
      lines.push(["count", format(values.count)]);
      break;
    case "heatmap":
      lines.push([plot.colorbar?.label ? oneLine(plot.colorbar.label) : "value", format(values.value)]);
      break;
    case "rule": {
      const axis = element.orientation === "vertical" ? xAxis : yAxis;
      lines.push([name(axis, "value"), format(values.value, axis.unit)]);
      break;
    }
    default:
      for (const [key, value] of Object.entries(values)) lines.push([key, format(value)]);
  }
  return lines;
}

/**
 * The handle of a figure attached to a container by `attachFigure`.
 */
export class FigureHandle {
  #container;
  #plot;
  #options;
  #document;
  #spec = null;
  #elements = new Map();
  #groups = new Set();
  #figure = null;
  #geometry = null;
  #svg = null;
  #pickers = new Map();
  #hidden = new Set();
  #selected = null;
  #tooltip = null;
  #shown = null;
  #listeners = [];
  #position = null;
  #role = null;
  #pointerInside = false;
  #pending = null;
  #frame = null;
  #idle = null;
  #disposed = false;

  /**
   * @param {Element} container the element the figure is drawn into
   * @param {import("./plot.mjs").Plot} plot the renderer
   * @param {object|string} spec a `figure/1` spec
   * @param {object} options see `attachFigure`
   */
  constructor(container, plot, spec, options) {
    this.#container = container;
    this.#plot = plot;
    this.#options = {
      readout: options.readout ?? true,
      select: options.select ?? true,
      toggle: options.toggle ?? true,
      format: options.format ?? formatValue,
    };
    this.#document = container.ownerDocument;
    injectStyle(this.#document);
    this.#render(spec);
    this.#listen();
  }

  /**
   * Draws a new spec into the container, keeping the hidden and the selected groups by their
   * legend id (a group the new spec lacks is dropped).
   *
   * @param {object|string} spec a `figure/1` spec
   * @throws {PlotError} for a spec which is refused, which leaves the figure as it was
   */
  update(spec) {
    if (this.#disposed) throw new Error("the figure is disposed");
    this.#render(spec);
  }

  /**
   * The hidden legend groups.
   *
   * @returns {string[]} their ids, sorted
   */
  hidden() {
    return [...this.#hidden].sort();
  }

  /**
   * The selected legend group.
   *
   * @returns {string|null} its id, or `null` without a selection
   */
  selected() {
    return this.#selected;
  }

  /**
   * Removes the listeners and the tooltip, cancels a pending read-out, frees the prepared
   * figures and leaves the static figure: every element shown, nothing selected and the legend
   * entries no buttons. Disposing again does nothing.
   */
  dispose() {
    if (this.#disposed) return;
    this.#disposed = true;
    for (const [target, type, listener] of this.#listeners) target.removeEventListener(type, listener);
    this.#listeners = [];
    this.#cancelFrame();
    this.#cancelIdle();
    this.#tooltip?.remove();
    this.#tooltip = null;
    this.#hidden.clear();
    this.#selected = null;
    this.#apply();
    for (const entry of this.#entries()) {
      for (const name of ["tabindex", "role", "aria-pressed"]) entry.removeAttribute(name);
    }
    if (this.#options.toggle && this.#svg) {
      if (this.#role === null) this.#svg.removeAttribute("role");
      else this.#svg.setAttribute("role", this.#role);
    }
    this.#releasePickers();
    this.#figure?.dispose();
    this.#figure = null;
    if (this.#position !== null) this.#container.style.position = this.#position;
  }

  /** Prepares and draws `spec`, keeping the state of the groups it still has. */
  #render(spec) {
    const parsed = typeof spec === "string" ? JSON.parse(spec) : spec;
    const figure = this.#plot.prepare(parsed);
    let svg;
    let geometry;
    try {
      geometry = figure.geometry();
      svg = parseSvg(this.#document, figure.svg());
    } catch (error) {
      figure.dispose();
      throw error;
    }
    this.#cancelFrame();
    this.#releasePickers();
    this.#figure?.dispose();
    this.#figure = figure;
    this.#geometry = geometry;
    this.#spec = parsed;
    this.#elements = new Map();
    this.#groups = new Set();
    for (const plot of parsed.plots ?? []) {
      for (const element of plot.elements ?? []) {
        this.#elements.set(element.id, { plot, element });
        const group = groupOf(element);
        if (group !== null) this.#groups.add(group);
      }
    }
    for (const group of [...this.#hidden]) if (!this.#groups.has(group)) this.#hidden.delete(group);
    if (this.#selected !== null && (!this.#groups.has(this.#selected) || this.#hidden.has(this.#selected))) {
      this.#selected = null;
    }
    const old = this.#svg ?? svgChild(this.#container);
    if (old && old.parentNode === this.#container) this.#container.replaceChild(svg, old);
    else this.#container.prepend(svg);
    this.#svg = svg;
    if (this.#options.toggle) {
      // the role `img` makes the children of the SVG presentational, which would hide the
      // buttons of the legend from assistive technology: an interactive figure is a `figure`
      this.#role = svg.getAttribute("role");
      svg.setAttribute("role", "figure");
      for (const entry of this.#entries()) {
        entry.setAttribute("tabindex", "0");
        entry.setAttribute("role", "button");
      }
    }
    this.#apply();
    this.#hideTooltip();
    this.#prefetch();
  }

  /** The legend entries of the drawn figure. */
  #entries() {
    return this.#svg ? [...this.#svg.querySelectorAll(".sbmlsim-legend-entry[data-legend]")] : [];
  }

  /** Sets the classes of the elements and the pressed state of the entries from the state. */
  #apply() {
    if (!this.#svg) return;
    for (const node of this.#svg.querySelectorAll("[data-element]")) {
      const group = node.getAttribute("data-legend");
      const hidden = group !== null && this.#hidden.has(group);
      const selected = this.#selected !== null && group === this.#selected;
      node.classList.toggle(HIDDEN, hidden);
      node.classList.toggle(SELECTED, selected);
      node.classList.toggle(DIMMED, this.#selected !== null && !selected);
    }
    if (!this.#options.toggle || this.#disposed) return;
    for (const entry of this.#entries()) {
      entry.setAttribute("aria-pressed", String(!this.#hidden.has(entry.getAttribute("data-legend"))));
    }
  }

  /** Registers a listener which `dispose` removes. */
  #on(target, type, listener) {
    target.addEventListener(type, listener);
    this.#listeners.push([target, type, listener]);
  }

  /** Wires the events of the container and the Escape of the document. */
  #listen() {
    const { toggle, select, readout } = this.#options;
    if (toggle || select) this.#on(this.#container, "click", (event) => this.#click(event));
    if (toggle) this.#on(this.#container, "keydown", (event) => this.#key(event));
    if (select) this.#on(this.#document, "keydown", (event) => this.#escape(event));
    if (readout || select) {
      this.#on(this.#container, "pointermove", (event) => this.#move(event));
      this.#on(this.#container, "pointerleave", () => this.#leave());
    }
  }

  /** The legend entry `event` is on, `null` when it is on none of the drawn figure. */
  #entryOf(event) {
    const entry = event.target?.closest?.(".sbmlsim-legend-entry[data-legend]");
    return entry && this.#svg?.contains(entry) ? entry : null;
  }

  /**
   * A click: a legend entry toggles its group, a data point (or, between the points, an element
   * of the SVG) selects its group, and the read-out of the point is announced.
   */
  #click(event) {
    const entry = this.#entryOf(event);
    if (entry) {
      if (this.#options.toggle) this.#toggle(entry.getAttribute("data-legend"));
      return;
    }
    if (!this.#options.select || !this.#svg?.contains(event.target)) return;
    this.#pointerInside = true;
    this.#cancelFrame();
    const point = this.#point(event);
    const hit = this.#pick(point, "visible");
    let group = hit?.legend ?? null;
    if (group === null) {
      const node = event.target.closest?.("[data-element]");
      const drawn = node && this.#svg.contains(node) ? node.getAttribute("data-legend") : null;
      if (drawn !== null && !this.#hidden.has(drawn)) group = drawn;
    }
    if (group === null) return;
    this.#select(group === this.#selected ? null : group);
    if (this.#options.readout && hit) this.#readout(point, event, true);
  }

  /**
   * Enter or Space on a legend entry toggles its group; Shift+Enter selects it (showing it when
   * it is hidden) or clears the selection when it is selected.
   */
  #key(event) {
    if (event.key !== "Enter" && event.key !== " " && event.key !== "Spacebar") return;
    const entry = this.#entryOf(event);
    if (!entry) return;
    event.preventDefault();
    const group = entry.getAttribute("data-legend");
    if (event.key === "Enter" && event.shiftKey) {
      if (!this.#options.select) return;
      if (this.#hidden.delete(group)) this.#releasePickers();
      this.#select(group === this.#selected ? null : group);
      return;
    }
    this.#toggle(group);
  }

  /**
   * Escape clears the selection of this figure when the key is pressed inside its container
   * (the focus is in it) or the pointer was last in it, so other figures keep theirs.
   */
  #escape(event) {
    if (event.key !== "Escape" || this.#selected === null) return;
    if (!this.#pointerInside && !this.#container.contains(event.target)) return;
    this.#select(null);
  }

  /** Hides or shows a legend group; hiding the selected group clears the selection. */
  #toggle(group) {
    if (this.#hidden.has(group)) this.#hidden.delete(group);
    else this.#hidden.add(group);
    if (this.#hidden.has(this.#selected)) this.#selected = null;
    this.#releasePickers();
    this.#apply();
    this.#hideTooltip();
    this.#prefetch();
  }

  /** Selects a legend group, `null` clears the selection. */
  #select(group) {
    this.#selected = group;
    this.#releasePickers();
    this.#apply();
    this.#prefetch();
  }

  /** The pixel of the figure under a pointer, the SVG possibly scaled by CSS. */
  #point({ clientX, clientY }) {
    const rect = this.#svg.getBoundingClientRect?.() ?? { left: 0, top: 0, width: 0, height: 0 };
    const sx = rect.width > 0 ? this.#geometry.width / rect.width : 1;
    const sy = rect.height > 0 ? this.#geometry.height / rect.height : 1;
    return { x: (clientX - rect.left) * sx, y: (clientY - rect.top) * sy };
  }

  /** A pointer move keeps its position; the read-out runs once per animation frame. */
  #move(event) {
    if (!this.#svg?.contains(event.target)) {
      this.#cancelFrame();
      this.#hideTooltip();
      return;
    }
    this.#pointerInside = true;
    if (!this.#options.readout) return;
    this.#pending = { clientX: event.clientX, clientY: event.clientY };
    if (this.#frame !== null) return;
    const view = this.#document.defaultView;
    if (typeof view?.requestAnimationFrame === "function") {
      const id = view.requestAnimationFrame(() => this.#flush());
      this.#frame = () => view.cancelAnimationFrame?.(id);
    } else {
      const id = setTimeout(() => this.#flush(), 16);
      this.#frame = () => clearTimeout(id);
    }
  }

  /** The read-out of the last pointer move, unless the pointer left meanwhile. */
  #flush() {
    this.#frame = null;
    const pending = this.#pending;
    this.#pending = null;
    if (pending === null || this.#disposed || !this.#svg) return;
    this.#readout(this.#point(pending), pending, false);
  }

  /** Cancels the read-out of a pending pointer move. */
  #cancelFrame() {
    this.#frame?.();
    this.#frame = null;
    this.#pending = null;
  }

  /** The pointer left the container. */
  #leave() {
    this.#pointerInside = false;
    this.#cancelFrame();
    this.#hideTooltip();
  }

  /** The read-out at a pixel: the selected group first, then every shown group. */
  #readout(point, at, announce) {
    let hit = this.#selected !== null ? this.#pick(point, "selected") : null;
    hit ??= this.#pick(point, "visible");
    if (!hit) {
      this.#hideTooltip();
      return;
    }
    this.#showTooltip(hit, at, announce);
  }

  /**
   * Whether a filter keeps a legend group: `visible` every shown group and the elements in no
   * group, `selected` the selected group alone.
   */
  #keeps(key) {
    const selected = this.#selected;
    const hidden = this.#hidden;
    return key === "selected" ? (group) => group !== null && group === selected : (group) => group === null || !hidden.has(group);
  }

  /** Whether the filter `key` keeps every element, so the drawn figure answers it. */
  #unfiltered(key) {
    return key === "visible" && this.#hidden.size === 0;
  }

  /**
   * The nearest point the filter `key` keeps within the reach of `(x, y)`: the nearest of the
   * panel under the pointer, else of a panel the pointer is near.
   */
  #pick({ x, y }, key) {
    if (this.#unfiltered(key)) return this.#figure.pick(x, y, RADIUS);
    const keeps = this.#keeps(key);
    for (const drawn of plotsAt(this.#geometry, x, y, RADIUS)) {
      const picker = this.#picker(key, drawn.id);
      let hit;
      if (picker) {
        hit = picker.figure.pick(x - drawn.rect.x + picker.rect.x, y - drawn.rect.y + picker.rect.y, RADIUS);
      } else {
        hit = this.#figure.pick(x, y, RADIUS);
        if (hit && (hit.plot !== drawn.id || !keeps(hit.legend ?? null))) hit = null;
      }
      if (hit) return hit;
    }
    return null;
  }

  /**
   * The prepared figure of the filter `key` of one plot, or `null` when the plot is laid out
   * otherwise than in the drawn figure (the drawn figure is filtered then).
   */
  #picker(key, id) {
    const cache = `${key}\u0000${id}`;
    if (this.#pickers.has(cache)) return this.#pickers.get(cache);
    const keeps = this.#keeps(key);
    let picker = null;
    let figure = null;
    try {
      figure = this.#plot.prepare(filteredSpec(this.#spec, this.#geometry, (element) => keeps(groupOf(element)), id));
      const geometry = figure.geometry();
      if (sameLayout(this.#geometry, geometry, id)) {
        picker = { figure, rect: geometry.plots.find((plot) => plot.id === id).rect };
      }
    } catch {
      // a filtered spec the renderer refuses: the drawn figure is filtered
    }
    if (picker === null) figure?.dispose();
    this.#pickers.set(cache, picker);
    return picker;
  }

  /** Frees the prepared figures of the filters, which a change of the state outdates. */
  #releasePickers() {
    this.#cancelIdle();
    for (const picker of this.#pickers.values()) picker?.figure.dispose();
    this.#pickers.clear();
  }

  /**
   * Prepares the filtered figures the state needs in idle time, one plot per idle callback, so
   * the first pointer move after a toggle or a selection does not prepare them.
   */
  #prefetch() {
    this.#cancelIdle();
    if (this.#disposed || !(this.#options.readout || this.#options.select)) return;
    const keys = ["visible", "selected"].filter(
      (key) => !this.#unfiltered(key) && (key === "visible" || this.#selected !== null),
    );
    const work = keys.flatMap((key) => this.#geometry.plots.map((plot) => [key, plot.id]));
    const view = this.#document.defaultView;
    const step = () => {
      this.#idle = null;
      if (this.#disposed) return;
      const next = work.shift();
      if (!next) return;
      this.#picker(...next);
      schedule();
    };
    const schedule = () => {
      if (work.length === 0) return;
      if (typeof view?.requestIdleCallback === "function") {
        const id = view.requestIdleCallback(step);
        this.#idle = () => view.cancelIdleCallback?.(id);
      } else {
        const id = setTimeout(step, 0);
        this.#idle = () => clearTimeout(id);
      }
    };
    schedule();
  }

  /** Cancels the preparation in idle time. */
  #cancelIdle() {
    this.#idle?.();
    this.#idle = null;
  }

  /** The tooltip, created on first use, positioned in the container. */
  #tooltipElement() {
    if (this.#tooltip) return this.#tooltip;
    const view = this.#document.defaultView;
    const position = view?.getComputedStyle?.(this.#container).position ?? this.#container.style.position;
    if (!position || position === "static") {
      this.#position = this.#container.style.position ?? "";
      this.#container.style.position = "relative";
    }
    const tooltip = this.#document.createElement("div");
    tooltip.className = TOOLTIP;
    tooltip.setAttribute("role", "status");
    tooltip.setAttribute("aria-live", "off");
    tooltip.hidden = true;
    this.#container.append(tooltip);
    this.#tooltip = tooltip;
    return tooltip;
  }

  /**
   * Shows the read-out of a hit next to the pointer `at`; `announce` (a click) makes it a polite
   * live region, a hover does not, so a moving pointer is not read aloud.
   */
  #showTooltip(hit, at, announce) {
    const tooltip = this.#tooltipElement();
    tooltip.setAttribute("aria-live", announce ? "polite" : "off");
    const key = `${hit.plot}\u0000${hit.element}\u0000${hit.index}`;
    if (key !== this.#shown || announce) {
      const { plot, element } = this.#elements.get(hit.element) ?? { plot: {}, element: { kind: "" } };
      const label = this.#document.createElement("div");
      label.className = `${TOOLTIP}-label`;
      label.textContent = hit.label ?? element.label ?? hit.element;
      const rows = readoutLines(hit, plot, element, this.#options.format).map(([name, value]) => {
        const row = this.#document.createElement("div");
        row.className = `${TOOLTIP}-row`;
        const span = this.#document.createElement("span");
        span.className = `${TOOLTIP}-name`;
        span.textContent = `${name}: `;
        row.append(span, value);
        return row;
      });
      tooltip.replaceChildren(label, ...rows);
      this.#shown = key;
    }
    tooltip.hidden = false;
    const box = this.#container.getBoundingClientRect?.() ?? { left: 0, top: 0 };
    const left = at.clientX - box.left - (this.#container.clientLeft || 0);
    const top = at.clientY - box.top - (this.#container.clientTop || 0);
    const width = tooltip.offsetWidth || 0;
    const height = tooltip.offsetHeight || 0;
    const room = this.#container.clientWidth || Infinity;
    const x = left + OFFSET + width > room ? Math.max(0, left - OFFSET - width) : left + OFFSET;
    // above the pointer, or below it where the container has no room above
    const y = top - OFFSET - height >= 0 ? top - OFFSET - height : top + OFFSET;
    tooltip.style.left = `${Math.round(x)}px`;
    tooltip.style.top = `${Math.round(y)}px`;
  }

  /** Hides the read-out. */
  #hideTooltip() {
    if (!this.#tooltip) return;
    this.#tooltip.hidden = true;
    this.#shown = null;
  }
}

/**
 * Renders a `figure/1` spec into `container` (replacing an SVG already there, e.g. the static
 * figure of a report) and makes it interactive: its legend toggles groups, a click selects a
 * group and a tooltip reads out the point under the pointer.
 *
 * @param {Element} container the element the figure is drawn into; it gets `position: relative`
 *   when it is static, for the tooltip
 * @param {import("./plot.mjs").Plot} plot the renderer, `loadPlot(...)`
 * @param {object|string} spec a `figure/1` spec, an object or its JSON text
 * @param {object} [options]
 * @param {boolean} [options.readout=true] the tooltip of the point under the pointer
 * @param {boolean} [options.select=true] the selection of a group by a click
 * @param {boolean} [options.toggle=true] the legend entries as toggles of their groups
 * @param {(value: number, unit?: string) => string} [options.format] the text of a value with
 *   its unit (`formatValue`, 4 significant digits, without)
 * @returns {FigureHandle} the handle of the figure
 * @throws {PlotError} for a spec which is refused
 */
export function attachFigure(container, plot, spec, options = {}) {
  return new FigureHandle(container, plot, spec, options);
}
