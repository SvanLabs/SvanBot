// The dashboard's reading of a fleet document (V11-11): what the page shows for each bot, what
// needs an operator's attention, and how fresh the stream is. Pure functions only, so the page's
// behaviour is tested without a browser.

/** A status older than this is not evidence of anything: the bot beats every five seconds. */
const STALE_MS = 15_000;

/** How long the stream may go quiet before it is late, then lost. */
const DELAYED_MS = 7_000;
const LOST_MS = 15_000;

export type Tone = "ok" | "warn" | "bad";

export interface SeatStatus {
  seat: number;
  name: string;
  stack: number;
  folded: boolean;
}

export interface TableStatus {
  id: string;
  seat: number | null;
  hand_id: string | null;
  street: string | null;
  board: string[];
  pot: number;
  big_blind: number;
  dealer: number;
  players: SeatStatus[];
}

/** One bot's last known state, as `svanbot dashboard` serves it. */
export interface BotStatus {
  bot: string;
  pid: number;
  version: string;
  started_ms: number;
  updated_ms: number;
  connected: boolean;
  table: TableStatus | null;
  hands: number;
  incidents: number;
  run_incidents: number;
  last_hand_ms: number | null;
  last_error: string | null;
}

/** Every bot's status, plus a line for each status file that could not be read. */
export interface Fleet {
  server_ms: number;
  bots: BotStatus[];
  problems: string[];
}

/** One line of the status strip. */
export interface View {
  tone: Tone;
  label: string;
  detail: string;
}

/** One thing worth an operator's attention. */
export interface Problem {
  tone: Tone;
  text: string;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isNumber(value: unknown): value is number {
  return typeof value === "number" && Number.isFinite(value);
}

function isString(value: unknown): value is string {
  return typeof value === "string";
}

function isSeat(value: unknown): value is SeatStatus {
  return (
    isRecord(value) &&
    isNumber(value.seat) &&
    isString(value.name) &&
    isNumber(value.stack) &&
    typeof value.folded === "boolean"
  );
}

function isTable(value: unknown): value is TableStatus {
  return (
    isRecord(value) &&
    isString(value.id) &&
    (value.seat === null || isNumber(value.seat)) &&
    (value.hand_id === null || isString(value.hand_id)) &&
    (value.street === null || isString(value.street)) &&
    Array.isArray(value.board) &&
    value.board.every(isString) &&
    isNumber(value.pot) &&
    isNumber(value.big_blind) &&
    isNumber(value.dealer) &&
    Array.isArray(value.players) &&
    value.players.every(isSeat)
  );
}

function isBot(value: unknown): value is BotStatus {
  return (
    isRecord(value) &&
    isString(value.bot) &&
    isNumber(value.pid) &&
    isString(value.version) &&
    isNumber(value.started_ms) &&
    isNumber(value.updated_ms) &&
    typeof value.connected === "boolean" &&
    (value.table === null || isTable(value.table)) &&
    isNumber(value.hands) &&
    isNumber(value.incidents) &&
    isNumber(value.run_incidents) &&
    (value.last_hand_ms === null || isNumber(value.last_hand_ms)) &&
    (value.last_error === null || isString(value.last_error))
  );
}

/** Reads a fleet document, refusing anything the page cannot trust. */
export function parseFleet(value: unknown): Fleet | null {
  if (
    !isRecord(value) ||
    !isNumber(value.server_ms) ||
    !Array.isArray(value.bots) ||
    !value.bots.every(isBot) ||
    !Array.isArray(value.problems) ||
    !value.problems.every(isString)
  ) {
    return null;
  }
  return {
    server_ms: value.server_ms,
    bots: value.bots,
    problems: value.problems,
  };
}

/** How long ago something happened, in the coarsest unit that fits. */
export function duration(ms: number): string {
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

/** Whether a bot's status is older than [`STALE_MS`]; a silent bot is unknown, not playing. */
export function isStale(status: BotStatus, nowMs: number): boolean {
  return nowMs - status.updated_ms > STALE_MS;
}

/** What one bot is doing, and why. Staleness wins: a silent bot is unknown, not playing. */
export function botView(status: BotStatus, nowMs: number): View {
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
    return {
      tone: "warn",
      label: "Waiting",
      detail: "connected, no table",
    };
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

function incidents(count: number): string {
  return `${count} fair-play incident${count === 1 ? "" : "s"}`;
}

/** Everything the strip should raise, one line per bot, worst first. */
export function fleetProblems(fleet: Fleet, nowMs: number, streamError: string | null): Problem[] {
  const problems: Problem[] = [];
  if (streamError !== null) {
    // The stream being down explains any staleness below, so it goes first.
    problems.push({ tone: "warn", text: streamError });
  }
  if (fleet.bots.length === 0) {
    problems.push({ tone: "warn", text: "No bot has reported yet" });
  }
  for (const status of fleet.bots) {
    // One line per bot: an incident outranks being offline, which outranks going quiet. A run
    // incident is raised now; the lifetime count belongs to the card, not the strip.
    if (status.run_incidents > 0) {
      problems.push({
        tone: "bad",
        text: `${status.bot} raised ${incidents(status.run_incidents)} this run`,
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
    problems.push({ tone: "bad", text: problem });
  }
  return problems;
}

/** Whether the page is hearing from the dashboard at all. */
export function freshness(
  lastMessageMs: number | null,
  nowMs: number,
): { tone: Tone; label: string } {
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
