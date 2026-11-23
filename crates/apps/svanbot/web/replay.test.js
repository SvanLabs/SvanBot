// The replayer, run against a stand-in for the browser: enough of a DOM to build the views, and
// a trace handed to it by hand. It holds what the page says about a decision, which is the part
// of V11-24 a reader actually sees.
import { beforeAll, expect, test } from "bun:test";

/** An element that keeps its attributes and children, and reads back as text. */
class Node {
  constructor(tag) {
    this.tag = tag;
    this.attrs = {};
    this.children = [];
  }
  setAttribute(name, value) {
    this.attrs[name] = value;
  }
  append(...children) {
    this.children.push(...children);
  }
  get textContent() {
    return this.children
      .map((child) => (typeof child === "string" ? child : child.textContent))
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
}

beforeAll(() => {
  globalThis.document = {
    createElement: (tag) => new Node(tag),
    createElementNS: (_ns, tag) => new Node(tag),
  };
});

/** A re-solved decision, as `/api/trace` writes one. */
function resolved(overrides = {}) {
  return {
    street: "flop",
    branch: "resolved",
    hole: "AhKh",
    board: "QhJhTh",
    values: [0, 400, -25],
    strategy: [0, 0.75, 0.25],
    taken: 1,
    actions: ["check", "bet 200", "fold"],
    ranges: [{ seat: 3, masses: [0, 0.1, 0.2, 0.3, 0.2, 0.1, 0.1, 0] }],
    ...overrides,
  };
}

/** A priced decision, as `/api/trace` writes one. */
function priced(overrides = {}) {
  return {
    street: "preflop",
    branch: "priced",
    hole: "AhAs",
    board: "",
    equity: 0.7,
    price: 0.25,
    exploit: null,
    ranges: [{ seat: 2, masses: [0.1, 0.1, 0.1, 0.1, 0.1, 0.2, 0.2, 0.1] }],
    ...overrides,
  };
}

// The module is imported once, after the stand-in document exists, and every test builds from it.
let replay;
beforeAll(async () => {
  replay = await import("./replay.js");
});

test("a resolved decision says what was taken by name, not by index", async () => {
  const text = replay.decisionView(resolved()).textContent;
  expect(text).toContain("Resolved");
  expect(text).toContain("took bet 200");
  expect(text).not.toContain("took action 2");
});

test("a resolved decision's value is in chips, and the mix is a share", async () => {
  const text = replay.decisionView(resolved()).textContent;
  expect(text).toContain("+400.0 chips, mixed 75%");
  // The chips a value is in are chips; the page must not claim they are big blinds or a share
  // of the pot.
  expect(text).not.toContain("bb");
  expect(text).not.toContain("of the pot");
});

test("a decision says what the bot held and what board it saw", async () => {
  const text = replay.decisionView(resolved()).textContent;
  expect(text).toContain("Ah Kh");
  expect(text).toContain("board Qh Jh Th");
});

test("a preflop decision says preflop rather than an empty board", async () => {
  const text = replay.decisionView(priced()).textContent;
  expect(text).toContain("Ah As");
  expect(text).toContain("preflop");
});

test("a priced decision says its price, and that the baseline exploits nothing", async () => {
  const text = replay.decisionView(priced()).textContent;
  expect(text).toContain("Priced");
  expect(text).toContain("equity 70.0%");
  expect(text).toContain("25.0% of the pot");
  expect(text).toContain("baseline read, no exploit");
});

test("a priced decision reads by an exploit budget when a fit passed the gate", async () => {
  const text = replay.decisionView(priced({ exploit: 0.4 })).textContent;
  expect(text).toContain("configured exploit budget 0.40");
});

test("a range names the seat it read, so it matches the table", async () => {
  const text = replay.decisionView(resolved()).textContent;
  expect(text).toContain("Seat 3");
});

test("a range with no seat says so rather than numbering the opponents itself", async () => {
  // A spot a sim built, or a hand traced before the seat was carried: the page must not invent
  // "Opponent 1" from the position in the list, which names nobody at the table.
  const text = replay.decisionView(
    resolved({ ranges: [{ seat: null, masses: [1, 0, 0, 0, 0, 0, 0, 0] }] }),
  ).textContent;
  expect(text).toContain("Seat unknown");
  expect(text).not.toContain("Opponent 1");
});

test("a range read by its opponent's own read names their class and the budget it earned", () => {
  const text = replay.decisionView(
    priced({
      ranges: [
        { seat: 3, masses: [1, 0, 0, 0, 0, 0, 0, 0], class: "fixed", trust: 0.62 },
        { seat: 4, masses: [1, 0, 0, 0, 0, 0, 0, 0] },
      ],
    }),
  ).textContent;
  expect(text).toContain("Seat 3: fixed, 62% of the budget");
  // The population's read names no class.
  expect(text).toContain("Seat 4");
  expect(text).not.toContain("Seat 4:");
});

test("a held read names its class and says it decided nothing", () => {
  const text = replay.decisionView(
    priced({ ranges: [{ seat: 3, masses: [1, 0, 0, 0, 0, 0, 0, 0], class: "fixed" }] }),
  ).textContent;
  expect(text).toContain("Seat 3: fixed, read held");
  expect(text).not.toContain("of the budget");
});

test("a decision with no action names still says which index was taken", async () => {
  const text = replay.decisionView(resolved({ actions: undefined })).textContent;
  expect(text).toContain("took action 2 of 3");
});

/** The page's title and headings, which is the street grouping a reader sees. */
function headings(page) {
  return page
    .flat(Infinity)
    .filter((node) => node && typeof node === "object")
    .flatMap((node) =>
      node.tag === "h1" || node.tag === "h2" ? [node] : node.all("h1").concat(node.all("h2")),
    )
    .map((node) => node.textContent.trim());
}

test("a hand's page groups its decisions by street, in the order they were made", async () => {
  // The ticket's item 8: the page is the reason the trace groups by street. Two on the flop
  // stay two, and the streets read preflop, flop, turn in the order they came.
  const page = replay.handPage(
    {
      trace: {
        traced: true,
        decisions: [
          priced(),
          resolved({ street: "flop" }),
          resolved({ street: "flop", taken: 0 }),
          resolved({ street: "turn", board: "QhJhTh2c" }),
        ],
      },
    },
    "Alpha",
    "h-1",
  );
  expect(headings(page)).toEqual(["Alpha h-1", "Preflop", "Flop", "Turn"]);
});

test("a hand with no trace says so rather than showing an empty page", async () => {
  // The ticket is explicit that an untraced hand is not a hand that made no decisions; the page
  // must say which it is holding.
  const page = replay.handPage(
    { trace: { traced: false, decisions: [], note: "no trace — played before traces" } },
    "Alpha",
    "h-1",
  );
  expect(
    page
      .flat(Infinity)
      .map((node) => node?.textContent ?? "")
      .join(" "),
  ).toContain("no trace");
});
