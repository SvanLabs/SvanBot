// The page's script, run against a stand-in for the browser: enough of a DOM to build the
// views, and an event stream the test feeds by hand. It holds what each view shows and that a
// name is written as text, never as markup.
import { beforeAll, expect, test } from "bun:test";

/** An element that keeps its attributes and children, and reads back as text or as markup. */
class Node {
  constructor(tag) {
    this.tag = tag;
    this.attrs = {};
    this.children = [];
    this.open = false;
  }
  setAttribute(name, value) {
    this.attrs[name] = value;
  }
  append(...children) {
    this.children.push(...children);
  }
  replaceChildren(...children) {
    this.children = children;
  }
  get childNodes() {
    return this.children;
  }
  set className(value) {
    this.attrs.class = value;
  }
  get className() {
    return this.attrs.class ?? "";
  }
  set textContent(value) {
    this.children = [String(value)];
  }
  get textContent() {
    return this.children
      .map((child) => (typeof child === "string" ? child : child.textContent))
      .join("");
  }
  get innerHTML() {
    return this.children
      .map((child) => {
        if (typeof child === "string") {
          return child.replaceAll("&", "&amp;").replaceAll("<", "&lt;");
        }
        const attrs = Object.entries(child.attrs)
          .map(([name, value]) => ` ${name}="${value}"`)
          .join("");
        return `<${child.tag}${attrs}${child.open ? " open" : ""}>${child.innerHTML}</${child.tag}>`;
      })
      .join("");
  }
  all(tag, found = []) {
    for (const child of this.children) {
      if (typeof child !== "string") {
        if (child.tag === tag) {
          found.push(child);
        }
        child.all(tag, found);
      }
    }
    return found;
  }
  querySelector(tag) {
    return this.all(tag)[0] ?? null;
  }
}

const ids = {
  view: new Node("main"),
  fresh: new Node("span"),
  count: new Node("span"),
  worst: new Node("strong"),
};
const listeners = {};
const NOW = 1_800_000_000_000;

function fleet() {
  const play = {
    champion: "308e7da4b29fd533",
    probe: null,
    counted: 200,
    won_mbb: 3_000,
    recent: [1_000, -3_000, 500],
  };
  const base = {
    pid: 7,
    version: "11.0.0",
    started_ms: NOW - 3_600_000,
    updated_ms: NOW - 1_000,
    connected: true,
    table: null,
    hands: 210,
    incidents: 0,
    run_incidents: 0,
    last_hand_ms: NOW - 30_000,
    last_error: null,
    play,
  };
  return {
    server_ms: NOW,
    problems: [],
    release: { current: "11.0.0", previous: "10.9.0", installed: ["10.9.0", "11.0.0"] },
    arena: [
      {
        commit: "abc1234def",
        champion: "308e7da4b29fd533",
        seed: 7,
        rows: [
          { opponent: "nit", deals: 400, mbb_per_100: 12_345.6, ci95: 2_000.4 },
          { opponent: "<b>maniac</b>", deals: 400, mbb_per_100: -5_000, ci95: 9_000 },
        ],
      },
      {
        commit: "older999",
        champion: "308e7da4b29fd533",
        seed: 7,
        rows: [{ opponent: "nit", deals: 400, mbb_per_100: 1_000, ci95: 500 }],
      },
    ],
    bots: [
      {
        ...base,
        bot: "SurSvan",
        table: {
          id: "t-17",
          seat: 2,
          hand_id: "h-1",
          street: "turn",
          board: ["Ah", "Kd", "7c", "2s"],
          pot: 640,
          big_blind: 20,
          dealer: 0,
          players: [
            { seat: 0, name: "<img src=x onerror=alert(1)>", stack: 1810, folded: false },
            { seat: 2, name: "SurSvan", stack: 2240, folded: false },
            { seat: 4, name: "someone", stack: 1950, folded: true },
          ],
        },
      },
      { ...base, bot: "MegaSvan", connected: false, last_error: "socket: reset" },
    ],
  };
}

/** Shows `path` with the fleet just received, and returns the view. */
function show(path) {
  globalThis.location.pathname = path;
  listeners.fleet({ data: JSON.stringify(fleet()) });
  return ids.view;
}

beforeAll(async () => {
  globalThis.document = {
    createElement: (tag) => new Node(tag),
    createElementNS: (_, tag) => new Node(tag),
    getElementById: (id) => ids[id],
    querySelector: (tag) => ids.view.querySelector(tag),
  };
  globalThis.location = { pathname: "/" };
  globalThis.EventSource = class {
    addEventListener(name, listener) {
      listeners[name] = listener;
    }
  };
  globalThis.setInterval = () => 0;
  Date.now = () => NOW;
  await import("./app.js");
});

test("the fleet page has one card a bot and the strip names the worst problem", () => {
  const view = show("/");
  expect(view.all("article").length).toBe(2);
  expect(ids.count.textContent).toBe("2 bots");
  expect(ids.fresh.textContent).toBe("live");
  expect(ids.worst.textContent).toBe("MegaSvan is offline: socket: reset");
  expect(view.textContent).toContain("t-17 · turn · pot 640");
  expect(view.textContent).toContain("+1.5 bb/100");
  expect(view.querySelector("a").attrs.href).toBe("/bot/SurSvan");
});

test("a bot's page has five panels, its table, and a chart with its numbers in a table", () => {
  const view = show("/bot/SurSvan");
  expect(view.all("section").map((panel) => panel.querySelector("h2").textContent)).toEqual([
    "Now",
    "Table",
    "Ledger",
    "Plays by",
    "Process",
  ]);
  expect(view.textContent).toContain("Ah Kd 7c 2s");
  expect(view.textContent).toContain("SurSvan (us)");
  const svg = view.querySelector("svg");
  expect(svg.attrs.role).toBe("img");
  expect(svg.attrs["aria-label"]).toContain("-1.5 big blinds");
  expect(svg.querySelector("polyline").attrs.points.split(" ").length).toBe(4);
  // The fallback: one row a hand, under the chart.
  const rows = view.querySelector("details").all("tr");
  expect(rows.length).toBe(1 + 3);
  expect(rows[2].textContent).toBe("2-3.00-2.00");
});

test("an opponent's name is text, whatever it holds", () => {
  const view = show("/bot/SurSvan");
  expect(view.all("img").length).toBe(0);
  expect(view.innerHTML).toContain("&lt;img src=x onerror=alert(1)>");
});

test("the release page says where the slots point and what each bot runs", () => {
  const view = show("/release");
  expect(view.textContent).toContain("Current11.0.0");
  expect(view.textContent).toContain("Previous10.9.0");
  expect(view.textContent).toContain("10.9.0, 11.0.0");
  expect(view.all("tr").length).toBe(1 + 2);
  expect(view.textContent).toContain("308e7da4");
});

test("a bot that has written no status says so", () => {
  expect(show("/bot/Nobody").textContent).toContain("This bot has written no status.");
});

test("the scoreboard has a whisker an opponent over the latest run, its numbers as text beside it", () => {
  const view = show("/scoreboard");
  expect(view.all("section").map((panel) => panel.querySelector("h2").textContent)).toEqual([
    "Latest run",
    "Trend",
  ]);
  expect(view.textContent).toContain("abc1234");
  // The champion is what the numbers mean: a promotion moves play under the same commit.
  expect(view.textContent).toContain("champion 308e7da4");
  expect(view.textContent).toContain("seed 7");
  // The Opponents table is a ladder: a head row, then the best win rate first, whatever order
  // the run played them in. Here the fixture lists nit (+12,345.6) before maniac (-5,000).
  const opponents = view.all("table")[0];
  const rows = opponents.all("tr");
  expect(rows.length).toBe(1 + 2);
  expect(rows[1].textContent).toContain("+12345.6 mbb/100");
  expect(rows[2].textContent).toContain("−5000.0 mbb/100");
  expect(rows[1].all("svg").length).toBe(1);
  // The trend keeps each run, newest first, and a run that did not play an opponent is a dash.
  const trendRows = view.all("table")[1].all("tr");
  expect(trendRows.length).toBe(1 + 2);
  expect(trendRows[1].textContent).toContain("abc1234");
  expect(trendRows[2].textContent).toContain("older99");
  expect(trendRows[2].textContent).toContain("—");
});

test("the scoreboard is a ladder, best win rate first", () => {
  // The spec asks for a win-rate ladder, not the order the run played them in. The fixture's
  // lines are nit then maniac; reversed here so the ranking cannot happen by accident.
  globalThis.location.pathname = "/scoreboard";
  const reversed = fleet();
  reversed.arena[0].rows.reverse();
  listeners.fleet({ data: JSON.stringify(reversed) });
  const rows = ids.view.all("table")[0].all("tr");
  expect(rows[1].textContent).toContain("+12345.6 mbb/100");
  expect(rows[2].textContent).toContain("−5000.0 mbb/100");
});

test("an opponent's name on the scoreboard is text too", () => {
  const view = show("/scoreboard");
  expect(view.all("b").length).toBe(0);
  expect(view.innerHTML).toContain("&lt;b>maniac&lt;/b>");
});

test("with no Arena run yet the scoreboard says how to make one", () => {
  globalThis.location.pathname = "/scoreboard";
  listeners.fleet({ data: JSON.stringify({ ...fleet(), arena: [] }) });
  expect(ids.view.textContent).toContain("svanbot arena");
});

test("a dropped stream keeps the last fleet on screen and says it dropped", () => {
  show("/");
  listeners.error();
  expect(ids.worst.textContent).toContain("The event stream dropped");
  expect(ids.view.all("article").length).toBe(2);
});

test("an open table fallback stays open when the page redraws", () => {
  const view = show("/bot/SurSvan");
  view.querySelector("details").open = true;
  const next = fleet();
  next.bots[0].hands += 1;
  listeners.fleet({ data: JSON.stringify(next) });
  expect(view.querySelector("details").open).toBe(true);
  expect(view.textContent).toContain("211");
});
