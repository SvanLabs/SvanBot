// The board (V11-44): the panels of a page in the reader's own arrangement, a view at a time, as
// v10's control room had them.
import { beforeAll, expect, test } from "bun:test";
import { byClass, standIn } from "./testdom.js";

let board;

beforeAll(async () => {
  globalThis.document = standIn;
  board = await import("./board.js");
});

const DEFAULTS = { left: ["now", "notes"], center: ["table"], right: ["activity"], hidden: [] };

test("a stored layout keeps the reader's order, drops panels that are gone and places new ones", () => {
  const stored = {
    left: ["activity", "gone", "now"],
    center: ["table", "table"],
    right: [],
    hidden: ["notes"],
  };
  expect(board.reconcile(stored, DEFAULTS)).toEqual({
    left: ["activity", "now"],
    center: ["table"],
    right: [],
    hidden: ["notes"],
  });
  // A layout that is not one, or none at all, is the default.
  expect(board.reconcile("junk", DEFAULTS)).toEqual(DEFAULTS);
  expect(board.reconcile(null, DEFAULTS)).toEqual(DEFAULTS);
});

test("a panel moves a step at a time, within its column and across, and never off the board", () => {
  const down = board.move(DEFAULTS, "now", "down");
  expect(down.left).toEqual(["notes", "now"]);
  expect(board.move(down, "now", "down")).toEqual(down);
  const across = board.move(DEFAULTS, "notes", "right");
  expect(across.left).toEqual(["now"]);
  expect(across.center).toEqual(["table", "notes"]);
  expect(board.move(DEFAULTS, "now", "left")).toEqual(DEFAULTS);
  // The original is never changed in place.
  expect(DEFAULTS.left).toEqual(["now", "notes"]);
});

test("a hidden panel leaves its column, and showing it puts it back at the foot of its old one", () => {
  const hidden = board.hide(DEFAULTS, "notes");
  expect(hidden.left).toEqual(["now"]);
  expect(hidden.hidden).toEqual(["notes"]);
  const shown = board.show(hidden, "notes", DEFAULTS);
  expect(shown.left).toEqual(["now", "notes"]);
  expect(shown.hidden).toEqual([]);
});

const WIDGETS = [
  { id: "now", title: "Now", body: () => "status" },
  { id: "notes", title: "Notes", body: () => "mine" },
  { id: "table", title: "Table", body: () => "felt" },
  { id: "activity", title: "Activity", body: () => "log" },
];
const VIEWS = [
  { id: "live", label: "Live", widgets: ["table", "activity"] },
  { id: "all", label: "All", widgets: null },
];

function titles(node) {
  return byClass(node, "widget").map((panel) => panel.querySelector("h2").textContent);
}

test("a view shows its panels in the reader's arrangement, and All shows every one", () => {
  const options = { layout: DEFAULTS, collapsed: [], arranging: false };
  expect(titles(board.board(WIDGETS, VIEWS[0], options))).toEqual(["Table", "Activity"]);
  expect(titles(board.board(WIDGETS, VIEWS[1], options))).toEqual([
    "Now",
    "Notes",
    "Table",
    "Activity",
  ]);
  // Moved across, a panel keeps its row: Activity lands above the Table in the centre.
  const moved = { ...options, layout: board.move(DEFAULTS, "activity", "left") };
  const columns = byClass(board.board(WIDGETS, VIEWS[1], moved), "column");
  expect(columns.map((column) => titles(column))).toEqual([
    ["Now", "Notes"],
    ["Activity", "Table"],
    [],
  ]);
});

test("a collapsed panel keeps its title and loses its body; arranging shows the moves and the hidden", () => {
  const collapsed = board.board(WIDGETS, VIEWS[1], {
    layout: board.hide(DEFAULTS, "notes"),
    collapsed: ["now"],
    arranging: true,
  });
  const [now] = byClass(collapsed, "widget");
  expect(now.textContent).not.toContain("status");
  expect(now.attrs["data-collapsed"]).toBe("true");
  const moves = byClass(now, "move").map((button) => button.attrs["data-dir"]);
  expect(moves).toEqual(["up", "down", "left", "right"]);
  const [tray] = byClass(collapsed, "tray");
  expect(tray.textContent).toContain("Notes");
  expect(byClass(tray, "unhide")[0].attrs["data-id"]).toBe("notes");
});

test("the view tabs mark the view in view", () => {
  const tabs = board.viewTabs(VIEWS, "all");
  const buttons = tabs.all("button");
  expect(buttons.map((button) => button.textContent)).toEqual(["Live", "All"]);
  expect(buttons.map((button) => button.attrs["aria-selected"])).toEqual(["false", "true"]);
  expect(buttons[0].attrs["data-act"]).toBe("view");
});

test("the store remembers a value for the page even where the browser keeps none", () => {
  board.store.write("svanbot.test", { left: ["a"] });
  expect(board.store.read("svanbot.test", null)).toEqual({ left: ["a"] });
  expect(board.store.read("svanbot.never", "fallback")).toBe("fallback");
});
