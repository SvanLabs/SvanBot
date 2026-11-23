// A stand-in for the browser's DOM, for the page's tests: enough to build the views, patch one
// into another, and read them back as text or as markup. A text child is a plain string.

/** An element that keeps its attributes and children, and reads back as text or as markup. */
export class Node {
  constructor(tag) {
    this.tag = tag;
    this.nodeName = tag;
    this.attrs = {};
    this.children = [];
    this.open = false;
  }
  setAttribute(name, value) {
    this.attrs[name] = value;
  }
  getAttribute(name) {
    return this.attrs[name] ?? null;
  }
  getAttributeNames() {
    return Object.keys(this.attrs);
  }
  removeAttribute(name) {
    delete this.attrs[name];
  }
  append(...children) {
    this.children.push(...children);
  }
  replaceChildren(...children) {
    this.children = children;
  }
  replaceChild(fresh, old) {
    this.children[this.children.indexOf(old)] = fresh;
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

/** Every element under `node` whose class list holds `name`. */
export function byClass(node, name, found = []) {
  for (const child of node.children) {
    if (typeof child !== "string") {
      if ((child.attrs.class ?? "").split(" ").includes(name)) {
        found.push(child);
      }
      byClass(child, name, found);
    }
  }
  return found;
}

/** The stand-in document the page's modules build with. */
export const standIn = {
  createElement: (tag) => new Node(tag),
  createElementNS: (_, tag) => new Node(tag),
};
