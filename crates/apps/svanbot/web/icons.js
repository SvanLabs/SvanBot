// The dashboard's icons (V11-45): our own, a path each on a 24-unit grid, drawn as a line in the
// text's colour. Decorative only: every icon sits beside the words that say the same thing.

import { h } from "./dom.js";

const PATHS = {
  bots: "M12 3v3M8 6h8a3 3 0 0 1 3 3v6a3 3 0 0 1-3 3H8a3 3 0 0 1-3-3V9a3 3 0 0 1 3-3zM9.5 11v1M14.5 11v1M9.5 15h5M3 11v3M21 11v3",
  cards:
    "M8 4h8a2 2 0 0 1 2 2v12a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2zM12 8.5l2.4 3.5-2.4 3.5-2.4-3.5z",
  trend: "M3 17l6-6 4 4 8-8M15 7h6v6",
  trophy:
    "M8 4h8v5a4 4 0 0 1-8 0zM8 6H5a3 3 0 0 0 3 4.5M16 6h3a3 3 0 0 1-3 4.5M12 13v4M8.5 20h7M10 17h4",
  chips:
    "M4 7c0-1.7 3.6-3 8-3s8 1.3 8 3-3.6 3-8 3-8-1.3-8-3zM4 7v5c0 1.7 3.6 3 8 3s8-1.3 8-3V7M4 12v5c0 1.7 3.6 3 8 3s8-1.3 8-3v-5",
  search: "M11 18a7 7 0 1 0 0-14 7 7 0 0 0 0 14zM20 20l-4-4",
  bolt: "M13 2 4 14h7l-1 8 9-12h-7z",
  pulse: "M3 12h4l2.5-6 5 12 2.5-6H21",
  alert: "M12 3 2 20h20zM12 10v4M12 17v.5",
};

/** The icon `name`, sized by the text around it. */
export function icon(name) {
  return h(
    "svg",
    { class: `icon icon-${name}`, viewBox: "0 0 24 24", "aria-hidden": "true" },
    h("path", { d: PATHS[name] ?? PATHS.bolt }),
  );
}
