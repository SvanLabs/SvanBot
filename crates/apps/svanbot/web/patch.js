// The redraw (V11-44): the page builds its whole view every second, and `patch` changes only
// what differs, node by node. An unchanged node is never touched, so the reader's focus, scroll,
// an open fallback and a note being typed all survive the second's redraw.

function isText(node) {
  return typeof node === "string" || node.nodeType === 3;
}

function textOf(node) {
  return typeof node === "string" ? node : node.data;
}

/** Whether two elements carry the same attributes. `open` is the reader's, not the page's. */
function sameAttributes(shown, next) {
  const names = (node) =>
    node
      .getAttributeNames()
      .filter((name) => name !== "open")
      .sort();
  const before = names(shown);
  const after = names(next);
  return (
    before.length === after.length &&
    before.every(
      (name, at) => name === after[at] && shown.getAttribute(name) === next.getAttribute(name),
    )
  );
}

/** A changed value's one flash: a glow that fades, unless the reader asked for less motion. */
const FLASH = [
  { filter: "brightness(1.9) drop-shadow(0 0 0.6rem currentColor)" },
  { filter: "none" },
];

function flash(node) {
  if (globalThis.matchMedia?.("(prefers-reduced-motion: reduce)").matches) {
    return;
  }
  node.animate?.(FLASH, { duration: 900, easing: "ease-out" });
}

/**
 * Makes `shown`'s children match `next`'s, keeping every node that did not change. A list that
 * changed length is laid again whole. An element with `data-keep` stays as the reader left it
 * while its key is the same: a note being typed is the reader's, not the stream's. An element
 * with `data-flash` flashes once when its text changes.
 */
export function patch(shown, next) {
  const before = [...shown.childNodes];
  const after = [...next.childNodes];
  if (before.length !== after.length) {
    shown.replaceChildren(...after);
    return;
  }
  before.forEach((node, at) => {
    const fresh = after[at];
    if (isText(node) || isText(fresh)) {
      if (!(isText(node) && isText(fresh) && textOf(node) === textOf(fresh))) {
        shown.replaceChild(fresh, node);
        if ((shown.getAttribute?.("data-flash") ?? null) !== null) {
          flash(shown);
        }
      }
      return;
    }
    if (node.nodeName !== fresh.nodeName || !sameAttributes(node, fresh)) {
      shown.replaceChild(fresh, node);
      return;
    }
    if (node.getAttribute("data-keep") === null && node.innerHTML !== fresh.innerHTML) {
      patch(node, fresh);
    }
  });
}
