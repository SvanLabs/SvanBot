// The dashboard's reading of a fleet document (V11-11): what the page shows for each bot, what
// needs an operator's attention, and how fresh the stream is. Pure functions only, so the page's
// behaviour is tested without a browser (`bun test`).

/** A status older than this is not evidence of anything: the bot beats every five seconds. */
export const STALE_MS = 15_000;

/** How long the stream may go quiet before it is late, then lost. */
const DELAYED_MS = 7_000;
const LOST_MS = 15_000;

/** Whether `run` is an Arena run the scoreboard can draw. */
function isArenaRun(run) {
  return (
    typeof run?.commit === "string" &&
    typeof run?.champion === "string" &&
    Number.isFinite(run.seed) &&
    Array.isArray(run.rows) &&
    run.rows.every(
      (row) =>
        typeof row?.opponent === "string" &&
        Number.isFinite(row.deals) &&
        Number.isFinite(row.mbb_per_100) &&
        Number.isFinite(row.ci95),
    )
  );
}

/** Whether `value` is a fleet document the page can show. */
export function isFleet(value) {
  return (
    typeof value === "object" &&
    value !== null &&
    Number.isFinite(value.server_ms) &&
    Array.isArray(value.bots) &&
    value.bots.every((bot) => typeof bot?.bot === "string" && Number.isFinite(bot.updated_ms)) &&
    Array.isArray(value.problems) &&
    // The scoreboard reads the Arena's runs straight through, so a bad one is refused here.
    Array.isArray(value.arena) &&
    value.arena.every(isArenaRun)
  );
}

/** How long ago something happened, in the coarsest unit that fits. */
export function duration(ms) {
  const seconds = Math.floor(Math.max(0, ms) / 1_000);
  if (seconds < 1) {
    return "less than a second";
  }
  if (seconds < 60) {
    return `${seconds}s`;
  }
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) {
    return `${minutes}m`;
  }
  const hours = Math.floor(minutes / 60);
  if (hours < 24) {
    return `${hours}h`;
  }
  return `${Math.floor(hours / 24)}d`;
}

/** Whether a bot's status is older than STALE_MS: a silent bot is unknown, not playing. */
export function isStale(status, nowMs) {
  return nowMs - status.updated_ms > STALE_MS;
}

/** What one bot is doing, and why: a tone (ok, warn, bad), a label and a detail. */
export function botView(status, nowMs) {
  if (isStale(status, nowMs)) {
    return {
      tone: "bad",
      label: "No recent status",
      detail: `no status for ${duration(nowMs - status.updated_ms)}`,
    };
  }
  if (!status.connected) {
    return {
      tone: "bad",
      label: "Offline",
      detail: status.last_error ?? "the venue connection is closed",
    };
  }
  const table = status.table;
  if (table === null) {
    return { tone: "warn", label: "Waiting", detail: "connected, no table" };
  }
  return {
    tone: "ok",
    label: "Playing",
    detail:
      table.street === null
        ? `${table.id} · between hands`
        : `${table.id} · ${table.street} · pot ${table.pot}`,
  };
}

/** Everything the strip should raise, one line per bot, the stream's own trouble first. */
export function fleetProblems(fleet, nowMs, streamError) {
  const problems = [];
  if (streamError !== null) {
    // The stream being down explains any staleness below, so it goes first.
    problems.push({ tone: "warn", text: streamError });
  }
  if (fleet.bots.length === 0) {
    problems.push({ tone: "warn", text: "No bot has reported yet" });
  }
  for (const status of fleet.bots) {
    // One line per bot: an incident outranks being offline, which outranks going quiet.
    if (status.run_incidents > 0) {
      const plural = status.run_incidents === 1 ? "" : "s";
      problems.push({
        tone: "bad",
        text: `${status.bot} raised ${status.run_incidents} fair-play incident${plural} this run`,
      });
    } else if (!status.connected) {
      problems.push({
        tone: "bad",
        text: `${status.bot} is offline: ${status.last_error ?? "the venue connection is closed"}`,
      });
    } else if (isStale(status, nowMs)) {
      problems.push({
        tone: "bad",
        text: `${status.bot} has not reported for ${duration(nowMs - status.updated_ms)}`,
      });
    }
  }
  for (const problem of fleet.problems) {
    problems.push({ tone: "bad", text: String(problem) });
  }
  return problems;
}

/** Whether the page is hearing from the dashboard at all. */
export function freshness(lastMessageMs, nowMs) {
  if (lastMessageMs === null) {
    return { tone: "warn", label: "connecting" };
  }
  const age = nowMs - lastMessageMs;
  if (age <= DELAYED_MS) {
    return { tone: "ok", label: "live" };
  }
  if (age <= LOST_MS) {
    return { tone: "warn", label: "delayed" };
  }
  return { tone: "bad", label: "lost" };
}

/** A bot's win rate in bb/100 over its counted hands, or null before the first. */
export function winRate(play) {
  if (!play || play.counted === 0) {
    return null;
  }
  // Winnings are in thousandths of a big blind: per hand, times 100 hands, over 1,000.
  return play.won_mbb / play.counted / 10;
}

/** A win rate as text, signed, to one decimal. */
export function rateText(rate) {
  return rate === null
    ? "no counted hands"
    : `${rate >= 0 ? "+" : "−"}${Math.abs(rate).toFixed(1)} bb/100`;
}

/** The running total, in big blinds, after each of the latest hands. */
export function running(recent) {
  let total = 0;
  return recent.map((mbb) => {
    total += mbb;
    return total / 1_000;
  });
}

/**
 * The points of a line chart of `values` in a box `width` by `height`, as an SVG `points`
 * string, with the height of the zero line. The line starts at zero, before the first hand.
 */
export function chart(values, width, height) {
  const all = [0, ...values];
  const low = Math.min(...all);
  const high = Math.max(...all);
  const span = high - low || 1;
  const y = (value) => Number((height - ((value - low) / span) * height).toFixed(1));
  const step = all.length > 1 ? width / (all.length - 1) : 0;
  return {
    points: all.map((value, at) => `${Number((at * step).toFixed(1))},${y(value)}`).join(" "),
    zero: y(0),
  };
}

/** The view a path names: the fleet, one bot, the release, or the Arena's scoreboard. */
export function route(path) {
  const bot = /^\/bot\/([A-Za-z0-9_-]{1,64})$/.exec(path);
  if (bot) {
    return { view: "bot", bot: bot[1] };
  }
  if (path === "/release") {
    return { view: "release" };
  }
  return { view: path === "/scoreboard" ? "scoreboard" : "fleet" };
}

/**
 * The latest Arena run in a fleet document, or null before the first: the scoreboard's data. The
 * fleet keeps its history newest first (`ARENA_HISTORY` of them), so the latest is the first.
 */
export function arenaRun(runs) {
  return Array.isArray(runs) && runs.length > 0 ? runs[0] : null;
}

/**
 * A run's rows as a ladder: the best win rate first, so a strong opponent and a weak one read at
 * a glance. The row's own order is the run's; the page ranks them.
 */
export function ladder(rows) {
  return [...rows].sort((a, b) => b.mbb_per_100 - a.mbb_per_100);
}

/** An Arena win rate as text: already per hundred hands, in thousandths of a big blind, signed. */
export function mbbText(mbb) {
  return `${mbb >= 0 ? "+" : "−"}${Math.abs(mbb).toFixed(1)} mbb/100`;
}

/** The span a scoreboard's whiskers are drawn across: every interval's ends, and zero. */
export function arenaSpan(rows) {
  // An interval's ends, not the ends of what is drawn: 12,345.6 ± 2,000.4 ends at 14,346.0, and a
  // whisker must be placed by that, or two rows with the same interval would not line up.
  const ends = rows.flatMap((row) => [row.mbb_per_100 - row.ci95, row.mbb_per_100 + row.ci95]);
  return { low: Math.min(0, ...ends), high: Math.max(0, ...ends) };
}

/**
 * A run's interval as a whisker in a box `width` wide: where the interval starts and ends, where
 * its middle is, and where zero falls. An interval running past the span is clamped to the box.
 */
export function whisker(mbb, ci95, span, width) {
  const spread = span.high - span.low;
  if (spread <= 0) {
    return { from: 0, to: 0, zero: 0, mean: 0 };
  }
  const x = (value) => Math.min(width, Math.max(0, ((value - span.low) / spread) * width));
  return {
    from: x(mbb - ci95),
    to: x(mbb + ci95),
    zero: x(0),
    mean: x(mbb),
  };
}
