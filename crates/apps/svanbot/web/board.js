// The board (V11-44): a page's panels in three columns, in the reader's own arrangement, shown a
// view at a time, as v10's control room had them. The arrangement, the view and what is folded
// are remembered in this browser only: they are the reader's, not the fleet's.

import { h } from "./dom.js";

export const COLUMNS = ["left", "center", "right"];
const NAMES = { left: "left", center: "centre", right: "right" };

/**
 * What this browser remembers. A private window, blocked storage or a page with no storage
 * still works: the value is kept for as long as the page is open.
 */
export const store = (() => {
  const held = new Map();
  const local = () => {
    try {
      return globalThis.localStorage ?? null;
    } catch {
      return null;
    }
  };
  return {
    read(key, fallback) {
      if (held.has(key)) {
        return held.get(key);
      }
      try {
        const text = local()?.getItem(key);
        return text === null || text === undefined ? fallback : JSON.parse(text);
      } catch {
        return fallback;
      }
    },
    write(key, value) {
      held.set(key, value);
      try {
        local()?.setItem(key, JSON.stringify(value));
      } catch {
        // Kept for this page only.
      }
    },
  };
})();

function copy(layout) {
  return {
    left: [...layout.left],
    center: [...layout.center],
    right: [...layout.right],
    hidden: [...layout.hidden],
  };
}

/**
 * A stored layout reconciled with the panels there are now: an unknown or repeated id is dropped
 * and a new panel lands in its default column, so a release that adds or removes a panel never
 * loses the reader's arrangement. Anything that is not a layout is the default.
 */
export function reconcile(stored, defaults) {
  if (stored === null || typeof stored !== "object") {
    return copy(defaults);
  }
  const known = new Set([...COLUMNS.flatMap((column) => defaults[column]), ...defaults.hidden]);
  const seen = new Set();
  const keep = (ids) =>
    (Array.isArray(ids) ? ids : []).filter(
      (id) => typeof id === "string" && known.has(id) && !seen.has(id) && seen.add(id),
    );
  const out = {
    left: keep(stored.left),
    center: keep(stored.center),
    right: keep(stored.right),
    hidden: keep(stored.hidden),
  };
  for (const column of [...COLUMNS, "hidden"]) {
    for (const id of defaults[column]) {
      if (!seen.has(id)) {
        out[column].push(id);
        seen.add(id);
      }
    }
  }
  return out;
}

function locate(layout, id) {
  for (const column of COLUMNS) {
    const at = layout[column].indexOf(id);
    if (at >= 0) {
      return [column, at];
    }
  }
  return null;
}

/** The layout with `id` a step `up`, `down`, `left` or `right`; at an edge, the same layout. */
export function move(layout, id, direction) {
  const found = locate(layout, id);
  if (!found) {
    return layout;
  }
  const [column, at] = found;
  const next = copy(layout);
  if (direction === "up" || direction === "down") {
    const to = at + (direction === "up" ? -1 : 1);
    if (to < 0 || to >= layout[column].length) {
      return layout;
    }
    [next[column][at], next[column][to]] = [next[column][to], next[column][at]];
    return next;
  }
  const target = COLUMNS[COLUMNS.indexOf(column) + (direction === "left" ? -1 : 1)];
  if (!target) {
    return layout;
  }
  next[column].splice(at, 1);
  next[target].splice(Math.min(at, next[target].length), 0, id);
  return next;
}

/** The layout with `id` taken off the board into the hidden tray. */
export function hide(layout, id) {
  const next = copy(layout);
  for (const column of COLUMNS) {
    next[column] = next[column].filter((other) => other !== id);
  }
  if (!next.hidden.includes(id)) {
    next.hidden.push(id);
  }
  return next;
}

/** The layout with a hidden `id` back at the foot of the column it has by default. */
export function show(layout, id, defaults) {
  const next = copy(layout);
  next.hidden = next.hidden.filter((other) => other !== id);
  const home = COLUMNS.find((column) => defaults[column].includes(id)) ?? "center";
  if (!locate(next, id)) {
    next[home].push(id);
  }
  return next;
}

/** The tabs of a page's views, the one in view selected. */
export function viewTabs(views, current) {
  return h(
    "div",
    { class: "view-tabs", role: "tablist", "aria-label": "Views" },
    views.map((view) =>
      h(
        "button",
        {
          type: "button",
          role: "tab",
          "aria-selected": String(view.id === current),
          "data-act": "view",
          "data-view": view.id,
        },
        view.label,
      ),
    ),
  );
}

const ARROWS = { up: "↑", down: "↓", left: "←", right: "→" };

/** A panel in its chrome: the title, folding, and the moves while the reader arranges. */
function panelOf(widget, folded, arranging) {
  return h(
    "section",
    {
      class: `card widget${widget.wide ? " wide" : ""}`,
      "data-widget": widget.id,
      "data-collapsed": folded ? "true" : null,
    },
    h(
      "header",
      { class: "widget-head" },
      h("h2", {}, widget.title),
      h(
        "span",
        { class: "widget-tools" },
        arranging
          ? [
              Object.entries(ARROWS).map(([dir, arrow]) =>
                h(
                  "button",
                  {
                    type: "button",
                    class: "move",
                    "data-act": "move",
                    "data-id": widget.id,
                    "data-dir": dir,
                    "aria-label": `Move ${widget.title} ${dir}`,
                  },
                  arrow,
                ),
              ),
              h(
                "button",
                {
                  type: "button",
                  class: "move hide",
                  "data-act": "hide",
                  "data-id": widget.id,
                  "aria-label": `Hide ${widget.title}`,
                },
                "×",
              ),
            ]
          : null,
        h(
          "button",
          {
            type: "button",
            class: "fold",
            "data-act": "fold",
            "data-id": widget.id,
            "aria-expanded": String(!folded),
            "aria-label": `${folded ? "Open" : "Fold"} ${widget.title}`,
          },
          folded ? "+" : "−",
        ),
      ),
    ),
    folded ? null : h("div", { class: "widget-body" }, widget.body()),
  );
}

/**
 * The board: the view's panels in the reader's columns. While the reader arranges, every panel
 * has its moves and the hidden ones wait in a tray to come back.
 */
export function board(widgets, view, { layout, collapsed, arranging }) {
  const byId = new Map(widgets.map((widget) => [widget.id, widget]));
  const inView = (id) => byId.has(id) && (view.widgets === null || view.widgets.includes(id));
  const columns = COLUMNS.map((column) =>
    h(
      "div",
      { class: `column ${column}`, "aria-label": `${NAMES[column]} column` },
      layout[column]
        .filter(inView)
        .map((id) => panelOf(byId.get(id), collapsed.includes(id), arranging)),
    ),
  );
  const hidden = layout.hidden.filter((id) => byId.has(id));
  return h(
    "div",
    { class: `board${arranging ? " arranging" : ""}` },
    arranging && hidden.length > 0
      ? h(
          "div",
          { class: "tray" },
          h("span", { class: "label" }, "Hidden panels"),
          hidden.map((id) =>
            h(
              "button",
              { type: "button", class: "unhide", "data-act": "unhide", "data-id": id },
              `+ ${byId.get(id).title}`,
            ),
          ),
        )
      : null,
    h("div", { class: "columns" }, columns),
  );
}
