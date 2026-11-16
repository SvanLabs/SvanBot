// The dashboard's reading of a fleet document (V11-11): what the page shows for each bot, what
// needs an operator's attention, and how fresh the stream is. Pure functions only, so the page's
// behaviour is tested without a browser (`bun test`).

/** A status older than this is not evidence of anything: the bot beats every five seconds. */
export const STALE_MS = 15_000;

/** How long the stream may go quiet before it is late, then lost. */
const DELAYED_MS = 7_000;
const LOST_MS = 15_000;

/** Whether `value` is a fleet document the page can show. */
export function isFleet(value) {
  return (
    typeof value === "object" &&
    value !== null &&
    Number.isFinite(value.server_ms) &&
    Array.isArray(value.bots) &&
    value.bots.every((bot) => typeof bot?.bot === "string" && Number.isFinite(bot.updated_ms)) &&
    Array.isArray(value.problems)
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
    if (status.incidents > 0) {
      const plural = status.incidents === 1 ? "" : "s";
      problems.push({
        tone: "bad",
        text: `${status.bot} raised ${status.incidents} fair-play incident${plural}`,
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
  return rate === null ? "no counted hands" : `${rate >= 0 ? "+" : "−"}${Math.abs(rate).toFixed(1)} bb/100`;
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

/** The view a path names: the fleet, one bot, or the release. */
export function route(path) {
  const bot = /^\/bot\/([A-Za-z0-9_-]{1,64})$/.exec(path);
  if (bot) {
    return { view: "bot", bot: bot[1] };
  }
  return { view: path === "/release" ? "release" : "fleet" };
}
