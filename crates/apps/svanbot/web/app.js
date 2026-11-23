// The dashboard's page (V11-11, V11-44): one event stream in, one view out. Everything it shows
// comes through `fleet.js`, and every name on it is set as text, never as markup: an opponent
// chooses their own name. The fleet and each bot are control rooms (`control.js`): boards of
// panels the reader arranges, folds and views a slice of at a time, remembered in this browser.

import { changes } from "./activity.js";
import { hide, move, reconcile, show, store } from "./board.js";
import {
  BOT_LAYOUT,
  BOT_VIEWS,
  botRoom,
  FLEET_LAYOUT,
  FLEET_VIEWS,
  fleetRoom,
  when,
} from "./control.js";
import { facts, h, panel, table } from "./dom.js";
import { dossierIndex, dossierPage } from "./dossier.js";
import {
  arenaLeagues,
  arenaRun,
  arenaSpan,
  byWinRate,
  fleetProblems,
  freshness,
  isFleet,
  mbbText,
  route,
  whisker,
} from "./fleet.js";
import { openPalette } from "./palette.js";
import { patch } from "./patch.js";
import { handPage } from "./replay.js";
import { botTabs } from "./room.js";

/** Each room's views and default arrangement. */
const ROOMS = {
  fleet: { views: FLEET_VIEWS, layout: FLEET_LAYOUT },
  bot: { views: BOT_VIEWS, layout: BOT_LAYOUT },
};

/** What the reader chose, remembered in this browser: a room's view, arrangement and folds. */
const reader = {
  arranging: false,
  theme: store.read("svanbot.theme", "arena"),
  view: (room) => store.read(`svanbot.view.${room}`, ROOMS[room].views[0].id),
  layout: (room) => reconcile(store.read(`svanbot.layout.${room}`, null), ROOMS[room].layout),
  collapsed: (room) => {
    const folded = store.read(`svanbot.collapsed.${room}`, []);
    return Array.isArray(folded) ? folded : [];
  },
  note: (key) => {
    const text = store.read(`svanbot.note.${key}`, "");
    return typeof text === "string" ? text : "";
  },
};

/** A room's reader state, as its board takes it. */
function uiOf(room) {
  return {
    view: reader.view(room),
    layout: reader.layout(room),
    collapsed: reader.collapsed(room),
    arranging: reader.arranging,
    theme: reader.theme,
    note: reader.note,
  };
}

/** The room of the page in view, if it is one. */
function roomOf(path) {
  const at = route(path);
  return at.view === "bot" ? "bot" : at.view === "fleet" ? "fleet" : null;
}

/**
 * Applies one of the reader's actions on the room in view, and redraws: a view chosen, a panel
 * moved, hidden, brought back or folded, arranging begun or done, the table's look, a reset.
 */
export function act(action, data = {}) {
  const room = roomOf(location.pathname);
  if (action === "palette") {
    openPalette({ fleet: state.fleet, room, go: navigate, act });
    return;
  }
  if (action === "theme") {
    reader.theme = data.theme;
    store.write("svanbot.theme", data.theme);
  } else if (action === "arrange") {
    reader.arranging = !reader.arranging;
  } else if (room !== null) {
    const layout = reader.layout(room);
    const folded = reader.collapsed(room);
    const keep = (key, value) => store.write(`svanbot.${key}.${room}`, value);
    const panels = [...layout.left, ...layout.center, ...layout.right];
    ({
      view: () => keep("view", data.view),
      move: () => keep("layout", move(layout, data.id, data.dir)),
      hide: () => keep("layout", hide(layout, data.id)),
      unhide: () => keep("layout", show(layout, data.id, ROOMS[room].layout)),
      fold: () =>
        keep(
          "collapsed",
          folded.includes(data.id) ? folded.filter((id) => id !== data.id) : [...folded, data.id],
        ),
      "fold-all": () => keep("collapsed", panels),
      "unfold-all": () => keep("collapsed", []),
      reset: () => {
        keep("layout", ROOMS[room].layout);
        keep("collapsed", []);
      },
    })[action]?.();
  }
  render();
}

/** A note is saved as it is typed; the page does not redraw for it. */
export function note(key, text) {
  store.write(`svanbot.note.${key}`, text);
}

/** Whether `path` is one of the dashboard's own pages, which the page can show without a load. */
function isPage(path) {
  return path === "/" || /^\/(release|scoreboard|dossier)$/.test(path) || path.startsWith("/bot/");
}

/**
 * Goes to `path` inside the page, with no reload: the event stream stays open, and where the
 * browser can, the old view crossfades into the new.
 */
export function navigate(path) {
  if (path !== location.pathname) {
    history.pushState(null, "", path);
  }
  globalThis.scrollTo?.({ top: 0, behavior: "instant" });
  if (document.startViewTransition) {
    document.startViewTransition(render);
  } else {
    render();
  }
}

/** The release page: what is installed, where the slots point, and what each bot runs. */
function releasePage(fleet, nowMs) {
  const release = fleet.release ?? { current: null, previous: null, installed: [] };
  return [
    h("h1", {}, "Release"),
    h(
      "div",
      { class: "grid" },
      panel(
        "Slots",
        facts([
          ["Current", release.current ?? "not set"],
          ["Previous", release.previous ?? "not set"],
          ["Installed", release.installed.length > 0 ? release.installed.join(", ") : "nothing"],
        ]),
      ),
      panel(
        "What each bot runs",
        table(
          "Builds and champions by bot",
          [
            ["Bot", null],
            ["Build", null],
            ["Started", null],
            ["Champion", null],
          ],
          fleet.bots.map((status) =>
            h(
              "tr",
              {},
              h("td", {}, h("a", { href: `/bot/${encodeURIComponent(status.bot)}` }, status.bot)),
              h("td", {}, status.version || "a checkout"),
              h("td", {}, when(status.started_ms, nowMs)),
              h("td", { class: "mono" }, status.play.champion.slice(0, 8) || "unknown"),
            ),
          ),
        ),
      ),
    ),
  ];
}

/**
 * The scoreboard: one board a league (V11-29), heads up first. The leagues measure different
 * things, so each has its own whiskers and its own trend, and no run is pooled across them.
 */
function scoreboardPage(fleet) {
  return [
    h("p", {}, h("a", { href: "/dossier" }, "Live opponents are in the dossiers")),
    scoreboardBody(fleet),
  ];
}

function scoreboardBody(fleet) {
  const boards = arenaLeagues(fleet.arena);
  if (boards.length === 0) {
    return [
      h("h1", {}, "Scoreboard"),
      h(
        "p",
        {},
        "The Arena has not played yet. Record a run with ",
        h("code", {}, "svanbot arena <bouts>"),
        ".",
      ),
    ];
  }
  return [h("h1", {}, "Scoreboard"), ...boards.flatMap((board) => leagueBoard(board))];
}

/** How a league's runs are played, for the board's caption. */
const PLAYED = {
  "heads-up":
    "by playing each deal from both seats, heads up against the pricer and the sparring archetypes",
  "3-handed":
    "by playing each deal from every seat of a table of three, against two of the pricer, v10.0.2 or one sparring archetype",
  "4-handed":
    "by playing each deal from every seat of a table of four, against three of the pricer, v10.0.2 or one sparring archetype",
  "5-handed":
    "by playing each deal from every seat of a table of five, against four of the pricer, v10.0.2 or one sparring archetype",
  "6-max":
    "by playing each deal from every seat of a full table, against five of the pricer or of one sparring archetype",
};

/** One league's board: its latest run, a whisker an opponent, and each opponent's trend. */
function leagueBoard({ league, runs }) {
  const latest = arenaRun(runs);
  const span = arenaSpan(latest.rows);
  // A trend is across commits, so its columns are every opponent any run played: taking them
  // from the newest run alone would drop an opponent it skipped from every row.
  const opponents = [...new Set(runs.flatMap((run) => run.rows.map((row) => row.opponent)))].sort(
    (a, b) => a.localeCompare(b),
  );
  const [width, height] = [300, 16];
  return [
    h(
      "h2",
      {},
      league === "heads-up" ? "Heads up " : `${league} `,
      h(
        "span",
        { class: "muted mono" },
        latest.commit.slice(0, 7),
        " · champion ",
        latest.champion.slice(0, 8),
        " · seed ",
        latest.seed,
      ),
    ),
    h(
      "div",
      { class: "grid" },
      panel(
        "Latest run",
        h(
          "p",
          { class: "muted" },
          `Milli-big-blinds a hundred deals, luck of the cards taken out ${PLAYED[league]}. ` +
            `Each whisker is the run's interval, its dark mark the mean, and the dashed line ` +
            `zero. The box spans ${mbbText(span.low)} to ${mbbText(span.high)}.`,
        ),
        table(
          "Each opponent, its whisker, its mean and its interval",
          [
            ["Opponent", null],
            ["Interval", null],
            ["Mean", "number"],
            ["Deals", "number"],
          ],
          byWinRate(latest.rows).map((row) => {
            const bar = whisker(row.mbb_per_100, row.ci95, span, width);
            const label = `${row.opponent}: ${mbbText(row.mbb_per_100)} over ${row.deals} deals, between ${mbbText(row.mbb_per_100 - row.ci95)} and ${mbbText(row.mbb_per_100 + row.ci95)}`;
            return h(
              "tr",
              {},
              h("td", {}, row.opponent),
              h(
                "td",
                {},
                h(
                  "svg",
                  {
                    class: "whisker",
                    viewBox: `0 0 ${width} ${height}`,
                    preserveAspectRatio: "none",
                    role: "img",
                    "aria-label": label,
                  },
                  h("title", {}, label),
                  h("line", { x1: bar.zero, x2: bar.zero, y1: 0, y2: height }),
                  h("line", {
                    x1: bar.from,
                    x2: bar.to,
                    y1: height / 2,
                    y2: height / 2,
                    class: "bar",
                  }),
                  h("line", {
                    x1: bar.mean,
                    x2: bar.mean,
                    y1: 0,
                    y2: height,
                    class: "mean",
                  }),
                ),
              ),
              h("td", { class: "number" }, mbbText(row.mbb_per_100)),
              h("td", { class: "number" }, String(row.deals)),
            );
          }),
        ),
      ),
      panel(
        "Trend",
        h("p", { class: "muted" }, "Each opponent's mean over the recorded runs, newest first."),
        h(
          "div",
          { class: "scroll", tabindex: 0 },
          table(
            "Each run's mean against each opponent",
            [["Commit", null], ...opponents.map((opponent) => [opponent, "number"])],
            runs.map((run) =>
              h(
                "tr",
                {},
                // A promotion moves play under one commit, so the commit alone does not name a
                // run: the champion is what makes two runs of one commit two labels.
                h(
                  "td",
                  { class: "mono" },
                  `${run.commit.slice(0, 7)} · ${run.champion.slice(0, 8)}`,
                ),
                opponents.map((opponent) => {
                  const found = run.rows.find((row) => row.opponent === opponent);
                  return h("td", { class: "number" }, found ? mbbText(found.mbb_per_100) : "—");
                }),
              ),
            ),
          ),
        ),
      ),
    ),
  ];
}

const state = {
  fleet: null,
  offset: 0,
  lastMessageMs: null,
  streamError: null,
  // The views the stream does not carry: one bot's hands, and one hand's trace. Fetched when the
  // path asks for them, and refetched when it changes.
  hands: null,
  handsFor: null,
  trace: null,
  traceFor: null,
  // What changed while the page was open, newest first: the activity log.
  log: [],
};

/** The most events the log keeps. */
const LOG_KEPT = 300;

/** The server's sentence for a refusal, or `fallback` when it sent none: a detail is never markup. */
export function textOf(body, fallback) {
  return typeof body?.detail === "string" ? body.detail : fallback;
}

/** Asks for what the path needs, once per path: the stream carries the fleet, not the traces. */
function load(at) {
  if (at.view === "bot" && state.handsFor !== at.bot) {
    state.handsFor = at.bot;
    state.hands = null;
    fetch(`/api/hands/${at.bot}`)
      .then((answer) =>
        // The server says why a 500 happened; a 404 is a name no bot has. Read the detail, and
        // fall back to the plain sentence if the body is not the JSON it should be.
        answer.ok
          ? answer.json()
          : answer.json().then(
              (body) =>
                Promise.reject(new Error(textOf(body, "The bot's store could not be read."))),
              () => Promise.reject(new Error("The bot's store could not be read.")),
            ),
      )
      .then((body) => {
        state.hands = body.hands ?? [];
        render();
      })
      .catch((error) => {
        state.hands = { error: error.message };
        render();
      });
  }
  const key = at.view === "hand" ? `${at.bot}/${at.hand}` : null;
  if (key !== null && state.traceFor !== key) {
    state.traceFor = key;
    state.trace = null;
    fetch(`/api/trace/${at.bot}/${at.hand}`)
      .then((answer) =>
        // A 500 carries why the store could not be read; a 404 is a hand nobody played. The
        // page must not show a broken store as a hand that does not exist.
        answer.ok
          ? answer.json()
          : answer.json().then(
              (body) =>
                Promise.reject(new Error(textOf(body, "This hand is not in the bot's store."))),
              () => Promise.reject(new Error("This hand is not in the bot's store.")),
            ),
      )
      .then((body) => {
        state.trace = body;
        render();
      })
      .catch((error) => {
        state.trace = { error: error.message };
        render();
      });
  }
}

function render() {
  const local = Date.now();
  const fresh = freshness(state.lastMessageMs, local);
  const pill = document.getElementById("fresh");
  pill.className = `pill ${fresh.tone}`;
  pill.textContent = fresh.label;
  if (state.fleet === null) {
    return;
  }
  // The server's clock ages the bots, so a page on a machine with a wrong clock still reads true.
  const nowMs = local + state.offset;
  const problems = fleetProblems(state.fleet, nowMs, state.streamError);
  const count = state.fleet.bots.length;
  document.getElementById("count").textContent = `${count} bot${count === 1 ? "" : "s"}`;
  // The fleet room lists every problem itself, so there the strip's line steps aside.
  const worst = document.getElementById("worst");
  const listed = roomOf(location.pathname) === "fleet" ? " listed" : "";
  worst.className = `worst ${problems[0]?.tone ?? "ok"}${listed}`;
  worst.textContent =
    problems.length === 0
      ? "Nothing is wrong."
      : problems.length === 1
        ? problems[0].text
        : `${problems[0].text} (and ${problems.length - 1} more)`;
  const at = route(location.pathname);
  load(at);
  // A tab a bot in the top bar, the bot in view lit; redrawn only when it changes.
  const holder = document.getElementById("bots");
  if (holder) {
    const tabs = h("nav", {}, botTabs(state.fleet, at.bot, nowMs));
    if (holder.innerHTML !== tabs.innerHTML) {
      holder.replaceChildren(...tabs.childNodes);
    }
  }
  // The page in view is marked in the nav, so the strip says where the reader is.
  const section = `/${location.pathname.split("/")[1] ?? ""}`;
  for (const link of document.querySelectorAll?.(".strip nav.pages a") ?? []) {
    const here = section !== "/bot" && link.getAttribute("href") === section;
    if (here) link.setAttribute("aria-current", "page");
    else link.removeAttribute("aria-current");
  }
  const room = (name) => ({
    fleet: state.fleet,
    nowMs,
    problems,
    log: state.log,
    hands: state.hands,
    ui: uiOf(name),
  });
  const pages = {
    hand: () => handPage(state, at.bot, at.hand),
    bot: () => botRoom(room("bot"), at.bot),
    dossier: () =>
      dossierPage(
        state.fleet.bots.find((bot) => bot.bot === at.bot),
        at.bot,
      ),
    dossiers: () => dossierIndex(state.fleet),
    release: () => releasePage(state.fleet, nowMs),
    scoreboard: () => scoreboardPage(state.fleet),
  };
  const view = (pages[at.view] ?? (() => fleetRoom(room("fleet"))))();
  // Redraw only what changed (`patch.js`): a redraw every second must not take the focus, the
  // scroll, an open fallback or a note being typed from the reader.
  patch(document.getElementById("view"), h("main", {}, view));
}

function take(text) {
  const message = JSON.parse(text);
  state.lastMessageMs = Date.now();
  if (Number.isFinite(message.server_ms)) {
    state.offset = message.server_ms - state.lastMessageMs;
  }
  if (isFleet(message)) {
    state.log = [...changes(state.fleet, message), ...state.log].slice(0, LOG_KEPT);
    state.fleet = message;
  }
  state.streamError = null;
  render();
}

// The reader's actions, by delegation: a link to one of the dashboard's pages, a click on anything
// carrying `data-act`, a note typed, the palette's keys, and the browser's back and forward.
document.addEventListener?.("click", (event) => {
  const link = event.target.closest?.("a[href]");
  const plain =
    event.button === 0 && !event.metaKey && !event.ctrlKey && !event.shiftKey && !event.altKey;
  if (link && plain && link.origin === location.origin && !link.target && isPage(link.pathname)) {
    event.preventDefault();
    navigate(link.pathname);
    return;
  }
  const target = event.target.closest?.("[data-act]");
  if (target) {
    act(target.dataset.act, { ...target.dataset });
  }
});
document.addEventListener?.("keydown", (event) => {
  const typing = /^(input|textarea|select)$/i.test(event.target.tagName ?? "");
  if ((event.key === "k" && (event.metaKey || event.ctrlKey)) || (event.key === "/" && !typing)) {
    event.preventDefault();
    act("palette");
  }
});
globalThis.addEventListener?.("popstate", () => render());
document.addEventListener?.("input", (event) => {
  const key = event.target.dataset?.note;
  if (key) {
    note(key, event.target.value);
  }
});

const stream = new EventSource("/api/events");
stream.addEventListener("fleet", (event) => take(event.data));
stream.addEventListener("tick", (event) => take(event.data));
stream.addEventListener("error", () => {
  state.streamError = "The event stream dropped; showing the last fleet received";
  render();
});
setInterval(render, 1_000);
