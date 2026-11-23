// The repo's front page and the GitHub Pages site, checked at their one seam: a tree in, the
// problems out. A link that resolves nowhere is a broken page on GitHub, and nothing else tells.
import { afterAll, expect, test } from "bun:test";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { problems } from "./site";

const scratch = mkdtempSync(join(tmpdir(), "svanbot-site-"));
afterAll(() => rmSync(scratch, { recursive: true, force: true }));

const PAGE = (body: string) =>
  `<!doctype html><html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><title>T</title></head><body>${body}</body></html>`;

/** A clone with a README and a site, each file as given. */
function tree(files: Record<string, string>): string {
  const root = mkdtempSync(join(scratch, "clone-"));
  for (const [path, text] of Object.entries(files)) {
    mkdirSync(join(root, path, ".."), { recursive: true });
    writeFileSync(join(root, path), text);
  }
  return root;
}

test("a relative link or asset that resolves to nothing is named, with the file it is in", () => {
  const root = tree({
    "README.md": "See [the runbook](deploy/README.md) and [the map](docs/missing.md).\n",
    "deploy/README.md": "# Runbook\n",
    "site/index.html": PAGE('<link rel="stylesheet" href="style.css"><img src="arena.svg">'),
    "site/style.css": "body {}\n",
  });
  expect(problems(root)).toEqual([
    "README.md links to docs/missing.md, which does not exist",
    "site/index.html links to arena.svg, which does not exist",
  ]);
});

test("external links, anchors and a link's own anchor are not files to find", () => {
  const root = tree({
    "README.md":
      "[venue](https://docs.openpoker.ai/llms-full.txt) [up](#how-a-bot-decides) [adr](docs/a.md#context) <mailto:x@y.z>\n",
    "docs/a.md": "# A\n",
    "site/index.html": PAGE(
      '<a href="https://github.com/SvanLabs/SvanBot">repo</a><a href="#arena">a</a>',
    ),
  });
  expect(problems(root)).toEqual([]);
});

test("a page without a title or a viewport is named: it renders badly in a tab and on a phone", () => {
  const root = tree({
    "README.md": "# R\n",
    "site/index.html": "<!doctype html><html><head></head><body></body></html>",
  });
  expect(problems(root)).toEqual([
    "site/index.html has no <title>",
    "site/index.html has no viewport meta",
  ]);
});

test("the repo's own README and site have no broken links", () => {
  expect(problems(join(import.meta.dir, ".."))).toEqual([]);
});
