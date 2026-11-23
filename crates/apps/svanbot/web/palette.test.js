// The command palette (V11-45): Ctrl/⌘ K, then a few letters, takes the reader to any bot, page
// or view. What it offers and how it ranks a query are tested here; the overlay is the browser's.
import { expect, test } from "bun:test";
import { commands, rank } from "./palette.js";

const FLEET = {
  bots: [
    { bot: "SurSvan", connected: true },
    { bot: "Svanism", connected: false },
  ],
};

test("the palette offers every bot, every page, and the room's views and looks", () => {
  const offered = commands(FLEET, "bot");
  const labels = offered.map((command) => command.label);
  expect(labels).toContain("Fleet overview");
  expect(labels).toContain("SurSvan");
  expect(labels).toContain("Svanism: dossier");
  expect(labels).toContain("Arena scoreboard");
  expect(labels).toContain("Release");
  expect(labels).toContain("View: Opponents");
  expect(labels).toContain("Look: midnight");
  const sur = offered.find((command) => command.label === "SurSvan");
  expect(sur.href).toBe("/bot/SurSvan");
  expect(sur.hint).toBe("bot · playing");
  const view = offered.find((command) => command.label === "View: Learning");
  expect(view.act).toEqual(["view", { view: "learning" }]);
  // The fleet room has no Learning view, so it offers none.
  expect(commands(FLEET, "fleet").map((command) => command.label)).not.toContain("View: Learning");
  // A page that is not a room offers no views.
  expect(commands(FLEET, null).some((command) => command.label.startsWith("View:"))).toBe(false);
});

test("a query keeps what holds its letters in order, best match first", () => {
  const offered = commands(FLEET, "bot");
  const found = rank(offered, "ism").map((command) => command.label);
  expect(found[0]).toBe("Svanism");
  expect(found).toContain("Svanism: dossier");
  expect(found).not.toContain("SurSvan");
  // Letters in order, not together: "srsv" is SurSvan.
  expect(rank(offered, "srsv")[0].label).toBe("SurSvan");
  // A word's start beats the middle of one.
  expect(rank(offered, "rel")[0].label).toBe("Release");
  // No query keeps everything in its order; no match is nothing.
  expect(rank(offered, "")).toEqual(offered);
  expect(rank(offered, "zzzz")).toEqual([]);
});
