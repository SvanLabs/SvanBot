// The redraw (V11-44): the page is rebuilt every second, and only what changed may change on
// screen, so a reader's focus, an open fallback and a note being typed all survive it.
import { beforeAll, expect, test } from "bun:test";
import { Node, standIn } from "./testdom.js";

let h;
let patch;

beforeAll(async () => {
  globalThis.document = standIn;
  ({ h } = await import("./dom.js"));
  ({ patch } = await import("./patch.js"));
});

function view(...children) {
  const main = new Node("main");
  main.append(...h("main", {}, children).children);
  return main;
}

test("an unchanged node is the same node after a redraw, and a changed one takes the new text", () => {
  const shown = view(h("section", { class: "card" }, h("h2", {}, "Now"), h("p", {}, "12 hands")));
  const [card] = shown.children;
  const [title] = card.children;
  patch(shown, view(h("section", { class: "card" }, h("h2", {}, "Now"), h("p", {}, "13 hands"))));
  expect(shown.children[0]).toBe(card);
  expect(card.children[0]).toBe(title);
  expect(shown.textContent).toBe("Now13 hands");
});

test("a node whose attributes changed is replaced, and so is a list that changed length", () => {
  const shown = view(h("span", { class: "pill ok" }, "playing"), h("ul", {}, h("li", {}, "a")));
  const [pill, list] = shown.children;
  patch(
    shown,
    view(
      h("span", { class: "pill bad" }, "offline"),
      h("ul", {}, h("li", {}, "a"), h("li", {}, "b")),
    ),
  );
  expect(shown.children[0]).not.toBe(pill);
  expect(shown.children[0].attrs.class).toBe("pill bad");
  expect(shown.children[1]).toBe(list);
  expect(list.all("li").length).toBe(2);
});

test("a kept node stays as the reader left it while its key holds, and goes when the key changes", () => {
  const note = (bot, text) => h("textarea", { "data-keep": bot }, text);
  const shown = view(note("SurSvan", "saved"));
  const [typed] = shown.children;
  typed.children = ["what the reader is typing"];
  patch(shown, view(note("SurSvan", "saved")));
  expect(shown.children[0]).toBe(typed);
  expect(typed.textContent).toBe("what the reader is typing");
  patch(shown, view(note("Svanar", "theirs")));
  expect(shown.children[0]).not.toBe(typed);
  expect(shown.textContent).toBe("theirs");
});

test("a value marked to flash flashes once when its text changes, and not when it holds", () => {
  const value = (text) => h("strong", { class: "big", "data-flash": "" }, text);
  const shown = view(value("12"), h("span", {}, "1s ago"));
  const [big, age] = shown.children;
  const flashed = [];
  big.animate = (_frames, options) => flashed.push(["big", options.duration]);
  age.animate = () => flashed.push(["age"]);
  patch(shown, view(value("12"), h("span", {}, "2s ago")));
  expect(flashed).toEqual([]);
  patch(shown, view(value("13"), h("span", {}, "3s ago")));
  expect(flashed).toEqual([["big", 900]]);
  expect(shown.children[0]).toBe(big);
  expect(big.textContent).toBe("13");
});

test("an open fallback stays open: whether it is open is the reader's, not the page's", () => {
  const shown = view(h("details", {}, h("summary", {}, "as a table"), h("p", {}, "1")));
  const [details] = shown.children;
  details.open = true;
  details.setAttribute("open", "");
  patch(shown, view(h("details", {}, h("summary", {}, "as a table"), h("p", {}, "2"))));
  expect(shown.children[0]).toBe(details);
  expect(details.open).toBe(true);
  expect(details.textContent).toBe("as a table2");
});
