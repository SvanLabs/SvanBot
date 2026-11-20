// The page's few elements, shared by the fleet view and the replayer: an element built from
// attributes and children, a definition list, a table, and a card.

const SVG = "http://www.w3.org/2000/svg";

/** An element with attributes and children. A string child is text. */
export function h(tag, attrs = {}, ...children) {
  const svg = ["svg", "polyline", "line", "title"].includes(tag) && attrs.svg !== false;
  const node = svg ? document.createElementNS(SVG, tag) : document.createElement(tag);
  for (const [name, value] of Object.entries(attrs)) {
    if (name !== "svg" && value !== null && value !== undefined) {
      node.setAttribute(name, String(value));
    }
  }
  // A view is built from lists of lists: flatten them all the way, or a list would be appended
  // as its own text.
  for (const child of children.flat(Number.POSITIVE_INFINITY)) {
    if (child !== null && child !== undefined) {
      // A number is text, as it would be in the DOM: `append(7)` makes a text node there too.
      node.append(typeof child === "object" ? child : String(child));
    }
  }
  return node;
}

/** A definition list from pairs of a term and what it is. */
export function facts(pairs) {
  return h(
    "dl",
    {},
    pairs.flatMap(([term, value]) => [h("dt", {}, term), h("dd", {}, value)]),
  );
}

export function table(caption, heads, rows) {
  return h(
    "table",
    {},
    h("caption", { class: "muted" }, caption),
    h(
      "thead",
      {},
      h(
        "tr",
        {},
        heads.map(([text, cls]) => h("th", { scope: "col", class: cls }, text)),
      ),
    ),
    h("tbody", {}, rows),
  );
}

export function panel(title, ...children) {
  return h("section", { class: "card" }, h("h2", {}, title), children);
}
