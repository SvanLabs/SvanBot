// The dossier page (V11-26), run against a stand-in for the browser: one bot's view of each
// opponent it has played, from the aggregates its status carries.
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

let dossier;
beforeAll(async () => {
  globalThis.document = {
    createElement: (tag) => new Node(tag),
    createElementNS: (_ns, tag) => new Node(tag),
  };
  dossier = await import("./dossier.js");
});

function row(overrides = {}) {
  return {
    name: "alpha_bot",
    class: "fixed",
    change: null,
    hands: 300,
    actions: 412,
    earned_pct: 62,
    ...overrides,
  };
}

function status(opponents) {
  return { bot: "SurSvan", opponents };
}

/** The page's views as one element, for its text and its parts. */
function page(views) {
  const root = new Node("main");
  root.append(...views.flat(Number.POSITIVE_INFINITY));
  return root;
}

test("a bot with no status, and one with no reads yet, say so", () => {
  expect(page(dossier.dossierPage(undefined, "SurSvan")).textContent).toContain(
    "this bot has written no status",
  );
  expect(page(dossier.dossierPage(status([]), "SurSvan")).textContent).toContain("no reads yet");
  // A status from before reads were in play carries no opponents at all.
  expect(page(dossier.dossierPage({ bot: "SurSvan" }, "SurSvan")).textContent).toContain(
    "no reads yet",
  );
});

test("held reads say the earned share is not spent, and reads in play do not", () => {
  const held = page(dossier.dossierPage(status([row()]), "SurSvan")).textContent;
  expect(held).toContain("Reads are held");
  expect(held).toContain("decides nothing yet");
  const playing = page(
    dossier.dossierPage({ ...status([row()]), reads_in_play: true }, "SurSvan"),
  ).textContent;
  expect(playing).not.toContain("Reads are held");
});

test("each opponent is a row in name order, with class, hands, actions, band and earned share", () => {
  const view = page(
    dossier.dossierPage(
      status([
        row({ name: "beta_bot", class: "unknown", hands: 12, actions: 20, earned_pct: 0 }),
        row({ name: "alpha_bot", class: "switching", change: 140, actions: 160 }),
      ]),
      "SurSvan",
    ),
  );
  const rows = view.all("tr").slice(1);
  expect(rows.map((tr) => tr.children[0].textContent)).toEqual(["alpha_bot", "beta_bot"]);
  const alpha = rows[0].textContent;
  expect(alpha).toContain("switching since hand 140");
  expect(alpha).toContain("300");
  expect(alpha).toContain("160");
  expect(alpha).toContain("read");
  expect(alpha).toContain("62%");
  const beta = rows[1].textContent;
  // Thin evidence is unknown, never fixed, and earns nothing.
  expect(beta).toContain("unknown");
  expect(beta).not.toContain("fixed");
  expect(beta).toContain("thin");
  expect(beta).toContain("0%");
});

test("the confidence band is no read, thin, or read at 30 actions", () => {
  expect(dossier.band(0)).toBe("no read");
  expect(dossier.band(29)).toBe("thin");
  expect(dossier.band(30)).toBe("read");
});

test("the earned share is a bar as wide as its percentage", () => {
  const bars = page(dossier.dossierPage(status([row({ earned_pct: 62 })]), "SurSvan")).all("meter");
  expect(bars.length).toBe(1);
  expect(bars[0].attrs.value).toBe("62");
  expect(bars[0].attrs.max).toBe("100");
});

test("the page says who an opponent is and what the share measures", () => {
  const text = page(dossier.dossierPage(status([row()]), "SurSvan")).textContent;
  expect(text).toContain("venue name");
  expect(text).toContain("a rename is a new opponent");
  expect(text).toContain("not a measured harm");
});

test("the index lists each bot's dossier with how many opponents it reads", () => {
  const fleet = {
    bots: [status([row(), row({ name: "beta_bot" })]), { bot: "AlphaSvan", opponents: [] }],
  };
  const view = page(dossier.dossierIndex(fleet));
  const links = view.all("a").map((a) => [a.attrs.href, a.textContent]);
  expect(links).toEqual([
    ["/bot/AlphaSvan/dossier", "AlphaSvan"],
    ["/bot/SurSvan/dossier", "SurSvan"],
  ]);
  expect(view.textContent).toContain("2 opponents");
  expect(view.textContent).toContain("0 opponents");
});
