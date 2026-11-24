// The control room's panels (V11-44), after v10's: the season race and a bot's standing, the
// opponent intel, the highlights, the activity log, the notes and the Arena's latest runs. Every
// name is text, never markup: an opponent chooses their own name.

import { highlights } from "./activity.js";
import { h } from "./dom.js";
import { band } from "./dossier.js";
import { chips } from "./felt.js";
import { arenaLeagues, byWinRate, duration, mbbText } from "./fleet.js";

/** A time of day, as the log shows it. */
export function clock(ms) {
  const at = new Date(ms);
  return [at.getHours(), at.getMinutes(), at.getSeconds()]
    .map((part) => String(part).padStart(2, "0"))
    .join(":");
}

/** Time left, to the hour: `3d 4h`, `5h 12m`, or `12m`. */
function left(seconds) {
  const s = Math.max(0, Math.floor(seconds));
  const [d, hr, m] = [
    Math.floor(s / 86_400),
    Math.floor((s % 86_400) / 3_600),
    Math.floor((s % 3_600) / 60),
  ];
  return d > 0 ? `${d}d ${hr}h` : hr > 0 ? `${hr}h ${m}m` : `${m}m`;
}

function signed(value, unit) {
  return `${value >= 0 ? "+" : "−"}${Math.abs(value).toFixed(1)} ${unit}`;
}

/** The season's line: its number, time left now, and how many are ranked. */
function seasonLine(season, nowMs) {
  const elapsed = (nowMs - (season.read_ms ?? nowMs)) / 1_000;
  const remaining = season.remaining_s === null ? null : season.remaining_s - elapsed;
  return h(
    "p",
    { class: "season-line" },
    h("strong", {}, `Season ${season.number}`),
    remaining === null ? null : h("span", {}, `${left(remaining)} left`),
    h("span", {}, `${chips(season.ranked)} ranked of ${chips(season.registered ?? season.ranked)}`),
    season.winding_down ? h("span", { class: "pill warn" }, "winding down") : null,
  );
}

/**
 * The season race: the top of the venue's leaderboard and every bot of ours wherever it ranks,
 * each score a bar against the leader's, ours marked, and a gap drawn where ranks are skipped.
 */
export function seasonRace(season, nowMs) {
  if (season === null || season === undefined) {
    return h("p", { class: "muted" }, "Reading the season from the venue…");
  }
  if (!Array.isArray(season.rows)) {
    return h("p", { class: "worst bad" }, `The season board could not be read: ${season.error}`);
  }
  const leader = Math.max(1, season.leader);
  const rows = [];
  season.rows.forEach((row, at) => {
    if (at > 0 && row.rank > season.rows[at - 1].rank + 1) {
      rows.push(h("li", { class: "season-gap", "aria-hidden": "true" }, "⋯"));
    }
    const share = Math.max(0, Math.min(100, (row.score / leader) * 100)).toFixed(1);
    rows.push(
      h(
        "li",
        { class: `season-row${row.ours ? " ours" : ""}` },
        h("span", { class: "season-rank" }, `#${row.rank}`),
        h("span", { class: "season-name" }, row.name),
        h(
          "span",
          { class: "season-track", "aria-hidden": "true" },
          h("i", { class: "season-bar", style: `--share:${share}%` }),
        ),
        h("span", { class: "season-score" }, chips(row.score)),
        h("span", { class: "season-hands" }, `${chips(row.hands)} h`),
      ),
    );
  });
  return [
    seasonLine(season, nowMs),
    season.error
      ? h(
          "p",
          { class: "muted" },
          `The last read failed (${season.error}); this is the board before it.`,
        )
      : null,
    h("ol", { class: "season" }, rows),
  ];
}

/** A bot's own place on the season board: its rank, score, and whom it trails by how much. */
export function standing(season, bot) {
  if (!Array.isArray(season?.rows)) {
    return seasonRace(season, Date.now());
  }
  const rows = season.rows;
  const mine = rows.find((row) => row.name === bot);
  if (!mine) {
    return h(
      "p",
      { class: "muted" },
      `${bot} is not on the season board: the official board counts bots with at least 10 hands.`,
    );
  }
  const above = rows.filter((row) => row.rank < mine.rank).at(-1);
  return [
    h(
      "div",
      { class: "standing" },
      h("strong", { class: "big" }, `#${mine.rank}`),
      h("span", { class: "muted" }, `of ${chips(season.ranked)} ranked · season ${season.number}`),
    ),
    h(
      "dl",
      {},
      h("dt", {}, "Score"),
      h("dd", {}, chips(mine.score)),
      h("dt", {}, "Hands"),
      h("dd", {}, `${chips(mine.hands)} played, ${chips(mine.won)} won`),
      h("dt", {}, "Rebuys"),
      h("dd", {}, String(mine.rebuys)),
      h("dt", {}, "Next up"),
      h(
        "dd",
        {},
        mine.rank === 1
          ? "leads the season"
          : above
            ? `${chips(above.score - mine.score)} behind ${above.name}`
            : `${chips(season.leader - mine.score)} behind the leader`,
      ),
    ),
  ];
}

const CLASSES = ["fixed", "switching", "learning", "unknown"];

/** The class as the intel says it: a switch names the hand the present strategy began at. */
function classText(row) {
  return row.class === "switching" && Number.isFinite(row.change)
    ? `switching since hand ${row.change}`
    : row.class;
}

/**
 * Opponent intel: one bot's reads (`docs/adr/0004-reads-are-per-bot.md`). The classes counted,
 * the players at its table now with what it reads of each, then every opponent by name, never
 * ranked.
 */
export function intel(status) {
  const opponents = Array.isArray(status.opponents) ? status.opponents : [];
  const byName = new Map(opponents.map((row) => [row.name, row]));
  const counts = CLASSES.map((name) => [
    name,
    opponents.filter((row) => row.class === name).length,
  ]);
  const seated = (status.table?.players ?? []).filter(
    (player) => player.seat !== status.table.seat,
  );
  return [
    h(
      "div",
      { class: "mix" },
      counts.map(([name, count]) =>
        h("span", { class: `class-tag ${name}` }, h("b", {}, String(count)), name),
      ),
      h(
        "span",
        { class: "muted" },
        status.reads_in_play ? "reads in play" : "reads held: kept and shown, deciding nothing",
      ),
    ),
    seated.length > 0
      ? h(
          "div",
          { class: "at-table" },
          h("h3", {}, "At the table now"),
          h(
            "ul",
            {},
            seated.map((player) => {
              const row = byName.get(player.name);
              return h(
                "li",
                {},
                h("span", { class: "intel-seat" }, player.name),
                row
                  ? h(
                      "span",
                      { class: `class-tag ${row.class}` },
                      `${classText(row)} · ${band(row.actions)}`,
                    )
                  : h("span", { class: "class-tag unknown" }, "no read"),
              );
            }),
          ),
        )
      : null,
    opponents.length === 0
      ? h("p", { class: "muted" }, "No reads yet: this bot has played no named opponent.")
      : [
          h("h3", {}, `${opponents.length} opponents, by name`),
          h(
            "ol",
            { class: "intel-list scroll", tabindex: 0 },
            [...opponents]
              .sort((a, b) => a.name.localeCompare(b.name))
              .map((row) =>
                h(
                  "li",
                  {},
                  h("span", { class: "intel-name" }, row.name),
                  h("span", { class: `class-tag ${row.class}` }, classText(row)),
                  h(
                    "small",
                    { class: "intel-facts" },
                    `${chips(row.hands)} hands · ${band(row.actions)} · `,
                    h("span", { class: "earned", style: `--share:${row.earned_pct}%` }),
                    `${row.earned_pct}% earned`,
                  ),
                ),
              ),
          ),
        ],
    h(
      "p",
      {},
      h("a", { href: `/bot/${encodeURIComponent(status.bot)}/dossier` }, "The full dossier"),
    ),
  ];
}

/** The highlights as tiles: the best and worst hand, the streak and the form. */
export function highlightsView(play) {
  const seen = highlights(play);
  if (seen === null) {
    return h("p", { class: "muted" }, "No counted hands yet.");
  }
  const tile = (label, value, tone) =>
    h(
      "div",
      { class: "mini" },
      h("span", { class: "label" }, label),
      h("strong", { class: tone }, value),
    );
  return h(
    "div",
    { class: "minis" },
    tile("Best hand", signed(seen.best, "bb"), "up"),
    tile("Worst hand", signed(seen.worst, "bb"), "down"),
    tile(
      "Streak",
      `${seen.streak.length} ${seen.streak.won ? "won" : "lost"}`,
      seen.streak.won ? "up" : "down",
    ),
    tile(`Form, ${seen.hands} hands`, signed(seen.form, "bb/100"), seen.form >= 0 ? "up" : "down"),
  );
}

/** The activity log, newest first: the fleet's, or one bot's own. */
export function activityView(log, bot) {
  const events = bot === null ? log : log.filter((event) => event.bot === bot);
  if (events.length === 0) {
    return h("p", { class: "muted" }, "Nothing has happened since this page opened.");
  }
  return h(
    "ol",
    { class: "log" },
    events
      .slice(0, 60)
      .map((event) =>
        h(
          "li",
          { class: event.tone },
          h("time", {}, clock(event.at)),
          bot === null ? h("b", {}, event.bot) : null,
          h("span", {}, event.text),
        ),
      ),
  );
}

/** The reader's own notes on `key` (a bot, or the fleet): kept in this browser only. */
export function notes(key, text) {
  return h(
    "div",
    { class: "notes" },
    h(
      "textarea",
      {
        "data-keep": key,
        "data-note": key,
        rows: 6,
        placeholder: "Reads, reminders, what to check next…",
        "aria-label": `Notes on ${key}`,
      },
      text,
    ),
    h("small", { class: "muted" }, "Saved in this browser as you type."),
  );
}

/**
 * A parameter set's numbers in words, "exploit 0.4", each to four places: 1/0.9 of a number reads
 * 0.5556, not to the last bit. The digest names the set exactly.
 */
function numbers(params) {
  return Object.entries(params?.numbers ?? {})
    .map(([name, value]) => `${name} ${Number(Number(value).toFixed(4))}`)
    .join(", ");
}

/** A step's tone: up or down only when its whole 95% interval is. */
function stepTone(step) {
  if (step.stage === "proposed") {
    return "proposed";
  }
  if (step.mbb_per_100 - step.ci95 > 0) {
    return "up";
  }
  return step.mbb_per_100 + step.ci95 < 0 ? "down" : "level";
}

/**
 * The unattended search (V11-49): what the learner last did and when, the champion it searches
 * round, what the canary has pending, and the newest steps first: a paired sim each, or a
 * challenger proposed.
 */
export function searchView(search, nowMs) {
  if (!search || (!search.last && (search.steps ?? []).length === 0)) {
    return [
      h("p", { class: "muted" }, "The search has not run yet."),
      h(
        "p",
        { class: "subtle" },
        "Start it with `svanbot-bot start learn` to run it unattended (deploy/README.md).",
      ),
    ];
  }
  const pending = search.probe
    ? ["warn", `Probe running · ${search.probe}`]
    : search.waiting
      ? ["warn", `Proposal waiting · ${search.waiting}`]
      : ["ok", "Canary free"];
  return [
    search.last
      ? h(
          "p",
          { class: "search-last" },
          h("strong", {}, `${duration(nowMs - search.last.at_ms)} ago`),
          " · ",
          search.last.note,
        )
      : null,
    h(
      "div",
      { class: "search-facts" },
      h("span", { class: "class-tag" }, `Champion · ${numbers(search.champion) || "unknown"}`),
      h("span", { class: "class-tag" }, `${search.simmed} sims round it`),
      h("span", { class: `pill ${pending[0]}` }, pending[1]),
    ),
    search.error ? h("p", { class: "bad" }, search.error) : null,
    h(
      "ul",
      { class: "search-steps" },
      (search.steps ?? []).map((step) =>
        h(
          "li",
          { class: `search-step ${stepTone(step)}` },
          h("span", { class: "search-what" }, numbers(step.challenger)),
          h("span", { class: "search-stage" }, step.stage),
          step.stage === "proposed"
            ? h("strong", {}, "to the canary")
            : h(
                "span",
                { class: "search-result" },
                `${mbbText(step.mbb_per_100)} ±${Math.round(step.ci95)}`,
              ),
        ),
      ),
    ),
  ];
}

/** The Arena's latest run a league, best row first, with the full boards a link away. */
export function arenaSummary(arena) {
  const boards = arenaLeagues(arena);
  if (boards.length === 0) {
    return h("p", { class: "muted" }, "The Arena has not played yet.");
  }
  return [
    boards.map((board) => {
      const run = board.runs[0];
      return h(
        "div",
        { class: "arena-league" },
        h("h3", {}, `${board.league} · ${String(run.commit).slice(0, 7)}`),
        h(
          "ul",
          {},
          byWinRate(run.rows).map((row) =>
            h(
              "li",
              {},
              h("span", {}, row.opponent),
              h("span", { class: row.mbb_per_100 >= 0 ? "up" : "down" }, mbbText(row.mbb_per_100)),
              h("small", { class: "muted" }, `±${Math.round(row.ci95)}`),
            ),
          ),
        ),
      );
    }),
    h("p", {}, h("a", { href: "/scoreboard" }, "The full scoreboard")),
  ];
}
