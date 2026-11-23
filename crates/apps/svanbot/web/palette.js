// The command palette (V11-45): Ctrl/⌘ K, or the search field in the header, then a few letters,
// takes the reader to any bot, page, view or look. The commands and their ranking are plain
// functions; the overlay is drawn on top of the page and is the browser's alone.

import { BOT_VIEWS, FLEET_VIEWS } from "./control.js";
import { h } from "./dom.js";
import { THEMES } from "./felt.js";

const ROOM_VIEWS = { fleet: FLEET_VIEWS, bot: BOT_VIEWS };

/**
 * Everything the palette offers: the fleet, every bot and its dossier, the pages, then the views
 * of the room in view and the table's looks. A command goes to an `href` or runs an `act`.
 */
export function commands(fleet, room) {
  const bots = fleet?.bots ?? [];
  return [
    { label: "Fleet overview", hint: "page", href: "/" },
    ...bots.map((status) => ({
      label: status.bot,
      hint: `bot · ${status.connected ? "playing" : "offline"}`,
      href: `/bot/${encodeURIComponent(status.bot)}`,
    })),
    ...bots.map((status) => ({
      label: `${status.bot}: dossier`,
      hint: "reads",
      href: `/bot/${encodeURIComponent(status.bot)}/dossier`,
    })),
    { label: "Opponents", hint: "every bot's dossier", href: "/dossier" },
    { label: "Arena scoreboard", hint: "page", href: "/scoreboard" },
    { label: "Release", hint: "page", href: "/release" },
    ...(ROOM_VIEWS[room] ?? []).map((view) => ({
      label: `View: ${view.label}`,
      hint: "view",
      act: ["view", { view: view.id }],
    })),
    ...THEMES.map((theme) => ({
      label: `Look: ${theme}`,
      hint: "table",
      act: ["theme", { theme }],
    })),
    ...(room ? [{ label: "Arrange panels", hint: "layout", act: ["arrange", {}] }] : []),
  ];
}

/**
 * How well `query` matches `label`, lower is better, or null when its letters are not all in
 * `label` in order. Letters close together, early, and at the start of a word count most.
 */
function score(label, query) {
  const text = label.toLowerCase();
  let at = -1;
  let first = -1;
  let gaps = 0;
  let starts = 0;
  for (const letter of query.toLowerCase()) {
    const next = text.indexOf(letter, at + 1);
    if (next < 0) {
      return null;
    }
    if (first < 0) {
      first = next;
    }
    if (at >= 0) {
      gaps += next - at - 1;
    }
    if (next === 0 || /[\s:·]/.test(text[next - 1])) {
      starts += 1;
    }
    at = next;
  }
  return first + gaps * 2 - starts * 3 + text.length * 0.01;
}

/** The commands that match `query`, best first; with no query, all of them as they were. */
export function rank(offered, query) {
  const wanted = query.trim();
  if (wanted === "") {
    return offered;
  }
  return offered
    .map((command, order) => ({ command, order, score: score(command.label, wanted) }))
    .filter((found) => found.score !== null)
    .sort((a, b) => a.score - b.score || a.order - b.order)
    .map((found) => found.command);
}

/**
 * Opens the palette over the page: a field, the matches under it, ↑ ↓ to choose, Enter to go,
 * Esc or a click outside to close. `go` follows an href; `act` runs a reader's action.
 */
export function openPalette({ fleet, room, go, act }) {
  if (document.querySelector(".palette")) {
    return;
  }
  const offered = commands(fleet, room);
  let found = offered;
  let chosen = 0;
  const field = h("input", {
    type: "search",
    class: "palette-field",
    placeholder: "Jump to a bot, page, view or look…",
    "aria-label": "Command",
    autocomplete: "off",
    spellcheck: "false",
  });
  const list = h("ul", { class: "palette-list", role: "listbox" });
  const panel = h(
    "div",
    { class: "palette-panel", role: "dialog", "aria-modal": "true", "aria-label": "Commands" },
    h(
      "div",
      { class: "palette-head" },
      h("span", { class: "palette-icon", "aria-hidden": "true" }, "⌘"),
      field,
    ),
    list,
    h(
      "div",
      { class: "palette-foot" },
      h("span", {}, h("kbd", {}, "↑"), h("kbd", {}, "↓"), " choose"),
      h("span", {}, h("kbd", {}, "↵"), " go"),
      h("span", {}, h("kbd", {}, "esc"), " close"),
    ),
  );
  const overlay = h("div", { class: "palette" }, panel);
  const close = () => overlay.remove();
  const run = (command) => {
    close();
    if (command.href) {
      go(command.href);
    } else {
      act(...command.act);
    }
  };
  const draw = () => {
    list.replaceChildren(
      ...(found.length === 0
        ? [h("li", { class: "palette-empty" }, "Nothing matches.")]
        : found.slice(0, 12).map((command, at) => {
            const item = h(
              "li",
              { role: "option", "aria-selected": String(at === chosen) },
              h("span", { class: "palette-label" }, command.label),
              h("span", { class: "palette-hint" }, command.hint),
            );
            item.addEventListener("mousedown", (event) => {
              event.preventDefault();
              run(command);
            });
            return item;
          })),
    );
  };
  field.addEventListener("input", () => {
    found = rank(offered, field.value);
    chosen = 0;
    draw();
  });
  field.addEventListener("keydown", (event) => {
    const shown = Math.min(found.length, 12);
    if (event.key === "ArrowDown" && shown > 0) {
      chosen = (chosen + 1) % shown;
      draw();
      event.preventDefault();
    } else if (event.key === "ArrowUp" && shown > 0) {
      chosen = (chosen + shown - 1) % shown;
      draw();
      event.preventDefault();
    } else if (event.key === "Enter" && found[chosen]) {
      run(found[chosen]);
    } else if (event.key === "Escape") {
      close();
    }
  });
  overlay.addEventListener("mousedown", (event) => {
    if (event.target === overlay) {
      close();
    }
  });
  draw();
  document.body.append(overlay);
  field.focus();
}
