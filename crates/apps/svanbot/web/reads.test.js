// The opponent stats panel (V11-121): every rate beside its counts, an interval past the floor,
// and the floor said out loud — rendered from what the app fetched, against a browser stand-in.
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
  replaceChildren(...children) {
    this.children = children;
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

let reads;
beforeAll(async () => {
  globalThis.document = {
    createElement: (tag) => new Node(tag),
    createElementNS: (_ns, tag) => new Node(tag),
  };
  reads = await import("./reads.js");
});

function stat(overrides = {}) {
  return {
    hits: 12,
    opportunities: 40,
    rate: 0.3,
    interval: [0.18, 0.45],
    floor: false,
    ...overrides,
  };
}

function opponent(overrides = {}) {
  return {
    name: "Villain",
    hands: 120,
    class: "fixed",
    earned: 0.5,
    stats: {
      vpip: stat(),
      pfr: stat({ hits: 0, opportunities: 0, rate: null, interval: null, floor: true }),
      fold_to_bet: stat(),
      aggression: stat(),
    },
    situations: [
      { street: "preflop", facing: false, n: 40, sparse: false },
      { street: "flop", facing: true, n: 2, sparse: true },
    ],
    hand_ids: ["h-1", "h-2"],
    ...overrides,
  };
}

/** The panel's nodes as one element, for its text. */
function text(nodes) {
  const root = new Node("main");
  root.append(...(Array.isArray(nodes) ? nodes.flat(Number.POSITIVE_INFINITY) : [nodes]));
  return root.textContent;
}

test("a rate is never shown without its counts", () => {
  expect(reads.statText(stat())).toBe("12 of 40 · 30% (18%–45%)");
  // Under the floor the counts stay and a number does not appear.
  const thin = reads.statText(
    stat({ hits: 1, opportunities: 1, rate: null, interval: null, floor: true }),
  );
  expect(thin).toBe("1 of 1 · not enough observations");
  expect(thin).not.toContain("%");
});

test("the panel says what it is doing, and what an unreadable store means", () => {
  expect(text(reads.readsPanel("Alpha", undefined))).toContain("Reading the store");
  expect(text(reads.readsPanel("Alpha", { error: "The store could not be read." }))).toContain(
    "could not be read",
  );
  expect(text(reads.readsPanel("Alpha", { opponents: [] }))).toContain("no reads yet");
});

test("an opponent's stats carry counts, the floor, and thin situations", () => {
  const shown = text(reads.readsPanel("Alpha", { opponents: [opponent()] }));
  expect(shown).toContain("Villain");
  expect(shown).toContain("12 of 40");
  expect(shown).toContain("0 of 0 · not enough observations");
  expect(shown).toContain("flop facing a bet: 2 (thin)");
  // The hands behind the stats link to the replayer.
  expect(shown).toContain("2 hands behind these stats");
});
