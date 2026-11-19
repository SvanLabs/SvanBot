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
    ranges: [{ opponent: 1, masses: [0, 0.1, 0.2, 0.3, 0.2, 0.1, 0.1, 0] }],
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
    ranges: [{ opponent: 1, masses: [0.1, 0.1, 0.1, 0.1, 0.1, 0.2, 0.2, 0.1] }],
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
  expect(text).toContain("exploit 0.40");
});

test("opponents are counted from one, and each range is shown", async () => {
  const text = replay.decisionView(resolved()).textContent;
  expect(text).toContain("Opponent 1");
});

test("a decision with no action names still says which index was taken", async () => {
  const text = replay.decisionView(resolved({ actions: undefined })).textContent;
  expect(text).toContain("took action 2 of 3");
});
