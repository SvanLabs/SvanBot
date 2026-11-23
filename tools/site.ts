// The repo's front page and its GitHub Pages site (`site/`), checked before they reach GitHub:
//   bun tools/site.ts
// Every relative link and asset in `README.md` and in each page of `site/` must name a file that
// exists, and every page must carry a <title> and a viewport meta, so it reads in a tab and on a
// phone. A link out of the repo (https:, mailto:) or to an anchor on the same page is not a file.
// Exits 1 and names each problem when there is one.
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { dirname, join, relative } from "node:path";

/** The links a Markdown or HTML text names: Markdown targets, and `href`/`src` attributes. */
export function links(text: string): string[] {
  const found: string[] = [];
  for (const match of text.matchAll(/\]\(([^)\s]+)(?:\s+"[^"]*")?\)/g))
    if (match[1]) found.push(match[1]);
  for (const match of text.matchAll(/\b(?:href|src)="([^"]+)"/g))
    if (match[1]) found.push(match[1]);
  return found;
}

/** The file a link names, without its anchor or query, or undefined when it names no file here. */
function target(link: string): string | undefined {
  if (/^[a-z][a-z0-9+.-]*:/i.test(link) || link.startsWith("#") || link.startsWith("//")) {
    return undefined;
  }
  const path = link.split("#")[0]?.split("?")[0] ?? "";
  return path === "" ? undefined : decodeURIComponent(path);
}

/** Everything wrong with the README and the site under `root`, one line a problem. */
export function problems(root: string): string[] {
  const found: string[] = [];
  const files = ["README.md"];
  const site = join(root, "site");
  if (existsSync(site)) {
    for (const name of readdirSync(site).sort()) {
      if (name.endsWith(".html")) files.push(join("site", name));
    }
  }
  for (const file of files) {
    const path = join(root, file);
    if (!existsSync(path)) continue;
    const text = readFileSync(path, "utf8");
    for (const link of links(text)) {
      const named = target(link);
      if (named === undefined) continue;
      const resolved = named.startsWith("/") ? join(root, named) : join(dirname(path), named);
      if (!existsSync(resolved)) {
        found.push(`${file} links to ${relative(dirname(path), resolved)}, which does not exist`);
      }
    }
    if (file.endsWith(".html")) {
      if (!/<title>[^<]+<\/title>/.test(text)) found.push(`${file} has no <title>`);
      if (!/<meta name="viewport"/.test(text)) found.push(`${file} has no viewport meta`);
    }
  }
  return found;
}

if (import.meta.main) {
  const found = problems(join(import.meta.dir, ".."));
  for (const problem of found) console.error(`site: ${problem}`);
  process.exit(found.length === 0 ? 0 : 1);
}
