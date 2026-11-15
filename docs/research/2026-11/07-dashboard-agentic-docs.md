# 07: Dashboard UX, the agentic workflow and the docs tree (v11.0.0 research)

Scope: everything here is read-only research against `/home/svan/SvanBot` at `4df58e6`, plus web sources listed at the end. Nothing was built or edited.

## Part A: the dashboard

### What is there now

- **Stack:** React 19.3, Vite 8, TypeScript 7, `lucide-react`, three self-hosted font families (Inter, Barlow Condensed, IBM Plex Mono) and Playwright for 23 browser specs (`web/package.json`, `web/tests/`). There is no chart library. Six charts are drawn by hand in inline SVG (`fun.tsx`, `lab.tsx`, `playercard.tsx`, `training.tsx`).
- **Size:** 4,046 lines of TS/TSX in `web/src`, but 259 lines run past 200 characters, so the code is denser than the line count suggests. `style.css` is one 125 KB file of 535 mostly minified lines, with 502 distinct hex literals and only 51 distinct custom properties.
- **Structure:** one `App` in `main.tsx` that holds about 20 `useState` hooks. It feeds a 31-widget board laid out in three columns (`WIDGET_DEFAULTS`) and six view tabs: Live, Opponents, Learning, Results, System, All (`widgets.tsx` `VIEWS`). Panels collapse, can be rearranged, and their layout persists to `/api/layout`.
- **Data:** one SSE stream (`/api/events`) plus about 25 independent `usePoll` timers at 20 s, 30 s, 60 s, 120 s and 300 s. Each timer has its own staleness note (`StaleNote`). The contract is `web/src/types.ts` and `docs/SPEC-dashboard.md`.
- **Theme:** dark is the default and light is set with `[data-theme="light"]`. The page has no `prefers-color-scheme` query, so it never follows the OS. It does have ten `prefers-reduced-motion` blocks, which is good.

### Concrete UX problems

1. **The page has no answer to "is anything wrong?"** Operator-dashboard guidance puts one status verdict top-left and keeps it to a few status indicators, with trends and tables below ([InfluxData](https://www.influxdata.com/blog/5-dashboard-design-best-practices/), [Domo](https://www.domo.com/learn/article/dashboard-design-examples-best-practices)). Here the hierarchy is flat: an 86 px brand bar, a breadcrumb row and an overview of four stats, then 31 panels that all carry the same visual weight. Each one has the same 3 px gradient stripe and 19 px display heading. Faults are scattered across panels (Health, Updates, Host, Monitor, Activity) instead of being raised to the top.
2. **Too many panels, and they overlap.** `fleet-race`, `highlights`, `monitor` and `activity` each sit on two views. Live alone is 2,507 px tall at 1440 px wide, and "All" is 6,743 px (SPEC-dashboard). Working-memory guidance puts the useful number of elements at 5–9 per screen ([Domo](https://www.domo.com/learn/article/dashboard-design-examples-best-practices)). The "fun" panels in `fun.tsx` (590 lines: badges, stories, toasts, races) sit beside operational ones, so an operator has to read past entertainment to find faults.
3. **Every panel has its own freshness clock.** About 25 pollers at five cadences means each panel can be stale in its own way. The banner reads "Update failed (...); showing data from HH:MM" once per panel. Better: one global connection and freshness indicator, with panels marked stale only by de-emphasis.
4. **Type is too small and too varied.** The base size is 12 px. The most common sizes are 11 px (54 uses), 11.5 px (33), 10 px (22) and 9 px (10), so data text is commonly 9–11.5 px in a mono face. It is hard to read on a TV or a laptop, and it erodes the hierarchy because everything is small.
5. **The token system leaks.** There are 502 raw hex values against 51 tokens. `:root` is declared in four separate places (lines 1, 124, 209, 416), and the light theme overrides only some of them, so hard-coded colours (for example `.positive{color:#8cd6ad}` and the topbar's `#191b15`) stay dark-theme values in light mode. The palette (amber, crimson, mint, red on olive black) gives win/loss meaning to red and green. That fails for the roughly 8% of men with red-green colour blindness unless sign and shape carry the meaning too ([WCAG 1.4.1 via Smashing](https://smashingmagazine.com/2022/07/accessibility-first-approach-chart-visual-design/)).
6. **The charts are not accessible.** The hand-rolled SVGs have no data-table alternative and their tooltips are hover-only. The guidance asks for text 4.5:1 against its background, marks 3:1 against neighbours, never colour alone, keyboard-reachable tooltips and a table fallback ([Salt DS](https://www.saltdesignsystem.com/salt/foundations/data-visualization/chart-accessibility), [Highcharts/BOIA](https://www.boia.org/blog/web-data-visualizations-and-accessibility-5-tips)).
7. **Decoration competes with data.** Gradient panel headings, a glowing status dot, an accent stripe on every panel and table themes ("arena", "midnight") with deal and chip animations all cost attention. "Jarring updates are worse than no updates" ([Domo](https://www.domo.com/learn/article/dashboard-design-examples-best-practices)). The animations belong on the public TV, not the operator's view.

### Proposed direction for v11

- **Information hierarchy in three bands.**
  1. A sticky **status strip**, about 48 px. It shows a fleet verdict (OK / degraded / down, with the reason), seats playing out of seats total, the build commit against the checkout (as `verify-install` computes it), learner and analyst liveness, and a single data-freshness age.
  2. **Per-bot cards.** For each bot: mode, table, rank and gap to fourth, score velocity and the last error. Each card shows only what someone would act on.
  3. **Drill-down pages**, not tabs over one board: Table/Live, Opponents, Learning, Season, System. Each page holds at most about 6 panels.

  Drop the 31-widget drag-and-drop board and the "All" view. Fixed layouts are easier to test and to reason about, and `/api/layout` plus `svan-layout:v1` can be deleted. Fun panels move to the public TV or to a "Highlights" page.
- **One live channel.** Push every fast-changing thing over the existing SSE stream as typed events, and keep polling only for slow facts (host and releases, every 60 s or more). Show one freshness indicator. This removes most of the 25 timers.
- **Design tokens, done once.** Keep one `tokens.css` with three layers: primitives (the palette), semantic tokens (`--surface-1..3`, `--text-1..2`, `--border`, `--status-ok/warn/crit`, `--delta-pos/neg`, `--series-1..6`) and component tokens. Dark is the default. Light comes from `prefers-color-scheme`, with a `[data-theme]` override. A lint step can fail the build on any hex literal outside `tokens.css`. The W3C DTCG format reached its first stable version (2025.10) and has a Resolver module made for light/dark contexts. If tokens ever need to feed both the CSS and the Rust-rendered pages, author them as DTCG JSON ([designtokens.org](https://www.designtokens.org/), [Resolver 2025.10](https://www.w3.org/community/reports/design-tokens/CG-FINAL-resolver-20251028/)). Validate the palette in both themes at 4.5:1 for text and 3:1 for marks.
- **Type scale.** Use four steps (12 / 14 / 18 / 28 px), tabular figures for numbers, and the mono face only for ids and hashes. Drop Barlow Condensed and keep two families.
- **Charts.** Win/loss shows a sign (`+`/`−`) and shape as well as colour. Each chart gets a "view as table" toggle and keyboard focus on its points. For time series, either keep the hand-written SVG with a small shared `<Spark>`/`<Line>` primitive (no dependency), or adopt **uPlot** (about 50 KB, canvas, and the fastest at streaming updates in the 2026 ApexCharts benchmark, though it has no real scatter mode) ([ApexCharts benchmark](https://apexcharts.com/blog/javascript-chart-library-benchmark/), [chartts survey](https://chartts.pages.dev/blog/best-lightweight-chart-libraries)). Avoid Recharts, which is about 500 KB of SVG and slow on updates.
- **Stack.** React 19 + Vite + TS is fine and is not the problem. The lightweight option is Preact with signals behind `preact/compat`, which suits an SSE-driven store well. That is my inference, since no source compared the two directly. Changing frameworks buys little compared with fixing the hierarchy and tokens, so the recommendation is to keep React and replace the 20-`useState` `App` with one SSE-fed store (`useSyncExternalStore`) and route-level pages. Keep Playwright, adding one axe accessibility check and one contrast check per page.

## Part B: the agentic workflow and the docs

### What is there now

- `CLAUDE.md` (94 lines) names "three that bite" and points to `AGENTS.md`. `AGENTS.md` (188 lines) is the brief.
- `.claude/` holds 2 subagents (`reviewer` on sonnet, `boundaries` on haiku, both read-only), 4 commands (`gate`, `golden`, `measure`, `new-file`), 4 skills (`ship-patch`, `verify-install`, `restore-store`, `public-tv`) and one PostToolUse hook (`provenance-after-commit.py`). It also has a `worktrees/` folder with **9 agent worktrees, 3 of them locked, using 22 GB** (`du -sh .claude/worktrees`). Each worktree carries its own `target/dev`.
- `docs/agents/` (issue tracker, labels, domain, local review) is there for the Matt Pocock skills. There are 25 top-level `docs/*.md` files, 5,531 lines in all, plus `superpowers/`, `studies/` and `adr/`. There is also a stray `.openclaude/` and a `.scratch/`.

### Findings against 2026 best practice

1. **Claude Code is not reading `AGENTS.md` by default.** Claude Code reads `AGENTS.md` only when there is no `CLAUDE.md`. If both exist it loads "your CLAUDE.md files only", unless `CLAUDE.md` imports `AGENTS.md` ([Claude Code memory docs](https://code.claude.com/docs/en/memory#agents-md)). `CLAUDE.md` has no `@AGENTS.md` import. So the two hard invariants (`valid_actions`, paired simulation), the layering rule and the branch rule are **not in context** for a Claude session until the model chooses to open the file. This is the single most important fix: make `CLAUDE.md` a few lines of Claude-only notes plus `@AGENTS.md`, or delete it.
2. **`CLAUDE.md` spends its lines on meta-text.** About half of it explains why it is short. Anthropic: "every line in it takes a little of Claude's attention on every task", so keep only what Claude cannot work out from the code, and stay under 200 lines ([best practices](https://code.claude.com/docs/en/best-practices)). `AGENTS.md` is at 188 lines and also carries rationale that `LESSONS.md` already holds.
3. **Commands should become skills.** "Custom commands have been merged into skills." Skills can bundle scripts and load progressively, and a skill wins a name clash ([skills docs](https://code.claude.com/docs/en/skills)). Move `gate`, `golden`, `measure` and `new-file` to `.claude/skills/<name>/SKILL.md`. Mark `golden` (it regenerates a snapshot) and `ship-patch` landing with `disable-model-invocation: true`, because they have side effects.
4. **Hard rules belong in hooks, not prose.** "If something has to happen every time, make it a hook, a deny rule, or a managed setting" ([best practices](https://code.claude.com/docs/en/best-practices)). The rule repeated most often here, never building into `target/release`, is still prose. Candidates:
   - a `PreToolUse` Bash hook that exits 2 on any `cargo` call without `CARGO_TARGET_DIR=target/dev`, or on any write to `target/release`;
   - a `PreToolUse` deny on `UPDATE_GOLDEN=1` and on `check-file-size.sh --baseline`;
   - a `Stop` hook running `scripts/check.sh commit` (exit 2 sends Claude back with the errors);
   - `permissions.deny` for `git push origin main` and for edits under `artifacts/`.

   Keep the set small. Anthropic warns that dozens of hooks usually means preferences have been hard-coded.
5. **Worktree hygiene is failing in practice.** The 22 GB comes from nine full `target/dev` trees. `AGENTS.md` rightly forbids a target directory shared *across* checkouts (#481: binaries bake in `CARGO_MANIFEST_DIR`), and the community advice to share one `CARGO_TARGET_DIR` across worktrees conflicts with that ([cargo-worktree](https://users.rust-lang.org/t/announcements-cargo-worktree-fix-build-isolation-in-git-worktrees/139192)). The fix is to keep per-worktree targets and add a shared compiler cache: `RUSTC_WRAPPER=sccache`, which caches by content and so is safe across paths ([claudio Rust tutorial](https://iron-ham.github.io/claudio/tutorials/rust-development)). Also:
   - set `cleanupPeriodDays` and run `git worktree prune` from a script;
   - add a `.worktreeinclude` for `.env`, and `worktree.symlinkDirectories` for `web/node_modules` ([worktrees](https://code.claude.com/docs/en/common-workflows#run-parallel-sessions-with-worktrees));
   - give each parallel agent a distinct dashboard port and a scratch `artifacts/` directory, because worktrees do not isolate ports or databases ([Upsun](https://developer.upsun.com/posts/ai/git-worktrees-for-parallel-ai-coding-agents)).

   The three locked worktrees need an operator decision before any of this.
6. **Subagents: keep two, and sharpen them.** Anthropic says subagents pay off for parallel independent work, for a reviewer that has not seen the code, and for least-privilege tools. They "cost tokens" and can be slower, and haiku suits mechanical checks ([best practices](https://code.claude.com/docs/en/best-practices)). `reviewer` and `boundaries` fit that well. `boundaries` is mechanical enough that it should become a `cargo tree`/grep script in `check.sh`, so it runs deterministically, with the agent kept only for judgment calls.
7. **Path-scoped rules can replace "read this when…" headers.** `.claude/rules/*.md` with `paths:` frontmatter loads only when matching files are touched ([memory docs](https://code.claude.com/docs/en/memory)). For example, `rules/web.md` would cover `web/**` with "the contract is `types.ts`" and rules for the decision path would cover `crates/libs/policy/**`. This is the right home for area rules that today sit in `AGENTS.md` sections 3 to 6. Claude-only, though: AGENTS.md stays the cross-tool source ([agents.md, Linux Foundation AAIF](https://codex.danielvaughan.com/2026/04/07/agents-md-open-standard-cross-tool-portability/)).
8. **Duplication.** The provenance rule is stated in four places: `CLAUDE.md`, `AGENTS.md` §0, `CONTRIBUTING.md` and `AI-PROVENANCE.md`. The `target/dev` rule appears in `CLAUDE.md`, in `AGENTS.md` twice, in `OPERATIONS.md` and in the commands. "How to review" lives in both `docs/agents/local-review.md` and the reviewer agent. `GUIDE.md` §11 (HTTP API reference) duplicates `SPEC-dashboard.md`.

### Proposed docs tree for v11

```
AGENTS.md                 ≤120 lines: loop, build/test/gate commands, layering, 2 invariants, branch rule, provenance one-liner
CLAUDE.md                 ≤15 lines: "@AGENTS.md" + Claude-only notes (hooks exist, skills list)
README.md                 product + quick start (trim; 474 lines today)
llms.txt                  keep (run-a-fleet path)
.claude/
  settings.json           + PreToolUse guards, Stop hook, deny rules, cleanupPeriodDays, worktree settings
  agents/reviewer.md      keep
  agents/boundaries.md    keep until scripted, then delete
  rules/web.md, rules/decision-path.md, rules/store.md, rules/learner.md   (new, path-scoped)
  skills/gate|golden|measure|new-file   (moved from commands/)
  skills/ship-patch|verify-install|restore-store|public-tv   keep
  skills/triage         (new: absorbs the HELL.md triage block)
docs/
  README.md               the map (keep, shorter)
  CONTRIBUTING.md         the standard; absorbs AI-PROVENANCE.md, RELEASE.md, docs/agents/local-review.md
  LESSONS.md              keep (the "why")
  GLOSSARY.md, adr/       keep (Pocock domain skills read these)
  ARCHITECTURE.md         keep
  OPERATIONS.md           runbook; absorbs HELL.md as its first section "When everything is broken"
  GUIDE.md                operator/reader guide; drop §11 (API ref, link the spec) and the duplicated quick start
  spec/                   move SPEC-*.md here: protocol, data, learner, dashboard, scoring
  spec/venue.md           merge SPEC-payouts + SPEC-pro + SPEC-competitions (all "what the venue offers, do we use it")
  agents/                 keep issue-tracker.md, triage-labels.md, domain.md (skill config)
  studies/                keep, or move to the archive branch (evidence, not docs)
  CHANGELOG.md, SECURITY.md, SUPPORT.md, CODE_OF_CONDUCT.md, THIRD-PARTY-NOTICES.md   keep (GitHub/legal conventions; generated)
```

**Delete or archive:**
- `docs/HELL.md`: merge it into `OPERATIONS.md` §0 and the new `triage` skill. Two triage entry points will drift.
- `docs/superpowers/` (2 plans, 2 specs). It is a past tool's working state that `docs-check.py` already exempts as "historical". The decisions it locked belong in an ADR (`adr/0003-trunk-on-main.md`), and the rest belongs in git history.
- `docs/AI-PROVENANCE.md`, folded into CONTRIBUTING.
- `.claude/commands/`, once its contents become skills.
- `.openclaude/` and `.scratch/`: they belong in `.gitignore`, not the tree. Check whether they are tracked.

Net result: 25 top-level docs drop to about 14, AGENTS and CLAUDE together go from 282 lines to about 135, and each rule has exactly one home with hooks enforcing the ones that cannot bend.

## Sources

- Claude Code best practices: https://code.claude.com/docs/en/best-practices
- Claude Code memory (CLAUDE.md, AGENTS.md precedence, `.claude/rules`): https://code.claude.com/docs/en/memory
- Claude Code skills (commands merged into skills): https://code.claude.com/docs/en/skills
- Claude Code worktrees and parallel sessions: https://code.claude.com/docs/en/common-workflows
- AGENTS.md under the Linux Foundation AAIF: https://codex.danielvaughan.com/2026/04/07/agents-md-open-standard-cross-tool-portability/
- cargo-worktree (build isolation in worktrees): https://users.rust-lang.org/t/announcements-cargo-worktree-fix-build-isolation-in-git-worktrees/139192
- Rust with parallel agents, sccache: https://iron-ham.github.io/claudio/tutorials/rust-development
- Worktree limits (ports, databases): https://developer.upsun.com/posts/ai/git-worktrees-for-parallel-ai-coding-agents
- Operator dashboard practices: https://www.influxdata.com/blog/5-dashboard-design-best-practices/ ; https://www.domo.com/learn/article/dashboard-design-examples-best-practices
- Chart accessibility: https://www.saltdesignsystem.com/salt/foundations/data-visualization/chart-accessibility ; https://smashingmagazine.com/2022/07/accessibility-first-approach-chart-visual-design/ ; https://www.boia.org/blog/web-data-visualizations-and-accessibility-5-tips
- DTCG design tokens 2025.10 and Resolver: https://www.designtokens.org/ ; https://www.w3.org/community/reports/design-tokens/CG-FINAL-resolver-20251028/
- Chart performance: https://apexcharts.com/blog/javascript-chart-library-benchmark/ ; https://chartts.pages.dev/blog/best-lightweight-chart-libraries
