// The page's element helper: an icon or a filled chart is drawn in SVG, so its shapes must be made
// in the SVG namespace, or the browser draws nothing.
import { beforeAll, expect, test } from "bun:test";
import { Node } from "./testdom.js";

const made = [];
let h;

beforeAll(async () => {
  globalThis.document = {
    createElement: (tag) => {
      made.push(["html", tag]);
      return new Node(tag);
    },
    createElementNS: (_, tag) => {
      made.push(["svg", tag]);
      return new Node(tag);
    },
  };
  ({ h } = await import("./dom.js"));
});

test("every shape an icon or a filled chart uses is made in the SVG namespace", () => {
  made.length = 0;
  h(
    "svg",
    { viewBox: "0 0 24 24" },
    h("path", { d: "M0 0h1" }),
    h("circle", { r: 1 }),
    h("rect", { width: 1 }),
    h("polygon", { points: "0,0 1,1" }),
    h("g", {}, h("polyline", { points: "0,0" }), h("line", {})),
  );
  expect(made.every(([space]) => space === "svg")).toBe(true);
  made.length = 0;
  // A child is built before the element that holds it.
  h("div", {}, h("span", {}, "text"));
  expect(made).toEqual([
    ["html", "span"],
    ["html", "div"],
  ]);
});
